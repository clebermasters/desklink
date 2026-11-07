//! Portal-authorized capture and emulation. Never opens /dev/input or /dev/uinput.
mod barriers;
mod clipboard_reads;
mod ei;
mod pipe;
mod selection;
use clipboard_reads::ClipboardReads;

use super::{Capabilities, Desktop, LocalEvent};
use anyhow::{Context, Result, ensure};
use ashpd::desktop::{
    clipboard::{Clipboard, SetSelectionOptions},
    input_capture::{self as capture, ActivatedBarrier, InputCapture},
    remote_desktop::{DeviceType, RemoteDesktop, SelectDevicesOptions},
};
use desklink_protocol::{
    CaptureSpan, ClipboardContent, ClipboardKind, Edge, Geometry, Input, MAX_CLIPBOARD, MAX_TEXT,
};
use futures_util::StreamExt;
use reis::event::EiEvent;
use std::{
    os::unix::net::UnixStream,
    sync::{Arc, Mutex, mpsc as stdmpsc},
    thread,
    time::{Duration, Instant},
};
use tokio::{sync::mpsc, task::JoinSet};

enum Command {
    Capture,
    Release,
    Warp(i32, i32),
    Input(Input),
    Clipboard(ClipboardContent),
}

pub struct Wayland {
    spans: Vec<CaptureSpan>,
    commands: mpsc::Sender<Command>,
    events: stdmpsc::Receiver<LocalEvent>,
    geometry: Arc<Mutex<Geometry>>,
    clipboard: bool,
}

impl Wayland {
    pub fn open(spans: Vec<CaptureSpan>, clipboard: bool) -> Result<Self> {
        let (commands, rx) = mpsc::channel(256);
        let (tx, events) = stdmpsc::sync_channel(1024);
        let (ready_tx, ready) = stdmpsc::sync_channel(1);
        let geometry = Arc::new(Mutex::new(Geometry {
            x: 0,
            y: 0,
            width: 4,
            height: 4,
        }));
        let shared = geometry.clone();
        let initial_spans = spans.clone();
        thread::Builder::new()
            .name("desklink-wayland".into())
            .spawn(move || {
                let result = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .map_err(anyhow::Error::from)
                    .and_then(|runtime| {
                        runtime.block_on(worker(
                            spans,
                            clipboard,
                            rx,
                            tx.clone(),
                            shared,
                            ready_tx.clone(),
                        ))
                    });
                if let Err(error) = result {
                    let message = format!("Wayland portal: {error:#}");
                    let _ = ready_tx.try_send(Err(message.clone()));
                    let _ = tx.try_send(LocalEvent::Failure(message));
                }
            })?;
        // The portal may present user approval dialogs. This timeout does not grant permissions.
        ready
            .recv_timeout(Duration::from_secs(120))
            .context("Wayland approval timed out; approve the input portal dialogs and retry")?
            .map_err(anyhow::Error::msg)?;
        Ok(Self {
            spans: initial_spans,
            commands,
            events,
            geometry,
            clipboard,
        })
    }
    fn command(&self, c: Command) -> Result<()> {
        self.commands
            .try_send(c)
            .context("Wayland backend stopped or input queue filled")
    }
}

impl Desktop for Wayland {
    // GNOME 48 discards capture device references on Disable, while the portal
    // permits ConnectToEIS only once. Reauthorize with a fresh session instead.
    fn can_update_capture_spans(&self) -> bool {
        false
    }
    fn set_capture_spans(&mut self, spans: &[CaptureSpan]) -> Result<()> {
        ensure!(
            spans == self.spans,
            "Restart sharing to apply changed Wayland capture edges."
        );
        Ok(())
    }

    fn geometry(&mut self) -> Result<Geometry> {
        Ok(*self
            .geometry
            .lock()
            .map_err(|_| anyhow::anyhow!("geometry lock poisoned"))?)
    }
    fn poll(&mut self) -> Result<Vec<LocalEvent>> {
        let mut events = Vec::new();
        for _ in 0..1024 {
            match self.events.try_recv() {
                Ok(e) => events.push(e),
                Err(stdmpsc::TryRecvError::Empty) => break,
                Err(stdmpsc::TryRecvError::Disconnected) => {
                    if events.is_empty() {
                        anyhow::bail!("Wayland session ended");
                    }
                    break;
                }
            }
        }
        Ok(events)
    }
    fn capture(&mut self) -> Result<Vec<Input>> {
        self.command(Command::Capture)?;
        Ok(Vec::new())
    }
    fn release(&mut self) -> Result<()> {
        self.command(Command::Release)
    }
    fn warp(&mut self, x: i32, y: i32) -> Result<()> {
        self.command(Command::Warp(x, y))
    }
    fn inject(&mut self, input: Input) -> Result<()> {
        self.command(Command::Input(input))
    }
    fn owns_clipboard(&self) -> bool {
        self.clipboard
    }
    fn clipboard_apply(&mut self, content: ClipboardContent) -> Result<()> {
        self.command(Command::Clipboard(content))
    }
}

pub fn capabilities() -> Capabilities {
    // Probes run on another thread because the application may already have a Tokio runtime.
    let result=thread::spawn(||->Result<String>{tokio::runtime::Builder::new_current_thread().enable_all().build()?.block_on(async {
        let input=InputCapture::new().await.context("InputCapture portal is unavailable")?;
        let remote=RemoteDesktop::new().await.context("RemoteDesktop portal is unavailable")?;
        let capabilities=input.supported_capabilities().await?;
        let devices=remote.available_device_types().await?;
        ensure!(capabilities.contains(capture::Capabilities::Keyboard|capture::Capabilities::Pointer),"portal does not support keyboard and pointer capture");
        ensure!(devices.contains(DeviceType::Keyboard|DeviceType::Pointer),"portal does not support keyboard and pointer injection");
        Ok(format!("InputCapture v{} and RemoteDesktop v{} are available; user approval and EIS device negotiation are required at run time",input.version(),remote.version()))
    })}).join().map_err(|_|anyhow::anyhow!("portal probe failed")).and_then(|r|r);
    match result {
        Ok(detail) => Capabilities {
            backend: "wayland-portals",
            capture: true,
            inject: true,
            geometry: None,
            detail,
        },
        Err(e) => Capabilities {
            backend: "wayland-portals",
            capture: false,
            inject: false,
            geometry: None,
            detail: format!("{e:#}"),
        },
    }
}

fn emit(tx: &stdmpsc::SyncSender<LocalEvent>, event: LocalEvent) -> Result<()> {
    tx.try_send(event).map_err(|_| {
        anyhow::anyhow!("Wayland event queue overflowed; ending capture to restore local control")
    })
}

async fn worker(
    spans: Vec<CaptureSpan>,
    use_clipboard: bool,
    mut commands: mpsc::Receiver<Command>,
    events: stdmpsc::SyncSender<LocalEvent>,
    shared: Arc<Mutex<Geometry>>,
    ready: stdmpsc::SyncSender<std::result::Result<(), String>>,
) -> Result<()> {
    let capture = InputCapture::new()
        .await
        .context("desktop does not implement the InputCapture portal")?;
    let (capture_session, caps) = capture
        .create_session(
            None,
            capture::CreateSessionOptions::default()
                .set_capabilities(capture::Capabilities::Keyboard | capture::Capabilities::Pointer),
        )
        .await?;
    ensure!(
        caps.contains(capture::Capabilities::Keyboard | capture::Capabilities::Pointer),
        "keyboard or pointer capture was denied"
    );
    let capture_session = Arc::new(capture_session);
    let capture_path = session_path(capture_session.as_ref())?;
    let remote = RemoteDesktop::new()
        .await
        .context("desktop does not implement the RemoteDesktop portal")?;
    let remote_session = Arc::new(remote.create_session(Default::default()).await?);
    let remote_path = session_path(remote_session.as_ref())?;
    let clipboard = if use_clipboard {
        let portal = Arc::new(Clipboard::new().await.context(
            "clipboard portal unavailable; disable clipboard explicitly to use input-only mode",
        )?);
        portal.request(&remote_session, Default::default()).await?;
        Some(portal)
    } else {
        None
    };
    remote
        .select_devices(
            &remote_session,
            SelectDevicesOptions::default().set_devices(DeviceType::Keyboard | DeviceType::Pointer),
        )
        .await?
        .response()?;
    let started = remote
        .start(&remote_session, None, Default::default())
        .await?
        .response()?;
    ensure!(
        started
            .devices()
            .contains(DeviceType::Keyboard | DeviceType::Pointer),
        "remote input permission was denied"
    );
    ensure!(
        !use_clipboard || started.is_clipboard_enabled(),
        "clipboard sharing permission was not granted by the portal"
    );
    let fd = remote
        .connect_to_eis(&remote_session, Default::default())
        .await?;
    let stream = UnixStream::from(fd);
    stream.set_nonblocking(true)?;
    let sender_context = reis::ei::Context::new(stream)?;
    sender_context.flush()?;
    let (sender_connection, mut sender_events) = sender_context
        .handshake_tokio("Desklink input", reis::ei::handshake::ContextType::Sender)
        .await?;
    let mut injection = ei::Injector::new(sender_connection);
    let (receiver_connection, mut receiver_events) =
        capture_stream(&capture, &capture_session).await?;
    let zones = capture
        .zones(&capture_session, Default::default())
        .await?
        .response()?;
    let geometry = geometry_from_zones(zones.regions())?;
    *shared
        .lock()
        .map_err(|_| anyhow::anyhow!("geometry lock poisoned"))? = geometry;
    let (barriers, barrier_edges) = barriers::build(zones.regions(), geometry, &spans)?;
    let mut activated = capture.receive_activated().await?;
    let mut deactivated = capture.receive_deactivated().await?;
    let mut disabled = capture.receive_disabled().await?;
    let mut zones_changed = capture.receive_zones_changed().await?;
    let response = capture
        .set_pointer_barriers(
            &capture_session,
            &barriers,
            zones.zone_set(),
            Default::default(),
        )
        .await?
        .response()?;
    ensure!(
        response.failed_barriers().is_empty(),
        "compositor rejected a configured screen edge"
    );
    capture.enable(&capture_session, Default::default()).await?;
    let mut selection = Box::pin(if let Some(c) = &clipboard {
        c.receive_signal("SelectionOwnerChanged")
            .await?
            .left_stream()
    } else {
        futures_util::stream::pending().right_stream()
    });
    let mut transfers = Box::pin(if let Some(c) = &clipboard {
        c.receive_signal("SelectionTransfer").await?.left_stream()
    } else {
        futures_util::stream::pending().right_stream()
    });
    let mut clipboard_reads = ClipboardReads::default();
    let mut clipboard_writes: JoinSet<Result<()>> = JoinSet::new();
    let mut owned: Option<Arc<ClipboardContent>> = None;
    let mut activation: Option<(u32, Instant, (f64, f64))> = None;
    let mut sharing = false;
    let mut announced = false;
    let mut fractions = ei::Fractions::default();
    let mut maintenance = tokio::time::interval(Duration::from_millis(25));
    let mut startup = Box::pin(tokio::time::sleep(Duration::from_secs(10)));
    let result:Result<()>=async {
        loop {
            tokio::select! {
                biased;
                signal=activated.next()=>{
                    let signal = signal.context("capture portal closed")?;
                    if signal.session_handle().as_str() != capture_path {
                        continue;
                    }
                    let edge = match signal.barrier_id() {
                        Some(ActivatedBarrier::Barrier(id)) => barrier_edges.get(&id).copied(),
                        _ => None,
                    }
                    .context("capture activated without a configured barrier")?;
                    let (mut x, mut y) = signal
                        .cursor_position()
                        .unwrap_or((geometry.x as f32, geometry.y as f32));
                    let (dx, dy) = match edge {
                        Edge::Left => {
                            x = geometry.x as f32;
                            (-1, 0)
                        }
                        Edge::Right => {
                            x = (geometry.x + geometry.width as i32 - 1) as f32;
                            (1, 0)
                        }
                        Edge::Top => {
                            y = geometry.y as f32;
                            (0, -1)
                        }
                        Edge::Bottom => {
                            y = (geometry.y + geometry.height as i32 - 1) as f32;
                            (0, 1)
                        }
                    };
                    let inset = geometry.entry(
                        edge,
                        match edge {
                            Edge::Left | Edge::Right => {
                                (y as f64 - geometry.y as f64) / (geometry.height - 1) as f64
                            }
                            _ => (x as f64 - geometry.x as f64) / (geometry.width - 1) as f64,
                        },
                    );
                    activation = Some((
                        signal
                            .activation_id()
                            .context("missing capture activation id")?,
                        Instant::now(),
                        (inset.0 as f64, inset.1 as f64),
                    ));
                    fractions = ei::Fractions::default();
                    emit(
                        &events,
                        LocalEvent::Pointer {
                            x: x as i32,
                            y: y as i32,
                            dx,
                            dy,
                        },
                    )?;
                }
                signal=deactivated.next()=>{
                    let signal = signal.context("capture portal closed")?;
                    if signal.session_handle().as_str() == capture_path
                        && activation
                            .as_ref()
                            .is_some_and(|(id, _, _)| Some(*id) == signal.activation_id())
                    {
                        activation = None;
                        if sharing {
                            sharing = false;
                            emit(&events, LocalEvent::CaptureLost)?;
                        }
                    }
                }
                signal=disabled.next()=>{
                    let signal=signal.context("capture portal closed")?;
                    if signal.session_handle().as_str()==capture_path {
                        emit(&events, LocalEvent::CaptureLost)?;
                        anyhow::bail!("The desktop disabled input capture. Start sharing again to request a fresh permission session.");
                    }
                }
                _=zones_changed.next()=>anyhow::bail!("display layout changed; restart sharing to reauthorize the capture zones"),
                event=sender_events.next()=>{
                    injection.event(event.context("EIS injection connection closed")??)?;
                }
                event=receiver_events.next()=>{
                    let event = event.context("EIS capture connection closed")??;
                    match event {
                        EiEvent::SeatAdded(e) => {
                            e.seat.bind_capabilities(ei::requested_capabilities());
                            receiver_connection.flush()?;
                        }
                        EiEvent::Disconnected(_) => anyhow::bail!("EIS capture was disconnected"),
                        event if activation.is_some() => {
                            if let Some(input) = fractions.input(event) {
                                emit(&events, LocalEvent::Input(input))?;
                            }
                        }
                        _ => {}
                    }
                }
                command=commands.recv()=>{
                    let Some(command) = command else {
                        break;
                    };
                    match command {
                        Command::Capture => {
                            ensure!(activation.is_some(), "capture was revoked before handoff");
                            sharing = true;
                        }
                        Command::Release => {
                            sharing = false;
                            if let Some((id, _, position)) = activation.take() {
                                capture
                                    .release(
                                        &capture_session,
                                        capture::ReleaseOptions::default()
                                            .set_activation_id(id)
                                            .set_cursor_position(position),
                                    )
                                    .await?;
                            }
                        }
                        Command::Warp(x, y) => injection.warp(x, y)?,
                        Command::Input(input) => injection.input(input)?,
                        Command::Clipboard(content) => {
                            ensure!(content.valid(), "invalid clipboard content");
                            if let Some(c) = &clipboard {
                                clipboard_reads.invalidate();
                                let mimes = match content.kind {
                                    ClipboardKind::Text => {
                                        vec!["text/plain;charset=utf-8", "text/plain", "UTF8_STRING"]
                                    }
                                    ClipboardKind::Png => vec!["image/png"],
                                };
                                owned = Some(Arc::new(content));
                                c.set_selection(
                                    &remote_session,
                                    SetSelectionOptions::default().set_mime_types(&mimes),
                                )
                                .await?;
                            }
                        }
                    }
                }
                change=selection.next()=>{
                    let change = selection::parse(&change.context("clipboard portal closed")?)?;
                    if change.session.as_str() != remote_path {
                        continue;
                    }
                    tracing::debug!(formats=change.mimes.len(),owner=change.owner,"portal clipboard selection changed");
                    clipboard_reads.invalidate();
                    if change.owner {continue;}
                    let mime = if change.mimes.iter().any(|m| m == "image/png") {
                        Some(("image/png", ClipboardKind::Png))
                    } else if change
                        .mimes
                        .iter()
                        .any(|m| m == "text/plain;charset=utf-8")
                    {
                        Some(("text/plain;charset=utf-8", ClipboardKind::Text))
                    } else if change.mimes.iter().any(|m| m == "text/plain") {
                        Some(("text/plain", ClipboardKind::Text))
                    } else {
                        None
                    };
                    if let (Some(c), Some((mime, kind))) = (&clipboard, mime) {
                        let (c, session) = (c.clone(), remote_session.clone());
                        clipboard_reads.start(async move {
                            let fd = c.selection_read(&session, mime).await?;
                            let limit = if kind == ClipboardKind::Text {
                                MAX_TEXT
                            } else {
                                MAX_CLIPBOARD
                            };
                            let bytes = tokio::time::timeout(Duration::from_secs(5), pipe::read(fd.into(), limit))
                                .await??;
                            let content = ClipboardContent { kind, bytes };
                            ensure!(content.valid(), "invalid local clipboard representation");
                            Ok(content)
                        });
                    }
                }
                transfer=transfers.next()=>{
                    let (session,mime,serial):(ashpd::zvariant::OwnedObjectPath,String,u32)=transfer.context("clipboard portal closed")?.body().deserialize()?;
                    if session.as_str() != remote_path {
                        continue;
                    }
                    if let (Some(c), Some(content)) = (&clipboard, &owned) {
                        if clipboard_writes.len() >= 4 {
                            c.selection_write_done(&remote_session, serial, false)
                                .await?;
                            continue;
                        }
                        let allowed = match content.kind {
                            ClipboardKind::Text => matches!(
                                mime.as_str(),
                                "text/plain;charset=utf-8" | "text/plain" | "UTF8_STRING"
                            ),
                            ClipboardKind::Png => mime == "image/png",
                        };
                        let (c, session, content) = (c.clone(), remote_session.clone(), content.clone());
                        clipboard_writes.spawn(async move {
                            let result: Result<()> = async {
                                ensure!(allowed, "clipboard format not offered");
                                let fd = c.selection_write(&session, serial).await?;
                                tokio::time::timeout(
                                    Duration::from_secs(5),
                                    pipe::write(fd.into(), &content.bytes),
                                )
                                .await??;
                                Ok(())
                            }
                            .await;
                            c.selection_write_done(&session, serial, result.is_ok())
                                .await?;
                            result
                        });
                    }
                }
                read=clipboard_reads.next(),if !clipboard_reads.is_empty()=>{
                    match read {
                        Some(Ok(content))=>emit(&events,LocalEvent::Clipboard(content))?,
                        Some(Err(error))=>tracing::warn!(%error,"clipboard owner could not be read"),
                        None=>{},
                    }
                }
                _=clipboard_writes.join_next(),if !clipboard_writes.is_empty()=>{

                },
                _=maintenance.tick()=>{
                    if !announced && injection.ready() {
                        announced = true;
                        let _ = ready.send(Ok(()));
                    }
                    if !sharing
                        && activation
                            .as_ref()
                            .is_some_and(|(_, since, _)| since.elapsed() > Duration::from_millis(300))
                    {
                        let (id, _, position) = activation.take().unwrap();
                        capture
                            .release(
                                &capture_session,
                                capture::ReleaseOptions::default()
                                    .set_activation_id(id)
                                    .set_cursor_position(position),
                            )
                            .await?;
                    }
                }
                _=&mut startup,if !announced=>anyhow::bail!("compositor did not provide absolute pointer, keyboard and button emulation devices"),
            }
        }Ok(())
    }.await;
    clipboard_reads.invalidate();
    clipboard_writes.abort_all();
    let _ = capture_session.close().await;
    let _ = remote_session.close().await;
    result
}

async fn capture_stream(
    capture: &InputCapture,
    session: &ashpd::desktop::Session<InputCapture>,
) -> Result<(reis::event::Connection, reis::tokio::EiConvertEventStream)> {
    let fd = capture.connect_to_eis(session, Default::default()).await?;
    let stream = UnixStream::from(fd);
    stream.set_nonblocking(true)?;
    let context = reis::ei::Context::new(stream)?;
    context.flush()?;
    tokio::time::timeout(
        Duration::from_secs(5),
        context.handshake_tokio(
            "Desklink capture",
            reis::ei::handshake::ContextType::Receiver,
        ),
    )
    .await
    .context("Capture event transport timed out")?
    .context("Capture event transport failed")
}

fn geometry_from_zones(zones: &[capture::Region]) -> Result<Geometry> {
    ensure!(!zones.is_empty(), "portal returned no screen zones");
    let x = zones.iter().map(|z| z.x_offset()).min().unwrap();
    let y = zones.iter().map(|z| z.y_offset()).min().unwrap();
    let right = zones
        .iter()
        .map(|z| z.x_offset() as i64 + z.width() as i64)
        .max()
        .unwrap();
    let bottom = zones
        .iter()
        .map(|z| z.y_offset() as i64 + z.height() as i64)
        .max()
        .unwrap();
    let g = Geometry {
        x,
        y,
        width: (right - x as i64).try_into()?,
        height: (bottom - y as i64).try_into()?,
    };
    ensure!(g.valid(), "invalid screen zones");
    Ok(g)
}

fn session_path<T: serde::Serialize>(session: &T) -> Result<String> {
    serde_json::to_value(session)?
        .as_str()
        .map(str::to_owned)
        .context("invalid portal session path")
}
