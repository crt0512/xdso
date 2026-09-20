//! scope mode : a pretend front panel, laid out like the real thing.
//!
//! the arrangement comes from `doc/ASCII.md`, which is my drawing of my
//! actual DSO5072P. the knobs, the grouping, the little LEDs, all of it is
//! where it is on the real box, so muscle memory transfers.
//!
//! everything here is a KeyTrigger underneath, same as the plain button grid
//! in [`crate::keypad`]. the difference is entirely in where things sit.
//!
//! ## knobs
//!
//! the scope has no "set the timebase to X" command, only "step it up" and
//! "step it down", so a knob is really two keys and sometimes a third for
//! pushing it in. the legend in ASCII.md spells this out :
//!
//! | drawing | means |
//! |---|---|
//! | `{X}` | knob you can also push |
//! | `(X)` | knob you cant push |
//! | `[X]` | button |
//! | `·` | LED |
//!
//! ## the F keys are not fixed
//!
//! worth saying because i assumed otherwise and it cost me. the side menu
//! keys do **not** map to fixed menu sections. they point at whatever the
//! scope is currently drawing next to them, which changes per menu and per
//! page. the only bits i reckons are consistent : F0 usually closes the
//! menu, F6 changes page in the menus that have pages, and F7 switches to
//! dual window mode. so dont go pressing them blind from a script unless you
//! are looking at the screen, which is how i managed to switch on the
//! bandwidth limit on both channels without noticing.
//!
//! you can turn them by scrolling over them or dragging them, as well as the
//! little step buttons either side, which autorepeat if you hold them down.
//! every step is a separate usb command though, so theres a cooldown, see
//! [`STEP_COOLDOWN`]. spinning a knob as fast as the mouse will go would
//! otherwise queue up hundreds of commands and the scope would still be
//! chewing through them a minute later.
//!
//! the one thing that doesnt map is pushing the Sec/Div knob. its drawn as
//! `{HS}` so it pushes on the real scope, but keyprotocol.inf has no key for
//! it, so ours doesnt.

use egui::{Align2, Color32, FontId, Rect, Sense, Stroke, Ui, Vec2, pos2, vec2};

use xdso_proto::{Settings, keys};

use crate::{hold, theme};

// everything here is at scale 1.0 and gets multiplied on the way out
const BTN_W: f32 = 66.0;
const BTN_H: f32 = 24.0;
const STEP: f32 = 20.0;
const DIAL: f32 = 34.0;
const LED_D: f32 = 8.0;
const PAD: f32 = 4.0;
const FONT: f32 = 10.5;

/// a button plus the indicator slot that always sits to its left
const BTN_SLOT: f32 = LED_D + PAD * 0.6 + BTN_W;
/// a dial and its two step squares
const KNOB_W: f32 = STEP * 2.0 + DIAL + PAD * 2.0;
/// what an egui group costs in border and margin, both sides
const GROUP_PAD: f32 = 20.0;

/// unscaled width the whole panel wants.
///
/// worked out rather than guessed, because guessing it low means the scale
/// comes out too big, the content overflows the panel and the left hand edge
/// of the vertical section disappears behind the measurements. the widest row
/// is the menu block with the Autoset and Run/Stop columns beside it
pub const NATURAL_W: f32 = (KNOB_W + PAD + BTN_SLOT * 3.0 + PAD * 2.0 + GROUP_PAD)
    + PAD
    + BTN_SLOT
    + PAD
    + BTN_SLOT
    + 16.0; // panel margin and a scrollbar

/// how far you have to scroll for one step of a knob.
///
/// tuned so one notch of a mouse wheel is one step. at 22 it was two, which
/// made the timebase leap about when you nudged it. a notch is the natural
/// quantum here so this one stays where it is
const STEP_PIXELS: f32 = 48.0;

/// drag pixels are worth more than scroll pixels.
///
/// a wheel notch is a discrete thing, but a drag is you grabbing the knob and
/// turning it, and 48 px of mouse travel per step feels like wading through
/// treacle. this puts a step at about 14 px of travel instead, which is close
/// enough to how far a real knob moves under your fingers
const DRAG_GAIN: f32 = 3.5;

/// the least time between two steps. each one is a usb command and the scope
/// wants ~20 ms of quiet between commands, so 50 ms leaves it room to
/// actually do the waveform read in between rather than falling behind
const STEP_COOLDOWN: f64 = 0.05;

/// how much turn were willing to keep hold of while the cooldown runs.
///
/// without a cap a fast spin queues up dozens of steps and the thing carries
/// on turning for a second after youve stopped, which feels haunted. two
/// steps of slack is enough to feel continuous and short enough to not
/// notice
const STEP_BACKLOG: f32 = 2.0;

/// how long the Autoset lamp stays lit after you press it.
///
/// the scope lights it while its working out what to do with your signal and
/// never tells us when its finished, so we just guess at five seconds. its
/// cosmetic, it only has to look about right
const AUTOSET_GLOW: f64 = 5.0;

fn autoset_id() -> egui::Id {
    egui::Id::new("xdso-autoset-pressed")
}

/// what sort of indicator a key has, if any
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Led {
    /// no indicator on the real scope either. Math is just a menu
    None,
    /// theres an LED there but we have no way to know its state
    Unknown,
    /// lit amber, the "im working on it" colour Autoset uses
    Busy,
    On,
    Off,
}

/// what colour each lamp glows. the channel buttons get their own colours
/// rather than their trace colours, same as the real panel
fn led_colour(inf_name: &str) -> Color32 {
    match inf_name {
        "VT-CH1-MENU-KEY" => theme::LED_CH1,
        "VT-CH2-MENU-KEY" => theme::LED_CH2,
        "VT-MATH-MENU-KEY" => theme::LED_MATH,
        _ => theme::RUN,
    }
}

impl Led {
    fn of(inf_name: &str, s: &Settings) -> Led {
        let on = |b: bool| if b { Led::On } else { Led::Off };
        match inf_name {
            "CT-RS-KEY" => on(s.trigger().running),
            "CT-SINGLESEQ-KEY" => on(s.trigger().mode == Some(xdso_proto::TrigMode::Single)),
            "VT-CH1-MENU-KEY" => on(s.channel(xdso_proto::Channel::Ch1).enabled),
            "VT-CH2-MENU-KEY" => on(s.channel(xdso_proto::Channel::Ch2).enabled),
            "VT-MATH-MENU-KEY" => on(s.math().enabled),
            "CT-AUTOSET-KEY" => Led::Unknown,
            _ => Led::None,
        }
    }
}

/// look a key up by its inf name. None if this firmware doesnt have it, and
/// the button draws disabled rather than vanishing
fn code(inf_name: &str) -> Option<u8> {
    keys::key_named(inf_name).map(|k| k.code)
}

/// sizes for one draw, all already multiplied by the scale factor
#[derive(Clone, Copy)]
struct S {
    k: f32,
}

impl S {
    fn btn(self) -> Vec2 {
        vec2(BTN_W * self.k, BTN_H * self.k)
    }

    /// what a button actually occupies now the lamp sits under it. every
    /// "line this up with that" sum wants this rather than the button height
    fn slot_h(self) -> f32 {
        (BTN_H + LED_D * 0.9 + PAD * 0.4) * self.k
    }
    fn font(self) -> f32 {
        FONT * self.k
    }
    fn pad(self) -> f32 {
        PAD * self.k
    }
    /// a knobs full footprint, dial row plus the label underneath
    fn knob(self) -> Vec2 {
        vec2(KNOB_W * self.k, (DIAL + 2.0 + FONT + 3.0) * self.k)
    }
}

/// draw the whole panel, return whichever key got pressed.
///
/// the scale comes from how much width we were actually given, so the panel
/// grows and shrinks with the window instead of sitting there at one size
pub fn draw(ui: &mut Ui, s: &Settings) -> Option<u8> {
    // measure the room before the scroll area gets hold of the ui, so both
    // the scale and the centring are against the panel itself
    let room_w = ui.available_width();
    let room_h = ui.available_height();
    let k = (room_w / NATURAL_W).clamp(0.7, 2.2);
    let z = S { k };
    let mut hit = None;
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        crate::layout::center_v(ui, "frontpanel", room_h, |ui| {
                ui.spacing_mut().item_spacing = Vec2::splat(z.pad());


                // centre the whole block in the panel.
                //
                // NATURAL_W is deliberately a slight over estimate (it has to be, or the
                // scale comes out too big and the content overflows) so centring on it
                // would leave the block off to one side. measure what we actually drew
                // last frame and centre on that instead. one frame of lag while you drag
                // the window, which nobody will ever notice
                let width_id = ui.id().with("drawn-width");
                let drawn: f32 = ui.data(|d| d.get_temp(width_id)).unwrap_or(NATURAL_W * k);
                let indent = ((room_w - drawn) * 0.5).max(0.0);
                let block = ui.horizontal(|ui| {
                    ui.add_space(indent);
                    ui.vertical(|ui| {
                        // menu block, then Autoset/Single, then Run/Stop off on its own to the
                        // right, exactly like the drawing
                        ui.horizontal_top(|ui| {
                            ui.group(|ui| {
                                ui.vertical(|ui| {
                                    head(ui, "Menu", z);
                                    ui.horizontal_top(|ui| {
                                        // the dial sits to the left, level with the middle of the
                                        // two button rows
                                        let rows = z.slot_h() * 2.0 + z.pad();
                                        ui.vertical(|ui| {
                                            ui.add_space(((rows - z.knob().y) * 0.5).max(0.0));
                                            hit = hit.or(knob(ui, z, "V0", "FN-MLEFT-KEY", "FN-MRIGHT-KEY", Some("FN-MZERO-KEY")));
                                        });
                                        ui.vertical(|ui| {
                                            ui.horizontal(|ui| {
                                                hit = hit.or(button(ui, z, "Save/Rec", "MENU-SR-KEY", s));
                                                hit = hit.or(button(ui, z, "Measure", "MENU-MEASURE-KEY", s));
                                                hit = hit.or(button(ui, z, "Acquire", "MENU-ACQUIRE-KEY", s));
                                            });
                                            ui.horizontal(|ui| {
                                                hit = hit.or(button(ui, z, "Utility", "MENU-UTILITY-KEY", s));
                                                hit = hit.or(button(ui, z, "Cursor", "MENU-CURSOR-KEY", s));
                                                hit = hit.or(button(ui, z, "Display", "MENU-DISPLAY-KEY", s));
                                            });
                                        });
                                    });
                                });
                            });

                            ui.vertical(|ui| {
                                ui.add_space(z.font() + z.pad());
                                hit = hit.or(button(ui, z, "Autoset", "CT-AUTOSET-KEY", s));
                                hit = hit.or(button(ui, z, "Single", "CT-SINGLESEQ-KEY", s));
                            });
                            ui.vertical(|ui| {
                                // sits between the two of them, one column further right
                                ui.add_space(z.font() + z.pad() + (z.slot_h() + z.pad()) * 0.5);
                                hit = hit.or(button(ui, z, "Run/Stop", "CT-RS-KEY", s));
                            });
                        });

                        ui.horizontal(|ui| {
                            hit = hit.or(button(ui, z, "F7", "FN-7-KEY", s));
                            hit = hit.or(button(ui, z, "Help", "CT-HELP-KEY", s));
                            hit = hit.or(button(ui, z, "Default", "CT-DS-KEY", s));
                            hit = hit.or(button(ui, z, "Save USB", "CT-STU-KEY", s));
                        });

                        // vertical, then horizontal, then trigger, left to right
                        ui.horizontal_top(|ui| {
                            ui.group(|ui| {
                                ui.vertical(|ui| {
                                    head(ui, "Vertical", z);
                                    ui.horizontal_top(|ui| {
                                        ui.vertical(|ui| {
                                            chan_head(ui, "CH 1", theme::CH[0], z);
                                            hit = hit.or(knob(ui, z, "Pos", "VT-CH1-PSUB-KEY", "VT-CH1-PADD-KEY", Some("VT-CH1-PZERO-KEY")));
                                            hit = hit.or(button_centred(ui, z, "CH1", "VT-CH1-MENU-KEY", s));
                                            hit = hit.or(knob(ui, z, "V/div", "VT-CH1-VBSUB-KEY", "VT-CH1-VBADD-KEY", None));
                                        });
                                        ui.vertical(|ui| {
                                            // math lines up with the CH1 and CH2 buttons, not with
                                            // the position knobs above them
                                            ui.add_space(z.font() + z.pad() + z.knob().y + z.pad());
                                            hit = hit.or(button_centred(ui, z, "Math", "VT-MATH-MENU-KEY", s));
                                        });
                                        ui.vertical(|ui| {
                                            chan_head(ui, "CH 2", theme::CH[1], z);
                                            hit = hit.or(knob(ui, z, "Pos", "VT-CH2-PSUB-KEY", "VT-CH2-PADD-KEY", Some("VT-CH2-PZERO-KEY")));
                                            hit = hit.or(button_centred(ui, z, "CH2", "VT-CH2-MENU-KEY", s));
                                            hit = hit.or(knob(ui, z, "V/div", "VT-CH2-VBSUB-KEY", "VT-CH2-VBADD-KEY", None));
                                        });
                                    });
                                });
                            });

                            ui.group(|ui| {
                                ui.vertical(|ui| {
                                    head(ui, "Horizontal", z);
                                    // the vertical box has a channel name
                                    // above its first knob and this one
                                    // doesnt, so leave the gap. on the real
                                    // scope the two boxes are the same height
                                    ui.add_space(z.font() + z.pad());
                                    hit = hit.or(knob(ui, z, "Pos", "HZ-PSUB-KEY", "HZ-PADD-KEY", Some("HZ-PZERO-KEY")));
                                    hit = hit.or(button_centred(ui, z, "Horiz", "HZ-MENU-KEY", s));
                                    // the real one pushes too, theres just no key for it
                                    hit = hit.or(knob(ui, z, "s/div", "HZ-TBSUB-KEY", "HZ-TBADD-KEY", None));
                                });
                            });

                            ui.group(|ui| {
                                ui.vertical(|ui| {
                                    head(ui, "Trigger", z);
                                    hit = hit.or(knob(ui, z, "Level", "TG-PSUB-KEY", "TG-PADD-KEY", Some("TG-PZERO-KEY")));
                                    hit = hit.or(button_centred(ui, z, "Trigger", "TG-MENU-KEY", s));
                                    hit = hit.or(button_centred(ui, z, "Set 50%", "TG-PHALF-KEY", s));
                                    hit = hit.or(button_centred(ui, z, "Force", "TG-FORCE-KEY", s));
                                    hit = hit.or(button_centred(ui, z, "Probe Chk", "TG-PROBECHECK-KEY", s));
                                });
                            });
                        });


                    })
                    .response
                    .rect
                    .width()
                });
                ui.data_mut(|d| d.insert_temp(width_id, block.inner));
        });
    });

    hit
}

fn head(ui: &mut Ui, text: &str, z: S) {
    ui.label(egui::RichText::new(text).color(theme::HEADING).size(z.font()));
}

fn chan_head(ui: &mut Ui, text: &str, colour: Color32, z: S) {
    ui.label(egui::RichText::new(text).color(colour).size(z.font()));
}

/// same, but nudged so it sits in the middle of a column rather than hard
/// against its left edge. the knobs are wider than the buttons, so without
/// this every button in the vertical, horizontal and trigger boxes looks
/// shoved over to one side
fn button_centred(ui: &mut Ui, z: S, label: &str, inf_name: &str, s: &Settings) -> Option<u8> {
    let mut hit = None;
    ui.horizontal(|ui| {
        ui.add_space(((z.knob().x - z.btn().x) * 0.5).max(0.0));
        hit = button(ui, z, label, inf_name, s);
    });
    hit
}

/// a front panel button with its indicator to the left of it.
///
/// the LED slot is always allocated, even for keys that dont have one, so
/// every button in a column starts at the same x
fn button(ui: &mut Ui, z: S, label: &str, inf_name: &str, s: &Settings) -> Option<u8> {
    let code = code(inf_name);
    let mut led = Led::of(inf_name, s);

    // autoset lights up while its thinking. we never find out when it stops,
    // so we light it for a few seconds and hope
    if inf_name == "CT-AUTOSET-KEY" {
        let pressed: f64 = ui.data(|d| d.get_temp(autoset_id())).unwrap_or(f64::NEG_INFINITY);
        let left = AUTOSET_GLOW - (ui.input(|i| i.time) - pressed);
        if left > 0.0 {
            led = Led::Busy;
            // make sure we come back and turn it off again
            ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(left.min(0.25)));
        }
    }

    let mut hit = None;

    // the lamp goes underneath the button, where it is on the real panel. the
    // slot is always allocated, even for keys that have no lamp, so buttons in
    // a column all line up with each other
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = z.pad() * 0.4;

        let b = egui::Button::new(egui::RichText::new(label).size(z.font()));
        // add_sized rather than min_size : inside a vertical layout a button
        // with only a minimum stretches to the full column width and drags the
        // whole panel out with it
        let r = ui.add_enabled_ui(code.is_some(), |ui| ui.add_sized(z.btn(), b)).inner;
        // a step key held down keeps stepping. most of the panels step keys
        // are knob squares rather than buttons, but the rule lives in one
        // place so it stays true wherever the key turns up
        let go = if hold::repeats(inf_name) { hold::fired(ui, &r) } else { r.clicked() };
        if go {
            hit = code;
            if inf_name == "CT-AUTOSET-KEY" {
                let now = ui.input(|i| i.time);
                ui.data_mut(|d| d.insert_temp(autoset_id(), now));
            }
            // otherwise the button keeps keyboard focus and the next space bar
            // press re-clicks it as well as firing Run/Stop
            r.surrender_focus();
        }

        let (slot, _) =
            ui.allocate_exact_size(Vec2::new(z.btn().x, LED_D * z.k * 0.9), Sense::hover());
        let dot = slot.center();
        let dim = Color32::from_rgb(40, 40, 40);
        match led {
            Led::None => {}
            Led::Unknown => {
                // theres a lamp there, we just cant read it
                ui.painter().circle_filled(dot, LED_D * z.k * 0.34, dim);
                ui.painter().circle_stroke(
                    dot,
                    LED_D * z.k * 0.34,
                    Stroke::new(1.0, Color32::from_rgb(70, 70, 70)),
                );
            }
            Led::Off => {
                ui.painter().circle_filled(dot, LED_D * z.k * 0.34, dim);
            }
            Led::On | Led::Busy => {
                let c = if led == Led::Busy { theme::STOP } else { led_colour(inf_name) };
                ui.painter().circle_filled(dot, LED_D * z.k * 0.62, c.gamma_multiply(0.3));
                ui.painter().circle_filled(dot, LED_D * z.k * 0.34, c);
            }
        }
    });
    hit
}

/// per knob scratch state, kept in egui memory so knobs stay stateless here
#[derive(Clone, Default)]
struct KnobState {
    /// scroll and drag pixels not yet turned into a step
    accum: f32,
    /// when we last emitted, so we can pace the usb commands
    last_step: f64,
}

/// a knob : step down, step up, and sometimes push it in.
///
/// turn it by scrolling over it or dragging it, or nudge it one step with the
/// little buttons either side. the label goes underneath, where a label on a
/// knob goes
fn knob(ui: &mut Ui, z: S, label: &str, minus: &str, plus: &str, push: Option<&str>) -> Option<u8> {
    let size = z.knob();
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click_and_drag());
    let id = resp.id;

    let dial_h = DIAL * z.k;
    let row = Rect::from_min_size(rect.min, vec2(rect.width(), dial_h));
    let minus_r = Rect::from_min_size(
        pos2(row.left(), row.center().y - STEP * z.k * 0.5),
        Vec2::splat(STEP * z.k),
    );
    let plus_r = Rect::from_min_size(
        pos2(row.right() - STEP * z.k, row.center().y - STEP * z.k * 0.5),
        Vec2::splat(STEP * z.k),
    );
    let dial_c = pos2(row.center().x, row.center().y);
    let dial_r = dial_h * 0.5;

    let mut hit = None;
    let pointer = ui.ctx().pointer_interact_pos();
    let over = |r: Rect| pointer.is_some_and(|p| r.contains(p));

    // the two step squares hold to repeat, so they fire on the press rather
    // than on the release. they arent widgets of their own, theyre hit tested
    // inside the knobs one big response, so they borrow a child id to hang
    // the timer off. see [`crate::hold`]
    let down = resp.is_pointer_button_down_on();
    let on_minus = down && over(minus_r);
    let on_plus = down && over(plus_r);
    if hold::fired_at(ui, id.with("minus"), on_minus) {
        hit = code(minus);
    } else if hold::fired_at(ui, id.with("plus"), on_plus) {
        hit = code(plus);
    } else if resp.clicked() && !over(minus_r) && !over(plus_r) {
        // pushing the dial in. thats a real click, you dont hold it
        if let Some(push) = push {
            if pointer.is_some_and(|p| p.distance(dial_c) <= dial_r) {
                hit = code(push);
            }
        }
    }

    // turning it : scroll wheel anywhere over the knob, or a drag on the dial.
    // both feed the same accumulator, with the drag scaled up by [`DRAG_GAIN`]
    // so a notch and a nudge of the hand both land somewhere sensible
    if hit.is_none() && !on_minus && !on_plus {
        let mut state: KnobState = ui.data_mut(|d| d.get_temp(id).unwrap_or_default());
        if resp.hovered() {
            let scroll = ui.input(|i| i.smooth_scroll_delta);
            state.accum += scroll.y + scroll.x;
        }
        if resp.dragged() {
            // right and up both mean "turn it up"
            let d = resp.drag_delta();
            state.accum += (d.x - d.y) * DRAG_GAIN;
        }

        // someone is spinning it faster than we can send. keep a couple of
        // steps of slack so it still feels continuous, bin the rest
        let cap = STEP_PIXELS * STEP_BACKLOG;
        state.accum = state.accum.clamp(-cap, cap);

        let now = ui.input(|i| i.time);
        if state.accum.abs() >= STEP_PIXELS {
            if now - state.last_step >= STEP_COOLDOWN {
                hit = code(if state.accum > 0.0 { plus } else { minus });
                // carry the remainder rather than zeroing it, otherwise a
                // slow steady drag throws away up to a whole step every time
                // and the knob feels notchy
                state.accum -= STEP_PIXELS * state.accum.signum();
                state.last_step = now;
            } else {
                // theres a step waiting on the cooldown. nothing else is going
                // to repaint us, so ask
                ui.ctx()
                    .request_repaint_after(std::time::Duration::from_secs_f64(STEP_COOLDOWN));
            }
        }
        ui.data_mut(|d| d.insert_temp(id, state));
    }

    // now paint it
    let p = ui.painter();
    let hovering_dial = pointer.is_some_and(|q| q.distance(dial_c) <= dial_r);
    let face = if hovering_dial && push.is_some() {
        Color32::from_rgb(82, 82, 88)
    } else if resp.dragged() {
        Color32::from_rgb(90, 90, 96)
    } else {
        Color32::from_rgb(58, 58, 62)
    };
    p.circle_filled(dial_c, dial_r, face);
    p.circle_stroke(dial_c, dial_r, Stroke::new(1.0 * z.k, theme::GRID_AXIS));
    // the notch, so it looks like something you could grip
    p.line_segment(
        [dial_c, pos2(dial_c.x, dial_c.y - dial_r * 0.78)],
        Stroke::new(2.0 * z.k, theme::TEXT),
    );

    step_button(ui, minus_r, "-", z, over(minus_r));
    step_button(ui, plus_r, "+", z, over(plus_r));

    ui.painter().text(
        pos2(rect.center().x, rect.bottom() - 1.0),
        Align2::CENTER_BOTTOM,
        label,
        FontId::proportional(z.font()),
        theme::TEXT_DIM,
    );

    if push.is_some() {
        resp.on_hover_text("scroll or drag to turn, click to push");
    } else {
        resp.on_hover_text("scroll or drag to turn");
    }
    hit
}

/// the little step squares either side of a dial. painted by hand so they line
/// up with each other and with the dial, which they did not when they were two
/// ordinary buttons in a horizontal layout
fn step_button(ui: &Ui, rect: Rect, glyph: &str, z: S, hovered: bool) {
    let p = ui.painter();
    let bg = if hovered { Color32::from_rgb(78, 78, 82) } else { Color32::from_rgb(54, 54, 58) };
    p.rect_filled(rect, 2.0 * z.k, bg);
    p.rect_stroke(
        rect,
        2.0 * z.k,
        Stroke::new(1.0, Color32::from_rgb(88, 88, 92)),
        egui::StrokeKind::Inside,
    );
    p.text(
        rect.center(),
        Align2::CENTER_CENTER,
        glyph,
        FontId::proportional(z.font() * 1.15),
        theme::TEXT,
    );
}

/// the F keys that live down the side of the display.
///
/// F0 at the top, F6 at the bottom, spread over exactly the height of the
/// graticule so they line up with whatever menu the scope is showing. theres
/// no F8 : the scope numbers these from zero and F7 is elsewhere, in the row
/// with Help and Default
pub fn f_column(ui: &mut Ui, area: Rect) -> Option<u8> {
    const N: usize = 7;
    let gap = 4.0;
    // F0 and F6 are half height on the real panel, the five in the middle are
    // full height. so thats six units of height to share out, not seven
    let unit = ((area.height() - gap * (N as f32 - 1.0)) / 6.0).max(14.0);
    let height_of = |i: usize| if i == 0 || i == N - 1 { unit * 0.5 } else { unit };
    let mut hit = None;

    let mut top = area.top();
    for i in 0..N {
        let h = height_of(i);
        if let Some(c) = code(&format!("FN-{i}-KEY")) {
            let font = (h * 0.34).clamp(8.0, 26.0);
            let rect = Rect::from_min_size(pos2(area.left(), top), vec2(area.width(), h));
            let b = egui::Button::new(egui::RichText::new(format!("F{i}")).size(font));
            let r = ui.put(rect, b);
            if r.clicked() {
                hit = Some(c);
                r.surrender_focus();
            }
        }
        top += h + gap;
    }
    hit
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_natural_width_covers_the_widest_row() {
        // row 3 is vertical + horizontal + trigger, and it must not be wider
        // than the row we sized the panel from
        let vertical = KNOB_W + BTN_SLOT + KNOB_W + PAD * 2.0 + GROUP_PAD;
        let horizontal = KNOB_W + GROUP_PAD;
        let trigger = KNOB_W + GROUP_PAD;
        let row3 = vertical + PAD + horizontal + PAD + trigger;
        assert!(row3 <= NATURAL_W, "row 3 is {row3}, panel only plans for {NATURAL_W}");
    }

    #[test]
    fn every_key_the_panel_names_actually_exists() {
        // a typo in an inf name would silently draw a dead button
        for name in [
            "MENU-SR-KEY", "MENU-MEASURE-KEY", "MENU-ACQUIRE-KEY", "MENU-UTILITY-KEY",
            "MENU-CURSOR-KEY", "MENU-DISPLAY-KEY", "FN-MLEFT-KEY", "FN-MRIGHT-KEY",
            "FN-MZERO-KEY", "CT-AUTOSET-KEY", "CT-SINGLESEQ-KEY", "CT-RS-KEY", "FN-7-KEY",
            "CT-HELP-KEY", "CT-DS-KEY", "CT-STU-KEY", "VT-CH1-PSUB-KEY", "VT-CH1-PADD-KEY",
            "VT-CH1-PZERO-KEY", "VT-CH1-MENU-KEY", "VT-CH1-VBSUB-KEY", "VT-CH1-VBADD-KEY",
            "VT-MATH-MENU-KEY", "VT-CH2-PSUB-KEY", "VT-CH2-PADD-KEY", "VT-CH2-PZERO-KEY",
            "VT-CH2-MENU-KEY", "VT-CH2-VBSUB-KEY", "VT-CH2-VBADD-KEY", "HZ-PSUB-KEY",
            "HZ-PADD-KEY", "HZ-PZERO-KEY", "HZ-MENU-KEY", "HZ-TBSUB-KEY", "HZ-TBADD-KEY",
            "TG-PSUB-KEY", "TG-PADD-KEY", "TG-PZERO-KEY", "TG-MENU-KEY", "TG-PHALF-KEY",
            "TG-FORCE-KEY", "TG-PROBECHECK-KEY",
        ] {
            assert!(code(name).is_some(), "{name} is not in keyprotocol.inf");
        }
        for i in 0..7 {
            assert!(code(&format!("FN-{i}-KEY")).is_some(), "side menu key {i} missing");
        }
    }

    #[test]
    fn the_lamps_we_can_read_and_the_one_we_cant() {
        let s = Settings::default();
        // autoset has a lamp but never tells us what its doing
        assert_eq!(Led::of("CT-AUTOSET-KEY", &s), Led::Unknown);
        // these we can read straight out of the settings
        for key in ["CT-RS-KEY", "VT-CH1-MENU-KEY", "VT-CH2-MENU-KEY", "VT-MATH-MENU-KEY"] {
            assert!(matches!(Led::of(key, &s), Led::On | Led::Off), "{key}");
        }
        // and plenty of keys have no lamp at all
        assert_eq!(Led::of("MENU-MEASURE-KEY", &s), Led::None);
    }

    #[test]
    fn the_channel_lamps_get_their_own_colours() {
        assert_eq!(led_colour("VT-CH1-MENU-KEY"), theme::LED_CH1);
        assert_eq!(led_colour("VT-CH2-MENU-KEY"), theme::LED_CH2);
        assert_eq!(led_colour("VT-MATH-MENU-KEY"), theme::LED_MATH);
        assert_eq!(led_colour("CT-RS-KEY"), theme::RUN);
    }
}
