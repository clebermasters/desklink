//! Runs the shared Desklink agent on a dedicated thread inside the app process.
use crate::{
    bridge::{Bridge, BridgeSink},
    desktop::AndroidDesktop,
    profile,
};
use anyhow::{Context, Result};
use desklink::runtime::{Host, run_with_host};
use desklink_hid::{HidDesktop, Options};
use desklink_platform::Desktop;
use desklink_settings::{
    Config,
    control::{Request, Status},
    identity_path, lock_profile,
};
use desklink_transport::Identity;
use serde::Serialize;
use std::{
    path::Path,
    sync::{Arc, Mutex, MutexGuard},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;

/// A receiver only needs polling for clipboard copies and crossing timeouts; input arrives
/// from the network immediately. 20 ms keeps the agent responsive without 250 wakeups/s.
pub const TICK: Duration = Duration::from_millis(20);
const STOP_DEADLINE: Duration = Duration::from_secs(3);

struct Running {
    thread: JoinHandle<()>,
    shutdown: CancellationToken,
}

/// What the agent's input drives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Receiver {
    /// This phone, through the accessibility service or the ADB helper.
    Phone,
    /// Another computer, through the phone's Bluetooth keyboard and mouse.
    Keyboard(Options),
}

pub struct Agent {
    bridge: Arc<Bridge>,
    receiver: Mutex<Receiver>,
    running: Mutex<Option<Running>>,
    error: Arc<Mutex<Option<String>>>,
}

#[derive(Debug, Serialize)]
pub struct AgentStatus {
    pub running: bool,
    pub error: Option<String>,
    pub status: Status,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl Agent {
    pub fn new(bridge: Arc<Bridge>) -> Self {
        Self {
            bridge,
            receiver: Mutex::new(Receiver::Phone),
            running: Mutex::new(None),
            error: Arc::new(Mutex::new(None)),
        }
    }
    pub fn bridge(&self) -> &Arc<Bridge> {
        &self.bridge
    }
    /// Takes effect at the next [`Agent::start`].
    pub fn set_receiver(&self, receiver: Receiver) {
        *lock(&self.receiver) = receiver;
    }
    /// Idempotent: starting a running agent does nothing. Configuration errors are returned
    /// synchronously; failures after startup are reported through [`Agent::status`].
    pub fn start(&self, dir: &Path) -> Result<()> {
        let mut running = lock(&self.running);
        if running.as_ref().is_some_and(|r| !r.thread.is_finished()) {
            return Ok(());
        }
        if let Some(previous) = running.take() {
            let _ = previous.thread.join();
        }
        let path = profile::config_path(dir);
        let config = Config::load(&path)?;
        let identity =
            Identity::load(&identity_path(&path)).context("cannot load this phone's identity")?;
        let desktop: Box<dyn Desktop + Send> = match *lock(&self.receiver) {
            Receiver::Phone => {
                self.bridge
                    .geometry()
                    .context("report the screen size before starting")?;
                Box::new(AndroidDesktop::new(self.bridge.clone()))
            }
            Receiver::Keyboard(options) => {
                Box::new(HidDesktop::new(BridgeSink(self.bridge.clone()), options)?)
            }
        };
        let profile_lock = lock_profile(&path)?;
        self.bridge.reopen();
        *lock(&self.error) = None;
        let shutdown = CancellationToken::new();
        let (bridge, error, token) = (self.bridge.clone(), self.error.clone(), shutdown.clone());
        let thread = thread::Builder::new()
            .name("desklink-agent".into())
            .spawn(move || {
                let result = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .context("cannot start the network runtime")
                    .and_then(|runtime| {
                        runtime.block_on(run_with_host(
                            config,
                            identity,
                            path,
                            Host {
                                desktop,
                                tick: TICK,
                                handle_signals: false,
                                shutdown: token,
                            },
                        ))
                    });
                drop(profile_lock);
                if let Err(e) = result {
                    tracing::error!(error = %format!("{e:#}"), "sharing stopped with an error");
                    *lock(&error) = Some(format!("{e:#}"));
                }
                bridge.close();
            })?;
        *running = Some(Running { thread, shutdown });
        Ok(())
    }
    /// Releases injected input and closes connections. Never blocks the caller indefinitely.
    pub fn stop(&self) {
        let Some(running) = lock(&self.running).take() else {
            return;
        };
        running.shutdown.cancel();
        let deadline = Instant::now() + STOP_DEADLINE;
        while !running.thread.is_finished() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        if running.thread.is_finished() {
            let _ = running.thread.join();
        } else {
            tracing::warn!("sharing agent did not stop within the deadline");
        }
        self.bridge.close();
    }
    pub fn is_running(&self) -> bool {
        lock(&self.running)
            .as_ref()
            .is_some_and(|r| !r.thread.is_finished())
    }
    pub fn status(&self, dir: &Path) -> AgentStatus {
        let running = self.is_running();
        let status = if running {
            desklink_settings::control::request(&profile::config_path(dir), Request::Status)
                .unwrap_or_default()
        } else {
            Status::default()
        };
        AgentStatus {
            running,
            error: lock(&self.error).clone(),
            status,
        }
    }
}
