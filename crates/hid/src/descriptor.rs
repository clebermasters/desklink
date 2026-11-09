//! The HID report descriptor: one keyboard and two pointers, distinguished by report ID.
//!
//! Hosts cache the descriptor when they pair, so both pointer styles are always present and
//! the choice between them can change without pairing again.

/// 8 bytes: modifier bits, reserved, six key usages. LED output report: 1 byte.
pub const KEYBOARD: u8 = 1;
/// 6 bytes: X and Y (0..=32767, absolute), wheel, horizontal pan.
pub const POINTER_ABSOLUTE: u8 = 2;
/// 7 bytes: buttons, X and Y (i16, relative), wheel, horizontal pan.
pub const POINTER_RELATIVE: u8 = 3;

/// 5 bytes: tip, barrel and in-range bits, X and Y (0..=32767, absolute). macOS supports
/// absolute pens with a pressed tip, unlike absolute mice, so left-button drags stay exact.
pub const PEN: u8 = 4;

pub const KEYBOARD_LEN: usize = 8;
pub const PEN_LEN: usize = 5;
pub const ABSOLUTE_LEN: usize = 6;
pub const RELATIVE_LEN: usize = 7;
/// Largest absolute coordinate; positions are scaled from the screen size onto 0..=ABS_MAX.
pub const ABS_MAX: u16 = 0x7fff;

#[rustfmt::skip]
pub const DESCRIPTOR: &[u8] = &[
    // Keyboard
    0x05, 0x01,       // Usage Page (Generic Desktop)
    0x09, 0x06,       // Usage (Keyboard)
    0xa1, 0x01,       // Collection (Application)
    0x85, KEYBOARD,   //   Report ID
    0x05, 0x07,       //   Usage Page (Keyboard/Keypad)
    0x19, 0xe0,       //   Usage Minimum (Left Control)
    0x29, 0xe7,       //   Usage Maximum (Right GUI)
    0x15, 0x00,       //   Logical Minimum (0)
    0x25, 0x01,       //   Logical Maximum (1)
    0x75, 0x01,       //   Report Size (1)
    0x95, 0x08,       //   Report Count (8)
    0x81, 0x02,       //   Input (Data, Variable, Absolute): modifier bits
    0x95, 0x01,       //   Report Count (1)
    0x75, 0x08,       //   Report Size (8)
    0x81, 0x01,       //   Input (Constant): reserved
    0x95, 0x05,       //   Report Count (5)
    0x75, 0x01,       //   Report Size (1)
    0x05, 0x08,       //   Usage Page (LEDs)
    0x19, 0x01,       //   Usage Minimum (Num Lock)
    0x29, 0x05,       //   Usage Maximum (Kana)
    0x91, 0x02,       //   Output (Data, Variable, Absolute)
    0x95, 0x01,       //   Report Count (1)
    0x75, 0x03,       //   Report Size (3)
    0x91, 0x01,       //   Output (Constant): padding
    0x95, 0x06,       //   Report Count (6)
    0x75, 0x08,       //   Report Size (8)
    0x15, 0x00,       //   Logical Minimum (0)
    0x26, 0xff, 0x00, //   Logical Maximum (255)
    0x05, 0x07,       //   Usage Page (Keyboard/Keypad)
    0x19, 0x00,       //   Usage Minimum (0)
    0x2a, 0xff, 0x00, //   Usage Maximum (255)
    0x81, 0x00,       //   Input (Data, Array): pressed keys
    0xc0,             // End Collection

    // Absolute pointer: lands exactly where the shared layout says the pointer is. It has no
    // buttons: macOS reads every absolute report as "all buttons up", which would end drags.
    // Buttons always travel in the relative pointer's reports.
    0x05, 0x01,       // Usage Page (Generic Desktop)
    0x09, 0x02,       // Usage (Mouse)
    0xa1, 0x01,       // Collection (Application)
    0x85, POINTER_ABSOLUTE,
    0x09, 0x01,       //   Usage (Pointer)
    0xa1, 0x00,       //   Collection (Physical)
    0x05, 0x01,       //     Usage Page (Generic Desktop)
    0x09, 0x30,       //     Usage (X)
    0x09, 0x31,       //     Usage (Y)
    0x15, 0x00,       //     Logical Minimum (0)
    0x26, 0xff, 0x7f, //     Logical Maximum (32767)
    0x75, 0x10,       //     Report Size (16)
    0x95, 0x02,       //     Report Count (2)
    0x81, 0x02,       //     Input (Data, Variable, Absolute)
    0x09, 0x38,       //     Usage (Wheel)
    0x15, 0x81,       //     Logical Minimum (-127)
    0x25, 0x7f,       //     Logical Maximum (127)
    0x75, 0x08,       //     Report Size (8)
    0x95, 0x01,       //     Report Count (1)
    0x81, 0x06,       //     Input (Data, Variable, Relative)
    0x05, 0x0c,       //     Usage Page (Consumer)
    0x0a, 0x38, 0x02, //     Usage (AC Pan)
    0x95, 0x01,       //     Report Count (1)
    0x81, 0x06,       //     Input (Data, Variable, Relative)
    0xc0,             //   End Collection
    0xc0,             // End Collection

    // Relative pointer: for hosts that do not accept absolute positions.
    0x05, 0x01,       // Usage Page (Generic Desktop)
    0x09, 0x02,       // Usage (Mouse)
    0xa1, 0x01,       // Collection (Application)
    0x85, POINTER_RELATIVE,
    0x09, 0x01,       //   Usage (Pointer)
    0xa1, 0x00,       //   Collection (Physical)
    0x05, 0x09,       //     Usage Page (Buttons)
    0x19, 0x01,       //     Usage Minimum (1)
    0x29, 0x05,       //     Usage Maximum (5)
    0x15, 0x00,       //     Logical Minimum (0)
    0x25, 0x01,       //     Logical Maximum (1)
    0x95, 0x05,       //     Report Count (5)
    0x75, 0x01,       //     Report Size (1)
    0x81, 0x02,       //     Input (Data, Variable, Absolute)
    0x95, 0x01,       //     Report Count (1)
    0x75, 0x03,       //     Report Size (3)
    0x81, 0x01,       //     Input (Constant): padding
    0x05, 0x01,       //     Usage Page (Generic Desktop)
    0x09, 0x30,       //     Usage (X)
    0x09, 0x31,       //     Usage (Y)
    0x16, 0x01, 0x80, //     Logical Minimum (-32767)
    0x26, 0xff, 0x7f, //     Logical Maximum (32767)
    0x75, 0x10,       //     Report Size (16)
    0x95, 0x02,       //     Report Count (2)
    0x81, 0x06,       //     Input (Data, Variable, Relative)
    0x09, 0x38,       //     Usage (Wheel)
    0x15, 0x81,       //     Logical Minimum (-127)
    0x25, 0x7f,       //     Logical Maximum (127)
    0x75, 0x08,       //     Report Size (8)
    0x95, 0x01,       //     Report Count (1)
    0x81, 0x06,       //     Input (Data, Variable, Relative)
    0x05, 0x0c,       //     Usage Page (Consumer)
    0x0a, 0x38, 0x02, //     Usage (AC Pan)
    0x95, 0x01,       //     Report Count (1)
    0x81, 0x06,       //     Input (Data, Variable, Relative)
    0xc0,             //   End Collection
    0xc0,             // End Collection

    // Pen: exact positions with a pressed tip, for left-button drags.
    0x05, 0x0d,       // Usage Page (Digitizer)
    0x09, 0x02,       // Usage (Pen)
    0xa1, 0x01,       // Collection (Application)
    0x85, PEN,        //   Report ID
    0x09, 0x20,       //   Usage (Stylus)
    0xa1, 0x00,       //   Collection (Physical)
    0x09, 0x42,       //     Usage (Tip Switch)
    0x09, 0x44,       //     Usage (Barrel Switch)
    0x09, 0x32,       //     Usage (In Range)
    0x15, 0x00,       //     Logical Minimum (0)
    0x25, 0x01,       //     Logical Maximum (1)
    0x75, 0x01,       //     Report Size (1)
    0x95, 0x03,       //     Report Count (3)
    0x81, 0x02,       //     Input (Data, Variable, Absolute)
    0x95, 0x05,       //     Report Count (5)
    0x81, 0x03,       //     Input (Constant): padding
    0x05, 0x01,       //     Usage Page (Generic Desktop)
    0x09, 0x30,       //     Usage (X)
    0x09, 0x31,       //     Usage (Y)
    0x15, 0x00,       //     Logical Minimum (0)
    0x26, 0xff, 0x7f, //     Logical Maximum (32767)
    0x75, 0x10,       //     Report Size (16)
    0x95, 0x02,       //     Report Count (2)
    0x81, 0x02,       //     Input (Data, Variable, Absolute)
    0xc0,             //   End Collection
    0xc0,             // End Collection
];

#[cfg(test)]
mod tests {
    use super::*;

    /// Short items as (tag and type, little-endian value).
    fn items() -> Vec<(u8, usize)> {
        let (mut i, mut out) = (0, vec![]);
        while i < DESCRIPTOR.len() {
            let prefix = DESCRIPTOR[i];
            let len = match prefix & 0x03 {
                3 => 4,
                n => n as usize,
            };
            let data = &DESCRIPTOR[i + 1..i + 1 + len];
            out.push((
                prefix & 0xfc,
                data.iter().rev().fold(0, |v, b| (v << 8) | *b as usize),
            ));
            i += 1 + len;
        }
        out
    }

    /// Sums the bits each report ID declares for Input items.
    fn input_bits(report: u8) -> usize {
        let (mut id, mut size, mut count, mut bits) = (0, 0, 0, 0);
        for (tag, value) in items() {
            match tag {
                0x84 => id = value as u8,
                0x74 => size = value,
                0x94 => count = value,
                0x80 if id == report => bits += size * count,
                _ => {}
            }
        }
        bits
    }

    #[test]
    fn declared_layouts_match_the_report_builders() {
        assert_eq!(input_bits(KEYBOARD), KEYBOARD_LEN * 8);
        assert_eq!(input_bits(POINTER_ABSOLUTE), ABSOLUTE_LEN * 8);
        assert_eq!(input_bits(POINTER_RELATIVE), RELATIVE_LEN * 8);
        assert_eq!(input_bits(PEN), PEN_LEN * 8);
    }

    #[test]
    fn collections_are_balanced() {
        let mut depth = 0i32;
        for (tag, _) in items() {
            match tag {
                0xa0 => depth += 1,
                0xc0 => depth -= 1,
                _ => {}
            }
            assert!(depth >= 0);
        }
        assert_eq!(depth, 0);
        assert_eq!(items().iter().filter(|(t, _)| *t == 0xa0).count(), 7);
    }
}
