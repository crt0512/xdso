//! wire protocol for the hantek DSO5000P (DSO5072P/5102P/5202P).
//!
//! no i/o lives in here. you hand [`frame::encode`] a command and it gives you
//! bytes, you hand [`frame::decode`] whatever came back off the wire and it
//! gives you a payload. everything else is table lookups and struct decoding.
//!
//! the reason this is its own crate is that the interesting parts (settings
//! layout, key numbering, the unit tables) are completely testable without a
//! scope plugged in, and i would rather not have usb or a gui anywhere near
//! them.

pub mod enums;
pub mod frame;
pub mod inf;
pub mod keys;
pub mod settings;
pub mod units;

pub use enums::*;
pub use frame::{Packet, ProtoError, cmd, reply};
pub use keys::{Key, keys};
pub use settings::{Channel, Settings};

/// vendor and product id the scope shows up as. yes its a compaq id, no idea
/// why, hantek presumably never bothered getting their own.
pub const VID: u16 = 0x049f;
pub const PID: u16 = 0x505a;

/// 3200 samples per channel, always. 16 horizontal divisions of 200.
pub const SAMPLES: usize = 3200;
/// horizontal divisions on the graticule
pub const H_DIVS: usize = 16;
/// vertical divisions on the graticule
pub const V_DIVS: usize = 8;
/// 200 samples per horizontal division
pub const SAMPLES_PER_DIV: usize = SAMPLES / H_DIVS;

/// adc counts per vertical division. 256 counts spread over the full 10
/// division range, of which 8 are actually on screen.
pub const COUNTS_PER_DIV: f32 = 25.6;
/// sample value 128 sits dead on the graticule centre
pub const CENTRE_COUNT: f32 = 128.0;

/// the scopes framebuffer, for when you grab its actual screen
pub const SCREEN_W: usize = 800;
/// same, vertically
pub const SCREEN_H: usize = 480;
