//! the .inf tables that ship on the scope itself.
//!
//! hantek keeps the settings layout in `protocol.inf` and the key numbering in
//! `keyprotocol.inf`, and both are the actual source of truth rather than
//! documentation. ReadSettings just dumps its internal struct at you and you
//! walk protocol.inf to find out what each byte meant. same deal for keys: the
//! code you send is the line number in keyprotocol.inf, 0 based.
//!
//! both files are baked into the binary with `include_str!` so the app stays a
//! single file you can copy around. theyre also on the scope at
//! `/OurProtocol/` if you ever need to check yours matches.

use std::collections::HashMap;
use std::sync::LazyLock;

const PROTOCOL_INF: &str = include_str!("../inf/protocol.inf");
const KEYPROTOCOL_INF: &str = include_str!("../inf/keyprotocol.inf");

/// one entry in the settings blob
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Field {
    pub name: &'static str,
    /// byte offset into the 208 byte ReadSettings payload
    pub offset: usize,
    /// 1, 2 or 8 bytes, little endian
    pub size: usize,
}

/// pull `[NAME] size` pairs out of the `[START]`..`[END]` body.
///
/// the files are CRLF because of course they are, so trim both ends. anything
/// outside the START/END markers is header junk like `[TOTAL] 208` and gets
/// skipped.
fn parse(src: &'static str) -> Vec<(&'static str, usize)> {
    let mut out = Vec::new();
    let mut started = false;
    for line in src.lines() {
        let line = line.trim();
        if line.starts_with("[START]") {
            started = true;
            continue;
        }
        if line.starts_with("[END]") {
            break;
        }
        if !started || !line.starts_with('[') {
            continue;
        }
        let Some(close) = line.find(']') else { continue };
        let name = &line[1..close];
        // no size on the line means 1 byte. doesnt happen in the files we ship
        // but the parser may as well not choke on it
        let size = line[close + 1..].trim().parse().unwrap_or(1);
        out.push((name, size));
    }
    out
}

/// every field in the ReadSettings blob, in wire order, with offsets resolved
pub static SETTINGS_FIELDS: LazyLock<Vec<Field>> = LazyLock::new(|| {
    let mut offset = 0;
    parse(PROTOCOL_INF)
        .into_iter()
        .map(|(name, size)| {
            let f = Field { name, offset, size };
            offset += size;
            f
        })
        .collect()
});

static SETTINGS_INDEX: LazyLock<HashMap<&'static str, usize>> = LazyLock::new(|| {
    SETTINGS_FIELDS
        .iter()
        .enumerate()
        .map(|(i, f)| (f.name, i))
        .collect()
});

/// how many bytes ReadSettings should hand back. 208 on firmware 3.40.
pub fn settings_len() -> usize {
    SETTINGS_FIELDS.last().map_or(0, |f| f.offset + f.size)
}

/// look a field up by its inf name, eg `VERT-CH1-VB`
pub fn field(name: &str) -> Option<&'static Field> {
    field_index(name).map(|i| &SETTINGS_FIELDS[i])
}

/// position of a field in the decoded value list
pub fn field_index(name: &str) -> Option<usize> {
    SETTINGS_INDEX.get(name).copied()
}

/// key names in code order. the index *is* the code you send with KeyTrigger.
pub static KEY_NAMES: LazyLock<Vec<&'static str>> =
    LazyLock::new(|| parse(KEYPROTOCOL_INF).into_iter().map(|(n, _)| n).collect());

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_blob_is_208_bytes() {
        // [TOTAL] at the top of protocol.inf says 208. if this ever fails
        // either the file changed or the parser is dropping lines
        assert_eq!(settings_len(), 208);
    }

    #[test]
    fn no_marker_lines_leaked_in() {
        for f in SETTINGS_FIELDS.iter() {
            assert!(!f.name.contains("TOTAL"), "header junk got through: {}", f.name);
            assert!(!f.name.is_empty());
        }
    }

    #[test]
    fn known_offsets() {
        // first field, and the one right after CH1s 2 byte position
        assert_eq!(field("VERT-CH1-DISP").unwrap().offset, 0);
        assert_eq!(field("VERT-CH1-POS").unwrap(), &Field { name: "VERT-CH1-POS", offset: 8, size: 2 });
        assert_eq!(field("VERT-CH2-DISP").unwrap().offset, 10);
    }

    #[test]
    fn key_codes_match_what_was_verified_on_hardware() {
        // these three were poked at a real scope: 41 steps the timebase up,
        // 40 steps it down, 19 toggles run/stop
        assert_eq!(KEY_NAMES[41], "HZ-TBADD-KEY");
        assert_eq!(KEY_NAMES[40], "HZ-TBSUB-KEY");
        assert_eq!(KEY_NAMES[19], "CT-RS-KEY");
        assert_eq!(KEY_NAMES.len(), 49);
    }
}
