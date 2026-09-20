//! one method per protocol command.
//!
//! every method takes `&mut self`, which is not just rust being fussy : the
//! scope has exactly one command in flight at a time and gets very confused if
//! you interleave them. the borrow checker enforcing that for free is nicer
//! than the mutex the python needed.

use std::time::{Duration, Instant};

use nusb::transfer::{Buffer, Bulk, In, Out, TransferError};
use nusb::{Device, Endpoint, Interface, MaybeFuture};

use xdso_proto::{Channel, PID, SAMPLES, SCREEN_H, SCREEN_W, Settings, VID, frame};

/// ReadSampleData channel index for the math trace
pub const MATH_CHANNEL: u8 = 3;

/// interface 0, the only one there is
const INTERFACE: u8 = 0;
const EP_OUT: u8 = 0x02;
const EP_IN: u8 = 0x81;

/// how much to ask for per read. has to be a multiple of the 512 byte max
/// packet size, and bigger than the ~10 KB the scope actually sends so one
/// read gets one whole protocol packet
const READ_LEN: usize = 16 * 1024;

/// default minimum gap between commands.
///
/// measured on real hardware : below about 15 ms the scope just doesnt answer,
/// roughly half the time. 20 ms leaves some margin. this is the single most
/// important number in the whole crate, if you shrink it everything gets
/// flaky in ways that look like random usb errors.
///
/// **measured from the end of the last reply, not from when we sent the last
/// command.** a settings read takes 5 ms so the gap is satisfied either way
/// and it all looks fine, but a waveform read takes 50 ms, uses the whole gap
/// up, and the next command goes out too early and gets silently binned.
///
/// changeable at runtime with [`Scope::set_gap`], the settings window in the
/// gui hangs off that
pub const CMD_GAP: Duration = Duration::from_millis(20);

/// the slowest we let anyone set the gap to. past this the app is unusable and
/// its almost certainly a fat fingered number
pub const GAP_MAX: Duration = Duration::from_millis(200);
/// below this the scope starts ignoring commands. we allow it, because its
/// your scope and every failure here is recoverable, but it will get flaky
pub const GAP_RELIABLE_MIN: Duration = Duration::from_millis(15);

// these are generous but not silly. the scope answers a settings read in ~5 ms
// and a waveform in 50-120 ms, so theres plenty of headroom, and when a
// command *does* get dropped we find out in a fraction of a second instead of
// sitting on our hands for over a second. that matters a lot with auto tuning,
// because the cost of a dropout is most of what decides whether shaving a
// millisecond off the gap was worth it
const T_SETTINGS: Duration = Duration::from_millis(400);
const T_SAMPLES: Duration = Duration::from_millis(700);
const T_SCREEN: Duration = Duration::from_millis(4000);
const T_DRAIN: Duration = Duration::from_millis(300);
/// RemoteShell runs a real command on a very slow cpu. `find /` is not quick
const T_SHELL: Duration = Duration::from_millis(8000);

/// how many replies to the wrong question well skip past before admitting the
/// pipe is beyond saving
const SKIP_LIMIT: u32 = 8;

/// longest shell command the scope will reliably run.
///
/// measured, because nothing documents it, and the answer is grim : anything
/// under about 80 characters is solid, 84 to 96 is a coin flip, and past 100
/// it fails every time. what "fails" means is the nasty part. sometimes the
/// read times out, and sometimes the command comes back looking like it
/// succeeded while having done absolutely nothing, which is how a file upload
/// ends up quietly missing most of itself.
///
/// so this is a *conservative* number, not the edge. we check against it and
/// complain loudly rather than letting callers find out the confusing way.
///
/// it bites in hacker mode too : a long one liner will be silently ignored.
pub const MAX_COMMAND: usize = 110;

/// base64 characters per upload command.
///
/// every chunk is its own round trip so bigger would be faster, but see
/// [`MAX_COMMAND`] : theres not much room. 64 encodes 48 bytes and leaves
/// plenty of headroom, which works out at a few hundred bytes a second. fine
/// for a config file, hopeless for anything big.
///
/// must be a multiple of 4 so every chunk is a whole base64 block
const UPLOAD_CHUNK: usize = 64;

/// how many times to prod the scope on startup before giving up on it
const WAKE_TRIES: u32 = 12;
/// short timeout for those prods, since a dropped command never answers and
/// waiting the full 1.2 s for each one would take forever
const T_WAKE: Duration = Duration::from_millis(400);

#[derive(Debug)]
pub enum Error {
    /// no scope on the bus. either its unplugged or udev isnt letting you at it
    NotFound,
    Usb(nusb::Error),
    Transfer(TransferError),
    Protocol(xdso_proto::ProtoError),
    /// the transfer stopped early. usually means someone yanked the cable
    /// mid screenshot
    Truncated { want: usize, got: usize },
    /// the scope keeps answering a question we didnt ask. something upstream
    /// left the pipe full and draining it didnt help
    OutOfSync,
    /// a command we ran on the scope printed something when it should have
    /// been quiet, which means it went wrong
    Shell(String),
    /// longer than the scope will accept. see [`MAX_COMMAND`]
    CommandTooLong(usize),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound => write!(
                f,
                "no DSO5000P found at {VID:04x}:{PID:04x}. plugged in? udev rule installed?"
            ),
            Self::Usb(e) => write!(f, "usb: {e}"),
            Self::Transfer(e) => write!(f, "transfer: {e}"),
            Self::Protocol(e) => write!(f, "protocol: {e}"),
            Self::Truncated { want, got } => write!(f, "short transfer: {got} of {want} bytes"),
            Self::OutOfSync => write!(f, "scope is answering questions we didnt ask"),
            Self::Shell(m) => write!(f, "on the scope: {m}"),
            Self::CommandTooLong(n) => write!(
                f,
                "command is {n} characters, the scope silently drops anything over {MAX_COMMAND}"
            ),
        }
    }
}

impl std::error::Error for Error {}

impl From<nusb::Error> for Error {
    fn from(e: nusb::Error) -> Self {
        Error::Usb(e)
    }
}
impl From<TransferError> for Error {
    fn from(e: TransferError) -> Self {
        Error::Transfer(e)
    }
}
impl From<xdso_proto::ProtoError> for Error {
    fn from(e: xdso_proto::ProtoError) -> Self {
        Error::Protocol(e)
    }
}

/// the scopes idea of the date and time.
///
/// seven bytes on the wire : year little endian u16, then month, day, hour,
/// minute, second. worked out by reading it and recognising the answer
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScopeTime {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
}

impl std::fmt::Display for ScopeTime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
            self.year, self.month, self.day, self.hour, self.minute, self.second
        )
    }
}

/// the scopes own 800x480 screen, decoded out of RGB565
pub struct Screen {
    pub width: usize,
    pub height: usize,
    /// 3 bytes per pixel, r g b
    pub rgb: Vec<u8>,
}

pub struct Scope {
    ep_in: Endpoint<Bulk, In>,
    ep_out: Endpoint<Bulk, Out>,
    /// when we last finished talking to the scope, so we can keep `gap` of
    /// quiet before bothering it again
    last_io: Instant,
    /// how much quiet the scope wants. CMD_GAP unless somebody changed it
    gap: Duration,
    /// kept alive because dropping it releases the interface
    _iface: Interface,
    _dev: Device,
}

impl Scope {
    /// find the scope and claim it.
    ///
    /// the DSO enumerates as a cdc ether gadget so linux binds a network
    /// driver to interface 0. we kick it off, which is why you can lose the
    /// `usb0` interface while this runs. it comes back when you unplug it
    pub fn open() -> Result<Self, Error> {
        let info = nusb::list_devices()
            .wait()?
            .find(|d| d.vendor_id() == VID && d.product_id() == PID)
            .ok_or(Error::NotFound)?;
        let dev = info.open().wait()?;
        let iface = dev.detach_and_claim_interface(INTERFACE).wait()?;
        let mut ep_in = iface.endpoint::<Bulk, In>(EP_IN)?;
        let mut ep_out = iface.endpoint::<Bulk, Out>(EP_OUT)?;

        // the cdc ether driver we just kicked off was using these endpoints,
        // so its data toggle is still sat in the scope while ours starts at
        // zero. mismatched toggle means the scope quietly bins our first
        // command and you get a mystery timeout. CLEAR_FEATURE ENDPOINT_HALT
        // resyncs both ends.
        //
        // the python never noticed this because its poll loop retries forever,
        // so the first command just silently vanished every startup
        let _ = ep_in.clear_halt().wait();
        let _ = ep_out.clear_halt().wait();

        let mut scope = Scope {
            ep_in,
            ep_out,
            last_io: Instant::now() - CMD_GAP,
            gap: CMD_GAP,
            _iface: iface,
            _dev: dev,
        };
        // whatever the last process to talk to it left lying around
        scope.drain();
        scope.wake()?;
        Ok(scope)
    }

    /// prod the scope until it starts answering.
    ///
    /// freshly claimed, it eats the first command. sometimes the first three.
    /// i have no idea what its doing in there and the python had the same
    /// problem, it just never noticed because its poll loop retried forever
    /// and the failures scrolled past. so : ask for settings until we get
    /// settings back, then hand over a device thats known to work
    fn wake(&mut self) -> Result<(), Error> {
        let mut last = Error::OutOfSync;
        for _ in 0..WAKE_TRIES {
            match self.settings_in(T_WAKE) {
                Ok(_) => return Ok(()),
                Err(e) => last = e,
            }
        }
        Err(last)
    }

    /// throw away anything still queued up. call this after an error, before
    /// trying again, or the next reply youll read is the previous ones tail
    pub fn drain(&mut self) {
        while self.read(T_DRAIN).is_ok() {}
    }

    /// how long to stay quiet between commands. clamped to something sane
    pub fn set_gap(&mut self, gap: Duration) {
        self.gap = gap.min(GAP_MAX);
    }

    pub fn gap(&self) -> Duration {
        self.gap
    }

    fn send(&mut self, cmd: u8, data: &[u8]) -> Result<(), Error> {
        self.send_raw(frame::encode(cmd, data))
    }

    /// RemoteShell only. same framing, different magic byte
    fn send_debug(&mut self, cmd: u8, data: &[u8]) -> Result<(), Error> {
        self.send_raw(frame::encode_debug(cmd, data))
    }

    fn send_raw(&mut self, packet: Vec<u8>) -> Result<(), Error> {
        // whoever called us has no idea how long ago the last reply landed, so
        // hold the gap here rather than trusting callers to behave
        let since = self.last_io.elapsed();
        if since < self.gap {
            std::thread::sleep(self.gap - since);
        }
        let r = self
            .ep_out
            .transfer_blocking(Buffer::from(packet), T_SETTINGS)
            .into_result();
        self.last_io = Instant::now();
        r?;
        Ok(())
    }

    /// wait for a reply carrying the command byte we want, skipping past
    /// leftovers from anything that got interrupted earlier
    fn await_reply(&mut self, want: u8, timeout: Duration) -> Result<Vec<u8>, Error> {
        for _ in 0..SKIP_LIMIT {
            let raw = self.read(timeout)?;
            if frame::decode(&raw)?.cmd == want {
                return Ok(raw);
            }
        }
        Err(Error::OutOfSync)
    }

    fn read(&mut self, timeout: Duration) -> Result<Vec<u8>, Error> {
        let done = self.ep_in.transfer_blocking(Buffer::new(READ_LEN), timeout);
        self.last_io = Instant::now();
        Ok(done.into_result()?.into_vec())
    }

    /// the whole front panel state. one packet, 213 bytes, ~5 ms, cheap
    /// enough to poll as often as you like
    pub fn settings(&mut self) -> Result<Settings, Error> {
        self.settings_in(T_SETTINGS)
    }

    fn settings_in(&mut self, timeout: Duration) -> Result<Settings, Error> {
        self.send(frame::cmd::READ_SETTINGS, &[])?;
        // if a previous command got interrupted theres still junk queued, so
        // skip past it instead of failing and leaving the pipe just as blocked
        // for the next caller
        let raw = self.await_reply(frame::reply::SETTINGS, timeout)?;
        Ok(Settings::decode(frame::decode(&raw)?.payload)?)
    }

    /// run a command on the scopes own linux, as root, and get stdout back.
    ///
    /// this is a real busybox shell but it is **not** a pty : theres no state
    /// between calls at all. no cwd, no env, no history, nothing. every line
    /// starts from scratch in `/`. faking that up is the callers problem, see
    /// the gui crates shell module
    ///
    /// replies come back in one packet and cap out somewhere between 4 KB and
    /// 16 KB. ask for 16 KB of output and you get nothing at all, so pipe big
    /// stuff through `head` or read it with ReadFile instead
    pub fn shell(&mut self, cmdline: &str) -> Result<String, Error> {
        // past this the scope either times out or, worse, cheerfully does
        // nothing and reports success. say so instead
        if cmdline.len() > MAX_COMMAND {
            return Err(Error::CommandTooLong(cmdline.len()));
        }
        self.send_debug(frame::cmd::REMOTE_SHELL, cmdline.as_bytes())?;
        let raw = self.await_reply(frame::reply::REMOTE_SHELL, T_SHELL)?;
        Ok(String::from_utf8_lossy(frame::decode(&raw)?.payload).into_owned())
    }

    /// send bytes, get the same bytes back. cheapest possible "are you alive"
    pub fn echo(&mut self, data: &[u8]) -> Result<Vec<u8>, Error> {
        self.send(frame::cmd::ECHO, data)?;
        let raw = self.await_reply(frame::reply::ECHO, T_SETTINGS)?;
        Ok(frame::decode(&raw)?.payload.to_vec())
    }

    /// pull a file off the scopes filesystem.
    ///
    /// this is a proper protocol command rather than anything cobbled together
    /// on top of the shell, so its fast and binary safe. it does give up on
    /// genuinely big files though : 131 KB is fine, 4.4 MB is not, and the cut
    /// off somewhere in between has never been pinned down. carve big ones up
    /// with `dd` over [`Scope::shell`] and fetch the pieces
    pub fn read_file(&mut self, path: &str) -> Result<Vec<u8>, Error> {
        // the leading null is part of the protocol, not a terminator
        let mut payload = vec![0u8];
        payload.extend_from_slice(path.as_bytes());
        self.send(frame::cmd::READ_FILE, &payload)?;

        let mut out = Vec::new();
        let mut skipped = 0;
        loop {
            let raw = self.read(T_SAMPLES)?;
            let packet = frame::decode(&raw)?;
            if packet.cmd != frame::reply::FILE {
                skipped += 1;
                if skipped > SKIP_LIMIT {
                    return Err(Error::OutOfSync);
                }
                continue;
            }
            // same shape as a screenshot : flag byte then the bytes
            match packet.screen_data() {
                Some(data) => out.extend_from_slice(data),
                None => break,
            }
        }
        Ok(out)
    }

    /// push a file onto the scope.
    ///
    /// theres no upload command in the protocol, so this goes the scenic
    /// route : base64 the thing, append it to a temp file a chunk at a time
    /// over the shell, then decode it into place with busyboxs own `base64 -d`.
    ///
    /// that makes it **slow**, on the order of a kilobyte a second, because
    /// every chunk is its own command round trip. fine for a config file or a
    /// script, dont try to put a firmware image through it
    pub fn write_file(&mut self, path: &str, data: &[u8]) -> Result<(), Error> {
        // short name on purpose : every character here is a character we cant
        // spend on the chunk, and MAX_COMMAND is not generous
        const TMP: &str = "/tmp/u.b64";
        // base64 is all letters, digits, + / and =, so it never needs escaping
        // inside quotes. that is the whole reason for encoding it
        let encoded = base64(data);

        self.shell_ok(&format!("rm -f {TMP}"))?;
        for chunk in encoded.as_bytes().chunks(UPLOAD_CHUNK) {
            let chunk = std::str::from_utf8(chunk).expect("base64 is ascii");
            // the chunk goes in bare. base64 has no shell metacharacters in it
            // at all, which is the entire reason were encoding rather than
            // shipping the bytes
            self.shell_ok(&format!("printf %s {chunk} >> {TMP}"))?;
        }

        // check the whole lot actually landed before decoding it. a chunk that
        // goes missing gives you a plausible looking short file otherwise, and
        // youd never know
        let sent = self.count_bytes(TMP)?;
        if sent != encoded.len() {
            return Err(Error::Truncated { want: encoded.len(), got: sent });
        }

        self.shell_ok(&format!("base64 -d {TMP} > {}", shell_quote(path)))?;
        let got = self.count_bytes(path)?;
        self.shell_ok(&format!("rm -f {TMP}"))?;
        if got != data.len() {
            return Err(Error::Truncated { want: data.len(), got });
        }
        Ok(())
    }

    /// how big a file on the scope is, or 0 if it isnt there
    fn count_bytes(&mut self, path: &str) -> Result<usize, Error> {
        let out = self.run(&format!("wc -c < {}", shell_quote(path)))?;
        Ok(out.trim().parse().unwrap_or(0))
    }

    /// run something on the scope where we only care that it didnt complain.
    ///
    /// note the **single** quoted wrapper. a double quoted one looks like it
    /// ought to work and mostly does, right up until you use `>>`, at which
    /// point the command silently does nothing at all and reports success.
    /// took a while to find that one. single quoting with the usual `'\''`
    /// escape works everywhere, so thats what we use, same as hacker mode
    fn shell_ok(&mut self, script: &str) -> Result<(), Error> {
        let out = self.run(script)?;
        if out.trim().is_empty() {
            Ok(())
        } else {
            Err(Error::Shell(out.trim().to_string()))
        }
    }

    /// hand a script to a real shell on the scope and give back its output,
    /// stderr and all
    fn run(&mut self, script: &str) -> Result<String, Error> {
        self.shell(&format!("sh -c {}", shell_quote(&format!("{{ {script} ; }} 2>&1"))))
    }

    /// **does not do what the name says. dont use it.**
    ///
    /// the python called this StartAcquisition and it looks like exactly what
    /// you want, since the Run/Stop key only toggles. it isnt. what it
    /// actually does is set TRIG-STATE to 3 and stop there : the scope then
    /// reports itself as running, to itself and to us, while the acquisition
    /// engine sits there doing nothing and every ReadSampleData comes back
    /// empty. measured :
    ///
    /// | after | TRIG-STATE | samples |
    /// |---|---|---|
    /// | this command | 3 | 0 |
    /// | Run/Stop key | 0 | 0 |
    /// | Run/Stop key again | 3 | 3200 |
    ///
    /// so it leaves the scope lying about itself, and the only way out is to
    /// toggle Run/Stop twice. press the key instead, see `xdso_proto::keys`.
    /// kept here so the next person doesnt have to find this out the hard way
    pub fn start_acquisition(&mut self) -> Result<(), Error> {
        self.send(frame::cmd::CONTROL, &[0x00, 0x00])?;
        self.await_reply(frame::reply::CONTROL, T_SETTINGS)?;
        Ok(())
    }

    /// what the scope thinks the time is.
    ///
    /// almost certainly wrong. mine is convinced its 2018, and the file
    /// timestamps in its filesystem agree with it, so at least its
    /// consistently wrong
    pub fn system_time(&mut self) -> Result<ScopeTime, Error> {
        self.send(frame::cmd::READ_SYSTEM_TIME, &[])?;
        let raw = self.await_reply(frame::reply::SYSTEM_TIME, T_SETTINGS)?;
        let p = frame::decode(&raw)?.payload;
        if p.len() < 7 {
            return Err(Error::Truncated { want: 7, got: p.len() });
        }
        Ok(ScopeTime {
            year: u16::from_le_bytes([p[0], p[1]]),
            month: p[2],
            day: p[3],
            hour: p[4],
            minute: p[5],
            second: p[6],
        })
    }

    /// lock the front panel so nobody can fiddle with the scope while youre
    /// driving it from here. unlocking again is on you
    pub fn lock_panel(&mut self, locked: bool) -> Result<(), Error> {
        self.send(frame::cmd::CONTROL, &[0x01, u8::from(locked)])?;
        self.await_reply(frame::reply::CONTROL, T_SETTINGS)?;
        Ok(())
    }

    /// 3200 samples for one channel, as screen counts (128 is the centre).
    ///
    /// comes back empty while acquisition is stopped, which is not an error,
    /// the scope just has nothing to give you.
    ///
    /// the wire format is **signed** : sample 0 means the graticule centre, so
    /// xoring 0x80 turns it into the offset binary counts everything else
    /// uses. this caught me out for ages so, the evidence : with ch1 flat at
    /// position -62 the raw bytes read 196.65 while the scope draws the trace
    /// at count 67.68. exactly 128 apart, and 196.65 ^ 0x80 = 68.65. same for
    /// ch2. the offset shows up as +128 or -128 depending which side of centre
    /// youre on, and those are the same thing mod 256, which is what the xor
    /// is really saying
    pub fn samples(&mut self, ch: Channel) -> Result<Vec<u8>, Error> {
        self.samples_raw(ch.index() as u8)
    }

    /// the math trace, or empty if the math channel is switched off.
    ///
    /// ReadSampleData takes a channel index and **3 is the math channel**.
    /// nothing documents that. index 2 comes back as a copy of ch1, 3 is the
    /// real thing : it has its own vertical position (it doesnt move when you
    /// shift ch1 up the screen) and it goes empty the moment MATH-DISP goes
    /// to 0, which is how it got confirmed.
    ///
    /// the bytes are signed on the wire like the channel traces, so the same
    /// 0x80 flip applies. that holds for the arithmetic modes and for an fft.
    ///
    /// for an fft only part of the buffer is spectrum and the rest is the
    /// time domain data it was computed from, still live. how much is
    /// **not settled** : i measured 625, 820 and 1070 by three different
    /// methods and they cant all be right, so the gui doesnt draw an fft at
    /// all rather than draw a frequency axis that might be a third out. see
    /// TODO.md for what was and wasnt established.
    ///
    /// theres no volts per division field for math anywhere in protocol.inf,
    /// so we can draw it but we cant tell you what its amplitude is. for an
    /// fft the vertical scale is `MATH-FFT-DB`, which the **channel V/div
    /// knob** drives while youre in fft mode rather than the channels own VB
    /// field. thats worth knowing : turning V/div in fft mode looks like it
    /// does nothing at all if you only watch `VERT-CHn-VB`
    pub fn math_samples(&mut self) -> Result<Vec<u8>, Error> {
        self.samples_raw(MATH_CHANNEL)
    }

    /// same, but by raw channel index, with the signed flip applied. 0 and 1
    /// are the two inputs, 3 is math, 2 appears to be a copy of ch1
    pub fn samples_raw(&mut self, ch: u8) -> Result<Vec<u8>, Error> {
        Ok(self.samples_unflipped(ch)?.iter().map(|b| b ^ 0x80).collect())
    }

    /// exactly what came off the wire, no flip. only the fft wants this
    pub fn samples_unflipped(&mut self, ch: u8) -> Result<Vec<u8>, Error> {
        self.send(frame::cmd::READ_SAMPLE_DATA, &[0x01, ch])?;
        let mut out = Vec::with_capacity(SAMPLES);
        let mut skipped = 0;
        loop {
            let raw = self.read(T_SAMPLES)?;
            let packet = frame::decode(&raw)?;
            if packet.cmd != frame::reply::SAMPLE_DATA {
                skipped += 1;
                if skipped > SKIP_LIMIT {
                    return Err(Error::OutOfSync);
                }
                continue; // leftovers from an interrupted command, bin it
            }
            match packet.sample_data() {
                Some(data) => out.extend_from_slice(data),
                None if packet.samples_done() => break,
                None => {} // empty packet, acquisition is stopped
            }
        }
        Ok(out)
    }

    /// the scopes actual screen. 77 packets, ~980 ms, and theres no way to
    /// make it faster so only do this when someone asks for it
    pub fn screenshot(&mut self) -> Result<Screen, Error> {
        self.send(frame::cmd::SCREENSHOT, &[])?;
        let want = SCREEN_W * SCREEN_H * 2;
        let mut buf = Vec::with_capacity(want);
        let mut skipped = 0;
        loop {
            let raw = self.read(T_SCREEN)?;
            let packet = frame::decode(&raw)?;
            if packet.cmd != frame::reply::SCREENSHOT {
                skipped += 1;
                if skipped > SKIP_LIMIT {
                    return Err(Error::OutOfSync);
                }
                continue;
            }
            // unlike sample data, anything that isnt MORE ends a screenshot
            match packet.screen_data() {
                Some(data) => buf.extend_from_slice(data),
                None => break,
            }
        }
        if buf.len() != want {
            return Err(Error::Truncated { want, got: buf.len() });
        }
        Ok(Screen { width: SCREEN_W, height: SCREEN_H, rgb: rgb565_to_rgb(&buf) })
    }

    /// press a front panel button. `code` is the line number in
    /// keyprotocol.inf, see `xdso_proto::keys`
    pub fn press(&mut self, code: u8) -> Result<(), Error> {
        self.send(frame::cmd::KEY_TRIGGER, &[code, 0x01])?;
        // it acks, but the ack carries nothing useful and sometimes doesnt
        // turn up at all. dont care either way
        let _ = self.read(T_SETTINGS);
        Ok(())
    }
}

/// RGB565 little endian -> 8 bit rgb triples.
///
/// the low bits get dropped rather than replicated, so a full white pixel
/// comes out as f8 fc f8 instead of ff ff ff. nobody has ever noticed
fn rgb565_to_rgb(raw: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(raw.len() / 2 * 3);
    for px in raw.chunks_exact(2) {
        let v = u16::from_le_bytes([px[0], px[1]]);
        out.push((((v >> 11) & 0x1f) << 3) as u8);
        out.push((((v >> 5) & 0x3f) << 2) as u8);
        out.push(((v & 0x1f) << 3) as u8);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rgb565_unpacks_the_right_channels() {
        // pure red, pure green, pure blue, black
        let raw = [0x00, 0xf8, 0xe0, 0x07, 0x1f, 0x00, 0x00, 0x00];
        assert_eq!(
            rgb565_to_rgb(&raw),
            vec![248, 0, 0, 0, 252, 0, 0, 0, 248, 0, 0, 0]
        );
    }

    #[test]
    fn a_whole_frame_is_the_right_size() {
        let raw = vec![0u8; SCREEN_W * SCREEN_H * 2];
        assert_eq!(rgb565_to_rgb(&raw).len(), SCREEN_W * SCREEN_H * 3);
    }
}

/// wrap in single quotes for the shell, escaping any single quotes by closing,
/// escaping and reopening. the usual `'\''` dance
fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// base64, because pulling in a crate for twenty lines of table lookup would
/// be daft. no line wrapping, the scope doesnt care
fn base64(data: &[u8]) -> String {
    const ALPHABET: &[u8; 64] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for group in data.chunks(3) {
        let b = [group[0], *group.get(1).unwrap_or(&0), *group.get(2).unwrap_or(&0)];
        let n = (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32;
        out.push(ALPHABET[(n >> 18 & 63) as usize] as char);
        out.push(ALPHABET[(n >> 12 & 63) as usize] as char);
        out.push(if group.len() > 1 { ALPHABET[(n >> 6 & 63) as usize] as char } else { '=' });
        out.push(if group.len() > 2 { ALPHABET[(n & 63) as usize] as char } else { '=' });
    }
    out
}

#[cfg(test)]
mod b64_tests {
    use super::base64;

    #[test]
    fn matches_the_usual_answers() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foob"), "Zm9vYg==");
        assert_eq!(base64(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn stays_inside_the_safe_alphabet() {
        // the whole point is that it never needs shell escaping
        let all: Vec<u8> = (0..=255u8).collect();
        for c in base64(&all).chars() {
            assert!(
                c.is_ascii_alphanumeric() || c == '+' || c == '/' || c == '=',
                "{c} would need quoting"
            );
        }
    }
}
