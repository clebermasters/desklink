//! The Bluetooth HID protocol (HIDP) as the device side speaks it: input reports go out on the
//! interrupt channel, and the host's requests on the control channel need short replies.
use desklink_hid::{Report, descriptor};

const DATA_INPUT: u8 = 0xa1;
const DATA_OTHER: u8 = 0xa0;
const DATA_OUTPUT: u8 = 0xa2;

const HANDSHAKE_SUCCESS: u8 = 0x00;
const HANDSHAKE_INVALID_REPORT_ID: u8 = 0x02;
const HANDSHAKE_UNSUPPORTED: u8 = 0x03;

const CONTROL: u8 = 0x1;
const GET_REPORT: u8 = 0x4;
const SET_REPORT: u8 = 0x5;
const GET_PROTOCOL: u8 = 0x6;
const SET_PROTOCOL: u8 = 0x7;
const GET_IDLE: u8 = 0x8;
const SET_IDLE: u8 = 0x9;
const DATA: u8 = 0xa;

const VIRTUAL_CABLE_UNPLUG: u8 = 0x5;
const REPORT_PROTOCOL: u8 = 0x1;

/// An input report as the interrupt channel carries it.
pub fn input(report: &Report) -> Vec<u8> {
    let mut frame = Vec::with_capacity(report.data.len() + 2);
    frame.extend([DATA_INPUT, report.id]);
    frame.extend_from_slice(&report.data);
    frame
}

/// What to do about one message from the host on the control channel.
#[derive(Debug, PartialEq, Eq)]
pub enum Control {
    Reply(Vec<u8>),
    /// The host removed this device ("virtual cable unplug"): forget it.
    Unplug,
    Ignore,
}

fn input_len(id: u8) -> Option<usize> {
    match id {
        descriptor::KEYBOARD => Some(descriptor::KEYBOARD_LEN),
        descriptor::POINTER_ABSOLUTE => Some(descriptor::ABSOLUTE_LEN),
        descriptor::POINTER_RELATIVE => Some(descriptor::RELATIVE_LEN),
        descriptor::PEN => Some(descriptor::PEN_LEN),
        _ => None,
    }
}

pub fn control(message: &[u8]) -> Control {
    let Some(&header) = message.first() else {
        return Control::Ignore;
    };
    let (kind, param) = (header >> 4, header & 0x0f);
    let handshake = |code| Control::Reply(vec![code]);
    match kind {
        CONTROL if param == VIRTUAL_CABLE_UNPLUG => Control::Unplug,
        CONTROL => Control::Ignore, // suspend and exit-suspend need no answer
        GET_REPORT => {
            let report_type = param & 0x03;
            let id = message.get(1).copied();
            match (report_type, id) {
                // Nothing is pressed or moving unless a report says so.
                (1, Some(id)) => match input_len(id) {
                    Some(len) => {
                        let mut reply = vec![DATA_OTHER | 1, id];
                        reply.resize(len + 2, 0);
                        Control::Reply(reply)
                    }
                    None => handshake(HANDSHAKE_INVALID_REPORT_ID),
                },
                (2, Some(descriptor::KEYBOARD)) => {
                    Control::Reply(vec![DATA_OTHER | 2, descriptor::KEYBOARD, 0])
                }
                _ => handshake(HANDSHAKE_INVALID_REPORT_ID),
            }
        }
        // Keyboard LEDs and the protocol choice need no action; the device only speaks the
        // report protocol, which every host uses with a report descriptor like this one.
        SET_REPORT | SET_PROTOCOL | SET_IDLE => handshake(HANDSHAKE_SUCCESS),
        GET_PROTOCOL => Control::Reply(vec![DATA_OTHER, REPORT_PROTOCOL]),
        GET_IDLE => Control::Reply(vec![DATA_OTHER, 0]),
        DATA => Control::Ignore,
        _ => handshake(HANDSHAKE_UNSUPPORTED),
    }
}

/// Output reports (keyboard LEDs) on the interrupt channel need no answer.
pub fn is_output(message: &[u8]) -> bool {
    message.first() == Some(&DATA_OUTPUT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_reports_are_framed_with_their_id() {
        let report = Report {
            id: descriptor::KEYBOARD,
            data: vec![0x08, 0, 0x06, 0, 0, 0, 0, 0],
            position_only: false,
        };
        assert_eq!(input(&report), [0xa1, 1, 0x08, 0, 0x06, 0, 0, 0, 0, 0]);
    }

    #[test]
    fn get_report_answers_with_an_idle_report_of_the_right_size() {
        assert_eq!(
            control(&[0x41, descriptor::POINTER_RELATIVE]),
            Control::Reply(vec![0xa1, 3, 0, 0, 0, 0, 0, 0, 0])
        );
        assert_eq!(
            control(&[0x49, descriptor::KEYBOARD, 0x09, 0]),
            Control::Reply(vec![0xa1, 1, 0, 0, 0, 0, 0, 0, 0, 0])
        );
        assert_eq!(control(&[0x41, 9]), Control::Reply(vec![0x02]));
        assert_eq!(
            control(&[0x43, 1]),
            Control::Reply(vec![0x02]),
            "no features"
        );
    }

    #[test]
    fn settings_succeed_and_the_protocol_is_report_mode() {
        assert_eq!(control(&[0x52, 1, 0x02]), Control::Reply(vec![0]));
        assert_eq!(control(&[0x71]), Control::Reply(vec![0]));
        assert_eq!(control(&[0x60]), Control::Reply(vec![0xa0, 1]));
    }

    #[test]
    fn unplug_forgets_and_unknown_requests_are_refused() {
        assert_eq!(control(&[0x15]), Control::Unplug);
        assert_eq!(control(&[0x13]), Control::Ignore);
        assert_eq!(control(&[0x30]), Control::Reply(vec![0x03]));
        assert_eq!(control(&[]), Control::Ignore);
        assert!(is_output(&[0xa2, 1, 0x02]));
    }
}
