//! Runs the real Android agent in-process against a headless controller over loopback TCP.
#[path = "support/controller.rs"]
mod controller;

use controller::Controller;
use desklink_android::{
    agent::Agent,
    bridge::{Action, Bridge, Taken},
    profile,
};
use desklink_protocol::{ClipboardContent, ClipboardKind, Edge};
use desklink_settings::{Config, pairing::PairingCode};
use desklink_transport::{Identity, parse_public};
use std::{
    sync::{Arc, mpsc},
    thread,
    time::{Duration, Instant},
};

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

/// Mirrors the app's input pump: drains the bridge on its own thread.
fn pump(bridge: Arc<Bridge>) -> mpsc::Receiver<Action> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        loop {
            match bridge.take(Duration::from_millis(200)) {
                Taken::Actions(actions) => {
                    for action in actions {
                        if tx.send(action).is_err() {
                            return;
                        }
                    }
                }
                Taken::Timeout => {}
                Taken::Closed => return,
            }
        }
    });
    rx
}

fn wait_for(rx: &mpsc::Receiver<Action>, what: &str, matches: impl Fn(&Action) -> bool) -> Action {
    let deadline = Instant::now() + Duration::from_secs(5);
    while let Some(left) = deadline.checked_duration_since(Instant::now()) {
        match rx.recv_timeout(left) {
            Ok(action) if matches(&action) => return action,
            Ok(_) => {}
            Err(_) => break,
        }
    }
    panic!("the phone never performed: {what}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn phone_receives_input_and_clipboard_then_returns_control() {
    let dir = tempfile::tempdir().unwrap();
    profile::ensure(dir.path(), "Test phone").unwrap();
    let port = free_port();
    Config::modify(&profile::config_path(dir.path()), |c| {
        c.listen = format!("127.0.0.1:{port}").parse()?;
        Ok(())
    })
    .unwrap();

    let controller_identity = Identity::create(&dir.path().join("controller.json")).unwrap();
    let code = PairingCode {
        version: 1,
        name: "Laptop".into(),
        public_key: hex::encode(controller_identity.public),
        // Nothing listens here; the phone's own outbound attempts fail and inbound wins.
        address: "127.0.0.1:9".into(),
    }
    .encode()
    .unwrap();
    profile::pair(dir.path(), &code, false).unwrap();
    let phone_key = parse_public(&profile::info(dir.path(), None).unwrap().public_key).unwrap();

    let bridge = Arc::new(Bridge::new());
    let agent = Agent::new(bridge.clone());
    assert!(agent.start(dir.path()).is_err(), "screen size is required");
    bridge.set_geometry(400, 800).unwrap();
    agent.start(dir.path()).unwrap();
    agent.start(dir.path()).unwrap(); // idempotent
    let actions = pump(bridge.clone());

    let mut pc = Controller::connect(&format!("127.0.0.1:{port}"), controller_identity, phone_key)
        .await
        .unwrap();
    assert_eq!(pc.phone_geometry.width, 400);

    pc.enter(Edge::Left, 0.5).await.unwrap();
    wait_for(&actions, "pointer appears at the left edge", |a| {
        *a == Action::Move { x: 1, y: 400 }
    });

    pc.move_to(200, 300).await.unwrap();
    wait_for(&actions, "pointer follows the mouse", |a| {
        *a == Action::Move { x: 200, y: 300 }
    });
    pc.click(1).await.unwrap();
    wait_for(&actions, "left button press", |a| {
        *a == Action::Button {
            button: 1,
            down: true,
        }
    });
    wait_for(&actions, "left button release", |a| {
        *a == Action::Button {
            button: 1,
            down: false,
        }
    });
    pc.type_text("A").await.unwrap();
    let key = wait_for(&actions, "shifted A", |a| {
        matches!(
            a,
            Action::Key {
                keycode: 29,
                down: true,
                ..
            }
        )
    });
    assert!(matches!(key, Action::Key { meta, .. } if meta & 1 == 1));

    pc.copy_text("from the laptop");
    wait_for(&actions, "remote copy reaches the phone clipboard", |a| {
        *a == Action::Clipboard(ClipboardContent {
            kind: ClipboardKind::Text,
            bytes: b"from the laptop".to_vec(),
        })
    });

    bridge
        .local_clipboard(ClipboardContent {
            kind: ClipboardKind::Text,
            bytes: b"from the phone".to_vec(),
        })
        .unwrap();
    let copied = pc.next_clipboard(Duration::from_secs(5)).await.unwrap();
    assert_eq!(copied.bytes, b"from the phone");

    // Hold Control, then leave: the phone must hide its pointer and release the key.
    pc.input(desklink_protocol::Input::Key {
        code: 29,
        down: true,
    })
    .unwrap();
    let position = pc.leave_through(Edge::Left).await.unwrap();
    assert!((0.0..=1.0).contains(&position));
    wait_for(&actions, "pointer hidden after return", |a| {
        *a == Action::Hide
    });
    wait_for(&actions, "held Control released", |a| {
        matches!(
            a,
            Action::Key {
                keycode: 113,
                down: false,
                ..
            }
        )
    });

    let status = agent.status(dir.path());
    assert!(status.running);
    assert!(status.status.peers.iter().any(|p| p.connected));

    pc.close();
    agent.stop();
    assert!(!agent.is_running());
    assert!(agent.status(dir.path()).error.is_none());
    // A stopped agent can start again on the same port and control socket.
    agent.start(dir.path()).unwrap();
    assert!(agent.is_running());
    agent.stop();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rotation_reaches_connected_and_new_peers() {
    let dir = tempfile::tempdir().unwrap();
    profile::ensure(dir.path(), "Rotating phone").unwrap();
    let port = free_port();
    Config::modify(&profile::config_path(dir.path()), |c| {
        c.listen = format!("127.0.0.1:{port}").parse()?;
        Ok(())
    })
    .unwrap();
    let path = dir.path().join("controller.json");
    let controller_identity = Identity::create(&path).unwrap();
    let code = PairingCode {
        version: 1,
        name: "Laptop".into(),
        public_key: hex::encode(controller_identity.public),
        address: "127.0.0.1:9".into(),
    }
    .encode()
    .unwrap();
    profile::pair(dir.path(), &code, false).unwrap();
    let phone_key = parse_public(&profile::info(dir.path(), None).unwrap().public_key).unwrap();
    let bridge = Arc::new(Bridge::new());
    bridge.set_geometry(400, 800).unwrap();
    let agent = Agent::new(bridge.clone());
    agent.start(dir.path()).unwrap();
    let address = format!("127.0.0.1:{port}");

    let mut first = Controller::connect(&address, controller_identity, phone_key)
        .await
        .unwrap();
    assert_eq!(
        (first.phone_geometry.width, first.phone_geometry.height),
        (400, 800)
    );
    bridge.set_geometry(800, 400).unwrap();
    let deadline = Instant::now() + Duration::from_secs(6);
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match first.next_message(left).await.unwrap() {
            desklink_protocol::Message::Geometry(g) => {
                assert_eq!((g.width, g.height), (800, 400));
                break;
            }
            _ => continue,
        }
    }
    first.close();
    tokio::time::sleep(Duration::from_millis(300)).await;
    let second = Controller::connect(&address, Identity::load(&path).unwrap(), phone_key)
        .await
        .unwrap();
    assert_eq!(
        (second.phone_geometry.width, second.phone_geometry.height),
        (800, 400),
        "a connection made after rotation advertises the new size"
    );
    second.close();
    agent.stop();
}
