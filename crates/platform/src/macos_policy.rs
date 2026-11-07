//! Quartz event conversion without framework calls, so it can be tested on every host.
use desklink_protocol::Input;
use std::collections::BTreeSet;

pub fn modifier_mask(code: u16) -> u64 {
    match code {
        42 | 54 => 1 << 17,
        29 | 97 => 1 << 18,
        56 | 100 => 1 << 19,
        125 | 126 => 1 << 20,
        58 => 1 << 16,
        _ => 0,
    }
}

/// Lock state is not a physically held modifier and must not be replayed at handoff.
pub fn modifier_transition(code: u16, flags: u64, held: &mut BTreeSet<u16>) -> Option<bool> {
    let mask = modifier_mask(code);
    if mask == 0 || code == 58 {
        return None;
    }
    let down = flags & mask != 0 && !held.contains(&code);
    if down {
        held.insert(code);
    } else {
        held.remove(&code);
    }
    Some(down)
}

pub fn quartz_button(button: u8) -> u32 {
    match button {
        1 => 0,
        3 => 1,
        2 => 2,
        other => u32::from(other.saturating_sub(1)),
    }
}

/// Quartz requires the specific button number for other-button drag events.
pub fn mouse_motion(held: &BTreeSet<u8>) -> (u32, u32) {
    if held.contains(&1) {
        (6, 0)
    } else if held.contains(&3) {
        (7, 1)
    } else if let Some(button) = held.first() {
        (27, quartz_button(*button))
    } else {
        (5, 0)
    }
}

pub fn held_inputs(
    mut key_down: impl FnMut(u16) -> bool,
    mut button_down: impl FnMut(u32) -> bool,
) -> Vec<Input> {
    let mut held: Vec<_> = super::keymap::MAC_EVDEV
        .iter()
        .filter_map(|(mac, code)| {
            (*code != 58 && key_down(*mac)).then_some(Input::Key {
                code: *code,
                down: true,
            })
        })
        .collect();
    held.extend((1..=8).filter_map(|button| {
        button_down(quartz_button(button)).then_some(Input::Button { button, down: true })
    }));
    super::order_held_inputs(&mut held);
    held
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn middle_and_extra_button_drags_preserve_the_pressed_button() {
        assert_eq!(mouse_motion(&BTreeSet::new()), (5, 0));
        for (button, expected) in [
            (1, (6, 0)),
            (3, (7, 1)),
            (2, (27, 2)),
            (4, (27, 3)),
            (8, (27, 7)),
        ] {
            assert_eq!(mouse_motion(&BTreeSet::from([button])), expected);
        }
    }

    #[test]
    fn caps_lock_is_not_a_held_modifier() {
        let mut held = BTreeSet::new();
        assert_eq!(modifier_transition(58, 1 << 16, &mut held), None);
        assert!(held.is_empty());
        assert_eq!(modifier_transition(29, 1 << 18, &mut held), Some(true));
        assert_eq!(held, BTreeSet::from([29]));
        assert_eq!(modifier_transition(29, 0, &mut held), Some(false));
        assert!(held.is_empty());
    }

    #[test]
    fn releasing_one_shift_preserves_the_other_side() {
        let mut held = BTreeSet::new();
        assert_eq!(modifier_transition(42, 1 << 17, &mut held), Some(true));
        assert_eq!(modifier_transition(54, 1 << 17, &mut held), Some(true));
        assert_eq!(modifier_transition(42, 1 << 17, &mut held), Some(false));
        assert_eq!(held, BTreeSet::from([54]));
    }

    #[test]
    fn handoff_snapshots_held_keys_and_buttons_but_not_lock_state() {
        let held = held_inputs(|mac| [0, 55, 57].contains(&mac), |button| button == 2);
        assert_eq!(
            held,
            vec![
                Input::Key {
                    code: 125,
                    down: true
                },
                Input::Key {
                    code: 30,
                    down: true
                },
                Input::Button {
                    button: 2,
                    down: true
                }
            ]
        );
    }
}
