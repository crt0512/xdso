//! the 1-2-5 tables the settings fields index into, and number formatting that
//! looks like the scopes own.

/// volts per division, indexed by `VERT-CHn-VB`
pub const VOLTS_DIV: [f64; 13] = [
    1e-3, 2e-3, 5e-3, 10e-3, 20e-3, 50e-3, 100e-3, 200e-3, 500e-3, 1.0, 2.0, 5.0, 10.0,
];

/// probe attenuation, indexed by `VERT-CHn-PROBE`
pub const PROBE_MULT: [u32; 4] = [1, 10, 100, 1000];

/// seconds per division, indexed by `HORIZ-TB`
pub const TIME_DIV: [f64; 32] = [
    2e-9, 5e-9, 10e-9, 20e-9, 50e-9, 100e-9, 200e-9, 500e-9, 1e-6, 2e-6, 5e-6, 10e-6, 20e-6,
    50e-6, 100e-6, 200e-6, 500e-6, 1e-3, 2e-3, 5e-3, 10e-3, 20e-3, 50e-3, 100e-3, 200e-3, 500e-3,
    1.0, 2.0, 5.0, 10.0, 20.0, 50.0,
];

/// volts per division for a channel, probe attenuation already folded in
pub fn volts_per_div(vb: u64, probe: u64) -> Option<f64> {
    let v = *VOLTS_DIV.get(usize::try_from(vb).ok()?)?;
    let p = *PROBE_MULT.get(usize::try_from(probe).ok()?)?;
    Some(v * p as f64)
}

/// seconds per division
pub fn seconds_per_div(tb: u64) -> Option<f64> {
    TIME_DIV.get(usize::try_from(tb).ok()?).copied()
}

const PREFIXES: [(f64, &str); 8] = [
    (1e9, "G"),
    (1e6, "M"),
    (1e3, "k"),
    (1.0, ""),
    (1e-3, "m"),
    (1e-6, "u"),
    (1e-9, "n"),
    (1e-12, "p"),
];

/// format a number the way the scope writes it : `200us`, `1.00V`, `1.000ms`.
///
/// `digits` is significant figures, and the rounding happens *before* we pick
/// the prefix. that ordering matters : a period of 999.96us rounded to 4 sig
/// figs is 1000.0us, and if you pick the prefix first youd print "1000us"
/// instead of stepping up to "1.000ms" like the scope does.
pub fn eng(value: f64, unit: &str, digits: usize) -> String {
    if !value.is_finite() {
        return "--".into();
    }
    if value == 0.0 {
        return format!("0{unit}");
    }

    let exp = value.abs().log10().floor() as i32;
    let factor = 10f64.powi(exp - digits as i32 + 1);
    let value = (value / factor).round() * factor;
    let a = value.abs();

    let (scale, prefix) = PREFIXES
        .iter()
        .find(|(s, _)| a >= *s)
        .copied()
        .unwrap_or((1e-12, "p"));

    // after scaling were somewhere in 1..1000, so this is always plain fixed
    // point and never flips to exponent notation
    let scaled = value / scale;
    let exp2 = scaled.abs().log10().floor() as i32;
    let decimals = (digits as i32 - 1 - exp2).max(0) as usize;
    format!("{scaled:.decimals$}{prefix}{unit}")
}

/// same but for a measurement that might not exist
pub fn eng_opt(value: Option<f64>, unit: &str, digits: usize) -> String {
    value.map_or_else(|| "--".into(), |v| eng(v, unit, digits))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_like_the_scope_does() {
        assert_eq!(eng(200e-6, "s", 3), "200us");
        assert_eq!(eng(1.0, "V", 3), "1.00V");
        assert_eq!(eng(1e-3, "s", 4), "1.000ms");
        assert_eq!(eng(0.0, "V", 4), "0V");
        assert_eq!(eng(2e-9, "s", 3), "2.00ns");
        assert_eq!(eng(50.0, "s", 3), "50.0s");
    }

    #[test]
    fn rounds_before_picking_the_prefix() {
        // the whole reason eng() rounds first. 999.96us must not print as
        // 1000us, it steps up a prefix
        assert_eq!(eng(999.96e-6, "s", 4), "1.000ms");
        assert_eq!(eng(999.4e-6, "s", 4), "999.4us");
    }

    #[test]
    fn handles_negatives_and_junk() {
        assert_eq!(eng(-1.5, "V", 3), "-1.50V");
        assert_eq!(eng(f64::NAN, "V", 4), "--");
        assert_eq!(eng(f64::INFINITY, "V", 4), "--");
        assert_eq!(eng_opt(None, "V", 4), "--");
    }

    #[test]
    fn big_and_small_ends() {
        assert_eq!(eng(1.234e9, "Hz", 4), "1.234GHz");
        assert_eq!(eng(1e-12, "s", 3), "1.00ps");
        // below the smallest prefix it just keeps counting in pico
        assert_eq!(eng(1e-13, "s", 3), "0.100ps");
    }

    #[test]
    fn table_lookups_reject_nonsense() {
        assert_eq!(volts_per_div(9, 0), Some(1.0));
        assert_eq!(volts_per_div(9, 1), Some(10.0)); // 1V/div through a 10x probe
        assert_eq!(volts_per_div(99, 0), None);
        assert_eq!(seconds_per_div(0), Some(2e-9));
        assert_eq!(seconds_per_div(31), Some(50.0));
        assert_eq!(seconds_per_div(32), None);
    }
}
