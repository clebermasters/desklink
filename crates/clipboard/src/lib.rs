//! Owns the OS clipboard on a dedicated thread. Never logs clipboard contents.
#![forbid(unsafe_code)]

use anyhow::{Context, Result, ensure};
use desklink_protocol::{ClipboardContent, ClipboardKind, MAX_CLIPBOARD, MAX_PIXELS, MAX_TEXT};
use image::ImageEncoder;
use std::{
    borrow::Cow,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};
use tokio::sync::{mpsc, watch};

pub struct ClipboardService {
    pub updates: mpsc::Receiver<ClipboardContent>,
    apply: watch::Sender<Option<Arc<ClipboardContent>>>,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}

impl ClipboardService {
    pub fn start() -> Result<Self> {
        let (tx, updates) = mpsc::channel(4);
        let (apply, mut rx) = watch::channel::<Option<Arc<ClipboardContent>>>(None);
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
                while !stop_worker.load(Ordering::Relaxed) {
                    if rx.has_changed().unwrap_or(false) {
                        let content = rx.borrow_and_update().clone();
                        if let Some(content) = content {
                            match write_content(&mut clipboard, &content) {
                                Ok(()) => {
                                    seen = Some(
                                        read_content(&mut clipboard)
                                            .map_or_else(|| content.hash(), |v| v.hash()),
                                    );
                                }
                                Err(e) => {
                                    tracing::warn!(error=%e,"could not apply remote clipboard")
                                }
                            }
                        }
                    }
                    if let Some(content) = read_content(&mut clipboard) {
                        let hash = content.hash();
                        if seen != Some(hash) && tx.try_send(content).is_ok() {
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
            stop,
            worker: Some(worker),
        })
    }
    pub fn apply(&self, content: ClipboardContent) {
        self.apply.send_replace(Some(Arc::new(content)));
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
}
