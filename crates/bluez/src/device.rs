//! This computer as a Bluetooth keyboard and mouse, through BlueZ's D-Bus API.
//!
//! bluetoothd publishes the service record and listens on the two HID channels (it can, as
//! root; this process needs no privileges) and hands each accepted channel over through
//! `org.bluez.Profile1.NewConnection`. Only one host is served: the one paired first. To
//! reconnect, for example after the host slept, this side opens the channels itself.
use crate::{hidp, record, socket};
use anyhow::{Context, Result, bail};
use desklink_hid::{
    Report, ReportSink,
    queue::{ReportQueue, Taken},
};
use std::{
    collections::HashMap,
    os::fd::OwnedFd,
    sync::{
        Arc, Condvar, Mutex, MutexGuard,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};
use zbus::{
    Connection, interface,
    zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, Value},
};

/// bluetoothd hands the interrupt channel to a second profile: one profile, one PSM.
const INTERRUPT_UUID: &str = "6e2c0f6a-0b3e-4f0b-9a54-4465736b6c6b";
const CONTROL_PATH: &str = "/org/desklink/hid/control";
const INTERRUPT_PATH: &str = "/org/desklink/hid/interrupt";
const AGENT_PATH: &str = "/org/desklink/hid/agent";
const RECONNECT_MIN: Duration = Duration::from_secs(2);
const RECONNECT_MAX: Duration = Duration::from_secs(30);
const BLUEZ: &str = "org.bluez";
/// A freshly started bluetoothd may take a moment before it answers.
const REGISTER_ATTEMPTS: u32 = 5;
const REGISTER_RETRY: Duration = Duration::from_millis(500);

/// Called with the host's address when one pairs, and with `None` when it removes this device.
pub type HostChanged = Box<dyn Fn(Option<&str>) + Send + Sync>;

pub struct Options {
    /// The name hosts show for this keyboard and mouse.
    pub name: String,
    /// The host paired earlier, if any.
    pub host: Option<String>,
    pub host_changed: HostChanged,
}

#[derive(Default)]
struct Channels {
    control: Option<Arc<OwnedFd>>,
    interrupt: Option<Arc<OwnedFd>>,
    /// Bumped on every disconnect, so threads of an old connection cannot end a new one.
    generation: u64,
}

impl Channels {
    fn connected(&self) -> bool {
        self.control.is_some() && self.interrupt.is_some()
    }
}

struct Shared {
    queue: ReportQueue,
    channels: Mutex<Channels>,
    host: Mutex<Option<String>>,
    pairing: AtomicBool,
    /// Wakes the reconnect thread early: input is waiting for a host that is not connected.
    wanted: (Mutex<bool>, Condvar),
    stopping: AtomicBool,
    host_changed: HostChanged,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl Shared {
    fn host(&self) -> Option<String> {
        lock(&self.host).clone()
    }

    fn connected(&self) -> bool {
        lock(&self.channels).connected()
    }

    /// Accepts the host, or during pairing the first computer to connect.
    fn admit(&self, address: &str) -> bool {
        let mut host = lock(&self.host);
        match host.as_deref() {
            Some(known) => known.eq_ignore_ascii_case(address),
            None if self.pairing.load(Ordering::SeqCst) => {
                *host = Some(address.to_string());
                drop(host);
                (self.host_changed)(Some(address));
                tracing::info!(%address, "paired a Bluetooth host");
                true
            }
            None => false,
        }
    }

    fn attach(self: &Arc<Self>, psm: u16, fd: OwnedFd) {
        let fd = Arc::new(fd);
        let mut channels = lock(&self.channels);
        let slot = if psm == record::PSM_CONTROL {
            &mut channels.control
        } else {
            socket::limit_send_buffer(&fd);
            &mut channels.interrupt
        };
        if let Some(old) = slot.replace(fd.clone()) {
            socket::shutdown(&old);
        }
        let generation = channels.generation;
        let connected = channels.connected();
        drop(channels);
        if connected {
            self.queue.clear();
            tracing::info!("Bluetooth host connected");
            if let Some(host) = self.host() {
                thread::Builder::new()
                    .name("desklink-hid-qos".into())
                    .spawn(move || crate::qos::improve_latency(&host))
                    .ok();
            }
        }
        let shared = self.clone();
        thread::Builder::new()
            .name(format!("desklink-hid-{psm}"))
            .spawn(move || shared.read(psm, fd, generation))
            .ok();
    }

    /// Serves the host's requests and notices when the channel goes away.
    fn read(&self, psm: u16, fd: Arc<OwnedFd>, generation: u64) {
        let mut buffer = [0u8; 256];
        loop {
            let message = match socket::recv(&fd, &mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(n) => &buffer[..n],
            };
            if psm == record::PSM_INTERRUPT {
                if !hidp::is_output(message) {
                    tracing::debug!(len = message.len(), "unexpected interrupt message");
                }
                continue;
            }
            match hidp::control(message) {
                hidp::Control::Reply(reply) => {
                    if socket::send(&fd, &reply).is_err() {
                        break;
                    }
                }
                hidp::Control::Unplug => {
                    tracing::info!("the host removed this keyboard and mouse");
                    lock(&self.host).take();
                    (self.host_changed)(None);
                    break;
                }
                hidp::Control::Ignore => {}
            }
        }
        self.disconnect(generation);
    }

    fn disconnect(&self, generation: u64) {
        let mut channels = lock(&self.channels);
        if channels.generation != generation {
            return;
        }
        let was_connected = channels.connected();
        for fd in [channels.control.take(), channels.interrupt.take()]
            .into_iter()
            .flatten()
        {
            socket::shutdown(&fd);
        }
        channels.generation += 1;
        drop(channels);
        self.queue.clear();
        if was_connected {
            tracing::info!("Bluetooth host disconnected");
        }
    }

    fn write(&self) {
        while !self.stopping.load(Ordering::SeqCst) {
            let reports = match self.queue.take(Duration::from_millis(500)) {
                Taken::Reports(reports) => reports,
                Taken::Timeout => continue,
                Taken::Closed => return,
            };
            let (fd, generation) = {
                let channels = lock(&self.channels);
                (channels.interrupt.clone(), channels.generation)
            };
            let Some(fd) = fd else { continue };
            for report in &reports {
                if let Err(error) = socket::send(&fd, &hidp::input(report)) {
                    tracing::debug!(%error, "cannot send a report");
                    self.disconnect(generation);
                    break;
                }
            }
        }
    }

    /// Opens the channels to the known host whenever they are down, backing off while it is
    /// unreachable and retrying at once when input is waiting for it.
    fn reconnect(self: &Arc<Self>) {
        let mut delay = RECONNECT_MIN;
        while !self.stopping.load(Ordering::SeqCst) {
            {
                let (wanted, wake) = &self.wanted;
                let guard = lock(wanted);
                let (mut guard, _) = wake
                    .wait_timeout_while(guard, delay, |w| !*w)
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                if *guard {
                    delay = RECONNECT_MIN;
                }
                *guard = false;
            }
            if self.stopping.load(Ordering::SeqCst) || self.connected() {
                delay = RECONNECT_MIN;
                continue;
            }
            let Some(host) = self.host() else { continue };
            let started = Instant::now();
            match self.open(&host) {
                Ok(()) => delay = RECONNECT_MIN,
                Err(error) => {
                    tracing::debug!(error = %format!("{error:#}"), "host not reachable");
                    delay = (delay * 2).min(RECONNECT_MAX);
                }
            }
            tracing::trace!(elapsed = ?started.elapsed(), "reconnect attempt");
        }
    }

    fn open(self: &Arc<Self>, host: &str) -> Result<()> {
        let control = socket::connect(host, record::PSM_CONTROL)?;
        let interrupt = socket::connect(host, record::PSM_INTERRUPT)?;
        self.attach(record::PSM_CONTROL, control);
        self.attach(record::PSM_INTERRUPT, interrupt);
        Ok(())
    }
}

struct Profile {
    psm: u16,
    shared: Arc<Shared>,
}

#[interface(name = "org.bluez.Profile1")]
impl Profile {
    fn release(&self) {}

    fn new_connection(
        &self,
        device: ObjectPath<'_>,
        fd: zbus::zvariant::OwnedFd,
        _properties: HashMap<String, OwnedValue>,
    ) -> zbus::fdo::Result<()> {
        let fd: OwnedFd = fd.into();
        let address = socket::address_of_device_path(device.as_str()).unwrap_or_default();
        if !self.shared.admit(&address) {
            tracing::warn!(%address, "refused a Bluetooth connection from an unknown computer");
            return Err(zbus::fdo::Error::AccessDenied(
                "not this keyboard's host".into(),
            ));
        }
        self.shared.attach(self.psm, fd);
        Ok(())
    }

    fn request_disconnection(&self, _device: ObjectPath<'_>) {
        let generation = lock(&self.shared.channels).generation;
        self.shared.disconnect(generation);
    }
}

#[derive(Debug, zbus::DBusError)]
#[zbus(prefix = "org.bluez.Error")]
enum AgentError {
    #[zbus(error)]
    ZBus(zbus::Error),
    Rejected(String),
}

/// Accepts "just works" pairing, and only while the pairing window is open.
struct Agent {
    shared: Arc<Shared>,
}

impl Agent {
    fn allow(&self, what: &str) -> Result<(), AgentError> {
        if self.shared.pairing.load(Ordering::SeqCst) {
            Ok(())
        } else {
            Err(AgentError::Rejected(format!(
                "{what} outside the pairing window"
            )))
        }
    }
}

#[interface(name = "org.bluez.Agent1")]
impl Agent {
    fn release(&self) {}
    fn request_pin_code(&self, _device: ObjectPath<'_>) -> Result<String, AgentError> {
        Err(AgentError::Rejected("no PIN entry".into()))
    }
    fn display_pin_code(&self, _device: ObjectPath<'_>, _pincode: String) {}
    fn request_passkey(&self, _device: ObjectPath<'_>) -> Result<u32, AgentError> {
        Err(AgentError::Rejected("no passkey entry".into()))
    }
    fn display_passkey(&self, _device: ObjectPath<'_>, _passkey: u32, _entered: u16) {}
    fn request_confirmation(
        &self,
        _device: ObjectPath<'_>,
        _passkey: u32,
    ) -> Result<(), AgentError> {
        self.allow("pairing")
    }
    fn request_authorization(&self, _device: ObjectPath<'_>) -> Result<(), AgentError> {
        self.allow("pairing")
    }
    fn authorize_service(&self, _device: ObjectPath<'_>, uuid: String) -> Result<(), AgentError> {
        // A host may also try audio or other services this computer offers: not through us.
        if uuid.eq_ignore_ascii_case(record::HID_UUID) || uuid.eq_ignore_ascii_case(INTERRUPT_UUID)
        {
            self.allow("the keyboard service")
        } else {
            Err(AgentError::Rejected(format!(
                "{uuid} is not offered while pairing"
            )))
        }
    }
    fn cancel(&self) {}
}

/// Running registration with bluetoothd. Dropping it stops serving the host.
pub struct HidDevice {
    shared: Arc<Shared>,
    connection: Connection,
    adapter: OwnedObjectPath,
    /// Registers the keyboard again whenever bluetoothd restarts.
    watcher: tokio::task::JoinHandle<()>,
}

/// Publishes the keyboard service and claims its two channels. bluetoothd forgets both when
/// it stops, so this runs again after every restart.
async fn register_profiles(connection: &Connection, name: &str) -> Result<()> {
    let profiles =
        zbus::Proxy::new(connection, BLUEZ, "/org/bluez", "org.bluez.ProfileManager1").await?;
    let common = |psm: u16| {
        let mut options: HashMap<&str, Value> = HashMap::new();
        options.insert("Role", "server".into());
        options.insert("PSM", Value::U16(psm));
        options.insert("RequireAuthentication", true.into());
        options.insert("RequireAuthorization", false.into());
        options.insert("AutoConnect", false.into());
        options
    };
    let mut control = common(record::PSM_CONTROL);
    control.insert("Name", name.into());
    control.insert("ServiceRecord", record::service_record(name).into());
    profiles
        .call_method(
            "RegisterProfile",
            &(
                ObjectPath::try_from(CONTROL_PATH)?,
                record::HID_UUID,
                control,
            ),
        )
        .await
        .map_err(bluez_error)?;
    profiles
        .call_method(
            "RegisterProfile",
            &(
                ObjectPath::try_from(INTERRUPT_PATH)?,
                INTERRUPT_UUID,
                common(record::PSM_INTERRUPT),
            ),
        )
        .await
        .map_err(bluez_error)?;
    Ok(())
}

/// Registers again each time bluetoothd comes back, for example after `systemctl restart
/// bluetooth`. The channels themselves drop with the old daemon; the reconnect thread
/// reopens them.
async fn follow_restarts(
    connection: Connection,
    name: String,
    mut restarts: zbus::fdo::NameOwnerChangedStream,
) {
    use futures_util::StreamExt;
    while let Some(signal) = restarts.next().await {
        let Ok(args) = signal.args() else { continue };
        if args.new_owner().is_none() {
            tracing::warn!("bluetoothd stopped; waiting for it to come back");
            continue;
        }
        let mut attempt = 1;
        loop {
            match register_profiles(&connection, &name).await {
                Ok(()) => {
                    tracing::info!("bluetoothd restarted; the keyboard is registered again");
                    break;
                }
                Err(error) if attempt < REGISTER_ATTEMPTS => {
                    tracing::debug!(error = %format!("{error:#}"), attempt, "registering again");
                    attempt += 1;
                    tokio::time::sleep(REGISTER_RETRY).await;
                }
                Err(error) => {
                    tracing::error!(
                        error = %format!("{error:#}"),
                        "cannot register the keyboard with the restarted bluetoothd; \
                         restart `desklink bluetooth run`"
                    );
                    break;
                }
            }
        }
    }
}

async fn find_adapter(connection: &Connection) -> Result<OwnedObjectPath> {
    let manager = zbus::fdo::ObjectManagerProxy::builder(connection)
        .destination("org.bluez")?
        .path("/")?
        .build()
        .await?;
    let objects = manager
        .get_managed_objects()
        .await
        .context("bluetoothd is not running")?;
    let mut adapters: Vec<_> = objects
        .into_iter()
        .filter(|(_, interfaces)| {
            interfaces
                .keys()
                .any(|i| i.as_str() == "org.bluez.Adapter1")
        })
        .map(|(path, _)| path)
        .collect();
    adapters.sort_by(|a, b| a.as_str().cmp(b.as_str()));
    adapters
        .into_iter()
        .next()
        .context("this computer has no Bluetooth adapter")
}

fn bluez_error(error: zbus::Error) -> anyhow::Error {
    let text = error.to_string();
    if text.contains("AlreadyExists") {
        anyhow::anyhow!("another Desklink keyboard is already running on this computer")
    } else if text.contains("UUID already registered") {
        anyhow::anyhow!(
            "bluetoothd's input plugin owns the keyboard service; run \
             `sudo scripts/linux-bluetooth-hid.sh enable` once"
        )
    } else {
        anyhow::Error::new(error)
    }
}

impl HidDevice {
    pub async fn start(options: Options) -> Result<Self> {
        if crate::input_plugin_active() == Some(true) {
            bail!(
                "bluetoothd's input plugin owns the keyboard channels; run \
                 `sudo scripts/linux-bluetooth-hid.sh enable` once (see docs/bluetooth-linux.md)"
            );
        }
        let shared = Arc::new(Shared {
            queue: ReportQueue::new(),
            channels: Mutex::new(Channels::default()),
            host: Mutex::new(options.host),
            pairing: AtomicBool::new(false),
            wanted: (Mutex::new(false), Condvar::new()),
            stopping: AtomicBool::new(false),
            host_changed: options.host_changed,
        });
        let connection = Connection::system()
            .await
            .context("cannot reach the system D-Bus")?;
        let adapter = find_adapter(&connection).await?;
        let server = connection.object_server();
        for (path, psm) in [
            (CONTROL_PATH, record::PSM_CONTROL),
            (INTERRUPT_PATH, record::PSM_INTERRUPT),
        ] {
            server
                .at(
                    path,
                    Profile {
                        psm,
                        shared: shared.clone(),
                    },
                )
                .await?;
        }
        server
            .at(
                AGENT_PATH,
                Agent {
                    shared: shared.clone(),
                },
            )
            .await?;
        // Subscribe before registering, so a restart in between is not missed.
        let restarts = zbus::fdo::DBusProxy::new(&connection)
            .await?
            .receive_name_owner_changed_with_args(&[(0, BLUEZ)])
            .await?;
        register_profiles(&connection, &options.name).await?;
        let watcher = tokio::spawn(follow_restarts(
            connection.clone(),
            options.name.clone(),
            restarts,
        ));

        let writer = shared.clone();
        thread::Builder::new()
            .name("desklink-hid-writer".into())
            .spawn(move || writer.write())?;
        let reconnect = shared.clone();
        thread::Builder::new()
            .name("desklink-hid-reconnect".into())
            .spawn(move || reconnect.reconnect())?;
        tracing::info!(adapter = %adapter.as_str(), "Bluetooth keyboard and mouse ready");
        Ok(Self {
            shared,
            connection,
            adapter,
            watcher,
        })
    }

    pub fn sink(&self) -> BluezSink {
        BluezSink(self.shared.clone())
    }

    pub fn host(&self) -> Option<String> {
        self.shared.host()
    }

    pub fn connected(&self) -> bool {
        self.shared.connected()
    }

    async fn set_adapter(&self, name: &str, value: Value<'_>) -> Result<()> {
        let properties = zbus::fdo::PropertiesProxy::builder(&self.connection)
            .destination("org.bluez")?
            .path(self.adapter.clone())?
            .build()
            .await?;
        properties
            .set("org.bluez.Adapter1".try_into()?, name, value)
            .await
            .with_context(|| format!("cannot set the adapter's {name}"))?;
        Ok(())
    }

    /// Makes this computer visible and accepts the first host that pairs and connects within
    /// `window`. Any earlier host is forgotten.
    pub async fn pair(&self, window: Duration) -> Result<String> {
        if let Some(old) = lock(&self.shared.host).take() {
            tracing::info!(address = %old, "forgetting the previous host");
            (self.shared.host_changed)(None);
        }
        let generation = lock(&self.shared.channels).generation;
        self.shared.disconnect(generation);
        let agents = zbus::Proxy::new(
            &self.connection,
            "org.bluez",
            "/org/bluez",
            "org.bluez.AgentManager1",
        )
        .await?;
        let agent = ObjectPath::try_from(AGENT_PATH)?;
        agents
            .call_method("RegisterAgent", &(&agent, "NoInputNoOutput"))
            .await
            .context("cannot register the pairing agent")?;
        agents
            .call_method("RequestDefaultAgent", &(&agent,))
            .await?;
        self.shared.pairing.store(true, Ordering::SeqCst);
        let seconds = window.as_secs().clamp(30, 600) as u32;
        let result = async {
            self.set_adapter("PairableTimeout", Value::U32(seconds))
                .await?;
            self.set_adapter("Pairable", true.into()).await?;
            self.set_adapter("DiscoverableTimeout", Value::U32(seconds))
                .await?;
            self.set_adapter("Discoverable", true.into()).await?;
            let deadline = Instant::now() + window;
            loop {
                if self.connected()
                    && let Some(host) = self.host()
                {
                    return Ok(host);
                }
                if Instant::now() > deadline {
                    bail!("no computer paired within {} seconds", window.as_secs());
                }
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
        }
        .await;
        self.shared.pairing.store(false, Ordering::SeqCst);
        let _ = self.set_adapter("Discoverable", false.into()).await;
        let _ = agents.call_method("UnregisterAgent", &(&agent,)).await;
        if let Ok(host) = &result {
            self.trust(host).await;
        }
        result
    }

    /// Lets the host reconnect without asking again.
    async fn trust(&self, host: &str) {
        let path = format!("{}/dev_{}", self.adapter.as_str(), host.replace(':', "_"));
        let result = async {
            let properties = zbus::fdo::PropertiesProxy::builder(&self.connection)
                .destination("org.bluez")?
                .path(path)?
                .build()
                .await?;
            properties
                .set("org.bluez.Device1".try_into()?, "Trusted", true.into())
                .await?;
            anyhow::Ok(())
        }
        .await;
        if let Err(error) = result {
            tracing::debug!(error = %format!("{error:#}"), "cannot mark the host trusted");
        }
    }
}

impl Drop for HidDevice {
    fn drop(&mut self) {
        self.watcher.abort();
        self.shared.stopping.store(true, Ordering::SeqCst);
        self.shared.queue.close();
        self.shared.wanted.1.notify_all();
        let generation = lock(&self.shared.channels).generation;
        self.shared.disconnect(generation);
        // bluetoothd unregisters the profiles and agent when this connection closes.
    }
}

/// Where `HidDesktop` sends its reports.
pub struct BluezSink(Arc<Shared>);

impl ReportSink for BluezSink {
    fn send(&mut self, report: Report) -> Result<()> {
        if self.0.connected() {
            if !self.0.queue.push(report) {
                tracing::warn!("the Bluetooth link stalled; dropped queued input");
            }
        } else {
            // Input for a host that is away: reach it now rather than at the next retry.
            let (wanted, wake) = &self.0.wanted;
            *lock(wanted) = true;
            wake.notify_one();
        }
        Ok(())
    }
}
