//! `desklink bluetooth`: this Linux computer controls a nearby computer that runs no Desklink
//! software by presenting itself to it as a Bluetooth keyboard and mouse.
//!
//! The controlled computer gets a receiver profile of its own (`<config dir>/bluetooth/`): an
//! identity, a configuration that only trusts this computer's main profile, and the keyboard
//! settings. The main profile pairs with it over loopback, so the layout, edge crossing and
//! hotkeys work exactly as with any other computer.
use anyhow::{Context, Result, bail, ensure};
use clap::Subcommand;
use desklink::{
    config::{self, Config, Peer},
    runtime::{Host, run_with_host},
};
use desklink_bluez::{HidDevice, Options as DeviceOptions};
use desklink_hid::{HidDesktop, Options, PointerMode};
use desklink_protocol::{DEFAULT_PORT, Edge, Geometry, Layout, Screen};
use desklink_transport::{Identity, parse_public};
use serde::{Deserialize, Serialize};
use std::{
    net::{Ipv4Addr, SocketAddr},
    path::{Path, PathBuf},
    time::Duration,
};
use tokio_util::sync::CancellationToken;

/// Loopback only: the receiver is reachable from this computer's main profile and nowhere else.
pub const LISTEN: SocketAddr =
    SocketAddr::new(std::net::IpAddr::V4(Ipv4Addr::LOCALHOST), DEFAULT_PORT + 1);
const PAIR_WINDOW: Duration = Duration::from_secs(120);
/// The receiver polls nothing locally; input arrives from the network immediately.
const TICK: Duration = Duration::from_millis(20);

#[derive(Subcommand)]
pub enum Command {
    /// Add a computer to control over Bluetooth, placed at a screen edge.
    Setup {
        /// Name shown in the layout, e.g. "Work MacBook".
        #[arg(long)]
        name: String,
        /// Its screen size in its own units (macOS: "looks like" resolution), e.g. 1512x982.
        #[arg(long, value_parser = parse_size)]
        size: (u32, u32),
        #[arg(long)]
        edge: Option<Edge>,
        /// Take the place of a paired computer, by name or key, e.g. the same Mac reached
        /// through a phone. That computer stays paired but leaves the layout.
        #[arg(long)]
        replace: Option<String>,
        /// Move the pointer with deltas instead of exact positions (hosts without absolute
        /// pointer support).
        #[arg(long)]
        relative: bool,
        /// Keep Control and Command as they are (the default swaps them for a Mac).
        #[arg(long)]
        no_swap: bool,
    },
    /// Make this computer visible for two minutes so the other computer can pair, then run.
    Pair,
    /// Run the keyboard and mouse for the paired computer. Keep `desklink run` (or the
    /// desktop app) running too: that is what captures this computer's keyboard and mouse.
    Run,
    /// Show the setup, the paired computer and what this computer is missing.
    Status,
    /// Remove the Bluetooth computer and its receiver profile.
    Remove,
}

fn parse_size(text: &str) -> Result<(u32, u32), String> {
    let (w, h) = text
        .split_once(['x', 'X'])
        .ok_or_else(|| "expected WIDTHxHEIGHT, e.g. 1512x982".to_string())?;
    let parse = |v: &str| v.trim().parse::<u32>().map_err(|e| e.to_string());
    let (w, h) = (parse(w)?, parse(h)?);
    if !(16..=16_384).contains(&w) || !(16..=16_384).contains(&h) {
        return Err("each side must be between 16 and 16384".into());
    }
    Ok((w, h))
}

/// How the receiver presents itself; `host` is remembered after pairing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    pub width: u32,
    pub height: u32,
    #[serde(default)]
    pub relative: bool,
    #[serde(default = "yes")]
    pub swap_control_and_command: bool,
    #[serde(default)]
    pub host: Option<String>,
}

fn yes() -> bool {
    true
}

impl Settings {
    fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path).with_context(|| {
            format!(
                "no Bluetooth computer set up ({}); run `desklink bluetooth setup` first",
                path.display()
            )
        })?;
        toml::from_str(&text).with_context(|| format!("invalid {}", path.display()))
    }
    fn save(&self, path: &Path) -> Result<()> {
        std::fs::write(path, toml::to_string_pretty(self)?)
            .with_context(|| format!("cannot write {}", path.display()))
    }
    fn options(&self) -> Options {
        Options {
            width: self.width,
            height: self.height,
            pointer: if self.relative {
                PointerMode::Relative
            } else {
                PointerMode::Absolute
            },
            swap_control_and_command: self.swap_control_and_command,
        }
    }
}

pub struct Paths {
    /// The main profile, which captures this computer's input.
    pub main: PathBuf,
    /// The receiver profile's configuration.
    pub receiver: PathBuf,
    pub settings: PathBuf,
}

impl Paths {
    pub fn new(main: &Path) -> Self {
        let dir = main
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("bluetooth");
        Self {
            main: main.to_path_buf(),
            receiver: dir.join("config.toml"),
            settings: dir.join("keyboard.toml"),
        }
    }
}

/// Creates the receiver profile and pairs it with the main one in both directions.
pub fn setup(
    paths: &Paths,
    name: &str,
    settings: Settings,
    edge: Option<Edge>,
    replace: Option<&str>,
) -> Result<Vec<String>> {
    let main_key = Identity::load(&config::identity_path(&paths.main))
        .context("run `desklink init` first: this computer has no identity yet")?
        .public;
    let main = Config::load(&paths.main)?;
    let dir = paths.receiver.parent().context("invalid profile path")?;
    std::fs::create_dir_all(dir)?;
    let id_path = config::identity_path(&paths.receiver);
    let receiver = if id_path.exists() {
        Identity::load(&id_path)?
    } else {
        Identity::create(&id_path)?
    };
    let main_hex = hex::encode(main_key);
    let receiver_hex = hex::encode(receiver.public);
    let mut receiver_config = Config::new(name.to_string(), LISTEN);
    receiver_config.clipboard = false; // nothing on the other computer could use one
    receiver_config.peers = vec![Peer {
        name: main.name.clone(),
        public_key: main_hex.clone(),
        address: None, // it connects to us
        edge: None,
    }];
    receiver_config.layout_owner = Some(main_hex);
    receiver_config.validate_for(receiver.public)?;
    receiver_config.save(&paths.receiver)?;

    let mut warnings = Vec::new();
    Config::modify(&paths.main, |config| {
        config
            .peers
            .retain(|p| parse_public(&p.public_key).ok() != Some(receiver.public));
        let replaced = match replace {
            Some(which) => {
                let peer = config
                    .peers
                    .iter_mut()
                    .find(|p| p.public_key == which || p.name.eq_ignore_ascii_case(which))
                    .with_context(|| format!("no paired computer called {which:?}"))?;
                // Its place goes to the new screen; its pairing stays, unplaced.
                let edge = peer.edge.take();
                Some((peer.public_key.clone(), edge))
            }
            None => None,
        };
        let edge = edge.or_else(|| replaced.as_ref().and_then(|(_, edge)| *edge));
        for other in &config.peers {
            if edge.is_some() && other.edge == edge {
                bail!(
                    "\"{}\" already sits at that edge. If it is the same computer through a \
                     phone, take its place with --replace \"{}\", or pick another edge",
                    other.name,
                    other.name
                );
            }
            if replaced.is_none() && other.name.eq_ignore_ascii_case(name) {
                warnings.push(format!(
                    "\"{}\" is already paired. If it is the same computer through a phone, \
                     run setup again with --replace \"{}\"",
                    other.name, other.name
                ));
            }
        }
        if let Some(layout) = &mut config.layout {
            place(
                layout,
                &hex::encode(main_key),
                &receiver_hex,
                name,
                edge,
                replaced.as_ref().map(|(key, _)| key.as_str()),
                &settings,
            )?;
        }
        config.peers.push(Peer {
            name: name.to_string(),
            public_key: receiver_hex,
            address: Some(LISTEN.to_string()),
            edge,
        });
        config.validate_for(main_key)
    })?;
    // Keep a pairing made earlier: it belongs to the same computer.
    let host = Settings::load(&paths.settings).ok().and_then(|s| s.host);
    Settings { host, ..settings }.save(&paths.settings)?;
    Ok(warnings)
}

/// Puts the new screen into a saved layout (which decides routing over peer edges): in the
/// replaced computer's place, or beside this computer's screen at `edge`.
fn place(
    layout: &mut Layout,
    own: &str,
    id: &str,
    name: &str,
    edge: Option<Edge>,
    replaced: Option<&str>,
    settings: &Settings,
) -> Result<()> {
    layout.screens.retain(|s| s.id != id);
    if let Some(old) = replaced.and_then(|key| layout.screen_mut(key)) {
        old.id = id.to_string();
        old.name = name.to_string();
    } else {
        let edge = edge.context(
            "the layout is arranged already: give --edge, or --replace the computer whose place \
             this one takes",
        )?;
        let own = layout
            .screen(own)
            .context("this computer is missing from its own layout")?
            .rect;
        // Same height as this screen, the controlled screen's proportions.
        let height = own.height.max(1);
        let width = (height as u64 * settings.width as u64 / settings.height.max(1) as u64)
            .clamp(1, i32::MAX as u64) as u32;
        let (x, y) = match edge {
            Edge::Left => (own.x - width as i32, own.y),
            Edge::Right => (own.x + own.width as i32, own.y),
            Edge::Top => (own.x, own.y - height as i32),
            Edge::Bottom => (own.x, own.y + own.height as i32),
        };
        layout.screens.push(Screen {
            id: id.to_string(),
            name: name.to_string(),
            rect: Geometry {
                x,
                y,
                width,
                height,
            },
        });
    }
    layout.revision += 1;
    layout.validate().map_err(|error| {
        anyhow::anyhow!(
            "{error} Another computer already sits there: use --replace \"NAME\", another \
             --edge, or arrange it in the layout editor"
        )
    })
}

pub fn remove(paths: &Paths) -> Result<()> {
    let id_path = config::identity_path(&paths.receiver);
    if let Ok(identity) = Identity::load(&id_path) {
        let id = hex::encode(identity.public);
        Config::modify(&paths.main, |config| {
            config.peers.retain(|p| p.public_key != id);
            if let Some(layout) = &mut config.layout
                && layout.screen(&id).is_some()
            {
                layout.screens.retain(|s| s.id != id);
                layout.revision += 1;
                // A computer placed only through this one would be cut off: keep the
                // arrangement valid by falling back to peer edges.
                if layout.validate().is_err() {
                    config.layout = None;
                }
            }
            Ok(())
        })?;
    }
    let dir = paths.receiver.parent().context("invalid profile path")?;
    if dir.exists() {
        std::fs::remove_dir_all(dir)?;
    }
    Ok(())
}

pub fn status(paths: &Paths) -> serde_json::Value {
    let settings = Settings::load(&paths.settings).ok();
    serde_json::json!({
        "configured": settings.is_some(),
        "profile": paths.receiver,
        "screen": settings.as_ref().map(|s| format!("{}x{}", s.width, s.height)),
        "paired_host": settings.as_ref().and_then(|s| s.host.clone()),
        "pointer": settings.as_ref().map(|s| if s.relative { "relative" } else { "absolute" }),
        "swap_control_and_command": settings.as_ref().map(|s| s.swap_control_and_command),
        "bluetoothd_input_plugin": match desklink_bluez::input_plugin_active() {
            Some(true) => "active: run `sudo scripts/linux-bluetooth-hid.sh enable` once",
            Some(false) => "disabled (ready)",
            None => "bluetoothd not running",
        },
        "low_latency_helper": Path::new(desklink_bluez::qos::INSTALLED_HELPER).is_file(),
    })
}

/// Runs the receiver until interrupted; with `pair`, first accepts a new host.
pub async fn run(paths: &Paths, pair: bool) -> Result<()> {
    let settings = Settings::load(&paths.settings)?;
    let config = Config::load(&paths.receiver)?;
    let _lock = config::lock_profile(&paths.receiver)?;
    let identity = Identity::load(&config::identity_path(&paths.receiver))?;
    ensure!(
        settings.host.is_some() || pair,
        "no computer paired yet: run `desklink bluetooth pair`"
    );
    let settings_path = paths.settings.clone();
    let device = HidDevice::start(DeviceOptions {
        name: config.name.clone(),
        host: settings.host.clone(),
        host_changed: Box::new(move |host| {
            let saved = Settings::load(&settings_path).and_then(|mut s| {
                s.host = host.map(str::to_string);
                s.save(&settings_path)
            });
            if let Err(error) = saved {
                tracing::warn!(error = %format!("{error:#}"), "cannot remember the paired computer");
            }
        }),
    })
    .await?;
    if pair {
        println!(
            "This computer is visible over Bluetooth for {} seconds. On the other computer, open \
             its Bluetooth settings and connect to it.",
            PAIR_WINDOW.as_secs()
        );
        let host = device.pair(PAIR_WINDOW).await?;
        println!("Paired {host}.");
    }
    let desktop = HidDesktop::new(device.sink(), settings.options())?;
    println!(
        "Controlling \"{}\" over Bluetooth. Keep `desklink run` running; Ctrl+C stops.",
        config.name
    );
    let result = run_with_host(
        config,
        identity,
        paths.receiver.clone(),
        Host {
            desktop: Box::new(desktop),
            tick: TICK,
            handle_signals: true,
            shutdown: CancellationToken::new(),
        },
    )
    .await;
    drop(device);
    result
}

pub async fn execute(main: &Path, command: Command) -> Result<()> {
    let paths = Paths::new(main);
    match command {
        Command::Setup {
            name,
            size: (width, height),
            edge,
            replace,
            relative,
            no_swap,
        } => {
            let name = name.trim();
            if name.is_empty() || name.len() > 64 || name.chars().any(char::is_control) {
                bail!("the name must be 1 to 64 printable characters");
            }
            let settings = Settings {
                width,
                height,
                relative,
                swap_control_and_command: !no_swap,
                host: None,
            };
            for warning in setup(&paths, name, settings, edge, replace.as_deref())? {
                eprintln!("warning: {warning}");
            }
            println!(
                "Added \"{name}\". Next: `desklink bluetooth pair`, then restart `desklink run` \
                 (or apply in the app) so it connects."
            );
            if desklink_bluez::input_plugin_active() == Some(true) {
                println!("First, once: sudo scripts/linux-bluetooth-hid.sh enable");
            }
        }
        Command::Pair => run(&paths, true).await?,
        Command::Run => run(&paths, false).await?,
        Command::Status => println!("{}", serde_json::to_string_pretty(&status(&paths))?),
        Command::Remove => {
            remove(&paths)?;
            println!(
                "Removed. Also remove this computer in the other computer's Bluetooth settings."
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn main_profile(dir: &Path) -> PathBuf {
        let path = dir.join("config.toml");
        Identity::create(&config::identity_path(&path)).unwrap();
        Config::new("Linux desktop".into(), "0.0.0.0:24880".parse().unwrap())
            .save(&path)
            .unwrap();
        path
    }

    fn settings() -> Settings {
        Settings {
            width: 1512,
            height: 982,
            relative: false,
            swap_control_and_command: true,
            host: None,
        }
    }

    #[test]
    fn sizes_parse_and_are_bounded() {
        assert_eq!(parse_size("1512x982"), Ok((1512, 982)));
        assert_eq!(parse_size("1920X1080"), Ok((1920, 1080)));
        assert!(parse_size("1512").is_err());
        assert!(parse_size("0x982").is_err());
        assert!(parse_size("99999x10").is_err());
    }

    #[test]
    fn setup_pairs_both_profiles_over_loopback_only() {
        let dir = tempfile::tempdir().unwrap();
        let main = main_profile(dir.path());
        let paths = Paths::new(&main);
        let warnings = setup(&paths, "Work MacBook", settings(), Some(Edge::Right), None).unwrap();
        assert!(warnings.is_empty());

        let main_key = hex::encode(
            Identity::load(&config::identity_path(&main))
                .unwrap()
                .public,
        );
        let receiver = Config::load(&paths.receiver).unwrap();
        assert_eq!(receiver.listen, LISTEN);
        assert!(receiver.listen.ip().is_loopback());
        assert!(!receiver.clipboard);
        assert_eq!(receiver.peers.len(), 1);
        assert_eq!(receiver.peers[0].public_key, main_key);
        assert_eq!(receiver.layout_owner.as_deref(), Some(main_key.as_str()));

        let controller = Config::load(&main).unwrap();
        let peer = &controller.peers[0];
        assert_eq!(peer.name, "Work MacBook");
        assert_eq!(peer.address.as_deref(), Some("127.0.0.1:24881"));
        assert_eq!(peer.edge, Some(Edge::Right));

        // Running setup again keeps one peer and the remembered host.
        let mut paired = Settings::load(&paths.settings).unwrap();
        paired.host = Some("FC:B2:14:8D:0B:69".into());
        paired.save(&paths.settings).unwrap();
        setup(&paths, "Work MacBook", settings(), Some(Edge::Right), None).unwrap();
        assert_eq!(Config::load(&main).unwrap().peers.len(), 1);
        assert_eq!(
            Settings::load(&paths.settings).unwrap().host.as_deref(),
            Some("FC:B2:14:8D:0B:69")
        );

        remove(&paths).unwrap();
        assert!(Config::load(&main).unwrap().peers.is_empty());
        assert!(!paths.receiver.exists());
    }

    /// The main profile with the same Mac reached through the phone, at the left, in a saved
    /// layout (which decides routing).
    fn with_phone_keyboard(dir: &Path) -> (PathBuf, String) {
        let main = main_profile(dir);
        let own = hex::encode(
            Identity::load(&config::identity_path(&main))
                .unwrap()
                .public,
        );
        let phone = hex::encode(Identity::create(&dir.join("other.json")).unwrap().public);
        Config::modify(&main, |c| {
            c.peers.push(Peer {
                name: "Computer (Bluetooth)".into(),
                public_key: phone.clone(),
                address: Some("192.168.0.187:24881".into()),
                edge: Some(Edge::Left),
            });
            let mut layout = c.editable_layout(parse_public(&own).unwrap());
            layout.revision = 1;
            c.layout = Some(layout);
            Ok(())
        })
        .unwrap();
        (main, phone)
    }

    #[test]
    fn setup_refuses_an_occupied_edge_and_warns_about_duplicates() {
        let dir = tempfile::tempdir().unwrap();
        let (main, _) = with_phone_keyboard(dir.path());
        let paths = Paths::new(&main);
        let error = setup(&paths, "Work MacBook", settings(), Some(Edge::Left), None).unwrap_err();
        assert!(error.to_string().contains("--replace"), "{error}");
        assert_eq!(Config::load(&main).unwrap().peers.len(), 1, "nothing added");
        let warnings = setup(
            &paths,
            "computer (bluetooth)",
            settings(),
            Some(Edge::Right),
            None,
        )
        .unwrap();
        assert_eq!(warnings.len(), 1, "same name: probably the same computer");
    }

    #[test]
    fn setup_takes_the_replaced_computers_place_in_the_layout() {
        let dir = tempfile::tempdir().unwrap();
        let (main, phone) = with_phone_keyboard(dir.path());
        let paths = Paths::new(&main);
        let before = Config::load(&main).unwrap().layout.unwrap();
        let old_rect = before.screen(&phone).unwrap().rect;
        setup(
            &paths,
            "Work MacBook",
            settings(),
            None,
            Some("Computer (Bluetooth)"),
        )
        .unwrap();

        let config = Config::load(&main).unwrap();
        let receiver = hex::encode(
            Identity::load(&config::identity_path(&paths.receiver))
                .unwrap()
                .public,
        );
        let layout = config.layout.as_ref().unwrap();
        assert_eq!(
            layout.screen(&receiver).unwrap().rect,
            old_rect,
            "same place"
        );
        assert!(
            layout.screen(&phone).is_none(),
            "the phone keyboard left the layout"
        );
        assert!(layout.revision > before.revision);
        let own = hex::encode(
            Identity::load(&config::identity_path(&main))
                .unwrap()
                .public,
        );
        let crossing = layout.crossing(&own, Edge::Left, 0.5).unwrap();
        assert_eq!(
            crossing.target, receiver,
            "crossing left reaches the new Mac"
        );
        let peer = |key: &str| config.peers.iter().find(|p| p.public_key == key).unwrap();
        assert_eq!(peer(&phone).edge, None, "still paired, unplaced");
        assert_eq!(peer(&receiver).edge, Some(Edge::Left), "inherits the edge");

        // Removing it again keeps a valid arrangement.
        remove(&paths).unwrap();
        let config = Config::load(&main).unwrap();
        assert!(config.layout.is_none_or(|l| l.validate().is_ok()));
    }

    #[test]
    fn setup_places_a_new_screen_beside_this_one_in_a_saved_layout() {
        let dir = tempfile::tempdir().unwrap();
        let (main, _) = with_phone_keyboard(dir.path());
        let paths = Paths::new(&main);
        setup(&paths, "Linux laptop", settings(), Some(Edge::Top), None).unwrap();
        let config = Config::load(&main).unwrap();
        let layout = config.layout.unwrap();
        let own = hex::encode(
            Identity::load(&config::identity_path(&main))
                .unwrap()
                .public,
        );
        let own_rect = layout.screen(&own).unwrap().rect;
        let target = layout.crossing(&own, Edge::Top, 0.5).unwrap().target;
        let rect = layout.screen(&target).unwrap().rect;
        assert_eq!(rect.height, own_rect.height);
        assert_eq!(
            rect.y + rect.height as i32,
            own_rect.y,
            "touches the top edge"
        );
        assert_eq!(rect.width, own_rect.height * 1512 / 982, "Mac proportions");
        assert!(
            setup(&paths, "Another", settings(), None, None).is_err(),
            "needs a place"
        );
    }

    #[test]
    fn settings_default_to_an_exact_pointer_and_mac_shortcuts() {
        let settings: Settings = toml::from_str("width = 1512\nheight = 982\n").unwrap();
        assert_eq!(settings.options().pointer, PointerMode::Absolute);
        assert!(settings.swap_control_and_command);
        assert_eq!(settings.host, None);
    }
}
