//! Linux evdev key codes (Desklink's wire key codes) to USB HID Keyboard/Keypad usages.

/// Modifier usages occupy 0xE0..=0xE7 and travel as bits of the report's first byte.
pub const LEFT_CTRL: u8 = 0xe0;
pub const LEFT_GUI: u8 = 0xe3;
pub const RIGHT_CTRL: u8 = 0xe4;
pub const RIGHT_GUI: u8 = 0xe7;

pub fn is_modifier(usage: u8) -> bool {
    (0xe0..=0xe7).contains(&usage)
}

/// `None` for codes a standard keyboard cannot send (e.g. vendor or media-only keys).
pub fn usage(code: u16) -> Option<u8> {
    let usage = match code {
        1 => 0x29,                              // Escape
        2..=10 => 0x1e + (code - 2) as u8,      // 1..9
        11 => 0x27,                             // 0
        12 => 0x2d,                             // -
        13 => 0x2e,                             // =
        14 => 0x2a,                             // Backspace
        15 => 0x2b,                             // Tab
        16 => 0x14,                             // Q
        17 => 0x1a,                             // W
        18 => 0x08,                             // E
        19 => 0x15,                             // R
        20 => 0x17,                             // T
        21 => 0x1c,                             // Y
        22 => 0x18,                             // U
        23 => 0x0c,                             // I
        24 => 0x12,                             // O
        25 => 0x13,                             // P
        26 => 0x2f,                             // [
        27 => 0x30,                             // ]
        28 => 0x28,                             // Enter
        29 => LEFT_CTRL,                        // Left Ctrl
        30 => 0x04,                             // A
        31 => 0x16,                             // S
        32 => 0x07,                             // D
        33 => 0x09,                             // F
        34 => 0x0a,                             // G
        35 => 0x0b,                             // H
        36 => 0x0d,                             // J
        37 => 0x0e,                             // K
        38 => 0x0f,                             // L
        39 => 0x33,                             // ;
        40 => 0x34,                             // '
        41 => 0x35,                             // `
        42 => 0xe1,                             // Left Shift
        43 => 0x31,                             // \
        44 => 0x1d,                             // Z
        45 => 0x1b,                             // X
        46 => 0x06,                             // C
        47 => 0x19,                             // V
        48 => 0x05,                             // B
        49 => 0x11,                             // N
        50 => 0x10,                             // M
        51 => 0x36,                             // ,
        52 => 0x37,                             // .
        53 => 0x38,                             // /
        54 => 0xe5,                             // Right Shift
        55 => 0x55,                             // Keypad *
        56 => 0xe2,                             // Left Alt / Option
        57 => 0x2c,                             // Space
        58 => 0x39,                             // Caps Lock
        59..=68 => 0x3a + (code - 59) as u8,    // F1..F10
        69 => 0x53,                             // Num Lock
        70 => 0x47,                             // Scroll Lock
        71 => 0x5f,                             // Keypad 7
        72 => 0x60,                             // Keypad 8
        73 => 0x61,                             // Keypad 9
        74 => 0x56,                             // Keypad -
        75 => 0x5c,                             // Keypad 4
        76 => 0x5d,                             // Keypad 5
        77 => 0x5e,                             // Keypad 6
        78 => 0x57,                             // Keypad +
        79 => 0x59,                             // Keypad 1
        80 => 0x5a,                             // Keypad 2
        81 => 0x5b,                             // Keypad 3
        82 => 0x62,                             // Keypad 0
        83 => 0x63,                             // Keypad .
        85 => 0x94,                             // Zenkaku/Hankaku
        86 => 0x64,                             // ISO \ (102nd key)
        87 => 0x44,                             // F11
        88 => 0x45,                             // F12
        89 => 0x87,                             // Ro
        90 => 0x92,                             // Katakana
        91 => 0x93,                             // Hiragana
        92 => 0x8a,                             // Henkan
        93 => 0x88,                             // Katakana/Hiragana
        94 => 0x8b,                             // Muhenkan
        95 => 0x8c,                             // Keypad JP comma
        96 => 0x58,                             // Keypad Enter
        97 => RIGHT_CTRL,                       // Right Ctrl
        98 => 0x54,                             // Keypad /
        99 => 0x46,                             // Print Screen
        100 => 0xe6,                            // Right Alt / Option
        102 => 0x4a,                            // Home
        103 => 0x52,                            // Up
        104 => 0x4b,                            // Page Up
        105 => 0x50,                            // Left
        106 => 0x4f,                            // Right
        107 => 0x4d,                            // End
        108 => 0x51,                            // Down
        109 => 0x4e,                            // Page Down
        110 => 0x49,                            // Insert
        111 => 0x4c,                            // Delete
        113 => 0x7f,                            // Mute
        114 => 0x81,                            // Volume Down
        115 => 0x80,                            // Volume Up
        116 => 0x66,                            // Power
        117 => 0x67,                            // Keypad =
        119 => 0x48,                            // Pause
        121 => 0x85,                            // Keypad ,
        122 => 0x90,                            // Hangul
        123 => 0x91,                            // Hanja
        124 => 0x89,                            // Yen
        125 => LEFT_GUI,                        // Left Super / Command
        126 => RIGHT_GUI,                       // Right Super / Command
        127 => 0x65,                            // Menu / Application
        183..=194 => 0x68 + (code - 183) as u8, // F13..F24
        _ => return None,
    };
    Some(usage)
}

/// Exchanges Control and Command/Windows, for Linux shortcuts typed into a Mac.
pub fn swap_control_and_command(usage: u8) -> u8 {
    match usage {
        LEFT_CTRL => LEFT_GUI,
        LEFT_GUI => LEFT_CTRL,
        RIGHT_CTRL => RIGHT_GUI,
        RIGHT_GUI => RIGHT_CTRL,
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn letters_digits_and_function_keys() {
        assert_eq!(usage(30), Some(0x04)); // A
        assert_eq!(usage(44), Some(0x1d)); // Z
        assert_eq!(usage(2), Some(0x1e)); // 1
        assert_eq!(usage(10), Some(0x26)); // 9
        assert_eq!(usage(11), Some(0x27)); // 0
        assert_eq!(usage(59), Some(0x3a)); // F1
        assert_eq!(usage(68), Some(0x43)); // F10
        assert_eq!(usage(88), Some(0x45)); // F12
        assert_eq!(usage(194), Some(0x73)); // F24
        assert_eq!(usage(240), None);
    }

    #[test]
    fn every_letter_maps_to_a_distinct_usage() {
        let letters = [
            30, 48, 46, 32, 18, 33, 34, 35, 23, 36, 37, 38, 50, 49, 24, 25, 16, 19, 31, 20, 22, 47,
            17, 45, 21, 44,
        ]; // a..z in evdev order of the alphabet
        for (i, code) in letters.iter().enumerate() {
            assert_eq!(usage(*code), Some(0x04 + i as u8), "letter {i}");
        }
    }

    #[test]
    fn modifiers_and_the_mac_swap() {
        for code in [29, 42, 56, 125, 97, 54, 100, 126] {
            assert!(is_modifier(usage(code).unwrap()), "{code}");
        }
        assert_eq!(swap_control_and_command(LEFT_CTRL), LEFT_GUI);
        assert_eq!(swap_control_and_command(RIGHT_GUI), RIGHT_CTRL);
        assert_eq!(swap_control_and_command(0x04), 0x04);
    }
}
