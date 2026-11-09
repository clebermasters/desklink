//! Runs the real keyboard-and-mouse agent in-process: a headless controller drives a computer
//! that runs no Desklink software, and the phone turns its input into Bluetooth HID reports.
#[path = "support/controller.rs"]
mod controller;

use controller::Controller;
use desklink_android::{
    agent::{Agent, Receiver},
    bridge::{Action, Bridge, Taken},
    profile,
};
use desklink_hid::{Options, PointerMode, Report, descriptor};
use desklink_protocol::{Edge, Input};
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

/// Mirrors KeyboardSharing's pump: every report it would send over Bluetooth.
fn pump(bridge: Arc<Bridge>) -> mpsc::Receiver<Report> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        loop {
            match bridge.take(Duration::from_millis(200)) {
                Taken::Actions(actions) => {
                    for action in actions {
                        let Action::Report(report) = action else {
                            panic!("the keyboard agent only sends reports: {action:?}");
                        };
                        if tx.send(report).is_err() {
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

fn wait_for(rx: &mpsc::Receiver<Report>, what: &str, matches: impl Fn(&Report) -> bool) -> Report {
    let deadline = Instant::now() + Duration::from_secs(5);
    while let Some(left) = deadline.checked_duration_since(Instant::now()) {
        match rx.recv_timeout(left) {
            Ok(report) if matches(&report) => return report,
            Ok(_) => {}
            Err(_) => break,
        }
    }
    panic!("the Bluetooth computer never received: {what}");
}

fn position(report: &Report) -> (u16, u16) {
    let d = &report.data;
    (
        u16::from_le_bytes([d[0], d[1]]),
        u16::from_le_bytes([d[2], d[3]]),
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn controller_drives_a_bluetooth_computer_through_the_phone() {
    let dir = tempfile::tempdir().unwrap();
    profile::ensure_keyboard(dir.path(), "Work MacBook").unwrap();
    let port = free_port();
    Config::modify(&profile::config_path(dir.path()), |c| {
        c.listen = format!("127.0.0.1:{port}").parse()?;
        Ok(())
    })
    .unwrap();
    let controller_identity = Identity::create(&dir.path().join("controller.json")).unwrap();
    let code = PairingCode {
        version: 1,
        name: "Linux desktop".into(),
        public_key: hex::encode(controller_identity.public),
        address: "127.0.0.1:9".into(),
    }
    .encode()
    .unwrap();
    profile::pair_keyboard(dir.path(), &code).unwrap();
    let key = parse_public(&profile::info(dir.path(), None).unwrap().public_key).unwrap();

    let bridge = Arc::new(Bridge::new());
    let agent = Agent::new(bridge.clone());
    agent.set_receiver(Receiver::Keyboard(Options {
        width: 1512,
        height: 982,
        pointer: PointerMode::Absolute,
        swap_control_and_command: true,
    }));
    agent.start(dir.path()).unwrap();
    let reports = pump(bridge.clone());

    let mut pc = Controller::connect(&format!("127.0.0.1:{port}"), controller_identity, key)
        .await
        .unwrap();
    assert_eq!(
        (pc.phone_geometry.width, pc.phone_geometry.height),
        (1512, 982),
        "the controller lays out the Mac's screen, not the phone's"
    );

    pc.enter(Edge::Left, 0.5).await.unwrap();
    let entry = wait_for(&reports, "pointer placed at the left edge", |r| {
        r.id == descriptor::POINTER_ABSOLUTE
    });
    let (x, y) = position(&entry);
    assert!(x < 100 && (16_000..16_800).contains(&y), "{x},{y}");

    pc.move_to(756, 491).await.unwrap();
    wait_for(&reports, "pointer at the centre", |r| {
        r.id == descriptor::POINTER_ABSOLUTE && {
            let (x, y) = position(r);
            (16_300..16_450).contains(&x) && (16_300..16_450).contains(&y)
        }
    });
    // A left click is the pen touching down and lifting at the pointer, so drags stay exact.
    pc.click(1).await.unwrap();
    let press = wait_for(&reports, "left button down (pen tip)", |r| {
        r.id == descriptor::PEN && r.data[0] == 0b101
    });
    let (x, y) = (
        u16::from_le_bytes([press.data[1], press.data[2]]),
        u16::from_le_bytes([press.data[3], press.data[4]]),
    );
    assert!(
        (16_300..16_450).contains(&x) && (16_300..16_450).contains(&y),
        "{x},{y}"
    );
    wait_for(&reports, "left button up (pen out of range)", |r| {
        r.id == descriptor::PEN && r.data[0] == 0
    });
    pc.click(3).await.unwrap();
    wait_for(&reports, "right button down", |r| {
        r.id == descriptor::POINTER_RELATIVE && r.data == [0b10, 0, 0, 0, 0, 0, 0]
    });

    // Control+C from Linux arrives as Command+C on the Mac.
    pc.input(Input::Key {
        code: 29,
        down: true,
    })
    .unwrap();
    pc.input(Input::Key {
        code: 46,
        down: true,
    })
    .unwrap();
    wait_for(&reports, "Command+C", |r| {
        r.id == descriptor::KEYBOARD && r.data[0] == 0x08 && r.data[2] == 0x06
    });

    // A remote copy has nowhere to go and must not disturb input.
    pc.copy_text("not for the Mac");

    // Leave with keys still held: the Mac must see every key released.
    let position = pc.leave_through(Edge::Left).await.unwrap();
    assert!((0.0..=1.0).contains(&position));
    wait_for(&reports, "all keys released after return", |r| {
        r.id == descriptor::KEYBOARD && r.data.iter().all(|b| *b == 0)
    });

    assert!(
        agent
            .status(dir.path())
            .status
            .peers
            .iter()
            .any(|p| p.connected)
    );
    agent.stop();
    assert!(!agent.is_running());
}
