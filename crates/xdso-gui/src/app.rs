//! the app itself. holds the link to the scope and draws a frame.
//!
//! the ui never touches usb. it reads whatever the poller last managed to get
//! and pushes [`Command`]s back the other way, so a click always feels instant
//! even when the scope is busy shipping a screenshot.
//!
//! theres three views and two layouts :
//!
//! | view | what youre looking at |
//! |---|---|
//! | waveform | samples, drawn here. ~13 fps |
//! | live screen | the scopes own framebuffer. 1 fps, but you can see its menus |
//! | hacker | a shell on the scopes linux, see [`crate::shell`] |
//!
//! and the layout picks whether the buttons sit under the display like a
//! normal program, or all round it like the actual scope.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Receiver;

use eframe::egui;
use egui::{Color32, Context, RichText, TextureHandle, Ui};

use xdso_proto::{CENTRE_COUNT, COUNTS_PER_DIV, Channel, units::eng};
use xdso_usb::{Command, Feed, Poller, Scope, Snapshot, Tuning};

use crate::plot::Graticule;
use crate::shell::{self, Shell};
use crate::sidebar::{self, Volts};
use crate::theme;
use crate::{keypad, panel, settings};

/// keys the host keeps for itself rather than passing to the scope
const K_SCREEN: char = 'g';
const K_SAVE: char = 'p';

/// how long auto tuning has to sit above its floor before the button goes
/// amber. short enough to notice, long enough that one bad frame doesnt do it
const AUTO_NAG: f64 = 5.0;

/// whole number only. the banner is pixel art and anything fractional makes a
/// mess of it
const LOGO_SCALE: f32 = 1.0;

/// how long an old frame keeps showing, faded, by default.
///
/// on by default because the scope gets slow exactly when you least want a
/// flickering trace : doing an fft it manages a couple of frames a second,
/// and without this the spectrum jumps about too much to read
const DEFAULT_PERSIST: f32 = 0.7;
/// as far as the slider goes
const MAX_PERSIST: f32 = 4.0;

/// how wide the F key strip is in remote mode
const F_STRIP: f32 = 56.0;
/// and the tallest well let that column get. on a tall window a full height
/// one looks ridiculous and you have to reach for it
const F_COLUMN_MAX: f32 = 430.0;

/// where the buttons live
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Layout {
    /// buttons arranged like the real front panel, see [`crate::panel`].
    ///
    /// the default, because this is a scope and it should look like one
    #[default]
    Scope,
    /// buttons in a grid under the display, like a program
    Computer,
    /// the front panel and nothing else. no display at all.
    ///
    /// for when the scope is sat in front of you and youre using the computer
    /// as a remote control, so drawing the trace over here is wasted effort.
    /// the poller drops to [`Feed::Settings`] to match, which is what makes
    /// the buttons feel as quick as they do
    Remote,
}

impl Layout {
    /// index into the per layout sidebar preference
    fn idx(self) -> usize {
        self as usize
    }

    /// how it goes in the config file. a number rather than the name so a
    /// typo in a rename cant silently reset everyones layout
    fn tag(self) -> u8 {
        self as u8
    }

    fn of_tag(n: u8) -> Layout {
        match n {
            1 => Layout::Computer,
            2 => Layout::Remote,
            _ => Layout::Scope,
        }
    }
}

enum Link {
    Up(Box<Poller>),
    /// couldnt open the scope, or it went away. holds the moan
    Down(String),
}

/// keep trying to open the scope on a thread, and hand it over when it turns
/// up.
///
/// opening takes the best part of a second (theres a drain and then the wake
/// handshake) so doing it on the ui thread would stutter every couple of
/// seconds for as long as the scope was away. the flag lets the app call the
/// whole thing off when it shuts down
fn reconnect_loop(ctx: Context, alive: Arc<AtomicBool>) -> Receiver<Scope> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("xdso-reconnect".into())
        .spawn(move || {
            while alive.load(Ordering::Relaxed) {
                if let Ok(scope) = Scope::open() {
                    let _ = tx.send(scope);
                    ctx.request_repaint();
                    return;
                }
                std::thread::sleep(std::time::Duration::from_secs(2));
            }
        })
        .expect("spawning a thread should not fail");
    rx
}

pub struct App {
    link: Link,
    force: Option<Channel>,
    tuning: Tuning,
    feed: Feed,
    layout: Layout,
    /// hacker mode takes over the whole window, so its not part of Layout
    hacker: bool,
    shell: Shell,
    /// banner, uploaded once on the first frame
    logo: Option<TextureHandle>,
    /// the scopes own screen, uploaded to the gpu when a new one turns up
    screen: Option<TextureHandle>,
    screen_seq: u64,
    show_settings: bool,
    /// remembered per layout. on by default in all three, but scope mode is
    /// where the room actually gets tight so its worth being able to drop it
    show_sidebar: [bool; 3],
    /// true while the link is down but were still showing the last frame
    offline: bool,
    /// when auto tuning first pushed the gap above its floor, so we can nag
    /// about it once its clear this isnt a passing hiccup
    slow_since: Option<f64>,
    /// have we told the poller about the tuning we loaded from last time
    tuning_sent: bool,
    /// how long old frames hang about before theyve faded out entirely.
    /// 0 turns it off
    persist: f32,
    /// ch1, ch2, math
    trails: [crate::trail::Trail; 3],
    /// someone clicked save csv, deal with it once we have the volts to hand
    want_csv: bool,
    /// the csv thread reports back through these, since it has a dialog up
    /// and we are not waiting around for it
    save_tx: std::sync::mpsc::Sender<String>,
    save_rx: Receiver<String>,
    /// what happened to the last png or csv we tried to write
    saved: Option<String>,
    /// the thread thats waiting for the scope to come back, if theres one
    reconnect: Option<Receiver<Scope>>,
    /// dropped to false on the way out, so that thread stops looping
    alive: Arc<AtomicBool>,
    /// the last snapshot we got while the scope was still talking to us.
    ///
    /// when the link drops we keep drawing this rather than throwing the
    /// traces away and showing a sad face. youre usually looking at a scope
    /// because you want to see the last thing it caught
    last: Snapshot,
}

/// keys we keep in eframes little config file.
///
/// deliberately all primitives. storing the structs themselves would mean
/// serde derives on `Tuning` all the way down in the usb crate, and that crate
/// has no business knowing this app exists
mod key {
    /// which of [`super::Layout`] youre in, by its tag. this replaced an
    /// older "scope-layout" bool, which couldnt say remote
    pub const LAYOUT: &str = "layout";
    pub const SIDEBAR_SCOPE: &str = "sidebar-scope";
    pub const SIDEBAR_COMPUTER: &str = "sidebar-computer";
    pub const SIDEBAR_REMOTE: &str = "sidebar-remote";
    pub const AUTO_GAP: &str = "auto-gap";
    pub const GAP_MIN_MS: &str = "gap-min-ms";
    pub const GAP_MAX_MS: &str = "gap-max-ms";
    pub const SETTINGS_EVERY: &str = "settings-every";
    pub const PERSIST: &str = "persist";
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, force: Option<Channel>) -> App {
        // 0.36 keeps a style per theme, so set both rather than fighting
        // whatever the desktop says it prefers. this app is dark either way
        cc.egui_ctx.all_styles_mut(|style| {
            style.visuals.panel_fill = theme::BG;
            style.visuals.window_fill = theme::BG;
            style.visuals.extreme_bg_color = theme::BG;
        });

        // whatever we were set to last time, falling back to the defaults
        let mut tuning = Tuning::default();
        let mut layout = Layout::default();
        let mut sidebar = [true, true, true];
        let mut persist = DEFAULT_PERSIST;
        if let Some(store) = cc.storage {
            let get = |k: &str, d: bool| eframe::get_value(store, k).unwrap_or(d);
            if let Some(n) = eframe::get_value::<u8>(store, key::LAYOUT) {
                layout = Layout::of_tag(n);
            }
            sidebar = [
                get(key::SIDEBAR_SCOPE, true),
                get(key::SIDEBAR_COMPUTER, true),
                get(key::SIDEBAR_REMOTE, true),
            ];
            tuning.auto = get(key::AUTO_GAP, tuning.auto);
            if let Some(ms) = eframe::get_value::<u64>(store, key::GAP_MIN_MS) {
                tuning.gap_min = std::time::Duration::from_millis(ms);
            }
            if let Some(ms) = eframe::get_value::<u64>(store, key::GAP_MAX_MS) {
                tuning.gap_max = std::time::Duration::from_millis(ms);
            }
            if let Some(n) = eframe::get_value::<u32>(store, key::SETTINGS_EVERY) {
                tuning.settings_every = n.max(1);
            }
            if let Some(v) = eframe::get_value::<f32>(store, key::PERSIST) {
                persist = v.clamp(0.0, MAX_PERSIST);
            }
        }

        let (save_tx, save_rx) = std::sync::mpsc::channel();

        App {
            link: connect(cc.egui_ctx.clone(), force),
            force,
            tuning,
            feed: Feed::default(),
            layout,
            hacker: false,
            shell: Shell::default(),
            logo: None,
            screen: None,
            screen_seq: 0,
            show_settings: false,
            show_sidebar: sidebar,
            offline: false,
            slow_since: None,
            tuning_sent: false,
            persist,
            trails: Default::default(),
            want_csv: false,
            save_tx,
            save_rx,
            saved: None,
            reconnect: None,
            alive: Arc::new(AtomicBool::new(true)),
            last: Snapshot::default(),
        }
    }
}

fn connect(ctx: Context, force: Option<Channel>) -> Link {
    match Scope::open() {
        Ok(scope) => up(scope, ctx, force),
        Err(e) => Link::Down(e.to_string()),
    }
}

fn up(scope: Scope, ctx: Context, force: Option<Channel>) -> Link {
    let wake = move || ctx.request_repaint();
    Link::Up(Box::new(Poller::spawn(scope, force, wake)))
}

impl Drop for App {
    fn drop(&mut self) {
        // stop the reconnect thread looking for a scope nobody wants any more
        self.alive.store(false, Ordering::Relaxed);
    }
}

impl eframe::App for App {
    /// eframe asks now and then, and on the way out. window size and position
    /// it handles itself, this is just our own bits
    fn save(&mut self, store: &mut dyn eframe::Storage) {
        eframe::set_value(store, key::LAYOUT, &self.layout.tag());
        eframe::set_value(store, key::SIDEBAR_SCOPE, &self.show_sidebar[0]);
        eframe::set_value(store, key::SIDEBAR_COMPUTER, &self.show_sidebar[1]);
        eframe::set_value(store, key::SIDEBAR_REMOTE, &self.show_sidebar[2]);
        eframe::set_value(store, key::AUTO_GAP, &self.tuning.auto);
        eframe::set_value(store, key::GAP_MIN_MS, &(self.tuning.gap_min.as_millis() as u64));
        eframe::set_value(store, key::GAP_MAX_MS, &(self.tuning.gap_max.as_millis() as u64));
        eframe::set_value(store, key::SETTINGS_EVERY, &self.tuning.settings_every);
        eframe::set_value(store, key::PERSIST, &self.persist);
    }

    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let ctx = &ctx;
        self.catch_png(ctx);

        let mut outbox = Vec::new();
        self.read_keyboard(ctx, &mut outbox);

        // if the poller thread died theres no point pretending were connected
        if let Link::Up(p) = &self.link {
            if !p.is_running() {
                self.link = Link::Down("scope stopped answering".into());
            }
        }

        // hotplug. the scope reboots itself now and then, quite apart from
        // anyone touching the cable, and having to click a button afterwards
        // got old fast
        if matches!(self.link, Link::Down(_)) {
            match self.reconnect.as_ref().map(Receiver::try_recv) {
                None => {
                    self.reconnect = Some(reconnect_loop(ctx.clone(), self.alive.clone()));
                }
                Some(Ok(scope)) => {
                    self.reconnect = None;
                    self.link = up(scope, ctx.clone(), self.force);
                    self.tuning_sent = false;
                }
                Some(Err(_)) => {} // still looking
            }
        }

        // the poller starts on defaults, so hand it whatever we loaded. once
        // is enough, and doing it here means it happens after the link is up
        if !self.tuning_sent {
            self.tuning_sent = true;
            outbox.push(Command::Tune(self.tuning));
        }

        let offline = matches!(self.link, Link::Down(_));
        let snap = match &self.link {
            Link::Down(msg) => {
                // nothing ever arrived, so theres nothing to keep showing
                if self.last.is_cold() {
                    let msg = msg.clone();
                    self.draw_disconnected(ui, &msg);
                    return;
                }
                // otherwise carry on drawing the last thing we saw. the fps
                // counter would be a lie though
                let mut held = self.last.clone();
                held.fps = 0.0;
                held.error = Some(msg.clone());
                held
            }
            Link::Up(p) => {
                for r in p.shell_replies().collect::<Vec<_>>() {
                    self.shell.reply(&r);
                }
                let s = p.snapshot();
                if !s.is_cold() {
                    self.last = s.clone();
                }
                s
            }
        };
        self.offline = offline;

        // a new screen grab, live feed or one shot, wants uploading once
        if snap.screen_seq != self.screen_seq {
            if let Some(s) = &snap.screen {
                let image = egui::ColorImage::from_rgb([s.width, s.height], &s.rgb);
                self.screen = Some(ctx.load_texture("scope-screen", image, Default::default()));
                self.screen_seq = snap.screen_seq;
            }
        }

        // remember this frame for the fade. the poller hands out the same Arc
        // until it actually reads a new trace, so this only grows when
        // theres genuinely something new
        let now = ctx.input(|i| i.time);
        let keep = self.persist as f64;
        for ch in Channel::ALL {
            self.trails[ch.index()].push(now, snap.waves[ch.index()].as_ref(), keep);
        }
        self.trails[2].push(now, snap.math.as_ref(), keep);

        let volts = [
            channel_volts(&snap, Channel::Ch1),
            channel_volts(&snap, Channel::Ch2),
        ];
        self.draw_live(ui, &snap, &volts, &mut outbox);

        if std::mem::take(&mut self.want_csv) {
            let dt = snap.settings.sample_interval().unwrap_or(0.0);
            self.saved = Some("picking a file...".into());
            crate::csv::ask_and_save(volts.clone(), dt, self.save_tx.clone());
        }
        if let Ok(msg) = self.save_rx.try_recv() {
            self.saved = Some(msg);
        }

        if settings::window(
            ctx,
            &mut self.show_settings,
            &mut self.tuning,
            self.feed,
            snap.gap,
            &mut self.persist,
        ) {
            outbox.push(Command::Tune(self.tuning));
        }

        if let Link::Up(p) = &self.link {
            for cmd in outbox {
                p.send(cmd);
            }
        }
    }
}

impl App {
    fn draw_disconnected(&mut self, ui: &mut Ui, msg: &str) {
        egui::CentralPanel::default().show(ui, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(ui.available_height() * 0.35);
                ui.label(RichText::new("no scope").color(theme::BAD).size(22.0));
                ui.add_space(6.0);
                ui.label(RichText::new(msg).color(theme::TEXT_DIM));
                ui.add_space(14.0);
                ui.label(
                    RichText::new("trying again every couple of seconds")
                        .color(theme::TEXT_FAINT)
                        .size(11.0),
                );
                ui.add_space(10.0);
                ui.label(
                    RichText::new(
                        "if it says permission denied, the udev rule isnt installed. \
                         see the readme",
                    )
                    .color(theme::TEXT_FAINT)
                    .size(11.0),
                );
            });
        });
    }

    fn draw_live(&mut self, ui: &mut Ui, snap: &Snapshot, volts: &Volts, out: &mut Vec<Command>) {
        egui::Panel::top(egui::Id::new("status"))
            .show(ui, |ui| self.draw_status(ui, snap, out));

        if self.hacker {
            let mut leaving = false;
            egui::CentralPanel::default().show(ui, |ui| match self.shell.draw(ui) {
                shell::Action::Run(cmd) => out.push(Command::Shell(cmd)),
                shell::Action::Download { remote, local } => {
                    out.push(Command::Download { remote, local })
                }
                shell::Action::Upload { local, remote } => {
                    out.push(Command::Upload { local, remote })
                }
                shell::Action::Leave => leaving = true,
                shell::Action::None => {}
            });
            if leaving {
                self.set_hacker(false, out);
            }
            return;
        }

        // remote mode : no display at all, the panel gets the whole window.
        // the F keys normally line up against the graticule, so with no
        // graticule to line up against they get a strip of their own down the
        // left, which is the side of the panel theyre on anyway
        if self.layout == Layout::Remote {
            if self.sidebar_on() {
                egui::Panel::right(egui::Id::new("sidebar"))
                    .exact_size(230.0)
                    .show(ui, |ui| sidebar::draw(ui, snap, volts));
            }
            egui::Panel::left(egui::Id::new("fkeys")).exact_size(F_STRIP).show(ui, |ui| {
                let room = ui.available_rect_before_wrap().shrink(6.0);
                // a full height F column would be daft on a tall window, so
                // cap it and sit it in the middle where your hand expects it
                let h = room.height().min(F_COLUMN_MAX);
                let area = egui::Rect::from_center_size(room.center(), egui::vec2(room.width(), h));
                if let Some(code) = panel::f_column(ui, area) {
                    out.push(Command::Press(code));
                }
            });
            egui::CentralPanel::default().show(ui, |ui| {
                if let Some(code) = panel::draw(ui, &snap.settings) {
                    out.push(Command::Press(code));
                }
            });
            return;
        }

        if self.layout == Layout::Computer {
            egui::Panel::bottom(egui::Id::new("keypad")).show(ui, |ui| {
                ui.add_space(3.0);
                if let Some(code) = keypad::draw(ui) {
                    out.push(Command::Press(code));
                }
                ui.add_space(3.0);
            });
        }

        // the button panel goes on first so it ends up outermost, which puts
        // the measurements between it and the display where I want them
        if self.layout == Layout::Scope {
            // track the window rather than sitting at one fixed size. the
            // panel works out its own scale from whatever width it lands with
            let want = (ui.available_width() * 0.38)
                .clamp(panel::NATURAL_W * 0.78, panel::NATURAL_W * 1.7);
            egui::Panel::right(egui::Id::new("frontpanel"))
                .resizable(false)
                .exact_size(want)
                // the panel does its own scrolling, so it can measure the
                // room it has before anything else gets hold of the ui
                .show(ui, |ui| {
                    if let Some(code) = panel::draw(ui, &snap.settings) {
                        out.push(Command::Press(code));
                    }
                });
        }

        if self.sidebar_on() {
            egui::Panel::right(egui::Id::new("sidebar"))
                .exact_size(230.0)
                .show(ui, |ui| sidebar::draw(ui, snap, volts));
        }

        egui::CentralPanel::default().show(ui, |ui| {
            // the F keys have to line up with the display itself, not with the
            // panel theyre in, so they get drawn in here where we know exactly
            // where the graticule ended up
            let area = ui.available_rect_before_wrap();
            let f_w = 40.0;
            let gap = 6.0;
            let room = egui::Rect::from_min_max(
                area.min,
                egui::pos2(area.max.x - f_w - gap, area.max.y),
            );
            let shown = match self.feed {
                Feed::Screen => self.draw_screen(ui, snap, room),
                // Paused never lands in self.feed. that one only ever gets
                // sent at the poller, self.feed keeps whichever view youll be
                // coming back to
                _ => draw_waveform(
                    ui,
                    snap,
                    room,
                    &self.trails,
                    self.persist as f64,
                    ui.input(|i| i.time),
                ),
            };
            let f_area = egui::Rect::from_min_max(
                egui::pos2(shown.right() + gap, shown.top()),
                egui::pos2(shown.right() + gap + f_w, shown.bottom()),
            );
            if let Some(code) = panel::f_column(ui, f_area) {
                out.push(Command::Press(code));
            }
        });
    }

    /// the scopes own framebuffer, letterboxed into whatever room theres left.
    /// returns where it ended up so the F keys can line up with it
    fn draw_screen(&self, ui: &mut Ui, snap: &Snapshot, area: egui::Rect) -> egui::Rect {
        // 800x480, keep it that shape
        let scale = (area.width() / 800.0).min(area.height() / 480.0);
        let size = egui::Vec2::new(800.0 * scale, 480.0 * scale);
        let rect = egui::Rect::from_center_size(area.center(), size);

        let Some(tex) = &self.screen else {
            ui.painter().text(
                area.center(),
                egui::Align2::CENTER_CENTER,
                if snap.grabbing {
                    "grabbing the scopes screen..."
                } else {
                    "waiting for the first screen grab"
                },
                egui::FontId::proportional(14.0),
                theme::TEXT_DIM,
            );
            return rect;
        };
        egui::Image::new(tex).paint_at(ui, rect);
        rect
    }

    /// its 96x16 pixel art, so it gets drawn at a whole number of source
    /// pixels per point with nearest neighbour filtering. scale it by
    /// anything fractional, or let egui interpolate it, and it turns to mush.
    /// 1:1 also happens to sit about the same height as everything else on
    /// the row, which is what we wanted anyway
    fn draw_logo(&mut self, ui: &mut Ui) {
        if self.logo.is_none() {
            if let Some(img) = crate::png::logo_image() {
                self.logo = Some(ui.ctx().load_texture(
                    "xdso-logo",
                    img,
                    egui::TextureOptions::NEAREST,
                ));
            }
        }
        let Some(tex) = &self.logo else { return };
        ui.add(egui::Image::new(tex).fit_to_exact_size(egui::vec2(96.0, 16.0) * LOGO_SCALE));
        ui.separator();
    }

    fn draw_status(&mut self, ui: &mut Ui, snap: &Snapshot, out: &mut Vec<Command>) {
        // wrapped, not plain horizontal : on a narrow window, or once the
        // reconnect button turns up, a plain one lets the right hand group
        // barge over the top of the buttons instead of moving down a line
        ui.horizontal_wrapped(|ui| {
            self.draw_logo(ui);

            let h = snap.settings.horizontal();
            let rate = snap.settings.sample_interval().filter(|dt| *dt > 0.0).map(|dt| 1.0 / dt);

            ui.label(
                RichText::new(
                    h.seconds_div.map_or("--/div".into(), |v| format!("{}/div", eng(v, "s", 3))),
                )
                .color(theme::TEXT)
                .size(14.0),
            );
            if let Some(r) = rate {
                ui.label(RichText::new(eng(r, "Sa/s", 3)).color(theme::TEXT_DIM).size(12.0));
            }
            if snap.settings.display().format == Some(xdso_proto::DisplayFormat::Xy) {
                ui.label(RichText::new("XY").color(Color32::from_rgb(200, 200, 100)));
            }

            ui.separator();
            // the live screen toggle. costs you the waveform feed while its on.
            // theres nowhere to put it in remote mode, so it goes grey there
            let remote = self.layout == Layout::Remote;
            let mut screen = self.feed == Feed::Screen;
            let hint = if remote {
                "remote mode has no display. press g or pick scope or computer first"
            } else {
                "the scopes own display at 1 fps. the only way to see its menus (g)"
            };
            if ui
                .add_enabled_ui(!remote, |ui| ui.toggle_value(&mut screen, "live screen"))
                .inner
                .on_hover_text(hint)
                .changed()
            {
                self.set_feed(if screen { Feed::Screen } else { Feed::Waveform }, out);
            }

            ui.separator();
            for (l, label, hint) in [
                (Layout::Scope, "scope", "buttons laid out like the real front panel"),
                (
                    Layout::Remote,
                    "remote",
                    "the front panel and nothing else. for when youre looking at the \
                     scope itself and just want the buttons over here",
                ),
                (Layout::Computer, "computer", "buttons in a grid under the display"),
            ] {
                if ui.selectable_label(self.layout == l, label).on_hover_text(hint).clicked() {
                    self.set_layout(l, out);
                }
            }

            ui.separator();
            let mut hacker = self.hacker;
            if ui
                .toggle_value(&mut hacker, RichText::new("hacker mode").color(theme::HACK))
                .on_hover_text("a root shell on the scopes own linux. polling stops while youre in there")
                .changed()
            {
                self.set_hacker(hacker, out);
            }

            if self.offline {
                ui.label(
                    RichText::new("reconnecting...").color(theme::BAD).size(12.0),
                )
                .on_hover_text("the link dropped, this is the last frame we got. were \
                                trying to open it again every couple of seconds");
            }

            ui.separator();
            self.draw_auto_gap(ui, snap, out);

            ui.separator();
            if ui.button("settings").clicked() {
                self.show_settings = !self.show_settings;
            }
            if ui.button("save png").clicked() {
                self.request_png(ui.ctx());
            }
            if ui
                .button("save csv")
                .on_hover_text("the samples as volts, one row per sample")
                .clicked()
            {
                // the volts live a level up, so flag it and let ui() do the
                // writing where it can actually see them
                self.want_csv = true;
            }
            let mut on = self.sidebar_on();
            if ui.toggle_value(&mut on, "info").changed() {
                self.show_sidebar[self.layout.idx()] = on;
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let rate = if snap.feed == Feed::Paused {
                    RichText::new("paused").color(theme::TEXT_FAINT)
                } else {
                    RichText::new(format!("{:4.1} fps", snap.fps)).color(theme::RUN)
                };
                ui.label(rate.size(13.0));
                self.draw_hint(ui, snap, out);
            });
        });
    }

    /// the one line of "why is nothing happening" text.
    ///
    /// when its stopped it turns into a button, because theres a real command
    /// for starting acquisition and "press space" is a poor substitute for
    /// something you can click
    fn draw_hint(&mut self, ui: &mut Ui, snap: &Snapshot, out: &mut Vec<Command>) {
        // snap.feed, not self.feed : self.feed is the view youve picked, but
        // the poller might be paused for hacker mode, in which case theres no
        // waveform because we asked for none and theres nothing to report
        let polling = snap.feed == Feed::Waveform;
        // in fft the channel traces are missing because we deliberately dont
        // ask for them, not because anything is wrong
        let no_waves = polling
            && !snap.settings.math().is_fft()
            && Channel::ALL.iter().all(|&c| snap.wave(c).is_none());
        if !self.offline
            && snap.error.is_none()
            && !snap.grabbing
            && !snap.is_cold()
            && no_waves
            && !snap.settings.trigger().running
        {
            if ui
                .button(RichText::new("stopped - start").color(theme::STOP).size(11.0))
                .on_hover_text("same as pressing Run/Stop on the front panel")
                .clicked()
            {
                // the Run/Stop *key*, not the StartAcquisition command. that
                // one sets the running flag without actually starting
                // anything, leaving the scope insisting its running while
                // sending nothing. see the note on Scope::start_acquisition
                if let Some(k) = xdso_proto::keys::key_named("CT-RS-KEY") {
                    out.push(Command::Press(k.code));
                }
            }
            return;
        }

        let (text, colour) = if self.offline {
            ("link dropped - showing the last frame we got", theme::BAD)
        } else if let Some(err) = &snap.error {
            (err.as_str(), theme::BAD)
        } else if snap.grabbing {
            ("grabbing scope screen...", theme::STOP)
        } else if let Some(saved) = &self.saved {
            (saved.as_str(), theme::TEXT_DIM)
        } else if snap.is_cold() {
            ("waiting for the scope...", theme::TEXT_DIM)
        } else if snap.feed == Feed::Screen {
            ("live screen, waveforms paused", theme::STOP)
        } else if snap.feed == Feed::Paused {
            ("polling paused while youre in the shell", theme::TEXT_DIM)
        } else if snap.feed == Feed::Settings {
            // on purpose, not broken. remote mode has nowhere to draw a trace
            // so we dont spend two round trips a frame fetching one
            ("remote mode, traces off so the buttons stay quick", theme::TEXT_DIM)
        } else if no_waves && snap.settings.trigger().running {
            // says running, sends nothing. something set the running flag
            // without starting the acquisition engine
            ("says RUN but sending nothing - toggle Run/Stop twice", theme::STOP)
        } else if no_waves {
            ("both channels are off", theme::TEXT_DIM)
        } else {
            return;
        };
        // truncate rather than let a long usb error barge leftwards over the
        // buttons, which right to left layouts will happily do
        ui.add(
            egui::Label::new(RichText::new(text).color(colour).size(11.0))
                .truncate()
                .selectable(false),
        );
    }

    /// the auto poll tuning button.
    ///
    /// normally its just a toggle showing whatever gap the poller has settled
    /// on. once its been running above its own floor for a few seconds though
    /// it goes amber, because that means the scope genuinely wants more room
    /// than youve told it it can have. clicking it in that state takes the
    /// number its arrived at and makes it the new floor
    fn draw_auto_gap(&mut self, ui: &mut Ui, snap: &Snapshot, out: &mut Vec<Command>) {
        let now = ui.input(|i| i.time);
        let over = self.tuning.auto && snap.gap > self.tuning.gap_min;
        if over {
            self.slow_since.get_or_insert(now);
        } else {
            self.slow_since = None;
        }
        let nagging = self.slow_since.is_some_and(|t| now - t >= AUTO_NAG);
        if over && !nagging {
            // come back and check again, the poller might not wake us
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(500));
        }

        let gap_ms = snap.gap.as_secs_f64() * 1000.0;
        if !self.tuning.auto {
            let mut on = false;
            if ui
                .toggle_value(&mut on, format!("gap {gap_ms:.0}ms"))
                .on_hover_text("set by hand. click to let the poller tune it instead")
                .changed()
            {
                self.tuning.auto = true;
                out.push(Command::Tune(self.tuning));
            }
            return;
        }

        if nagging {
            let r = ui
                .button(RichText::new(format!("auto {gap_ms:.0}ms")).color(theme::STOP))
                .on_hover_text(format!(
                    "the scope has wanted {gap_ms:.0} ms for a while now. click to make that                      the fastest well ask for"
                ));
            if r.clicked() {
                self.tuning.gap_min = snap.gap;
                self.slow_since = None;
                out.push(Command::Tune(self.tuning));
            }
        } else {
            let mut on = true;
            if ui
                .toggle_value(&mut on, format!("auto {gap_ms:.0}ms"))
                .on_hover_text("backing off when the scope stops answering, creeping back down when it settles. click to set it by hand instead")
                .changed()
            {
                self.tuning.auto = false;
                self.tuning.cmd_gap = snap.gap;
                out.push(Command::Tune(self.tuning));
            }
        }
    }

    fn sidebar_on(&self) -> bool {
        self.show_sidebar[self.layout.idx()]
    }

    fn set_feed(&mut self, feed: Feed, out: &mut Vec<Command>) {
        self.feed = feed;
        self.clear_trails();
        out.push(Command::SetFeed(self.want_feed()));
    }

    /// what the poller should actually be doing, which isnt always the view
    /// youve picked.
    ///
    /// `self.feed` is the view youll be coming back to. hacker mode wants the
    /// usb pipe to itself, and remote mode has no display to fill, so both of
    /// them override it without forgetting it
    fn want_feed(&self) -> Feed {
        if self.hacker {
            Feed::Paused
        } else if self.layout == Layout::Remote {
            Feed::Settings
        } else {
            self.feed
        }
    }

    /// switch layouts, and tell the poller if that changed its job
    fn set_layout(&mut self, layout: Layout, out: &mut Vec<Command>) {
        if self.layout == layout {
            return;
        }
        let was = self.want_feed();
        self.layout = layout;
        if self.want_feed() != was {
            self.clear_trails();
            out.push(Command::SetFeed(self.want_feed()));
        }
    }

    /// throw the fade history away. anything that stops the traces for a
    /// while wants this, otherwise you come back to a screenful of ghosts
    /// from before you left
    fn clear_trails(&mut self) {
        for t in &mut self.trails {
            t.clear();
        }
    }

    /// in and out of hacker mode.
    ///
    /// polling stops entirely while were in there. every shell command would
    /// otherwise have to queue behind a waveform read, and nobody is looking
    /// at the graph while theyre typing at a terminal anyway. `self.feed`
    /// keeps whatever you were looking at so it comes back on the way out
    fn set_hacker(&mut self, on: bool, out: &mut Vec<Command>) {
        if self.hacker == on {
            return;
        }
        self.hacker = on;
        self.clear_trails();
        out.push(Command::SetFeed(self.want_feed()));
    }

    fn read_keyboard(&mut self, ctx: &Context, out: &mut Vec<Command>) {
        // in hacker mode every keystroke belongs to the shell. without this,
        // typing `ls` would press the CH1 menu and then Single Seq
        if self.hacker {
            if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                self.set_hacker(false, out);
            }
            return;
        }

        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }

        let typed: Vec<char> = ctx.input(|i| {
            i.events
                .iter()
                .filter_map(|e| match e {
                    egui::Event::Text(t) => Some(t.chars().collect::<Vec<_>>()),
                    // space never turns up as text, it has its own key
                    egui::Event::Key { key: egui::Key::Space, pressed: true, .. } => Some(vec![' ']),
                    _ => None,
                })
                .flatten()
                .collect()
        });

        for c in typed {
            let c = c.to_ascii_lowercase();
            if c == K_SCREEN {
                // asking for the scopes screen in remote mode means you want
                // a display, and remote hasnt got one. so take you back to
                // scope mode rather than quietly doing nothing
                if self.layout == Layout::Remote {
                    self.set_layout(Layout::Scope, out);
                    self.set_feed(Feed::Screen, out);
                } else {
                    let next =
                        if self.feed == Feed::Screen { Feed::Waveform } else { Feed::Screen };
                    self.set_feed(next, out);
                }
            } else if c == K_SAVE {
                self.request_png(ctx);
            } else if let Some(key) = xdso_proto::keys::key_for_char(c) {
                out.push(Command::Press(key.code));
            }
        }
    }

    fn request_png(&mut self, ctx: &Context) {
        self.saved = None;
        ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
    }

    /// the screenshot we asked eframe for comes back as an input event a frame
    /// or two later
    fn catch_png(&mut self, ctx: &Context) {
        let shot = ctx.input(|i| {
            i.events.iter().find_map(|e| match e {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        let Some(image) = shot else { return };
        self.saved = Some(match crate::png::save(&image) {
            Ok(path) => format!("saved {path}"),
            Err(e) => format!("save failed: {e}"),
        });
    }
}

/// returns the graticule rect, so the F keys can be lined up against it
fn draw_waveform(
    ui: &mut Ui,
    snap: &Snapshot,
    area: egui::Rect,
    trails: &[crate::trail::Trail; 3],
    persist: f64,
    now: f64,
) -> egui::Rect {
    let g = Graticule::fit(area.shrink(4.0));
    let p = ui.painter_at(area);
    let d = snap.settings.display();

    g.draw_grid(&p, d.grid_kind);

    let dots = d.mode == Some(xdso_proto::DisplayMode::Dots);

    // we dont draw an fft. the spectrum is on the math channel and the
    // frequency axis depends on how much of the buffer is really spectrum,
    // which i guessed at three different numbers and got wrong every time.
    // saying so beats drawing something that looks plausible and isnt
    if snap.settings.math().is_fft() {
        p.text(
            g.rect.center(),
            egui::Align2::CENTER_CENTER,
            "fft isnt supported yet, press g for the scopes own screen",
            egui::FontId::proportional(13.0),
            theme::STOP,
        );
        return g.rect;
    }

    let xy = d.format == Some(xdso_proto::DisplayFormat::Xy);
    match (xy, snap.wave(Channel::Ch1), snap.wave(Channel::Ch2)) {
        (true, Some(a), Some(b)) => g.draw_xy(&p, a, b, dots),
        _ => {
            for ch in Channel::ALL {
                for (frame, alpha) in trails[ch.index()].iter(now, persist) {
                    g.draw_wave(&p, frame, fade(theme::CH[ch.index()], alpha), dots);
                }
            }
            // math goes on top, its usually the one you turned on to look at
            for (frame, alpha) in trails[2].iter(now, persist) {
                g.draw_wave(&p, frame, fade(theme::MATH, alpha), dots);
            }
            // only worth drawing when the trigger is watching a channel we can
            // see. on EXT or AC line the level means nothing on this screen
            let t = snap.settings.trigger();
            if let Some(src) = t.source.and_then(trigger_channel) {
                g.draw_trigger(&p, t.level, theme::CH[src.index()]);
            }
        }
    }
    g.rect
}

/// an older frame, dimmed. straight alpha rather than darkening, so faded
/// traces still read as the same colour and dont muddy where they overlap
fn fade(colour: Color32, alpha: f32) -> Color32 {
    if alpha >= 1.0 {
        colour
    } else {
        colour.gamma_multiply(alpha)
    }
}

/// which channel the trigger is looking at, if its looking at one at all
fn trigger_channel(src: xdso_proto::TrigSource) -> Option<Channel> {
    match src {
        xdso_proto::TrigSource::Ch1 => Some(Channel::Ch1),
        xdso_proto::TrigSource::Ch2 => Some(Channel::Ch2),
        _ => None,
    }
}

/// sample counts -> volts, using the channels own scale and position.
///
/// the samples come back with the vertical position already applied (thats
/// what the scope draws) so we take it back off to get the actual voltage at
/// the probe tip
fn channel_volts(snap: &Snapshot, ch: Channel) -> Option<Vec<f32>> {
    let counts = snap.wave(ch)?;
    let c = snap.settings.channel(ch);
    let vdiv = c.volts_div?;
    if vdiv <= 0.0 {
        return None;
    }
    let pos = c.position as f32;
    Some(
        counts
            .iter()
            .map(|&b| (b as f32 - CENTRE_COUNT - pos) / COUNTS_PER_DIV * vdiv as f32)
            .collect(),
    )
}
