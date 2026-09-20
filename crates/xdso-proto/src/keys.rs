//! the scopes front panel, as key codes.
//!
//! KeyTrigger takes the line number of a key in keyprotocol.inf, 0 based, and
//! presses it. so the table below is generated from the file rather than typed
//! out, which means it cant drift out of sync even if hantek adds a key. the
//! only hand written bits are the pretty labels and the keyboard shortcuts.

use std::borrow::Cow;
use std::sync::LazyLock;

use crate::inf::KEY_NAMES;

/// one button, ready to stick on screen
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Key {
    /// what you send with KeyTrigger
    pub code: u8,
    /// the raw name from keyprotocol.inf
    pub inf_name: &'static str,
    /// what the front panel calls it
    pub label: Cow<'static, str>,
    /// host keyboard shortcut, if it has one
    pub shortcut: Option<char>,
}

/// nice names for the raw inf names. anything missing falls back to a tidied
/// up version of the inf name so the panel is never missing a button
const LABELS: &[(&str, &str)] = &[
    // the scope numbers these from zero, and theres no F8. F0 to F6 run down
    // the side of the display, F7 sits in the row with Help and Default
    ("FN-0-KEY", "F0"),
    ("FN-1-KEY", "F1"),
    ("FN-2-KEY", "F2"),
    ("FN-3-KEY", "F3"),
    ("FN-4-KEY", "F4"),
    ("FN-5-KEY", "F5"),
    ("FN-6-KEY", "F6"),
    ("FN-7-KEY", "F7"),
    ("FN-MLEFT-KEY", "Menu <"),
    ("FN-MRIGHT-KEY", "Menu >"),
    ("FN-MZERO-KEY", "Menu 0"),
    ("MENU-SR-KEY", "Save/Rec"),
    ("MENU-MEASURE-KEY", "Measure"),
    ("MENU-ACQUIRE-KEY", "Acquire"),
    ("MENU-UTILITY-KEY", "Utility"),
    ("MENU-CURSOR-KEY", "Cursor"),
    ("MENU-DISPLAY-KEY", "Display"),
    ("CT-AUTOSET-KEY", "Autoset"),
    ("CT-SINGLESEQ-KEY", "Single"),
    ("CT-RS-KEY", "Run/Stop"),
    ("CT-HELP-KEY", "Help"),
    ("CT-DS-KEY", "Default"),
    ("CT-STU-KEY", "Auto Scale"),
    ("VT-MATH-MENU-KEY", "Math"),
    ("VT-CH1-MENU-KEY", "CH1"),
    ("VT-CH1-PSUB-KEY", "CH1 Pos-"),
    ("VT-CH1-PADD-KEY", "CH1 Pos+"),
    ("VT-CH1-PZERO-KEY", "CH1 Pos0"),
    ("VT-CH1-VBSUB-KEY", "CH1 V-"),
    ("VT-CH1-VBADD-KEY", "CH1 V+"),
    ("VT-CH2-MENU-KEY", "CH2"),
    ("VT-CH2-PSUB-KEY", "CH2 Pos-"),
    ("VT-CH2-PADD-KEY", "CH2 Pos+"),
    ("VT-CH2-PZERO-KEY", "CH2 Pos0"),
    ("VT-CH2-VBSUB-KEY", "CH2 V-"),
    ("VT-CH2-VBADD-KEY", "CH2 V+"),
    ("HZ-MENU-KEY", "Horiz"),
    ("HZ-PSUB-KEY", "H Pos-"),
    ("HZ-PADD-KEY", "H Pos+"),
    ("HZ-PZERO-KEY", "H Pos0"),
    ("HZ-TBSUB-KEY", "Time -"),
    ("HZ-TBADD-KEY", "Time +"),
    ("TG-MENU-KEY", "Trigger"),
    ("TG-PSUB-KEY", "Trig Lvl-"),
    ("TG-PADD-KEY", "Trig Lvl+"),
    ("TG-PZERO-KEY", "Trig Lvl0"),
    ("TG-PHALF-KEY", "Trig 50%"),
    ("TG-FORCE-KEY", "Force Trig"),
    ("TG-PROBECHECK-KEY", "Probe Chk"),
];

/// keyboard shortcuts, keyed by inf name so they survive any renumbering
const SHORTCUTS: &[(char, &str)] = &[
    (' ', "CT-RS-KEY"),
    ('a', "CT-AUTOSET-KEY"),
    ('s', "CT-SINGLESEQ-KEY"),
    ('t', "TG-FORCE-KEY"),
    ('5', "TG-PHALF-KEY"),
    ('[', "HZ-TBSUB-KEY"),
    (']', "HZ-TBADD-KEY"),
    (',', "HZ-PSUB-KEY"),
    ('.', "HZ-PADD-KEY"),
    ('1', "VT-CH1-MENU-KEY"),
    ('q', "VT-CH1-VBSUB-KEY"),
    ('w', "VT-CH1-VBADD-KEY"),
    ('e', "VT-CH1-PSUB-KEY"),
    ('r', "VT-CH1-PADD-KEY"),
    ('2', "VT-CH2-MENU-KEY"),
    ('z', "VT-CH2-VBSUB-KEY"),
    ('x', "VT-CH2-VBADD-KEY"),
    ('c', "VT-CH2-PSUB-KEY"),
    ('v', "VT-CH2-PADD-KEY"),
    ('-', "TG-PSUB-KEY"),
    ('=', "TG-PADD-KEY"),
    ('m', "MENU-MEASURE-KEY"),
    ('u', "MENU-UTILITY-KEY"),
    ('d', "MENU-DISPLAY-KEY"),
    ('k', "MENU-CURSOR-KEY"),
    ('j', "MENU-ACQUIRE-KEY"),
    ('h', "CT-HELP-KEY"),
    ('n', "VT-MATH-MENU-KEY"),
    ('b', "MENU-SR-KEY"),
    ('f', "FN-0-KEY"),
    ('3', "FN-1-KEY"),
    ('4', "FN-2-KEY"),
    ('6', "FN-3-KEY"),
    ('7', "FN-4-KEY"),
];

static KEYS: LazyLock<Vec<Key>> = LazyLock::new(|| {
    KEY_NAMES
        .iter()
        .enumerate()
        .map(|(code, &inf_name)| Key {
            code: code as u8,
            inf_name,
            label: LABELS
                .iter()
                .find(|(n, _)| *n == inf_name)
                .map_or_else(|| Cow::Owned(tidy(inf_name)), |(_, l)| Cow::Borrowed(*l)),
            shortcut: SHORTCUTS.iter().find(|(_, n)| *n == inf_name).map(|(c, _)| *c),
        })
        .collect()
});

/// last resort label for a key we have no nice name for. `TG-WHATEVER-KEY`
/// becomes `Tg Whatever`
fn tidy(inf_name: &str) -> String {
    inf_name
        .trim_end_matches("-KEY")
        .split('-')
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                Some(f) => f.to_uppercase().chain(c.flat_map(char::to_lowercase)).collect::<String>(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// every key the scope knows about, in code order
pub fn keys() -> &'static [Key] {
    &KEYS
}

/// find the key a host keypress should fire
pub fn key_for_char(c: char) -> Option<&'static Key> {
    let c = c.to_ascii_lowercase();
    keys().iter().find(|k| k.shortcut == Some(c))
}

/// find a key by its inf name, for when you want to press one from code
pub fn key_named(inf_name: &str) -> Option<&'static Key> {
    keys().iter().find(|k| k.inf_name == inf_name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn f_keys_are_numbered_from_zero() {
        // there is no F8. F0..F6 are the side menu, F7 is the odd one out
        assert_eq!(key_named("FN-0-KEY").unwrap().label, "F0");
        assert_eq!(key_named("FN-6-KEY").unwrap().label, "F6");
        assert_eq!(key_named("FN-7-KEY").unwrap().label, "F7");
        assert!(keys().iter().all(|k| k.label != "F8"), "F8 does not exist");
    }

    #[test]
    fn code_is_the_line_number() {
        assert_eq!(key_named("CT-RS-KEY").unwrap().code, 19);
        assert_eq!(key_named("HZ-TBADD-KEY").unwrap().code, 41);
        assert_eq!(keys().len(), 49);
    }

    #[test]
    fn every_key_got_a_label() {
        for k in keys() {
            assert!(!k.label.is_empty(), "{} has no label", k.inf_name);
        }
    }

    #[test]
    fn shortcuts_point_at_real_keys() {
        // a typo in SHORTCUTS would otherwise just silently do nothing
        for (c, name) in SHORTCUTS {
            assert!(key_named(name).is_some(), "shortcut {c} points at missing key {name}");
        }
        assert_eq!(key_for_char(' ').unwrap().label, "Run/Stop");
        assert_eq!(key_for_char('W').unwrap().inf_name, "VT-CH1-VBADD-KEY");
    }

    #[test]
    fn no_shortcut_is_bound_twice() {
        let mut seen: Vec<char> = SHORTCUTS.iter().map(|(c, _)| *c).collect();
        seen.sort_unstable();
        let before = seen.len();
        seen.dedup();
        assert_eq!(before, seen.len(), "a shortcut is bound to two keys");
    }

    #[test]
    fn fallback_label_is_readable() {
        assert_eq!(tidy("TG-PROBECHECK-KEY"), "Tg Probecheck");
    }
}
