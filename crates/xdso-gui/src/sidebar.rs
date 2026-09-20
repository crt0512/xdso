//! everything the scope knows about itself, down the right hand side.
//!
//! all of it comes out of one ReadSettings, which costs about 5 ms, so theres
//! no reason to be shy about showing the lot.

use egui::{RichText, Ui};

use xdso_dsp::{Waveform, measure};
use xdso_proto::{Channel, Settings, units::eng};
use xdso_usb::Snapshot;

use crate::theme;

/// volts per channel, worked out once a frame and shared with the measurements
pub type Volts = [Option<Vec<f32>>; 2];

pub fn draw(ui: &mut Ui, snap: &Snapshot, volts: &Volts) {
    let live_screen = snap.feed == xdso_usb::Feed::Screen;
    let s = &snap.settings;
    ui.spacing_mut().item_spacing.y = 3.0;

    // grab the panel height now, before the scroll area gets hold of the ui,
    // so we can centre against the panel rather than the viewport
    let room = ui.available_height();

    // eight measurements plus everything else doesnt fit on a short window,
    // and silently clipping the bottom off is worse than a scrollbar
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        crate::layout::center_v(ui, "sidebar", room, |ui| {
        for ch in Channel::ALL {
            channel_block(ui, s, ch);
            ui.add_space(4.0);
        }

        let m = s.math();
        if m.enabled {
            ui.label(
                RichText::new(format!("Math  {}", m.mode.map_or("?", |x| x.label())))
                    .color(theme::MATH)
                    .size(14.0),
            );
            if m.is_fft() {
                ui.label(
                    RichText::new(format!(
                        "  {}  {}",
                        m.fft_source.map_or("?", |x| x.label()),
                        m.fft_window.map_or("?", |x| x.label()),
                    ))
                    .color(theme::MATH)
                    .size(11.0),
                );
                // the V/div knob drives the dB scale while youre in fft,
                // not the channels own volts per division
                ui.label(
                    RichText::new(format!("  dB scale {} (V/div knob)", m.fft_db))
                        .color(theme::MATH)
                        .size(11.0),
                );
                ui.label(
                    RichText::new("  not supported yet, press g")
                        .color(theme::STOP)
                        .size(11.0),
                );

            }
            // the scope has a scale and a position for math, theyre in its own
            // math menu, but neither is in the settings blob. changing the
            // scale on the front panel moves none of the 119 fields, so we can
            // draw the trace and name the operation but not put volts on it
            if !m.is_fft() {
                ui.label(
                    RichText::new("  the scope doesnt send its scale")
                        .color(theme::MATH)
                        .size(11.0),
                );
            }
            ui.add_space(4.0);
        }

        ui.separator();
        trigger_block(ui, s);

        ui.separator();
        if m.is_fft() {
            // theres nothing to measure : we dont fetch the channel traces
            // while the scope is doing an fft, and measuring a spectrum with
            // tools meant for a time domain trace would be nonsense anyway
            ui.label(RichText::new("Measure").color(theme::HEADING).size(13.0));
            ui.label(
                RichText::new("  off while the fft is up")
                    .color(theme::TEXT_FAINT)
                    .size(11.0),
            );
        } else if live_screen {
            // no waveforms are being polled, so theres nothing to measure.
            // rows full of -- just look broken
            ui.label(RichText::new("Measure").color(theme::HEADING).size(13.0));
            ui.label(
                RichText::new("  paused while the live screen is up")
                    .color(theme::TEXT_FAINT)
                    .size(11.0),
            );
        } else {
            measure_block(ui, s, volts);
        }
        });
    });
}

fn channel_block(ui: &mut Ui, s: &Settings, ch: Channel) {
    let c = s.channel(ch);
    let colour = if c.enabled { theme::CH[ch.index()] } else { theme::CH_OFF };

    let head = match (c.enabled, c.volts_div) {
        (true, Some(v)) => format!("{}  {}/div", ch.label(), eng(v, "V", 3)),
        (true, None) => format!("{}  ?/div", ch.label()),
        (false, _) => format!("{}  off", ch.label()),
    };
    ui.label(RichText::new(head).color(colour).size(14.0));

    if c.enabled {
        let detail = format!(
            "  {}  {}x  pos {}{}",
            c.coupling.map_or("?", |x| x.label()),
            c.probe.map_or("?".to_string(), |p| p.to_string()),
            c.position,
            if c.bandwidth_limit { "  20MHz" } else { "" },
        );
        ui.label(RichText::new(detail).color(colour).size(11.0));
    }
}

fn trigger_block(ui: &mut Ui, s: &Settings) {
    let t = s.trigger();
    ui.label(RichText::new("Trigger").color(theme::HEADING).size(13.0));
    ui.label(
        RichText::new(format!(
            "  {}  {}  {}",
            t.kind.map_or("?", |x| x.label()),
            t.source.map_or("?", |x| x.label()),
            t.slope.map_or("?", |x| x.label()),
        ))
        .color(theme::TEXT_DIM)
        .size(11.0),
    );
    ui.label(
        RichText::new(format!(
            "  {}   {}",
            t.mode.map_or("?", |x| x.label()),
            if t.running { "RUN" } else { "STOP" }
        ))
        .color(if t.running { theme::RUN } else { theme::STOP })
        .size(11.0),
    );
    // the scopes own hardware frequency counter, nothing to do with our
    // measurements. it reads 0 when theres no signal to count
    if t.frequency_hz > 0.0 {
        ui.label(
            RichText::new(format!("  counter {}", eng(t.frequency_hz, "Hz", 5)))
                .color(theme::TEXT_DIM)
                .size(11.0),
        );
    }

    ui.add_space(4.0);
    let a = s.acquire();
    // the average count field keeps a stale value while youre in sample mode,
    // so only show it when its actually averaging
    let extra = match a.mode {
        Some(xdso_proto::AcqMode::Average) => format!(" x{}", a.averages),
        _ => String::new(),
    };
    ui.label(
        RichText::new(format!("Acq  {}{extra}", a.mode.map_or("?", |x| x.label())))
            .color(theme::TEXT_DIM)
            .size(11.0),
    );
    let d = s.display();
    ui.label(
        RichText::new(format!(
            "Disp {}  {}",
            d.mode.map_or("?", |x| x.label()),
            d.format.map_or("?", |x| x.label())
        ))
        .color(theme::TEXT_DIM)
        .size(11.0),
    );
}

fn measure_block(ui: &mut Ui, s: &Settings, volts: &Volts) {
    ui.label(RichText::new("Measure").color(theme::HEADING).size(13.0));

    let dt = s.sample_interval().unwrap_or(0.0);
    let slots: Vec<_> = s
        .measurements()
        .into_iter()
        .filter(|m| m.kind.is_some_and(|k| !k.is_off()))
        .collect();

    if slots.is_empty() {
        ui.label(
            RichText::new("  none enabled on the scope")
                .color(theme::TEXT_FAINT)
                .size(11.0),
        );
        return;
    }

    egui::Grid::new("measurements").num_columns(2).spacing([8.0, 2.0]).show(ui, |ui| {
        for slot in slots {
            let Some(kind) = slot.kind else { continue };
            // the slots own channel is the source, the other one is "channel
            // 2" for the measurements that compare a pair. so a Delay1-2 slot
            // set to CH2 measures ch2 against ch1, which is the sane reading
            // of a setting you had to go and change on purpose
            let other = slot.source.other();
            let wave = |ch: Channel| {
                volts[ch.index()].as_ref().map(|v| Waveform { volts: v, dt })
            };
            let shown = wave(slot.source)
                .and_then(|w| measure(kind, &w, wave(other).as_ref()))
                .map_or_else(|| "--".to_string(), |m| m.format());

            ui.label(RichText::new(kind.label()).color(theme::TEXT_DIM).size(11.0));
            ui.label(
                RichText::new(shown)
                    .color(theme::CH[slot.source.index()])
                    .size(12.0)
                    .monospace(),
            );
            ui.end_row();
        }
    });
}
