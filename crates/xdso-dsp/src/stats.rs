//! the boring maths bits. histograms, edge finding, medians.

/// the flat top and flat bottom of a waveform, ignoring overshoot and ringing.
///
/// taking max and min would be wrong the moment theres any ringing on an edge,
/// so instead we split at the midpoint and histogram each half. the busiest
/// bin is where the trace spends its time, which is the flat bit. thats the
/// same trick the scope uses and its why Vamp and Pk-Pk can disagree.
pub fn top_base(v: &[f32]) -> (f64, f64) {
    let lo = v.iter().copied().fold(f32::INFINITY, f32::min) as f64;
    let hi = v.iter().copied().fold(f32::NEG_INFINITY, f32::max) as f64;
    // an empty slice leaves lo at +inf and hi at -inf, so check the span is a
    // real number before trusting it
    let span = hi - lo;
    if !span.is_finite() || span <= 1e-12 {
        return (hi, lo); // flat, or empty
    }
    let mid = (hi + lo) / 2.0;

    let upper: Vec<f32> = v.iter().copied().filter(|&x| x as f64 >= mid).collect();
    let lower: Vec<f32> = v.iter().copied().filter(|&x| (x as f64) < mid).collect();
    (mode_of(&upper).unwrap_or(hi), mode_of(&lower).unwrap_or(lo))
}

/// centre of the busiest of 32 bins. None if theres not enough to bin
fn mode_of(part: &[f32]) -> Option<f64> {
    const BINS: usize = 32;
    if part.len() < 4 {
        return None;
    }
    let lo = part.iter().copied().fold(f32::INFINITY, f32::min) as f64;
    let hi = part.iter().copied().fold(f32::NEG_INFINITY, f32::max) as f64;
    let width = hi - lo;
    if width <= 0.0 {
        return Some(lo);
    }
    let mut counts = [0u32; BINS];
    for &x in part {
        let i = ((x as f64 - lo) / width * BINS as f64) as usize;
        counts[i.min(BINS - 1)] += 1;
    }
    let best = counts.iter().enumerate().max_by_key(|(_, &c)| c)?.0;
    // centre of that bin
    Some(lo + width * (best as f64 + 0.5) / BINS as f64)
}

/// where the trace crosses `level`, in fractional sample positions.
///
/// the fractional part matters more than youd think : at 3200 samples a whole
/// sample of error on each edge is most of a percent on a frequency reading,
/// so we linearly interpolate between the two samples either side.
pub fn crossings(v: &[f32], level: f64, rising: bool) -> Vec<f64> {
    let mut out = Vec::new();
    if v.len() < 2 {
        return out;
    }
    let above = |x: f32| x as f64 >= level;
    for i in 0..v.len() - 1 {
        let (a, b) = (v[i], v[i + 1]);
        let crossed = if rising { !above(a) && above(b) } else { above(a) && !above(b) };
        if !crossed {
            continue;
        }
        let (a, b) = (a as f64, b as f64);
        let frac = if b != a { (level - a) / (b - a) } else { 0.0 };
        out.push(i as f64 + frac.clamp(0.0, 1.0));
    }
    out
}

/// median, because one glitchy edge shouldnt move the answer. sorts in place
pub fn median(xs: &mut [f64]) -> Option<f64> {
    if xs.is_empty() {
        return None;
    }
    xs.sort_by(f64::total_cmp);
    let n = xs.len();
    Some(if n % 2 == 1 { xs[n / 2] } else { (xs[n / 2 - 1] + xs[n / 2]) / 2.0 })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn top_base_ignores_ringing() {
        let mut v = vec![0.0f32; 200];
        for x in v.iter_mut().skip(100) {
            *x = 1.0;
        }
        v[100] = 1.4; // overshoot
        v[99] = -0.4; // preshoot
        let (top, base) = top_base(&v);
        assert!((top - 1.0).abs() < 0.05, "top {top}");
        assert!(base.abs() < 0.05, "base {base}");
    }

    #[test]
    fn flat_input_doesnt_explode() {
        assert_eq!(top_base(&[2.0, 2.0, 2.0]), (2.0, 2.0));
        assert_eq!(top_base(&[]), (f64::NEG_INFINITY, f64::INFINITY));
    }

    #[test]
    fn crossings_interpolate() {
        // straight line 0..4, crossing 1.5 should land exactly between 1 and 2
        let v = [0.0f32, 1.0, 2.0, 3.0, 4.0];
        assert_eq!(crossings(&v, 1.5, true), vec![1.5]);
        assert!(crossings(&v, 1.5, false).is_empty());

        let down = [4.0f32, 3.0, 2.0, 1.0, 0.0];
        assert_eq!(crossings(&down, 2.5, false), vec![1.5]);
    }

    #[test]
    fn crossings_on_junk() {
        assert!(crossings(&[], 0.0, true).is_empty());
        assert!(crossings(&[1.0], 0.0, true).is_empty());
        // a step with no in between sample still counts, frac clamps
        assert_eq!(crossings(&[0.0, 1.0], 0.5, true), vec![0.5]);
    }

    #[test]
    fn medians() {
        assert_eq!(median(&mut [3.0, 1.0, 2.0]), Some(2.0));
        assert_eq!(median(&mut [4.0, 1.0, 3.0, 2.0]), Some(2.5));
        assert_eq!(median(&mut []), None);
    }
}
