//! packet framing. das oszi protocol, such as it is.
//!
//! every packet looks the same in both directions :
//!
//! | byte | what |
//! |---|---|
//! | 0 | `0x53`, always |
//! | 1..3 | length, little endian. counts the command byte and the checksum, not these 3 |
//! | 3 | command (going out) or command echo (coming back) |
//! | 4.. | payload |
//! | last | checksum, low byte of the sum of everything before it |
//!
//! what sits at the front of the payload depends on the command, which is
//! annoying but thats the protocol. replies that span several packets put a
//! continuation flag in the first payload byte, replies that fit in one dont
//! bother. see [`Packet::sample_data`] and friends.

/// commands we send
pub mod cmd {
    /// send bytes, get the same bytes back. useless except as a way to prod
    /// the scope and see if its awake
    pub const ECHO: u8 = 0x00;
    /// dump the whole front panel state, 208 bytes, one packet, ~5 ms
    pub const READ_SETTINGS: u8 = 0x01;
    /// 3200 samples for one channel, 3 packets, ~50 ms
    pub const READ_SAMPLE_DATA: u8 = 0x02;
    /// read a file off the scopes filesystem. payload is a null byte then the
    /// name. fast, but it gives up on genuinely big files
    pub const READ_FILE: u8 = 0x10;
    /// run a shell command. **goes out as a debug packet**, see
    /// [`encode_debug`]. payload is the command line as plain bytes
    pub const REMOTE_SHELL: u8 = 0x11;
    /// panel lock and acquisition control. `[1, 1]` locks the front panel,
    /// `[1, 0]` unlocks it, `[0, 0]` starts acquisition
    pub const CONTROL: u8 = 0x12;
    /// press a button. payload is `[code, 0x01]`
    pub const KEY_TRIGGER: u8 = 0x13;
    /// the whole 800x480 framebuffer. 77 packets, ~1030 ms. use sparingly
    pub const SCREENSHOT: u8 = 0x20;
    /// the scopes clock
    pub const READ_SYSTEM_TIME: u8 = 0x21;
}

/// command bytes the scope echoes back at us. they are not the same numbers
/// it was sent, dont ask why
pub mod reply {
    pub const ECHO: u8 = 0x80;
    /// confirmed on hardware, a settings read comes back as 213 bytes with
    /// 0x81 in byte 3
    pub const SETTINGS: u8 = 0x81;
    pub const SAMPLE_DATA: u8 = 0x82;
    pub const FILE: u8 = 0x90;
    pub const REMOTE_SHELL: u8 = 0x91;
    pub const CONTROL: u8 = 0x92;
    /// the ack for a key press. we never look at it, it carries nothing
    pub const KEY_TRIGGER: u8 = 0x93;
    pub const SCREENSHOT: u8 = 0xa0;
    pub const SYSTEM_TIME: u8 = 0xa1;
}

/// first byte of a multi packet reply payload
pub mod flag {
    /// more data in this packet, keep reading
    pub const MORE: u8 = 0x01;
    /// nothing in this one. the scope sends these while acquisition is stopped
    pub const EMPTY: u8 = 0x00;
}

const MAGIC: u8 = 0x53;

/// RemoteShell goes out with this in byte 0 instead of 0x53. the python called
/// it the debug flag and thats as good a name as any. why running a command
/// needs a different packet type when every other command manages fine with
/// 0x53 is anyones guess
const MAGIC_DEBUG: u8 = 0x43;

const HEADER: usize = 4;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProtoError {
    /// first byte was not 0x53, so we are out of sync with the stream
    BadMagic(u8),
    /// fewer bytes than a header plus checksum. usually a timeout that
    /// returned something useless
    Short(usize),
    /// settings blob was not the size protocol.inf says it should be
    WrongLength { want: usize, got: usize },
}

impl std::fmt::Display for ProtoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BadMagic(b) => write!(f, "bad magic 0x{b:02x}, expected 0x53"),
            Self::Short(n) => write!(f, "runt packet, {n} bytes"),
            Self::WrongLength { want, got } => write!(f, "expected {want} bytes, got {got}"),
        }
    }
}

impl std::error::Error for ProtoError {}

/// build a command packet ready to shove at the out endpoint
pub fn encode(cmd: u8, data: &[u8]) -> Vec<u8> {
    encode_with(MAGIC, cmd, data)
}

/// same but with the debug magic byte. only RemoteShell wants this, and it
/// wants it every time. send it a 0x53 packet and it ignores you
pub fn encode_debug(cmd: u8, data: &[u8]) -> Vec<u8> {
    encode_with(MAGIC_DEBUG, cmd, data)
}

fn encode_with(magic: u8, cmd: u8, data: &[u8]) -> Vec<u8> {
    let len = data.len() + 2; // command byte + checksum
    let mut p = Vec::with_capacity(HEADER + data.len() + 1);
    p.push(magic);
    p.push(len as u8);
    p.push((len >> 8) as u8);
    p.push(cmd);
    p.extend_from_slice(data);
    p.push(checksum(&p));
    p
}

/// low byte of the sum of every preceding byte. thats the entire algorithm
pub fn checksum(bytes: &[u8]) -> u8 {
    bytes.iter().fold(0u8, |a, b| a.wrapping_add(*b))
}

/// a reply, sliced up but not interpreted
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Packet<'a> {
    /// what the scope echoed in byte 3
    pub cmd: u8,
    /// everything between the header and the checksum
    pub payload: &'a [u8],
}

/// split a raw bulk read into header + payload.
///
/// we slice on the length we actually read rather than the declared length,
/// same as the python did. the declared length is right on every packet i have
/// seen but if the firmware ever lies about it id rather keep the bytes.
pub fn decode(buf: &[u8]) -> Result<Packet<'_>, ProtoError> {
    if buf.len() < HEADER + 1 {
        return Err(ProtoError::Short(buf.len()));
    }
    if buf[0] != MAGIC && buf[0] != MAGIC_DEBUG {
        return Err(ProtoError::BadMagic(buf[0]));
    }
    Ok(Packet { cmd: buf[3], payload: &buf[HEADER..buf.len() - 1] })
}

impl<'a> Packet<'a> {
    /// declared payload length, for when you want to check the scope agrees
    /// with how much we actually read
    pub fn declared_len(buf: &[u8]) -> Option<usize> {
        (buf.len() >= 3).then(|| u16::from_le_bytes([buf[1], buf[2]]) as usize)
    }

    /// sample data chunk. payload is `[flag, channel, ..samples]`
    ///
    /// returns None when the flag says this packet carries nothing, which is
    /// what you get while acquisition is stopped.
    pub fn sample_data(&self) -> Option<&'a [u8]> {
        match self.payload {
            [flag::MORE, _ch, rest @ ..] => Some(rest),
            _ => None,
        }
    }

    /// true once the scope has told us thats the lot.
    ///
    /// note this is only right for sample data. an EMPTY flag there means
    /// "nothing this time, keep reading", but on a screenshot anything that
    /// isnt MORE ends the transfer. the two loops differ so dont share this.
    pub fn samples_done(&self) -> bool {
        !matches!(self.payload.first(), Some(&flag::MORE) | Some(&flag::EMPTY))
    }

    /// screenshot chunk. payload is `[flag, ..pixels]`, one byte less of
    /// preamble than sample data because consistency is for cowards
    pub fn screen_data(&self) -> Option<&'a [u8]> {
        match self.payload {
            [flag::MORE, rest @ ..] => Some(rest),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_a_bare_command() {
        // ReadSettings: no data, so length is just cmd + checksum
        let p = encode(cmd::READ_SETTINGS, &[]);
        assert_eq!(p, vec![0x53, 0x02, 0x00, 0x01, 0x56]);
        assert_eq!(checksum(&p[..p.len() - 1]), *p.last().unwrap());
    }

    #[test]
    fn encodes_a_key_press() {
        let p = encode(cmd::KEY_TRIGGER, &[19, 0x01]);
        assert_eq!(&p[..4], &[0x53, 0x04, 0x00, 0x13]);
        assert_eq!(p[4..6], [19, 0x01]);
        assert_eq!(checksum(&p[..p.len() - 1]), *p.last().unwrap());
    }

    #[test]
    fn round_trips_through_decode() {
        let p = encode(cmd::READ_SAMPLE_DATA, &[0x01, 0x00]);
        let d = decode(&p).unwrap();
        assert_eq!(d.cmd, cmd::READ_SAMPLE_DATA);
        assert_eq!(d.payload, &[0x01, 0x00]);
        assert_eq!(Packet::declared_len(&p), Some(4));
    }

    #[test]
    fn remote_shell_uses_the_debug_magic() {
        let p = encode_debug(cmd::REMOTE_SHELL, b"uname -a");
        assert_eq!(p[0], 0x43);
        assert_eq!(p[3], 0x11);
        assert_eq!(&p[4..p.len() - 1], b"uname -a");
        assert_eq!(checksum(&p[..p.len() - 1]), *p.last().unwrap());
        // and it still decodes, the reply magic is whatever the scope feels like
        assert_eq!(decode(&p).unwrap().payload, b"uname -a");
    }

    #[test]
    fn rejects_junk() {
        assert_eq!(decode(&[]), Err(ProtoError::Short(0)));
        assert_eq!(decode(&[0x42, 0, 0, 0, 0]), Err(ProtoError::BadMagic(0x42)));
    }

    #[test]
    fn strips_the_right_preamble_per_command() {
        let sample = Packet { cmd: reply::SAMPLE_DATA, payload: &[0x01, 0x00, 1, 2, 3] };
        assert_eq!(sample.sample_data(), Some(&[1u8, 2, 3][..]));
        let screen = Packet { cmd: reply::SCREENSHOT, payload: &[0x01, 1, 2, 3] };
        assert_eq!(screen.screen_data(), Some(&[1u8, 2, 3][..]));

        // stopped acquisition: flag 0, no samples, and not the end either
        let idle = Packet { cmd: reply::SAMPLE_DATA, payload: &[0x00] };
        assert_eq!(idle.sample_data(), None);
        assert!(!idle.samples_done());
    }
}
