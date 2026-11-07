//! Owns the OS clipboard on a dedicated thread. Never logs clipboard contents.
#![forbid(unsafe_code)]
// On Android the app owns the clipboard; only the shared types and PNG encoding are used there.
#![cfg_attr(target_os = "android", allow(dead_code, unused_imports))]

use anyhow::{Context, Result, ensure};
use desklink_protocol::{ClipboardContent, ClipboardKind, MAX_CLIPBOARD, MAX_PIXELS, MAX_TEXT};
use image::ImageEncoder;
#[cfg(not(target_os = "android"))]
use std::borrow::Cow;
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread,
    time::Duration,
};
use tokio::sync::{mpsc, watch};

pub struct ClipboardService {
    updates: mpsc::Receiver<Observation>,
    apply: watch::Sender<Option<Arc<Applied>>>,
    revision: Arc<AtomicU64>,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}

struct Observation {
    revision: u64,
    content: ClipboardContent,
}
struct Applied {
    revision: u64,
    content: ClipboardContent,
}

impl ClipboardService {
    #[cfg(target_os = "android")]
    pub fn start() -> Result<Self> {
        anyhow::bail!("the Android app owns clipboard access")
    }
    #[cfg(not(target_os = "android"))]
    pub fn start() -> Result<Self> {
        let (tx, updates) = mpsc::channel(4);
        let (apply, mut rx) = watch::channel::<Option<Arc<Applied>>>(None);
        let revision = Arc::new(AtomicU64::new(0));
        let worker_revision = revision.clone();
        let (ready_tx, ready) = std::sync::mpsc::sync_channel(1);
        let stop = Arc::new(AtomicBool::new(false));
        let stop_worker = stop.clone();
        let worker = thread::Builder::new()
            .name("desklink-clipboard".into())
            .spawn(move || {
                let mut clipboard = match arboard::Clipboard::new() {
                    Ok(c) => {
                        let _ = ready_tx.send(Ok(()));
                        c
                    }
                    Err(e) => {
                        let _ = ready_tx.send(Err(anyhow::anyhow!(e)));
                        return;
                    }
                };
                let mut seen = None;
                let mut observed_revision = 0;
                while !stop_worker.load(Ordering::Relaxed) {
                    if rx.has_changed().unwrap_or(false) {
                        let content = rx.borrow_and_update().clone();
                        if let Some(applied) = content {
                            observed_revision = applied.revision;
                            match write_content(&mut clipboard, &applied.content) {
                                Ok(()) => {
                                    seen = Some(
                                        read_content(&mut clipboard)
                                            .map_or_else(|| applied.content.hash(), |v| v.hash()),
                                    );
                                }
                                Err(e) => {
                                    tracing::warn!(error=%e,"could not apply remote clipboard");
                                    seen =
                                        read_content(&mut clipboard).map(|content| content.hash());
                                }
                            }
                        }
                    }
                    if worker_revision.load(Ordering::Acquire) == observed_revision
                        && let Some(content) = read_content(&mut clipboard)
                    {
                        let hash = content.hash();
                        if seen != Some(hash)
                            && worker_revision.load(Ordering::Acquire) == observed_revision
                            && tx
                                .try_send(Observation {
                                    revision: observed_revision,
                                    content,
                                })
                                .is_ok()
                        {
                            seen = Some(hash);
                        }
                    }
                    thread::sleep(Duration::from_millis(350));
                }
            })?;
        ready
            .recv_timeout(Duration::from_secs(5))
            .context("clipboard initialization timed out")??;
        Ok(Self {
            updates,
            apply,
            revision,
            stop,
            worker: Some(worker),
        })
    }
    pub fn apply(&self, content: ClipboardContent) {
        // Invalidate observations immediately, before the OS worker gets CPU time.
        let revision = self.revision.fetch_add(1, Ordering::AcqRel).wrapping_add(1);
        self.apply
            .send_replace(Some(Arc::new(Applied { revision, content })));
    }

    /// A queued observation from before a remote apply is no longer a new local copy.
    pub async fn next_change(&mut self) -> Option<ClipboardContent> {
        while let Some(observation) = self.updates.recv().await {
            if observation.revision == self.revision.load(Ordering::Acquire) {
                return Some(observation.content);
            }
        }
        None
    }
}

impl Drop for ClipboardService {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        // Do not block shutdown on an unresponsive external clipboard owner.
        if self.worker.as_ref().is_some_and(|w| w.is_finished()) {
            let _ = self.worker.take().unwrap().join();
        }
    }
}

#[cfg(not(target_os = "android"))]
fn read_content(clipboard: &mut arboard::Clipboard) -> Option<ClipboardContent> {
    if let Ok(image) = clipboard.get_image()
        && let Ok(content) = encode_image(image.width, image.height, &image.bytes)
    {
        return Some(content);
    }
    clipboard
        .get_text()
        .ok()
        .filter(|s| s.len() <= MAX_TEXT)
        .map(|s| ClipboardContent {
            kind: ClipboardKind::Text,
            bytes: s.into_bytes(),
        })
}

pub fn encode_image(width: usize, height: usize, rgba: &[u8]) -> Result<ClipboardContent> {
    ensure!(
        width > 0 && height > 0 && (width as u64).saturating_mul(height as u64) <= MAX_PIXELS,
        "clipboard image dimensions exceed the limit"
    );
    ensure!(
        width.checked_mul(height).and_then(|v| v.checked_mul(4)) == Some(rgba.len()),
        "invalid RGBA image size"
    );
    let mut png = Vec::new();
    image::codecs::png::PngEncoder::new(&mut png).write_image(
        rgba,
        width as u32,
        height as u32,
        image::ExtendedColorType::Rgba8,
    )?;
    ensure!(
        png.len() <= MAX_CLIPBOARD,
        "compressed clipboard image exceeds the limit"
    );
    Ok(ClipboardContent {
        kind: ClipboardKind::Png,
        bytes: png,
    })
}

#[cfg(not(target_os = "android"))]
fn write_content(clipboard: &mut arboard::Clipboard, content: &ClipboardContent) -> Result<()> {
    ensure!(content.valid(), "invalid clipboard content");
    match content.kind {
        ClipboardKind::Text => {
            clipboard.set_text(std::str::from_utf8(&content.bytes)?.to_owned())?
        }
        ClipboardKind::Png => {
            // Read IHDR dimensions before invoking the decoder to bound decompression memory.
            ensure!(
                content.bytes.len() >= 33 && &content.bytes[12..16] == b"IHDR",
                "missing PNG header"
            );
            let w = u32::from_be_bytes(content.bytes[16..20].try_into()?);
            let h = u32::from_be_bytes(content.bytes[20..24].try_into()?);
            ensure!(
                w > 0 && h > 0 && (w as u64) * (h as u64) <= MAX_PIXELS,
                "image dimensions exceed the limit"
            );
            let image =
                image::load_from_memory_with_format(&content.bytes, image::ImageFormat::Png)?
                    .to_rgba8();
            ensure!(
                image.width() == w && image.height() == h,
                "inconsistent PNG dimensions"
            );
            clipboard.set_image(arboard::ImageData {
                width: w as usize,
                height: h as usize,
                bytes: Cow::Owned(image.into_raw()),
            })?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn image_round_trip_retains_alpha() {
        let p = [255, 0, 0, 128, 0, 20, 40, 255];
        let c = encode_image(2, 1, &p).unwrap();
        let im = image::load_from_memory(&c.bytes).unwrap().to_rgba8();
        assert_eq!(im.as_raw(), &p);
    }
    #[test]
    fn invalid_image_sizes_fail_before_encoding() {
        assert!(encode_image(2, 2, &[0; 4]).is_err());
        assert!(encode_image(usize::MAX, 100, &[]).is_err());
    }

    fn service_fixture() -> (ClipboardService, mpsc::Sender<Observation>) {
        let (tx, updates) = mpsc::channel(4);
        let (apply, _) = watch::channel(None);
        (
            ClipboardService {
                updates,
                apply,
                revision: Arc::new(AtomicU64::new(0)),
                stop: Arc::new(AtomicBool::new(false)),
                worker: None,
            },
            tx,
        )
    }
    fn text(value: &str) -> ClipboardContent {
        ClipboardContent {
            kind: ClipboardKind::Text,
            bytes: value.as_bytes().to_vec(),
        }
    }

    #[tokio::test]
    async fn remote_apply_discards_already_queued_local_observations() {
        let (mut service, tx) = service_fixture();
        tx.send(Observation {
            revision: 0,
            content: text("stale local copy"),
        })
        .await
        .unwrap();
        service.apply(text("new remote copy"));
        tx.send(Observation {
            revision: 1,
            content: text("later local copy"),
        })
        .await
        .unwrap();
        assert_eq!(service.next_change().await, Some(text("later local copy")));
    }

    #[tokio::test]
    async fn delayed_old_reads_do_not_echo_and_cancelling_a_wait_does_not_lose_new_reads() {
        let (mut service, tx) = service_fixture();
        service.apply(text("remote one"));
        service.apply(text("remote two"));
        tx.send(Observation {
            revision: 1,
            content: text("late first read"),
        })
        .await
        .unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(20), service.next_change())
                .await
                .is_err()
        );
        tx.send(Observation {
            revision: 2,
            content: text("fresh user copy"),
        })
        .await
        .unwrap();
        assert_eq!(service.next_change().await, Some(text("fresh user copy")));
    }
}
