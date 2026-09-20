//! colours. dark, because youre looking at glowing traces

use egui::Color32;

/// ch1 yellow, ch2 cyan. same as the scopes own front panel so you dont have
/// to think about which is which
pub const CH: [Color32; 2] = [
    Color32::from_rgb(255, 255, 0),
    Color32::from_rgb(0, 255, 255),
];

/// a channel thats switched off
pub const CH_OFF: Color32 = Color32::from_rgb(95, 95, 95);

pub const BG: Color32 = Color32::from_rgb(12, 12, 14);
pub const GRID_DOT: Color32 = Color32::from_rgb(58, 58, 58);
pub const GRID_AXIS: Color32 = Color32::from_rgb(95, 95, 95);
pub const GRID_EDGE: Color32 = Color32::from_rgb(70, 70, 70);

pub const TEXT: Color32 = Color32::from_rgb(210, 210, 210);
pub const TEXT_DIM: Color32 = Color32::from_rgb(150, 150, 150);
pub const TEXT_FAINT: Color32 = Color32::from_rgb(110, 110, 110);
pub const HEADING: Color32 = Color32::from_rgb(180, 180, 180);

pub const RUN: Color32 = Color32::from_rgb(0, 210, 0);
pub const STOP: Color32 = Color32::from_rgb(255, 170, 0);
pub const BAD: Color32 = Color32::from_rgb(255, 80, 80);
pub const SHORTCUT: Color32 = Color32::from_rgb(120, 190, 120);

/// the xy mode trace. purple, because thats what the scope draws it in, and
/// its not either channels colour because its both of them at once
pub const XY: Color32 = Color32::from_rgb(205, 115, 240);

/// front panel lamps. the channel buttons light in their own colours rather
/// than the trace colours, same as the real panel does it
pub const LED_CH1: Color32 = Color32::from_rgb(70, 235, 100);
pub const LED_CH2: Color32 = Color32::from_rgb(80, 160, 255);
pub const LED_MATH: Color32 = Color32::from_rgb(255, 95, 95);

/// the math trace. magenta, same as the scope draws it, and the M marker
/// down the left edge of its screen
pub const MATH: Color32 = Color32::from_rgb(255, 105, 215);

/// hacker mode green. not quite phosphor but close enough
pub const HACK: Color32 = Color32::from_rgb(120, 230, 140);
