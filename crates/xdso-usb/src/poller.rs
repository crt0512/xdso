//! the thread that actually talks to the scope.
//!
//! it owns the [`Scope`] outright, so nothing else can accidentally issue a
//! command in the middle of one of ours. the ui pushes [`Command`]s down a
//! channel and reads the latest [`Snapshot`] back, and never blocks on usb.
//! thats the whole idea : a screenshot takes a second, and during that second
//! the window still drags, buttons still light up, nothing spins.
//!
//! the python version had a lock and fired a fresh thread per button press,
//! which worked but meant a key press could land between the two halves of a
//! waveform read. this cant.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use xdso_proto::{Channel, Settings};

use crate::device::{CMD_GAP, Scope, Screen};

/// what the poller spends its time pulling off the scope
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Feed {
    /// 3200 samples a channel, ~13 fps, and we draw the display ourselves
    #[default]
    Waveform,
    /// the scopes own framebuffer at a whole 1 fps. the only way to see its
    /// menus, and it costs you the waveform feed while youre in it
    Screen,
    /// settings only, no traces at all.
    ///
    /// remote mode uses this. theres no display in remote mode so fetching
    /// 3200 samples a channel is two round trips a frame spent on data
    /// nobody is looking at, and it puts every button press behind them.
    /// reading just the settings keeps the sidebar honest and leaves the pipe
    /// almost entirely to the buttons, which is the whole point of the mode
    Settings,
    /// nothing at all. hacker mode uses this so the shell gets the whole usb
    /// pipe to itself instead of queueing behind a waveform read every 80 ms
    Paused,
}

/// knobs you can twiddle at runtime. the settings window in the gui is just a
/// front end for this
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tuning {
    /// quiet time between commands, when youre setting it by hand. the single
    /// biggest lever on frame rate
    pub cmd_gap: Duration,
    /// poll settings every nth waveform pass. they barely ever change and
    /// asking every time costs a whole round trip for nothing
    pub settings_every: u32,
    /// let the poller find the gap itself, see [`Worker::retune`]
    pub auto: bool,
    /// how low auto tuning is allowed to push the gap
    pub gap_min: Duration,
    /// and how high. past this were not compensating any more, somethings
    /// properly wrong
    pub gap_max: Duration,
}

impl Default for Tuning {
    fn default() -> Self {
        Tuning {
            cmd_gap: CMD_GAP,
            settings_every: 5,
            auto: true,
            // 18 ms, not 15, on purpose. the scope will *answer* at 15 but its
            // right on the edge, and a dropped command costs far more than the
            // few milliseconds a frame you save : you wait out a read timeout,
            // drain the pipe and sit out a retry. measured slower overall than
            // just leaving a bit of room. you can wind it down in settings if
            // you want to live dangerously
            gap_min: Duration::from_millis(18),
            gap_max: Duration::from_millis(60),
        }
    }
}

/// how long a clean run has to be before auto tuning tries shaving another
/// millisecond off. short enough to settle quickly, long enough not to
/// oscillate
const AUTO_SETTLE: Duration = Duration::from_secs(2);
/// how long to doze between checks for work while the feed is paused. short
/// enough that a button press still feels instant, long enough to not spin
const IDLE_TICK: Duration = Duration::from_millis(40);

/// how long to leave between settings reads on the settings only feed. the
/// sidebar keeps up fine at this and it leaves plenty of room for keys
const SETTINGS_TICK: Duration = Duration::from_millis(120);

/// one step, up or down
const AUTO_STEP: Duration = Duration::from_millis(1);
/// how long we respect a gap that failed before trying under it again.
///
/// without this the tuner sits on the edge oscillating : creep down, drop a
/// command, back off, creep down again, every few seconds forever. a dropped
/// command costs a read timeout plus a drain plus a retry, which is far more
/// than the millisecond it was trying to save
const BAD_GAP_AMNESTY: Duration = Duration::from_secs(30);

/// something for the scope to do, whenever it next gets a gap
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// press a front panel key by its keyprotocol.inf code
    Press(u8),
    /// grab the scopes own screen once, without changing feed
    Screenshot,
    /// waveforms or live screen
    SetFeed(Feed),
    /// poll only this channel, or None for whatever the scope has enabled.
    /// one channel is roughly twice the frame rate
    ForceChannel(Option<Channel>),
    /// change the timings
    Tune(Tuning),
    /// run a command on the scopes own linux. the answer comes back through
    /// [`Poller::shell_replies`]
    Shell(String),
    /// start acquisition. unlike the Run/Stop key this doesnt toggle
    StartAcquisition,
    /// pull a file off the scope and write it here
    Download { remote: String, local: String },
    /// push a local file onto the scope. slow, see `Scope::write_file`
    Upload { local: String, remote: String },
    /// pack it in
    Stop,
}

/// what came back from a [`Command::Shell`]
#[derive(Debug, Clone)]
pub struct ShellReply {
    pub cmdline: String,
    pub output: Result<String, String>,
}

/// everything the ui needs to draw a frame. cheap to clone
#[derive(Clone, Default)]
pub struct Snapshot {
    pub settings: Settings,
    /// raw screen counts per channel, None if that channel is off, stopped, or
    /// were in screen feed and not asking for waveforms at all
    pub waves: [Option<Arc<Vec<u8>>>; 2],
    /// the math trace, same units, only polled when the scope says its on
    pub math: Option<Arc<Vec<u8>>>,
    /// waveform polls per second, measured not guessed
    pub fps: f32,
    /// last thing that went wrong, cleared on the next good poll
    pub error: Option<String>,
    /// most recent screen grab, live feed or one shot
    pub screen: Option<Arc<Screen>>,
    /// bumps every time `screen` is replaced, so the ui knows when to bother
    /// re-uploading the texture
    pub screen_seq: u64,
    /// true while a screen grab is in flight, so the ui can say so
    pub grabbing: bool,
    pub feed: Feed,
    /// the gap were actually using right now. with auto tuning on this
    /// wanders about on its own, so the ui shows it rather than the setting
    pub gap: Duration,
}

impl Snapshot {
    /// nothing has come back from the scope yet
    pub fn is_cold(&self) -> bool {
        self.settings.is_empty()
    }

    pub fn wave(&self, ch: Channel) -> Option<&[u8]> {
        self.waves[ch.index()].as_deref().map(|v| v.as_slice())
    }
}

pub struct Poller {
    tx: Sender<Command>,
    shell_rx: Receiver<ShellReply>,
    shared: Arc<Mutex<Snapshot>>,
    running: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl Poller {
    /// take the scope and start pulling data off it.
    ///
    /// `wake` gets called every time theres a new snapshot. in the gui thats
    /// `ctx.request_repaint()`, so the window redraws when theres actually
    /// something new instead of spinning at 60 fps for no reason
    pub fn spawn<F>(scope: Scope, force: Option<Channel>, wake: F) -> Poller
    where
        F: Fn() + Send + 'static,
    {
        let (tx, rx) = channel();
        let (shell_tx, shell_rx) = channel();
        let shared = Arc::new(Mutex::new(Snapshot::default()));
        let running = Arc::new(AtomicBool::new(true));
        let handle = {
            let shared = Arc::clone(&shared);
            let running = Arc::clone(&running);
            std::thread::Builder::new()
                .name("xdso-poller".into())
                .spawn(move || {
                    let mut w = Worker {
                        scope,
                        force,
                        feed: Feed::default(),
                        tuning: Tuning::default(),
                        last_trouble: Instant::now(),
                        known_bad: None,
                        retry_bad_at: Instant::now(),
                        state: Snapshot::default(),
                        shell_tx,
                        shared,
                        wake,
                    };
                    w.run(rx);
                    running.store(false, Ordering::Relaxed);
                })
                .expect("spawning a thread should not fail")
        };
        Poller { tx, shell_rx, shared, running, handle: Some(handle) }
    }

    /// whatever the poller last managed to read
    pub fn snapshot(&self) -> Snapshot {
        self.shared.lock().expect("poller thread panicked").clone()
    }

    /// queue something up. silently does nothing if the thread is already gone
    pub fn send(&self, cmd: Command) {
        let _ = self.tx.send(cmd);
    }

    /// anything that came back from a [`Command::Shell`] since you last asked
    pub fn shell_replies(&self) -> impl Iterator<Item = ShellReply> + '_ {
        self.shell_rx.try_iter()
    }

    /// false once the thread has stopped, eg because the scope went away
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::Relaxed)
    }
}

impl Drop for Poller {
    fn drop(&mut self) {
        self.send(Command::Stop);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

/// give up after this many failures in a row and let the thread die, so the ui
/// can say the link is gone and offer to reconnect.
///
/// theres a 100 ms sleep after each failure, so this is several seconds of
/// solid nothing. a transient usb hiccup recovers long before it, but an
/// unplugged cable never will, and sitting there retrying forever just means
/// the window carries on claiming to be live
const GIVE_UP_AFTER: u32 = 25;

struct Worker<F: Fn()> {
    scope: Scope,
    force: Option<Channel>,
    feed: Feed,
    tuning: Tuning,
    /// when we last saw a failure, which is also the clock auto tuning uses to
    /// decide its been quiet long enough to try going faster
    last_trouble: Instant,
    /// the fastest gap weve actually seen fail. we sit a step above it rather
    /// than rediscovering it every few seconds
    known_bad: Option<Duration>,
    /// when were next willing to poke below `known_bad` again, in case the
    /// scope was only busy for a moment
    retry_bad_at: Instant,
    state: Snapshot,
    shell_tx: Sender<ShellReply>,
    shared: Arc<Mutex<Snapshot>>,
    wake: F,
}

impl<F: Fn()> Worker<F> {
    fn run(&mut self, rx: Receiver<Command>) {
        let mut last = Instant::now();
        let mut pass: u32 = 0;
        let mut failures: u32 = 0;

        loop {
            // commands first. a button the user just clicked should not wait
            // behind a waveform read, never mind a screenshot
            loop {
                match rx.try_recv() {
                    Ok(Command::Stop) | Err(TryRecvError::Disconnected) => return,
                    Err(TryRecvError::Empty) => break,
                    Ok(cmd) => self.handle(cmd),
                }
            }

            if self.feed == Feed::Paused {
                // publish once on the way in so the ui stops showing a frame
                // rate for something that isnt running, then keep quiet
                if self.state.fps != 0.0 {
                    self.state.fps = 0.0;
                    self.publish();
                }
                std::thread::sleep(IDLE_TICK);
                continue;
            }

            let result = match self.feed {
                Feed::Waveform => self.poll_waveform(pass),
                Feed::Screen => self.poll_screen(),
                Feed::Settings => self.poll_settings(),
                Feed::Paused => unreachable!("handled above"),
            };

            match result {
                Ok(()) => {
                    let now = Instant::now();
                    self.state.fps = 1.0 / now.duration_since(last).as_secs_f32().max(1e-6);
                    last = now;
                    pass = pass.wrapping_add(1);
                    failures = 0;
                    self.state.error = None;
                    self.retune_down();
                }
                Err(e) => {
                    self.state.error = Some(e.to_string());
                    failures += 1;
                    self.retune_up();
                    if failures >= GIVE_UP_AFTER {
                        // its not coming back on its own. stop, so the ui can
                        // hold up the last frame and offer to reconnect
                        self.publish();
                        return;
                    }
                    // something is out of step. bin whatever is queued and
                    // give the scope a moment before having another go
                    self.scope.drain();
                    std::thread::sleep(Duration::from_millis(100));
                }
            }
            self.publish();
        }
    }

    fn handle(&mut self, cmd: Command) {
        match cmd {
            Command::Stop => {}
            Command::ForceChannel(c) => self.force = c,
            Command::Tune(t) => {
                let was_auto = self.tuning.auto;
                self.tuning = t;
                if t.auto {
                    // coming out of manual, start from wherever we were and
                    // let it settle, rather than jumping to a limit
                    let from = if was_auto { self.scope.gap() } else { t.cmd_gap };
                    self.scope.set_gap(from.clamp(t.gap_min, t.gap_max));
                } else {
                    self.scope.set_gap(t.cmd_gap);
                }
            }
            Command::SetFeed(f) => {
                self.feed = f;
                if f != Feed::Waveform {
                    // stale traces under a live screen, or under a terminal,
                    // would just be confusing
                    self.state.waves = [None, None];
                    self.state.math = None;
                }
            }
            Command::Press(code) => {
                if let Err(e) = self.scope.press(code) {
                    self.state.error = Some(format!("key {code}: {e}"));
                    self.scope.drain();
                }
            }
            Command::StartAcquisition => {
                if let Err(e) = self.scope.start_acquisition() {
                    self.state.error = Some(format!("start: {e}"));
                    self.scope.drain();
                }
            }
            Command::Screenshot => {
                if let Err(e) = self.grab_screen() {
                    self.state.error = Some(format!("screenshot: {e}"));
                    self.scope.drain();
                }
            }
            Command::Shell(cmdline) => {
                let output = self.scope.shell(&cmdline).map_err(|e| {
                    self.scope.drain();
                    e.to_string()
                });
                let _ = self.shell_tx.send(ShellReply { cmdline, output });
            }
            Command::Download { remote, local } => {
                let output = self.download(&remote, &local).inspect_err(|_| {
                    self.scope.drain();
                });
                let _ = self.shell_tx.send(ShellReply { cmdline: remote, output });
            }
            Command::Upload { local, remote } => {
                let output = self.upload(&local, &remote).inspect_err(|_| {
                    self.scope.drain();
                });
                let _ = self.shell_tx.send(ShellReply { cmdline: remote, output });
            }
        }
    }

    fn download(&mut self, remote: &str, local: &str) -> Result<String, String> {
        let data = self.scope.read_file(remote).map_err(|e| e.to_string())?;
        std::fs::write(local, &data).map_err(|e| format!("cant write {local}: {e}"))?;
        Ok(format!("pulled {} bytes into {local}", data.len()))
    }

    fn upload(&mut self, local: &str, remote: &str) -> Result<String, String> {
        let data = std::fs::read(local).map_err(|e| format!("cant read {local}: {e}"))?;
        let n = data.len();
        self.scope.write_file(remote, &data).map_err(|e| e.to_string())?;
        Ok(format!("pushed {n} bytes to {remote}"))
    }

    /// something went wrong, so back off a millisecond.
    ///
    /// the scope stops answering when you talk to it too fast, and how fast is
    /// too fast depends on what its busy doing, so theres no one right number
    /// to hard code. this finds it
    fn retune_up(&mut self) {
        self.last_trouble = Instant::now();
        if !self.tuning.auto {
            return;
        }
        let failed_at = self.scope.gap();
        // remember what didnt work, so we settle a step above it instead of
        // walking back down into it every couple of seconds
        self.known_bad = Some(self.known_bad.map_or(failed_at, |b| b.max(failed_at)));
        self.retry_bad_at = Instant::now() + BAD_GAP_AMNESTY;
        self.scope.set_gap((failed_at + AUTO_STEP).min(self.tuning.gap_max));
    }

    /// its been quiet for a while, so try going a millisecond faster.
    ///
    /// resets the clock afterwards, so we creep down one step at a time rather
    /// than sprinting to the floor the moment things settle
    fn retune_down(&mut self) {
        if !self.tuning.auto || self.last_trouble.elapsed() < AUTO_SETTLE {
            return;
        }
        // dont creep back under something we already know fails, at least not
        // until the amnesty is up and its worth another go
        let floor = match self.known_bad {
            Some(bad) if Instant::now() < self.retry_bad_at => {
                (bad + AUTO_STEP).max(self.tuning.gap_min)
            }
            _ => self.tuning.gap_min,
        };
        let now = self.scope.gap();
        if now > floor {
            self.scope.set_gap((now - AUTO_STEP).max(floor));
        }
        self.last_trouble = Instant::now();
    }

    /// remote mode : the settings and nothing else.
    ///
    /// paced rather than run flat out. the settings barely ever change and
    /// hammering them at 50 a second would put the button presses right back
    /// behind a read again, which is exactly what this feed exists to avoid
    fn poll_settings(&mut self) -> Result<(), crate::Error> {
        self.state.settings = self.scope.settings()?;
        self.state.waves = [None, None];
        self.state.math = None;
        std::thread::sleep(SETTINGS_TICK);
        Ok(())
    }

    fn poll_waveform(&mut self, pass: u32) -> Result<(), crate::Error> {
        if pass % self.tuning.settings_every.max(1) == 0 {
            self.state.settings = self.scope.settings()?;
        }
        // an fft takes the display over completely : the scope stops drawing
        // the channel traces and we stop drawing them too, so fetching them
        // is two round trips a frame spent on data nobody looks at. it also
        // keeps us off the scopes back while its busy doing the transform,
        // which is exactly when it can least spare the attention
        let fft = self.state.settings.math().is_fft();

        for ch in Channel::ALL {
            let wanted = !fft
                && self.force.is_none_or(|f| f == ch)
                && self.state.settings.channel(ch).enabled;
            if !wanted {
                self.state.waves[ch.index()] = None;
                continue;
            }
            let w = self.scope.samples(ch)?;
            // a short read means acquisition stopped mid transfer. treat it as
            // no data rather than drawing half a trace
            self.state.waves[ch.index()] =
                (w.len() == xdso_proto::SAMPLES).then(|| Arc::new(w));
        }

        // math costs another whole round trip, so only when its actually on.
        // that includes an fft, the spectrum comes down this same channel
        let _ = fft;
        self.state.math = if self.state.settings.math().enabled {
            let w = self.scope.math_samples()?;
            (w.len() == xdso_proto::SAMPLES).then(|| Arc::new(w))
        } else {
            None
        };
        Ok(())
    }

    /// one screen grab per pass, which is about 1 fps and there is nothing to
    /// be done about that. settings come along for the ride since 25 ms on top
    /// of 1030 is neither here nor there
    fn poll_screen(&mut self) -> Result<(), crate::Error> {
        self.state.settings = self.scope.settings()?;
        self.grab_screen()
    }

    fn grab_screen(&mut self) -> Result<(), crate::Error> {
        self.state.grabbing = true;
        self.publish();
        let got = self.scope.screenshot();
        self.state.grabbing = false;
        self.state.screen = Some(Arc::new(got?));
        self.state.screen_seq = self.state.screen_seq.wrapping_add(1);
        Ok(())
    }

    fn publish(&self) {
        if let Ok(mut guard) = self.shared.lock() {
            let mut next = self.state.clone();
            next.feed = self.feed;
            next.gap = self.scope.gap();
            *guard = next;
        }
        (self.wake)();
    }
}
