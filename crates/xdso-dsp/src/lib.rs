//! measurements, computed on the host.
//!
//! the scope can compute all of these itself but it wont tell us the answers
//! over usb, only which ones you picked in the measure menu. so we read the
//! same waveform its looking at and do the maths here. turns out thats fine,
//! and it means we can measure things the scope doesnt offer if we ever want
//! to.
//!
//! all of this works on volts and seconds, never adc counts. converting is the
//! callers job (see `xdso_proto::Settings::channel`).

pub mod stats;

use xdso_proto::MeasureKind;
use xdso_proto::units::eng;

pub use stats::{crossings, median, top_base};

/// a channels trace, already in volts, with the time between samples
#[derive(Debug, Clone, Copy)]
pub struct Waveform<'a> {
    pub volts: &'a [f32],
    /// seconds per sample
    pub dt: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unit {
    Volt,
    Second,
    Hertz,
    Percent,
}

impl Unit {
    pub fn suffix(self) -> &'static str {
        match self {
            Unit::Volt => "V",
            Unit::Second => "s",
            Unit::Hertz => "Hz",
            Unit::Percent => "%",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Measurement {
    pub value: f64,
    pub unit: Unit,
}

impl Measurement {
    fn new(value: f64, unit: Unit) -> Option<Self> {
        value.is_finite().then_some(Measurement { value, unit })
    }

    /// how it should read on screen
    pub fn format(self) -> String {
        match self.unit {
            // percentages dont get an engineering prefix, nobody writes 1.2mPercent
            Unit::Percent => format!("{:.1}%", self.value),
            u => eng(self.value, u.suffix(), 4),
        }
    }
}

/// a trace flatter than this has no edges worth measuring
const FLAT_VOLTS: f64 = 1e-9;

/// compute one measurement, or None if we cant do that one.
///
/// `other` is the second channel, for the measurements that compare two
/// traces. pass None and those just come back None.
///
/// the ones we still cant do are BWidth, FOVShoot and RPREShoot, because i
/// have not worked out what the scope means by them. they come back None and
/// the ui shows `--`.
pub fn measure(kind: MeasureKind, w: &Waveform, other: Option<&Waveform>) -> Option<Measurement> {
    use MeasureKind as M;
    let v = w.volts;
    if v.is_empty() || kind == M::Off {
        return None;
    }

    // the ones that need both traces get handled before anything else, since
    // none of the single channel working below applies to them
    if let Some(m) = two_channel(kind, w, other) {
        return m;
    }

    let (top, base) = top_base(v);
    let amp = top - base;
    let mid = (top + base) / 2.0;
    let min = v.iter().copied().fold(f32::INFINITY, f32::min) as f64;
    let max = v.iter().copied().fold(f32::NEG_INFINITY, f32::max) as f64;
    let mean = v.iter().map(|&x| x as f64).sum::<f64>() / v.len() as f64;

    // the level only measurements dont care about edges, do them first
    match kind {
        M::Mean => return Measurement::new(mean, Unit::Volt),
        M::PkPk => return Measurement::new(max - min, Unit::Volt),
        M::Minimum => return Measurement::new(min, Unit::Volt),
        M::Maximum => return Measurement::new(max, Unit::Volt),
        M::CyclicRms => {
            let ms = v.iter().map(|&x| (x as f64) * (x as f64)).sum::<f64>() / v.len() as f64;
            return Measurement::new(ms.sqrt(), Unit::Volt);
        }
        // the "period" ones are the same sums, but over a whole number of
        // cycles rather than the whole capture. on a square wave thats the
        // difference between a mean of 2.5 V and one of 2.8 V, depending on
        // how much of a extra half cycle happened to be on screen
        M::PeriodMean | M::PeriodRms => {
            let w = whole_cycles(v).unwrap_or(v);
            let value = match kind {
                M::PeriodMean => w.iter().map(|&x| x as f64).sum::<f64>() / w.len() as f64,
                _ => {
                    let ms =
                        w.iter().map(|&x| (x as f64) * (x as f64)).sum::<f64>() / w.len() as f64;
                    ms.sqrt()
                }
            };
            return Measurement::new(value, Unit::Volt);
        }
        M::Vtop => return Measurement::new(top, Unit::Volt),
        M::Vbase => return Measurement::new(base, Unit::Volt),
        M::Vmid => return Measurement::new(mid, Unit::Volt),
        M::Vamp => return Measurement::new(amp, Unit::Volt),
        M::Overshoot if amp > 1e-12 => {
            return Measurement::new((max - top) / amp * 100.0, Unit::Percent);
        }
        M::Preshoot if amp > 1e-12 => {
            return Measurement::new((base - min) / amp * 100.0, Unit::Percent);
        }
        _ => {}
    }

    if amp < FLAT_VOLTS {
        return None; // flat line. no edges, nothing to time
    }

    let rising = crossings(v, mid, true);
    let falling = crossings(v, mid, false);

    match kind {
        M::Frequency | M::Period => {
            let period = period_of(&rising, &falling)? * w.dt;
            if period <= 0.0 {
                return None;
            }
            match kind {
                M::Frequency => Measurement::new(1.0 / period, Unit::Hertz),
                _ => Measurement::new(period, Unit::Second),
            }
        }

        M::PosPulseWidth | M::NegPulseWidth | M::PosDuty | M::NegDuty => {
            let positive = matches!(kind, M::PosPulseWidth | M::PosDuty);
            let (first, second) = if positive { (&rising, &falling) } else { (&falling, &rising) };
            let width = median(&mut spans(first, second))? * w.dt;
            match kind {
                M::PosPulseWidth | M::NegPulseWidth => Measurement::new(width, Unit::Second),
                _ => {
                    let period = period_of(&rising, &falling)? * w.dt;
                    (period > 0.0)
                        .then(|| Measurement::new(width / period * 100.0, Unit::Percent))
                        .flatten()
                }
            }
        }

        M::RiseTime | M::FallTime => {
            let going_up = kind == M::RiseTime;
            let low = crossings(v, base + 0.1 * amp, going_up);
            let high = crossings(v, base + 0.9 * amp, going_up);
            // on a rising edge you hit 10% then 90%, falling is the other way
            let (from, to) = if going_up { (&low, &high) } else { (&high, &low) };
            let span = median(&mut spans(from, to))? * w.dt;
            Measurement::new(span, Unit::Second)
        }

        _ => None,
    }
}

/// the slice covering a whole number of cycles, first rising edge to last.
///
/// returns None if theres less than one full cycle on screen, in which case
/// the caller may as well use the lot
fn whole_cycles(v: &[f32]) -> Option<&[f32]> {
    let (top, base) = top_base(v);
    if top - base < FLAT_VOLTS {
        return None;
    }
    let edges = crossings(v, (top + base) / 2.0, true);
    let (first, last) = (edges.first()?, edges.last()?);
    // round inwards so we never include a partial cycle at either end
    let a = first.ceil() as usize;
    let b = last.floor() as usize;
    (b > a).then(|| &v[a..b])
}

/// the measurements that compare two traces.
///
/// returns None if `kind` isnt one of them, Some(None) if it is but we cant
/// work it out, which is how the caller knows to stop rather than fall through
/// to the single channel code.
///
/// **these are an interpretation.** the scope computes them too but wont tell
/// us its answers, and the manual doesnt define them, so the names are the
/// only clue :
///
/// | kind | what we take it to mean |
/// |---|---|
/// | Delay1-2Rise | first rising edge of the source, to the **nearest** rising edge of the other channel |
/// | Delay1-2Fall | same on falling edges |
/// | FRF | **f**irst **r**ise of the source, to the first **f**all of the other channel **after it** |
/// | FFR | first fall of the source, to the first rise of the other after it |
/// | LRR | **l**ast **r**ise of the source to the last **r**ise of the other |
///
/// the delays use the nearest edge rather than the next one, so they come out
/// signed and small, which is what you want from a delay reading.
///
/// FRF and FFR take the first edge **after**, rather than literally the first
/// edge in the capture. taken literally the answer flips sign depending on
/// whether the capture happened to start high or low, so it flickers between
/// +500 us and -500 us frame to frame on a square wave, which is useless. the
/// "after" reading is stable and is the number you actually wanted.
///
/// to check these against the scope : put the same signal on both channels and
/// the two delays should read about zero, then turn one measurement on in the
/// scopes own measure menu and compare.
fn two_channel(
    kind: MeasureKind,
    a: &Waveform,
    b: Option<&Waveform>,
) -> Option<Option<Measurement>> {
    use MeasureKind as M;
    if !matches!(kind, M::Delay12Rise | M::Delay12Fall | M::Frf | M::Ffr | M::Lrr) {
        return None;
    }
    let Some(b) = b else { return Some(None) };

    // each channel gets its own mid level, they can be on completely
    // different volts per division
    let ea = |rising| mid_crossings(a, rising);
    let eb = |rising| mid_crossings(b, rising);

    let span = match kind {
        M::Delay12Rise => pair_nearest(&ea(true), &eb(true)),
        M::Delay12Fall => pair_nearest(&ea(false), &eb(false)),
        M::Frf => pair_after(&ea(true), &eb(false)),
        M::Ffr => pair_after(&ea(false), &eb(true)),
        M::Lrr => pair_last(&ea(true), &eb(true)),
        _ => None,
    };
    Some(span.and_then(|s| Measurement::new(s * a.dt, Unit::Second)))
}

/// mid level crossings of a trace, or nothing if its too flat to have edges
fn mid_crossings(w: &Waveform, rising: bool) -> Vec<f64> {
    let (top, base) = top_base(w.volts);
    if top - base < FLAT_VOLTS {
        return Vec::new();
    }
    crossings(w.volts, (top + base) / 2.0, rising)
}

/// first edge of a, to whichever edge of b is closest to it. signed
fn pair_nearest(a: &[f64], b: &[f64]) -> Option<f64> {
    let first = *a.first()?;
    let near = b.iter().copied().min_by(|p, q| {
        (p - first).abs().total_cmp(&(q - first).abs())
    })?;
    Some(near - first)
}

/// first edge of a, to the first edge of b that comes after it
fn pair_after(a: &[f64], b: &[f64]) -> Option<f64> {
    let first = *a.first()?;
    b.iter().copied().find(|&x| x > first).map(|x| x - first)
}

fn pair_last(a: &[f64], b: &[f64]) -> Option<f64> {
    Some(b.last()? - a.last()?)
}

/// median gap between consecutive mid crossings, in samples.
///
/// prefers the rising edges and falls back to falling, same as the scope. one
/// crossing is not a period so we need at least two.
fn period_of(rising: &[f64], falling: &[f64]) -> Option<f64> {
    let edges = if rising.len() >= 2 { rising } else { falling };
    if edges.len() < 2 {
        return None;
    }
    let mut gaps: Vec<f64> = edges.windows(2).map(|w| w[1] - w[0]).collect();
    median(&mut gaps)
}

/// for each crossing in `from`, how far to the next one in `to`. thats one
/// pulse width, in samples
fn spans(from: &[f64], to: &[f64]) -> Vec<f64> {
    from.iter()
        .filter_map(|&t| to.iter().find(|&&n| n > t).map(|&n| n - t))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::TAU;

    const DT: f64 = 1e-6; // 1 MSa/s, so a 1000 sample period is 1 kHz

    /// square wave, `period` samples long, `duty` of it high
    fn square(n: usize, period: f64, duty: f64, low: f32, high: f32) -> Vec<f32> {
        (0..n)
            .map(|i| if (i as f64 % period) < period * duty { high } else { low })
            .collect()
    }

    fn sine(n: usize, period: f64, amp: f32) -> Vec<f32> {
        (0..n).map(|i| (amp as f64 * (TAU * i as f64 / period).sin()) as f32).collect()
    }

    fn m(kind: MeasureKind, v: &[f32]) -> Option<Measurement> {
        measure(kind, &Waveform { volts: v, dt: DT }, None)
    }

    /// same but with a second channel hooked up
    fn m2(kind: MeasureKind, a: &[f32], b: &[f32]) -> Option<Measurement> {
        measure(kind, &Waveform { volts: a, dt: DT }, Some(&Waveform { volts: b, dt: DT }))
    }

    #[test]
    fn levels_off_a_square_wave() {
        // 800 sample period so 3200 samples is exactly 4 cycles. with 1000 you
        // get 3.2 cycles and the mean is 1.125, which is correct but a rubbish
        // thing to assert against
        let v = square(3200, 800.0, 0.5, -1.0, 3.0);
        assert_eq!(m(MeasureKind::Maximum, &v).unwrap().value, 3.0);
        assert_eq!(m(MeasureKind::Minimum, &v).unwrap().value, -1.0);
        assert_eq!(m(MeasureKind::PkPk, &v).unwrap().value, 4.0);
        assert_eq!(m(MeasureKind::Vtop, &v).unwrap().value, 3.0);
        assert_eq!(m(MeasureKind::Vbase, &v).unwrap().value, -1.0);
        assert_eq!(m(MeasureKind::Vamp, &v).unwrap().value, 4.0);
        assert_eq!(m(MeasureKind::Vmid, &v).unwrap().value, 1.0);
        assert!((m(MeasureKind::Mean, &v).unwrap().value - 1.0).abs() < 1e-6);
    }

    #[test]
    fn timing_off_a_square_wave() {
        let v = square(3200, 1000.0, 0.5, 0.0, 1.0);
        let f = m(MeasureKind::Frequency, &v).unwrap();
        assert_eq!(f.unit, Unit::Hertz);
        assert!((f.value - 1000.0).abs() < 1.0, "got {} Hz", f.value);

        let p = m(MeasureKind::Period, &v).unwrap();
        assert!((p.value - 1e-3).abs() < 1e-6, "got {} s", p.value);
    }

    #[test]
    fn duty_cycle_is_not_always_half() {
        let v = square(3200, 1000.0, 0.25, 0.0, 1.0);
        let d = m(MeasureKind::PosDuty, &v).unwrap();
        assert_eq!(d.unit, Unit::Percent);
        assert!((d.value - 25.0).abs() < 1.0, "got {}%", d.value);

        let n = m(MeasureKind::NegDuty, &v).unwrap();
        assert!((n.value - 75.0).abs() < 1.0, "got {}%", n.value);

        let w = m(MeasureKind::PosPulseWidth, &v).unwrap();
        assert!((w.value - 250e-6).abs() < 2e-6, "got {} s", w.value);
    }

    #[test]
    fn sine_frequency_and_rms() {
        let v = sine(3200, 400.0, 2.0); // 400 samples at 1 MSa/s = 2.5 kHz
        let f = m(MeasureKind::Frequency, &v).unwrap();
        assert!((f.value - 2500.0).abs() < 25.0, "got {} Hz", f.value);

        // rms of a 2 V amplitude sine is 2/sqrt(2)
        let r = m(MeasureKind::CyclicRms, &v).unwrap();
        assert!((r.value - 2.0 / 2f64.sqrt()).abs() < 0.02, "got {} V", r.value);
    }

    #[test]
    fn rise_time_on_a_ramped_edge() {
        // 100 sample linear ramp from 0 to 1, so 10% to 90% is 80 samples
        let mut v = vec![0.0f32; 500];
        for i in 0..100 {
            v[200 + i] = i as f32 / 99.0;
        }
        for x in v.iter_mut().skip(300) {
            *x = 1.0;
        }
        let r = m(MeasureKind::RiseTime, &v).unwrap();
        assert!((r.value - 80e-6).abs() < 3e-6, "got {} s", r.value);
    }

    #[test]
    fn a_flat_line_has_no_timing() {
        let v = vec![1.0f32; 3200];
        assert!(m(MeasureKind::Frequency, &v).is_none());
        assert!(m(MeasureKind::RiseTime, &v).is_none());
        assert!(m(MeasureKind::PosDuty, &v).is_none());
        // but the levels still work
        assert_eq!(m(MeasureKind::Mean, &v).unwrap().value, 1.0);
        assert_eq!(m(MeasureKind::PkPk, &v).unwrap().value, 0.0);
    }

    #[test]
    fn empty_and_off_give_nothing() {
        assert!(m(MeasureKind::Mean, &[]).is_none());
        assert!(m(MeasureKind::Off, &[1.0, 2.0]).is_none());
        // the two channel ones need a second trace, without one they give up
        // rather than panicking or inventing a number
        assert!(m(MeasureKind::Delay12Rise, &square(3200, 1000.0, 0.5, 0.0, 1.0)).is_none());
        assert!(m(MeasureKind::Lrr, &square(3200, 1000.0, 0.5, 0.0, 1.0)).is_none());
    }

    #[test]
    fn overshoot_is_measured_against_the_flat_top() {
        let mut v = square(3200, 1000.0, 0.5, 0.0, 1.0);
        v[500] = 1.2; // a 20% spike on top of a 1 V step
        let o = m(MeasureKind::Overshoot, &v).unwrap();
        assert!((o.value - 20.0).abs() < 1.0, "got {}%", o.value);
    }

    /// shift a wave right by `n` samples, filling from the left with its own
    /// first value. thats one channel arriving late
    fn delayed(v: &[f32], n: usize) -> Vec<f32> {
        let mut out = vec![v[0]; n];
        out.extend_from_slice(&v[..v.len() - n]);
        out
    }

    #[test]
    fn same_signal_on_both_channels_has_no_delay() {
        // this is the case i actually wired up : ch2 on the same reference
        // as ch1. both delays should read zero
        let a = square(3200, 1000.0, 0.5, 0.0, 1.0);
        let d = m2(MeasureKind::Delay12Rise, &a, &a).unwrap();
        assert_eq!(d.unit, Unit::Second);
        assert!(d.value.abs() < 1e-9, "got {} s", d.value);
        assert!(m2(MeasureKind::Delay12Fall, &a, &a).unwrap().value.abs() < 1e-9);
        assert!(m2(MeasureKind::Lrr, &a, &a).unwrap().value.abs() < 1e-9);
    }

    #[test]
    fn a_shifted_channel_shows_up_as_delay() {
        let a = square(3200, 1000.0, 0.5, 0.0, 1.0);
        let b = delayed(&a, 50); // 50 samples at 1 MSa/s is 50 us late
        let d = m2(MeasureKind::Delay12Rise, &a, &b).unwrap();
        assert!((d.value - 50e-6).abs() < 2e-6, "got {} s", d.value);

        // and the other way round it reads negative, which is the whole point
        // of taking the nearest edge instead of the next one
        let back = m2(MeasureKind::Delay12Rise, &b, &a).unwrap();
        assert!((back.value + 50e-6).abs() < 2e-6, "got {} s", back.value);
    }

    #[test]
    fn frf_and_ffr_are_half_a_period_apart_on_a_square_wave() {
        // rise of a to the next fall of b. on a 50% square thats half a
        // period, 500 us, and it stays 500 us whichever phase the capture
        // happens to start on
        let a = square(3200, 1000.0, 0.5, 0.0, 1.0);
        let f = m2(MeasureKind::Frf, &a, &a).unwrap();
        assert!((f.value - 500e-6).abs() < 5e-6, "got {} s", f.value);

        let g = m2(MeasureKind::Ffr, &a, &a).unwrap();
        assert!((g.value - 500e-6).abs() < 5e-6, "got {} s", g.value);

        // the capture starting high rather than low must not flip the sign
        let b = square(3200, 1000.0, 0.5, 1.0, 0.0);
        let h = m2(MeasureKind::Frf, &b, &b).unwrap();
        assert!(h.value > 0.0, "sign flipped on a different starting phase: {}", h.value);
    }

    #[test]
    fn two_channel_measurements_need_edges_on_both() {
        let good = square(3200, 1000.0, 0.5, 0.0, 1.0);
        let flat = vec![1.0f32; 3200];
        assert!(m2(MeasureKind::Delay12Rise, &good, &flat).is_none());
        assert!(m2(MeasureKind::Delay12Rise, &flat, &good).is_none());
    }

    #[test]
    fn period_mean_ignores_the_ragged_ends() {
        // 3200 samples of a 1000 sample square is 3.2 cycles, so the plain
        // mean is dragged up by the extra fifth of a high period
        let v = square(3200, 1000.0, 0.5, 0.0, 1.0);
        // 3 whole cycles plus 200 extra high samples : 1700 / 3200 = 0.53125
        let plain = m(MeasureKind::Mean, &v).unwrap().value;
        let period = m(MeasureKind::PeriodMean, &v).unwrap().value;
        assert!((plain - 0.53125).abs() < 1e-4, "plain mean should be skewed high, got {plain}");
        assert!(
            (period - 0.5).abs() < 0.02,
            "over whole cycles it should be 0.5, got {period}"
        );
        assert!(period < plain, "the whole cycle version must undo the skew");
    }

    #[test]
    fn period_rms_does_the_same() {
        let v = square(3200, 1000.0, 0.5, 0.0, 1.0);
        let period = m(MeasureKind::PeriodRms, &v).unwrap().value;
        // rms of a 0/1 square over whole cycles is sqrt(0.5)
        assert!((period - 0.5f64.sqrt()).abs() < 0.02, "got {period}");
    }

    #[test]
    fn period_measurements_fall_back_on_a_flat_line() {
        // no edges means no cycles, so it uses the whole capture rather than
        // giving up. a flat lines mean is well defined either way
        let v = vec![2.0f32; 3200];
        assert_eq!(m(MeasureKind::PeriodMean, &v).unwrap().value, 2.0);
    }

    #[test]
    fn formatting_matches_the_scope() {
        assert_eq!(Measurement { value: 1e-3, unit: Unit::Second }.format(), "1.000ms");
        assert_eq!(Measurement { value: 49.9, unit: Unit::Percent }.format(), "49.9%");
        assert_eq!(Measurement { value: 1000.0, unit: Unit::Hertz }.format(), "1.000kHz");
    }
}
