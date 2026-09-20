//! the settings window. mostly a front end for [`Tuning`].
//!
//! theres not much in here but what is in here matters : the command gap is
//! the single biggest lever on frame rate, and its also the thing that makes
//! everything go flaky if you get greedy with it.

use std::time::Duration;

use egui::{Context, RichText, Slider};

use xdso_usb::{CMD_GAP, Feed, GAP_MAX, GAP_RELIABLE_MIN, Tuning};

use crate::theme;

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

/// draw the window, return true if anything changed and the poller needs
/// telling about it
pub fn window(
    ctx: &Context,
    open: &mut bool,
    tuning: &mut Tuning,
    feed: Feed,
    current_gap: Duration,
    persist: &mut f32,
) -> bool {
    let mut changed = false;
    egui::Window::new("settings")
        .open(open)
        .resizable(false)
        .default_width(360.0)
        .show(ctx, |ui| {
            ui.label(RichText::new("timing").color(theme::HEADING).size(13.0));

            changed |= ui
                .checkbox(&mut tuning.auto, "find the command gap automatically")
                .changed();
            ui.label(
                RichText::new(
                    "backs off a millisecond whenever the scope stops answering, then creeps \
                     back down once its been quiet for a bit. how fast is too fast depends on \
                     what the scope is busy doing, so theres no one right number",
                )
                .color(theme::TEXT_FAINT)
                .size(11.0),
            );

            ui.add_space(6.0);
            if tuning.auto {
                changed |= range(ui, tuning);
                ui.add_space(4.0);
                ui.label(
                    RichText::new(format!("using {:.0} ms right now", ms(current_gap)))
                        .color(if current_gap > tuning.gap_min {
                            theme::STOP
                        } else {
                            theme::RUN
                        })
                        .size(12.0)
                        .monospace(),
                );
            } else {
                changed |= manual(ui, tuning);
            }

            ui.add_space(6.0);
            let mut every = tuning.settings_every;
            if ui.add(Slider::new(&mut every, 1..=20).text("settings every n passes")).changed() {
                tuning.settings_every = every;
                changed = true;
            }
            ui.label(
                RichText::new(
                    "how often to re-read the front panel state. it barely ever changes, so \
                     asking every frame costs a whole round trip for nothing",
                )
                .color(theme::TEXT_FAINT)
                .size(11.0),
            );

            ui.add_space(8.0);
            ui.separator();
            ui.label(RichText::new("display").color(theme::HEADING).size(13.0));
            ui.add(
                Slider::new(persist, 0.0..=4.0)
                    .suffix(" s")
                    .text("persistence"),
            );
            ui.label(
                RichText::new(
                    "how long an old frame hangs about, fading. the scope only manages a \
                     couple of frames a second when its busy, and an fft is about as busy as \
                     it gets, so without this the trace jumps around too much to read. 0 \
                     turns it off",
                )
                .color(theme::TEXT_FAINT)
                .size(11.0),
            );

            ui.add_space(8.0);
            ui.separator();
            ui.label(RichText::new("roughly what that buys you").color(theme::HEADING).size(13.0));
            ui.label(
                RichText::new(estimate(*tuning, feed, current_gap))
                    .color(theme::TEXT_DIM)
                    .size(11.0)
                    .monospace(),
            );

            ui.add_space(8.0);
            if ui.button("back to defaults").clicked() {
                *tuning = Tuning::default();
                changed = true;
            }
            ui.label(
                RichText::new(format!("default gap is {} ms", CMD_GAP.as_millis()))
                    .color(theme::TEXT_FAINT)
                    .size(10.0),
            );
        });
    changed
}

/// the floor and ceiling auto tuning works between
fn range(ui: &mut egui::Ui, tuning: &mut Tuning) -> bool {
    let mut changed = false;
    let mut lo = ms(tuning.gap_min);
    let mut hi = ms(tuning.gap_max);

    if ui.add(Slider::new(&mut lo, 5.0..=ms(GAP_MAX)).suffix(" ms").text("fastest allowed")).changed()
    {
        tuning.gap_min = Duration::from_secs_f64(lo / 1000.0);
        changed = true;
    }
    if ui.add(Slider::new(&mut hi, 5.0..=ms(GAP_MAX)).suffix(" ms").text("slowest allowed")).changed()
    {
        tuning.gap_max = Duration::from_secs_f64(hi / 1000.0);
        changed = true;
    }
    // a floor above the ceiling would have it stepping in both directions
    // forever, so keep them the right way round
    if tuning.gap_min > tuning.gap_max {
        std::mem::swap(&mut tuning.gap_min, &mut tuning.gap_max);
        changed = true;
    }

    if ms(tuning.gap_min) < ms(GAP_RELIABLE_MIN) {
        ui.label(
            RichText::new(format!(
                "under {:.0} ms the scope starts ignoring commands, so it will spend its time \
                 backing off and trying again. not harmful, just pointless",
                ms(GAP_RELIABLE_MIN)
            ))
            .color(theme::STOP)
            .size(11.0),
        );
    }
    changed
}

/// one slider, for when youd rather set it yourself
fn manual(ui: &mut egui::Ui, tuning: &mut Tuning) -> bool {
    let mut gap = ms(tuning.cmd_gap);
    let slider = Slider::new(&mut gap, 5.0..=ms(GAP_MAX)).suffix(" ms").text("command gap");
    if ui.add(slider).changed() {
        tuning.cmd_gap = Duration::from_secs_f64(gap / 1000.0);
        return true;
    }
    if gap < ms(GAP_RELIABLE_MIN) {
        ui.label(
            RichText::new(format!(
                "below {:.0} ms the scope starts ignoring commands. it will still work, it \
                 will just throw errors and retry a lot",
                ms(GAP_RELIABLE_MIN)
            ))
            .color(theme::STOP)
            .size(11.0),
        );
    } else {
        ui.label(
            RichText::new(
                "quiet time between commands, measured from the end of the last reply. 20 ms \
                 is the safe default",
            )
            .color(theme::TEXT_FAINT)
            .size(11.0),
        );
    }
    false
}

/// back of an envelope, not a promise. the scopes own speed wanders about
/// depending on what its doing, and it gets slower per channel when both
/// channels are switched on at the front panel
fn estimate(t: Tuning, feed: Feed, current: Duration) -> String {
    let gap = ms(current) as f32;
    if feed == Feed::Screen {
        return format!(
            "live screen : ~{:.0} ms a frame, about 1 fps.\nthe gap barely matters here, \
             the scope paces the transfer itself",
            1030.0 + gap
        );
    }
    // a waveform read is ~50 ms of scope time plus the gap, settings ~5 ms
    let wave = 50.0 + gap;
    let settings = (5.0 + gap) / t.settings_every.max(1) as f32;
    let one = wave + settings;
    let two = wave * 2.0 + settings;
    format!(
        "one channel  : ~{one:.0} ms a frame, {:.1} fps\ntwo channels : ~{two:.0} ms a frame, \
         {:.1} fps",
        1000.0 / one,
        1000.0 / two
    )
}
