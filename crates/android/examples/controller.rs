//! Headless controller for testing a phone without sharing this computer's real input.
//!
//! ```text
//! # 1. Print a pairing code for this test controller and pair the phone with it.
//! cargo run -p desklink-android --example controller -- --identity /tmp/ctl.json --print-code
//! # 2. Drive the phone (its pairing code supplies the key and address).
//! cargo run -p desklink-android --example controller -- --identity /tmp/ctl.json \
//!     --phone-code 'desklink://pair/…' --address 127.0.0.1:24880 \
//!     --script 'enter left 0.5; moveto 200 300; click; type Hello; leave left'
//! ```
//! Steps: `enter EDGE POS`, `move DX DY`, `moveto X Y`, `click [BUTTON]`, `type TEXT`,
//! `key EVDEV`, `chord EVDEV…`, `scroll V [H]`, `copy TEXT`, `expect-copy TEXT`, `wait MS`, `leave EDGE`.
#[path = "../tests/support/controller.rs"]
mod controller;

use anyhow::{Context, Result, bail, ensure};
use controller::Controller;
use desklink_protocol::{Edge, Input};
use desklink_settings::pairing::PairingCode;
use desklink_transport::{Identity, parse_public};
use std::{path::PathBuf, time::Duration};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    let mut identity_path = None;
    let mut phone_code = None;
    let mut address = None;
    let mut script = String::new();
    let mut print_code = false;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--identity" => identity_path = args.next().map(PathBuf::from),
            "--phone-code" => phone_code = args.next(),
            "--address" => address = args.next(),
            "--script" => script = args.next().unwrap_or_default(),
            "--print-code" => print_code = true,
            other => bail!("unknown argument {other}"),
        }
    }
    let identity_path = identity_path.context("--identity PATH is required")?;
    let identity = if identity_path.exists() {
        Identity::load(&identity_path)?
    } else {
        Identity::create(&identity_path)?
    };
    if print_code {
        println!(
            "{}",
            PairingCode {
                version: 1,
                name: "Test controller".into(),
                public_key: hex::encode(identity.public),
                address: address.unwrap_or_else(|| "10.0.2.2:24881".into()),
            }
            .encode()?
        );
        return Ok(());
    }
    let phone = PairingCode::decode(&phone_code.context("--phone-code is required")?)?;
    let key = parse_public(&phone.public_key)?;
    let address = address.unwrap_or(phone.address);
    let mut pc = Controller::connect(&address, identity, key).await?;
    println!(
        "connected; phone screen {}x{}",
        pc.phone_geometry.width, pc.phone_geometry.height
    );
    for step in script.split(';').map(str::trim).filter(|s| !s.is_empty()) {
        run(&mut pc, step)
            .await
            .with_context(|| format!("step `{step}` failed"))?;
        println!("ok: {step}");
    }
    pc.close();
    Ok(())
}

async fn run(pc: &mut Controller, step: &str) -> Result<()> {
    let (command, rest) = step.split_once(' ').unwrap_or((step, ""));
    let numbers = || -> Result<Vec<f64>> {
        rest.split_whitespace()
            .filter(|w| w.parse::<f64>().is_ok())
            .map(|w| Ok(w.parse()?))
            .collect()
    };
    let edge = || -> Result<Edge> {
        rest.split_whitespace()
            .next()
            .context("edge required")?
            .parse()
            .map_err(|e: String| anyhow::anyhow!(e))
    };
    match command {
        "enter" => {
            pc.enter(edge()?, *numbers()?.first().unwrap_or(&0.5))
                .await?
        }
        "move" => {
            let n = numbers()?;
            ensure!(n.len() == 2, "move DX DY");
            pc.move_by(n[0] as i32, n[1] as i32).await?;
        }
        "moveto" => {
            let n = numbers()?;
            ensure!(n.len() == 2, "moveto X Y");
            pc.move_to(n[0] as i32, n[1] as i32).await?;
        }
        "click" => pc.click(numbers()?.first().map_or(1, |b| *b as u8)).await?,
        "type" => pc.type_text(rest).await?,
        "key" => pc.key(rest.trim().parse()?).await?,
        "chord" => {
            let codes: Vec<u16> = rest
                .split_whitespace()
                .map(str::parse)
                .collect::<Result<_, _>>()?;
            for code in &codes {
                pc.input(Input::Key {
                    code: *code,
                    down: true,
                })?;
                tokio::time::sleep(Duration::from_millis(30)).await;
            }
            for code in codes.iter().rev() {
                pc.input(Input::Key {
                    code: *code,
                    down: false,
                })?;
            }
        }
        "scroll" => {
            let n = numbers()?;
            pc.input(Input::Scroll {
                vertical: *n.first().unwrap_or(&-1.0) as i32,
                horizontal: *n.get(1).unwrap_or(&0.0) as i32,
            })?;
        }
        "copy" => pc.copy_text(rest),
        "expect-copy" => {
            // A newly connected peer first receives the current clipboard; wait for this copy.
            let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
            loop {
                let left = deadline.saturating_duration_since(tokio::time::Instant::now());
                let content = pc.next_clipboard(left).await?;
                if content.bytes == rest.as_bytes() {
                    break;
                }
            }
        }
        "wait" => tokio::time::sleep(Duration::from_millis(rest.trim().parse()?)).await,
        "leave" => {
            let position = pc.leave_through(edge()?).await?;
            println!("control returned at position {position:.3}");
        }
        other => bail!("unknown step {other}"),
    }
    Ok(())
}
