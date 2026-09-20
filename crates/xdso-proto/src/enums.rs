//! the little enums the settings blob is full of.
//!
//! most of these are single bytes that index a menu on the scope. where a
//! value is confirmed against what the screen actually shows it says so,
//! otherwise the order is taken from the on screen menu and is a decent guess.
//! a byte we dont have a name for comes back as None rather than being made up.

/// build an enum that maps a raw settings byte to a label.
///
/// every one of these needs the exact same from_raw/label pair so heres a
/// macro instead of writing it out eight times like a mug.
macro_rules! scope_enum {
    ($(#[$outer:meta])* $name:ident { $($variant:ident = $label:literal),* $(,)? }) => {
        $(#[$outer])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum $name { $($variant),* }

        impl $name {
            /// every variant in wire order, so the index is the raw value
            pub const ALL: &'static [$name] = &[$($name::$variant),*];

            /// None means the scope sent a value this table doesnt cover
            pub fn from_raw(v: u64) -> Option<Self> {
                usize::try_from(v).ok().and_then(|i| Self::ALL.get(i).copied())
            }

            /// what the scope calls it on screen
            pub fn label(self) -> &'static str {
                match self { $($name::$variant => $label),* }
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(self.label())
            }
        }
    };
}

scope_enum!(
    /// input coupling. DC and AC confirmed on screen, GND assumed third
    Coupling { Dc = "DC", Ac = "AC", Gnd = "GND" }
);

scope_enum!(
    /// what the trigger is looking for
    TrigType { Edge = "Edge", Video = "Video", Pulse = "Pulse", Slope = "Slope", Overtime = "Overtime", Swap = "Swap" }
);

scope_enum!(
    TrigMode { Auto = "Auto", Normal = "Normal", Single = "Single" }
);

scope_enum!(
    TrigSource { Ch1 = "CH1", Ch2 = "CH2", Ext = "EXT", ExtDiv5 = "EXT/5", AcLine = "AC Line" }
);

scope_enum!(
    TrigSlope { Rising = "Rising", Falling = "Falling" }
);

scope_enum!(
    AcqMode { Sample = "Sample", PeakDetect = "Peak Detect", Average = "Average" }
);

scope_enum!(
    /// YT is the normal time domain view, XY plots ch1 against ch2
    DisplayFormat { Yt = "YT", Xy = "XY" }
);

scope_enum!(
    DisplayMode { Vectors = "Vectors", Dots = "Dots" }
);

scope_enum!(
    /// what MATH-MODE means.
    MathMode {
        Ch1PlusCh2 = "CH1+CH2",
        Ch1MinusCh2 = "CH1-CH2",
        Ch2MinusCh1 = "CH2-CH1",
        Ch1TimesCh2 = "CH1xCH2",
        Ch1OverCh2 = "CH1/CH2",
        Ch2OverCh1 = "CH2/CH1",
        Fft = "FFT",
    }
);

scope_enum!(
    /// which channel the fft is chewing on
    FftSource { Ch1 = "CH1", Ch2 = "CH2" }
);

scope_enum!(
    /// fft window function, in the order the scopes own menu lists them
    FftWindow {
        Hanning = "Hanning",
        Flattop = "Flattop",
        Rectangular = "Rectangular",
        Bartlett = "Bartlett",
        Blackman = "Blackman",
    }
);

scope_enum!(
    /// what the MEASURE-ITEMn bytes mean.
    ///
    /// lifted verbatim from the scopes own `/OurLanguages/English.lan`, which
    /// lists them in enum order starting at Off. so this is the firmwares own
    /// table, not something i guessed at. whether we can actually *compute*
    /// each one is a separate question, see the xdso-dsp crate.
    MeasureKind {
        Off = "Off",
        Frequency = "Frequency",
        Period = "Period",
        Mean = "Mean",
        PkPk = "Pk-Pk",
        CyclicRms = "Cyclic RMS",
        Minimum = "Minimum",
        Maximum = "Maximum",
        RiseTime = "Rise Time",
        FallTime = "Fall Time",
        PosPulseWidth = "+Pulse Width",
        NegPulseWidth = "-Pulse Width",
        Delay12Rise = "Delay1-2Rise",
        Delay12Fall = "Delay1-2Fall",
        PosDuty = "+Duty",
        NegDuty = "-Duty",
        Vbase = "Vbase",
        Vtop = "Vtop",
        Vmid = "Vmid",
        Vamp = "Vamp",
        Overshoot = "Overshoot",
        Preshoot = "Preshoot",
        PeriodMean = "Period Mean",
        PeriodRms = "Period RMS",
        FovShoot = "FOVShoot",
        RpreShoot = "RPREShoot",
        BWidth = "BWidth",
        Frf = "FRF",
        Ffr = "FFR",
        Lrr = "LRR",
    }
);

impl MeasureKind {
    /// off means the slot is empty, dont draw a row for it
    pub fn is_off(self) -> bool {
        self == MeasureKind::Off
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_values_line_up_with_the_index() {
        assert_eq!(Coupling::from_raw(0), Some(Coupling::Dc));
        assert_eq!(Coupling::from_raw(1), Some(Coupling::Ac));
        assert_eq!(Coupling::from_raw(2), Some(Coupling::Gnd));
        assert_eq!(Coupling::from_raw(3), None);
        assert_eq!(Coupling::from_raw(u64::MAX), None);
    }

    #[test]
    fn measure_table_matches_the_lan_file() {
        assert_eq!(MeasureKind::ALL.len(), 30);
        assert_eq!(MeasureKind::from_raw(0), Some(MeasureKind::Off));
        assert_eq!(MeasureKind::from_raw(4).unwrap().label(), "Pk-Pk");
        assert_eq!(MeasureKind::from_raw(14).unwrap().label(), "+Duty");
        assert_eq!(MeasureKind::from_raw(29).unwrap().label(), "LRR");
    }
}
