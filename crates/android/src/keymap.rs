//! Physical Linux evdev key identifiers to Android `KeyEvent` key codes and meta state.
//!
//! Desklink transmits physical keys; Android applies its own keyboard layout to the key codes.

const META_SHIFT_ON: i32 = 0x1;
const META_ALT_ON: i32 = 0x2;
const META_ALT_LEFT_ON: i32 = 0x10;
const META_ALT_RIGHT_ON: i32 = 0x20;
const META_SHIFT_LEFT_ON: i32 = 0x40;
const META_SHIFT_RIGHT_ON: i32 = 0x80;
const META_CTRL_ON: i32 = 0x1000;
const META_CTRL_LEFT_ON: i32 = 0x2000;
const META_CTRL_RIGHT_ON: i32 = 0x4000;
const META_META_ON: i32 = 0x10000;
const META_META_LEFT_ON: i32 = 0x20000;
const META_META_RIGHT_ON: i32 = 0x40000;
const META_CAPS_LOCK_ON: i32 = 0x100000;
const META_NUM_LOCK_ON: i32 = 0x200000;

/// Returns the Android key code for an evdev key, or `None` for keys Android cannot represent.
pub fn android_keycode(evdev: u16) -> Option<i32> {
    Some(match evdev {
        1 => 111, // ESCAPE
        2..=10 => 8 + (evdev as i32 - 2),
        11 => 7,                              // 0
        12 => 69,                             // MINUS
        13 => 70,                             // EQUALS
        14 => 67,                             // DEL (backspace)
        15 => 61,                             // TAB
        16 => 45,                             // Q
        17 => 51,                             // W
        18 => 33,                             // E
        19 => 46,                             // R
        20 => 48,                             // T
        21 => 53,                             // Y
        22 => 49,                             // U
        23 => 37,                             // I
        24 => 43,                             // O
        25 => 44,                             // P
        26 => 71,                             // LEFT_BRACKET
        27 => 72,                             // RIGHT_BRACKET
        28 => 66,                             // ENTER
        29 => 113,                            // CTRL_LEFT
        30 => 29,                             // A
        31 => 47,                             // S
        32 => 32,                             // D
        33 => 34,                             // F
        34 => 35,                             // G
        35 => 36,                             // H
        36 => 38,                             // J
        37 => 39,                             // K
        38 => 40,                             // L
        39 => 74,                             // SEMICOLON
        40 => 75,                             // APOSTROPHE
        41 => 68,                             // GRAVE
        42 => 59,                             // SHIFT_LEFT
        43 => 73,                             // BACKSLASH
        44 => 54,                             // Z
        45 => 52,                             // X
        46 => 31,                             // C
        47 => 50,                             // V
        48 => 30,                             // B
        49 => 42,                             // N
        50 => 41,                             // M
        51 => 55,                             // COMMA
        52 => 56,                             // PERIOD
        53 => 76,                             // SLASH
        54 => 60,                             // SHIFT_RIGHT
        55 => 155,                            // NUMPAD_MULTIPLY
        56 => 57,                             // ALT_LEFT
        57 => 62,                             // SPACE
        58 => 115,                            // CAPS_LOCK
        59..=68 => 131 + (evdev as i32 - 59), // F1..F10
        69 => 143,                            // NUM_LOCK
        70 => 116,                            // SCROLL_LOCK
        71 => 151,                            // NUMPAD_7
        72 => 152,
        73 => 153,
        74 => 156, // NUMPAD_SUBTRACT
        75 => 148, // NUMPAD_4
        76 => 149,
        77 => 150,
        78 => 157, // NUMPAD_ADD
        79 => 145, // NUMPAD_1
        80 => 146,
        81 => 147,
        82 => 144,  // NUMPAD_0
        83 => 158,  // NUMPAD_DOT
        86 => 73,   // 102ND (ISO key beside left shift)
        87 => 141,  // F11
        88 => 142,  // F12
        89 => 217,  // RO
        92 => 214,  // HENKAN
        93 => 215,  // KATAKANA_HIRAGANA
        94 => 213,  // MUHENKAN
        96 => 160,  // NUMPAD_ENTER
        97 => 114,  // CTRL_RIGHT
        98 => 154,  // NUMPAD_DIVIDE
        99 => 120,  // SYSRQ
        100 => 58,  // ALT_RIGHT
        102 => 122, // MOVE_HOME
        103 => 19,  // DPAD_UP
        104 => 92,  // PAGE_UP
        105 => 21,  // DPAD_LEFT
        106 => 22,  // DPAD_RIGHT
        107 => 123, // MOVE_END
        108 => 20,  // DPAD_DOWN
        109 => 93,  // PAGE_DOWN
        110 => 124, // INSERT
        111 => 112, // FORWARD_DEL
        113 => 164, // VOLUME_MUTE
        114 => 25,  // VOLUME_DOWN
        115 => 24,  // VOLUME_UP
        117 => 161, // NUMPAD_EQUALS
        119 => 121, // BREAK
        121 => 159, // NUMPAD_COMMA
        122 => 218, // KANA (Hangul toggle)
        124 => 216, // YEN
        125 => 117, // META_LEFT
        126 => 118, // META_RIGHT
        127 => 82,  // MENU (compose)
        139 => 82,  // MENU
        158 => 4,   // BACK
        159 => 125, // FORWARD
        163 => 87,  // MEDIA_NEXT
        164 => 85,  // MEDIA_PLAY_PAUSE
        165 => 88,  // MEDIA_PREVIOUS
        166 => 86,  // MEDIA_STOP
        172 => 3,   // HOME
        217 => 84,  // SEARCH
        224 => 220, // BRIGHTNESS_DOWN
        225 => 221, // BRIGHTNESS_UP
        _ => return None,
    })
}

/// Tracks modifier keys so every injected key carries the meta state Android expects.
#[derive(Debug, Default)]
pub struct Modifiers {
    held: i32,
    caps: bool,
}

impl Modifiers {
    /// Records a key transition and returns the meta state to attach to that event.
    pub fn update(&mut self, evdev: u16, down: bool) -> i32 {
        let bit = match evdev {
            42 => META_SHIFT_LEFT_ON,
            54 => META_SHIFT_RIGHT_ON,
            29 => META_CTRL_LEFT_ON,
            97 => META_CTRL_RIGHT_ON,
            56 => META_ALT_LEFT_ON,
            100 => META_ALT_RIGHT_ON,
            125 => META_META_LEFT_ON,
            126 => META_META_RIGHT_ON,
            _ => 0,
        };
        if down {
            self.held |= bit;
        } else {
            self.held &= !bit;
        }
        if evdev == 58 && down {
            self.caps = !self.caps;
        }
        self.meta()
    }
    pub fn meta(&self) -> i32 {
        let mut meta = self.held | META_NUM_LOCK_ON;
        if self.held & (META_SHIFT_LEFT_ON | META_SHIFT_RIGHT_ON) != 0 {
            meta |= META_SHIFT_ON;
        }
        if self.held & (META_CTRL_LEFT_ON | META_CTRL_RIGHT_ON) != 0 {
            meta |= META_CTRL_ON;
        }
        if self.held & (META_ALT_LEFT_ON | META_ALT_RIGHT_ON) != 0 {
            meta |= META_ALT_ON;
        }
        if self.held & (META_META_LEFT_ON | META_META_RIGHT_ON) != 0 {
            meta |= META_META_ON;
        }
        if self.caps {
            meta |= META_CAPS_LOCK_ON;
        }
        meta
    }
    /// Held modifiers end with the session; the caps-lock toggle is a property of the phone.
    pub fn release(&mut self) {
        self.held = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn letters_digits_and_navigation_map_to_android() {
        assert_eq!(android_keycode(30), Some(29)); // A
        assert_eq!(android_keycode(44), Some(54)); // Z
        assert_eq!(android_keycode(2), Some(8)); // 1
        assert_eq!(android_keycode(10), Some(16)); // 9
        assert_eq!(android_keycode(11), Some(7)); // 0
        assert_eq!(android_keycode(59), Some(131)); // F1
        assert_eq!(android_keycode(68), Some(140)); // F10
        assert_eq!(android_keycode(88), Some(142)); // F12
        assert_eq!(android_keycode(14), Some(67)); // backspace
        assert_eq!(android_keycode(111), Some(112)); // delete
        assert_eq!(android_keycode(105), Some(21)); // left
        assert_eq!(android_keycode(183), None); // F13 has no Android key code
    }

    #[test]
    fn distinct_physical_keys_do_not_collide() {
        let allowed_aliases = [(86, 43), (139, 127)];
        let mut seen: HashMap<i32, u16> = HashMap::new();
        for evdev in 1..=767u16 {
            if let Some(code) = android_keycode(evdev) {
                assert!(code > 0);
                if let Some(previous) = seen.insert(code, evdev) {
                    assert!(
                        allowed_aliases.contains(&(evdev, previous)),
                        "evdev {evdev} and {previous} both map to {code}"
                    );
                }
            }
        }
    }

    #[test]
    fn meta_state_follows_modifiers_and_shortcuts() {
        let mut m = Modifiers::default();
        let ctrl = m.update(29, true);
        assert_eq!(
            ctrl,
            META_CTRL_ON | META_CTRL_LEFT_ON | META_NUM_LOCK_ON,
            "a modifier's own down event carries its bit"
        );
        assert_eq!(
            m.update(46, true),
            META_CTRL_ON | META_CTRL_LEFT_ON | META_NUM_LOCK_ON
        );
        m.update(54, true);
        assert_eq!(m.meta() & META_SHIFT_ON, META_SHIFT_ON);
        assert_eq!(m.update(29, false) & META_CTRL_ON, 0);
        m.release();
        assert_eq!(m.meta(), META_NUM_LOCK_ON);
    }

    #[test]
    fn caps_lock_toggles_on_press_and_survives_release() {
        let mut m = Modifiers::default();
        m.update(58, true);
        m.update(58, false);
        assert_ne!(m.meta() & META_CAPS_LOCK_ON, 0);
        m.release();
        assert_ne!(m.meta() & META_CAPS_LOCK_ON, 0);
        m.update(58, true);
        assert_eq!(m.meta() & META_CAPS_LOCK_ON, 0);
    }
}
