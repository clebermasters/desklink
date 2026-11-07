use anyhow::{Context, Result};
use desklink_settings::control::{self, Request, Status};
use std::{
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::Duration,
};

pub struct Reply {
    pub request: Request,
    pub result: Result<Status, String>,
}
pub struct Client {
    tx: mpsc::Sender<Request>,
    pub replies: mpsc::Receiver<Reply>,
    stop: Arc<AtomicBool>,
}
impl Client {
    pub fn new(profile: PathBuf) -> Self {
        let (tx, requests) = mpsc::channel::<Request>();
        let (replies_tx, replies) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        thread::Builder::new()
            .name("desklink-ui-status".into())
            .spawn(move || {
                while !stopped.load(Ordering::Relaxed) {
                    let request = match requests.recv_timeout(Duration::from_millis(400)) {
                        Ok(r) => r,
                        Err(mpsc::RecvTimeoutError::Timeout) => Request::Status,
                        Err(_) => break,
                    };
                    let result =
                        control::request(&profile, request.clone()).map_err(|e| e.to_string());
                    if replies_tx.send(Reply { request, result }).is_err() {
                        break;
                    }
                }
            })
            .expect("status worker creation");
        Self { tx, replies, stop }
    }
    pub fn send(&self, request: Request) {
        let _ = self.tx.send(request);
    }
}
impl Drop for Client {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

pub fn spawn(profile: &Path) -> Result<Child> {
    let logfile = profile.with_file_name("desklink.log");
    let mut options = std::fs::OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let log = options.open(logfile)?;
    Command::new(std::env::current_exe()?)
        .args(["--config"])
        .arg(profile)
        .arg("run")
        .env("NO_COLOR", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::from(log))
        .spawn()
        .context("Could not start the sharing agent")
}

pub fn last_error(profile: &Path) -> String {
    std::fs::read_to_string(profile.with_file_name("desklink.log"))
        .ok()
        .and_then(|s| {
            s.lines()
                .rev()
                .find_map(|line| line.strip_prefix("desklink: ").map(str::to_owned))
        })
        .unwrap_or_else(|| {
            "Sharing could not start. Check desktop permissions and connection settings.".into()
        })
}
