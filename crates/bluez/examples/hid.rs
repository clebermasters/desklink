//! Hardware check: presents this computer as a Bluetooth keyboard and mouse and plays reports
//! typed on stdin, without the rest of Desklink.
//!
//!   cargo run -p desklink-bluez --example hid -- HOST_FILE [pair]
//!
//! Each stdin line is either `wait MS`, `status`, or comma-separated `id:hex` reports, sent
//! `--gap` (default 8) milliseconds apart, e.g. `3:01000000000000,3:00000000000000` clicks.
use anyhow::{Context, Result, bail};
use desklink_bluez::{HidDevice, Options};
use desklink_hid::{Report, ReportSink};
use std::{path::PathBuf, time::Duration};
use tokio::io::{AsyncBufReadExt, BufReader};

fn parse(report: &str) -> Result<Report> {
    let (id, hex) = report.trim().split_once(':').context("expected id:hex")?;
    if hex.len() % 2 != 0 {
        bail!("odd hex length in {report}");
    }
    let data = (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Report {
        id: id.parse()?,
        data,
        position_only: false,
    })
}

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "desklink_bluez=debug".into()),
        )
        .init();
    let mut args = std::env::args().skip(1);
    let host_file = PathBuf::from(args.next().context("usage: hid HOST_FILE [pair]")?);
    let pair = args.next().as_deref() == Some("pair");
    let host = std::fs::read_to_string(&host_file)
        .ok()
        .map(|h| h.trim().to_string());
    let file = host_file.clone();
    let device = HidDevice::start(Options {
        name: "Desklink".into(),
        host,
        host_changed: Box::new(move |host| {
            let _ = std::fs::write(&file, host.unwrap_or(""));
        }),
    })
    .await?;
    if pair {
        println!("visible for 10 minutes: connect to this computer from the other one");
        println!("paired {}", device.pair(Duration::from_secs(600)).await?);
    }
    let mut sink = device.sink();
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    while let Some(line) = lines.next_line().await? {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(ms) = line.strip_prefix("wait ") {
            tokio::time::sleep(Duration::from_millis(ms.parse()?)).await;
        } else if line == "status" {
            println!("host={:?} connected={}", device.host(), device.connected());
        } else {
            let (gap, reports) = match line.strip_prefix("gap ") {
                Some(rest) => {
                    let (gap, reports) = rest.split_once(' ').context("gap MS reports")?;
                    (gap.parse()?, reports)
                }
                None => (8, line),
            };
            let mut sent = 0;
            for report in reports.split(',') {
                sink.send(parse(report)?)?;
                sent += 1;
                tokio::time::sleep(Duration::from_millis(gap)).await;
            }
            println!("sent {sent}");
        }
    }
    Ok(())
}
