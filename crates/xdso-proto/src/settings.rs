//! ReadSettings, decoded.
//!
//! the scope hands back 208 bytes that are literally its internal settings
//! struct, and protocol.inf says where everything is. we decode that into a
//! flat list of u64 once, then the typed views on top pull out the bits you
//! actually want with real enums instead of magic numbers.
//!
//! every getter returns Option where the scope could hand us a value we have
//! no name for. it will happily do that, so dont unwrap in the ui.

use crate::enums::*;
use crate::frame::ProtoError;
use crate::inf::{self, SETTINGS_FIELDS};
use crate::units::{self, PROBE_MULT};

/// which input were talking about
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Channel {
    Ch1,
    Ch2,
}

impl Channel {
    pub const ALL: [Channel; 2] = [Channel::Ch1, Channel::Ch2];

    /// 0 or 1, which is what the wire wants
    pub fn index(self) -> usize {
        self as usize
    }

    pub fn label(self) -> &'static str {
        match self {
            Channel::Ch1 => "CH1",
            Channel::Ch2 => "CH2",
        }
    }

    /// 1 or 2, for building the inf field names
    fn num(self) -> u8 {
        self as u8 + 1
    }

    pub fn from_index(i: usize) -> Channel {
        if i == 0 { Channel::Ch1 } else { Channel::Ch2 }
    }

    /// the other one. theres only two of them
    pub fn other(self) -> Channel {
        match self {
            Channel::Ch1 => Channel::Ch2,
            Channel::Ch2 => Channel::Ch1,
        }
    }
}

/// the whole front panel state, one u64 per inf field
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Settings {
    values: Vec<u64>,
}

impl Settings {
    /// decode the 208 byte payload. every field is little endian, sizes come
    /// straight from protocol.inf
    pub fn decode(payload: &[u8]) -> Result<Self, ProtoError> {
        let want = inf::settings_len();
        if payload.len() < want {
            return Err(ProtoError::WrongLength { want, got: payload.len() });
        }
        let values = SETTINGS_FIELDS
            .iter()
            .map(|f| {
                let mut buf = [0u8; 8];
                buf[..f.size].copy_from_slice(&payload[f.offset..f.offset + f.size]);
                u64::from_le_bytes(buf)
            })
            .collect();
        Ok(Settings { values })
    }

    /// true before the first successful poll. the ui uses this to avoid
    /// drawing a screenful of zeroes and pretending theyre real
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// raw value by inf name, eg `TRIG-FREQUENCY`. returns 0 for a field that
    /// doesnt exist, which is the same thing the python did
    pub fn raw(&self, name: &str) -> u64 {
        self.get(name).unwrap_or(0)
    }

    /// raw value, or None if that field isnt in protocol.inf at all
    pub fn get(&self, name: &str) -> Option<u64> {
        self.values.get(inf::field_index(name)?).copied()
    }

    fn ch_raw(&self, ch: Channel, suffix: &str) -> u64 {
        self.raw(&format!("VERT-CH{}-{}", ch.num(), suffix))
    }

    pub fn channel(&self, ch: Channel) -> ChannelSettings {
        let vb = self.ch_raw(ch, "VB");
        let probe = self.ch_raw(ch, "PROBE");
        ChannelSettings {
            enabled: self.ch_raw(ch, "DISP") != 0,
            volts_div: units::volts_per_div(vb, probe),
            probe: PROBE_MULT.get(probe as usize).copied(),
            coupling: Coupling::from_raw(self.ch_raw(ch, "COUP")),
            position: signed16(self.ch_raw(ch, "POS")),
            bandwidth_limit: self.ch_raw(ch, "20MHZ") != 0,
            fine: self.ch_raw(ch, "FINE") != 0,
        }
    }

    pub fn trigger(&self) -> TriggerSettings {
        TriggerSettings {
            running: self.raw("TRIG-STATE") != 0,
            kind: TrigType::from_raw(self.raw("TRIG-TYPE")),
            source: TrigSource::from_raw(self.raw("TRIG-SRC")),
            mode: TrigMode::from_raw(self.raw("TRIG-MODE")),
            slope: TrigSlope::from_raw(self.raw("TRIG-EDGE-SLOPE")),
            level: signed16(self.raw("TRIG-VPOS")),
            // the scopes own hardware counter, reported in milli hertz
            frequency_hz: self.raw("TRIG-FREQUENCY") as f64 / 1000.0,
        }
    }

    pub fn horizontal(&self) -> HorizontalSettings {
        let tb = self.raw("HORIZ-TB");
        HorizontalSettings {
            seconds_div: units::seconds_per_div(tb),
            window_zoom: self.raw("HORIZ-WIN-STATE") != 0,
        }
    }

    pub fn display(&self) -> DisplaySettings {
        DisplaySettings {
            mode: DisplayMode::from_raw(self.raw("DISPLAY-MODE")),
            format: DisplayFormat::from_raw(self.raw("DISPLAY-FORMAT")),
            grid_kind: self.raw("DISPLAY-GRID-KIND") as u8,
            persist: self.raw("DISPLAY-PERSIST") != 0,
        }
    }

    /// the math channel.
    ///
    /// the scope has a volts per division and a position for math, you can
    /// see them in its own math menu, but **neither is in the settings blob**.
    /// changing the scale on the front panel moves none of the 119 fields, so
    /// we can draw the trace and name the operation but not scale it
    pub fn math(&self) -> MathSettings {
        MathSettings {
            enabled: self.raw("MATH-DISP") != 0,
            mode: MathMode::from_raw(self.raw("MATH-MODE")),
            fft_source: FftSource::from_raw(self.raw("MATH-FFT-SRC")),
            fft_window: FftWindow::from_raw(self.raw("MATH-FFT-WIN")),
            fft_factor: self.raw("MATH-FFT-FACTOR") as u8,
            fft_db: self.raw("MATH-FFT-DB") as u8,
        }
    }

    pub fn acquire(&self) -> AcquireSettings {
        AcquireSettings {
            mode: AcqMode::from_raw(self.raw("ACQURIE-MODE")),
            averages: self.raw("ACQURIE-AVG-CNT") as u32,
        }
    }

    /// the 8 measurement slots, in menu order. slots set to Off are kept so
    /// the indexes still mean something
    pub fn measurements(&self) -> Vec<MeasureSlot> {
        (1..=8)
            .map(|i| MeasureSlot {
                kind: MeasureKind::from_raw(self.raw(&format!("MEASURE-ITEM{i}"))),
                // src is 0 or 1. mask it because a stale byte up there would
                // otherwise index off the end of the channel array
                source: Channel::from_index((self.raw(&format!("MEASURE-ITEM{i}-SRC")) & 1) as usize),
            })
            .collect()
    }

    /// seconds between samples. `HORIZ-TB` is per division and theres 200
    /// samples in a division
    pub fn sample_interval(&self) -> Option<f64> {
        Some(self.horizontal().seconds_div? / crate::SAMPLES_PER_DIV as f64)
    }
}

/// 2 byte fields that hold a position are signed, and the wire is unsigned.
/// a position of -62 arrives as 65474
fn signed16(v: u64) -> i32 {
    let v = v as u16;
    if v > 32767 { v as i32 - 65536 } else { v as i32 }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChannelSettings {
    pub enabled: bool,
    /// volts per division with the probe attenuation already folded in
    pub volts_div: Option<f64>,
    pub probe: Option<u32>,
    pub coupling: Option<Coupling>,
    /// vertical offset in adc counts, signed, centre is 0
    pub position: i32,
    pub bandwidth_limit: bool,
    pub fine: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TriggerSettings {
    /// false means acquisition is stopped and youll get no samples
    pub running: bool,
    pub kind: Option<TrigType>,
    pub source: Option<TrigSource>,
    pub mode: Option<TrigMode>,
    pub slope: Option<TrigSlope>,
    pub level: i32,
    pub frequency_hz: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HorizontalSettings {
    pub seconds_div: Option<f64>,
    pub window_zoom: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DisplaySettings {
    pub mode: Option<DisplayMode>,
    pub format: Option<DisplayFormat>,
    /// 0 full grid, 1 sparse, 2 none. matches the display menu
    pub grid_kind: u8,
    pub persist: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MathSettings {
    pub enabled: bool,
    /// which operation, None if its a value we dont have a name for
    pub mode: Option<MathMode>,
    /// which channel the fft is looking at
    pub fft_source: Option<FftSource>,
    pub fft_window: Option<FftWindow>,
    /// **not confirmed.** the scopes fft menu has an FFT Zoom of x1, x2, x5
    /// or x10 on its second page and this is the only field left that could
    /// be it. mine reads 0 with the zoom on x1
    pub fft_factor: u8,
    /// **not confirmed either.** mine reads 3 while the screen says 10.0dB
    /// per division, so its probably an index into a list of dB scales rather
    /// than the Vrms/dBrms toggle, which would only need one bit
    pub fft_db: u8,
}

impl MathSettings {
    /// the fft takes the display over completely : the scope stops drawing
    /// the channel traces and the x axis turns into frequency
    pub fn is_fft(&self) -> bool {
        self.enabled && self.mode == Some(MathMode::Fft)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AcquireSettings {
    pub mode: Option<AcqMode>,
    pub averages: u32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MeasureSlot {
    pub kind: Option<MeasureKind>,
    pub source: Channel,
}


#[cfg(test)]
mod tests {
    use super::*;

    /// a blob with everything zeroed except what the test sets
    fn blob(edits: &[(&str, u64)]) -> Vec<u8> {
        let mut b = vec![0u8; inf::settings_len()];
        for (name, v) in edits {
            let f = inf::field(name).unwrap_or_else(|| panic!("no field {name}"));
            let bytes = v.to_le_bytes();
            b[f.offset..f.offset + f.size].copy_from_slice(&bytes[..f.size]);
        }
        b
    }

    #[test]
    fn short_payload_is_rejected() {
        assert_eq!(
            Settings::decode(&[0u8; 10]),
            Err(ProtoError::WrongLength { want: 208, got: 10 })
        );
    }

    #[test]
    fn decodes_a_channel() {
        let s = Settings::decode(&blob(&[
            ("VERT-CH1-DISP", 1),
            ("VERT-CH1-VB", 9),    // 1 V/div
            ("VERT-CH1-PROBE", 1), // through a 10x probe
            ("VERT-CH1-COUP", 1),
            ("VERT-CH1-POS", 65474), // -62
        ]))
        .unwrap();
        let ch = s.channel(Channel::Ch1);
        assert!(ch.enabled);
        assert_eq!(ch.volts_div, Some(10.0));
        assert_eq!(ch.probe, Some(10));
        assert_eq!(ch.coupling, Some(Coupling::Ac));
        assert_eq!(ch.position, -62);

        // ch2 untouched, so off and sitting at centre
        assert!(!s.channel(Channel::Ch2).enabled);
        assert_eq!(s.channel(Channel::Ch2).position, 0);
    }

    #[test]
    fn decodes_the_wide_fields() {
        // TRIG-FREQUENCY is 8 bytes of milli hertz
        let s = Settings::decode(&blob(&[("TRIG-FREQUENCY", 1_000_000)])).unwrap();
        assert_eq!(s.trigger().frequency_hz, 1000.0);
    }

    #[test]
    fn timebase_becomes_a_sample_interval() {
        let s = Settings::decode(&blob(&[("HORIZ-TB", 15)])).unwrap(); // 200us/div
        assert_eq!(s.horizontal().seconds_div, Some(200e-6));
        assert_eq!(s.sample_interval(), Some(1e-6));
    }

    #[test]
    fn unknown_enum_values_come_back_as_none() {
        let s = Settings::decode(&blob(&[("TRIG-TYPE", 200)])).unwrap();
        assert_eq!(s.trigger().kind, None);
    }

    #[test]
    fn measurement_slots_keep_their_positions() {
        let s = Settings::decode(&blob(&[
            ("MEASURE-ITEM1", 1), // frequency on ch1
            ("MEASURE-ITEM3", 4), // pk-pk on ch2
            ("MEASURE-ITEM3-SRC", 1),
        ]))
        .unwrap();
        let m = s.measurements();
        assert_eq!(m.len(), 8);
        assert_eq!(m[0].kind, Some(MeasureKind::Frequency));
        assert_eq!(m[0].source, Channel::Ch1);
        assert_eq!(m[1].kind, Some(MeasureKind::Off));
        assert_eq!(m[2].kind, Some(MeasureKind::PkPk));
        assert_eq!(m[2].source, Channel::Ch2);
    }
}
