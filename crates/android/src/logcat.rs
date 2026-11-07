//! Routes `tracing` output to Android's logcat. Log lines never contain clipboard data or keys.
use std::{
    ffi::{CString, c_char, c_int},
    io::{self, Write},
};
use tracing::{Level, Metadata};
use tracing_subscriber::fmt::MakeWriter;

#[link(name = "log")]
unsafe extern "C" {
    fn __android_log_write(priority: c_int, tag: *const c_char, text: *const c_char) -> c_int;
}

const TAG: &[u8] = b"Desklink\0";

pub struct Logcat;

pub struct Line {
    priority: c_int,
    buffer: Vec<u8>,
}

impl Write for Line {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.buffer.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Drop for Line {
    fn drop(&mut self) {
        let text: Vec<u8> = self.buffer.drain(..).filter(|b| *b != 0).collect();
        let text = String::from_utf8_lossy(&text);
        if let Ok(text) = CString::new(text.trim_end()) {
            // SAFETY: both pointers are valid NUL-terminated strings for the duration of the call.
            unsafe {
                __android_log_write(self.priority, TAG.as_ptr().cast(), text.as_ptr());
            }
        }
    }
}

impl<'a> MakeWriter<'a> for Logcat {
    type Writer = Line;
    fn make_writer(&'a self) -> Line {
        Line {
            priority: 4,
            buffer: Vec::with_capacity(160),
        }
    }
    fn make_writer_for(&'a self, meta: &Metadata<'_>) -> Line {
        let priority = match *meta.level() {
            Level::ERROR => 6,
            Level::WARN => 5,
            Level::INFO => 4,
            Level::DEBUG => 3,
            Level::TRACE => 2,
        };
        Line {
            priority,
            buffer: Vec::with_capacity(160),
        }
    }
}

/// Safe to call repeatedly; only the first call installs the subscriber.
pub fn init() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            "desklink=info,desklink_android=info,desklink_transport=info,desklink_core=info",
        )
        .with_writer(Logcat)
        .with_ansi(false)
        .without_time()
        .with_target(false)
        .try_init();
}
