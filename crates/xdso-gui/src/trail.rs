//! fading old frames, so a slow feed doesnt flicker.
//!
//! the scope only manages a couple of frames a second when its busy, and an
//! fft is about as busy as it gets. at that rate a trace that only ever shows
//! the newest frame jumps about and is genuinely hard to read. keeping the
//! last few and fading them out gives you the same thing a scopes own
//! persistence does : the shape stays put and the noise smears.
//!
//! the scope has its own DISPLAY-PERSIST setting with the same idea, but it
//! applies that to *its* screen, not to what it sends us, so this is ours.

use std::collections::VecDeque;
use std::sync::Arc;

/// the frames were still showing for one trace
#[derive(Default)]
pub struct Trail {
    frames: VecDeque<(f64, Arc<Vec<u8>>)>,
}

impl Trail {
    /// remember this frame, if its actually a new one.
    ///
    /// the poller hands out the same `Arc` until it reads a fresh trace, so
    /// pointer equality is all it takes to tell a new frame from the ui
    /// simply redrawing at 60 fps
    pub fn push(&mut self, now: f64, frame: Option<&Arc<Vec<u8>>>, keep: f64) {
        if let Some(f) = frame {
            let repeat = self.frames.back().is_some_and(|(_, prev)| Arc::ptr_eq(prev, f));
            if !repeat {
                self.frames.push_back((now, Arc::clone(f)));
            }
        }
        while self.frames.front().is_some_and(|(t, _)| now - t > keep) {
            self.frames.pop_front();
        }
        // a hard cap as well, in case someone winds the time right up on a
        // fast feed and we end up hoarding hundreds of them
        while self.frames.len() > MAX_FRAMES {
            self.frames.pop_front();
        }
    }

    /// oldest first, each with how faded it should be. 1.0 is the newest
    pub fn iter(&self, now: f64, keep: f64) -> impl Iterator<Item = (&[u8], f32)> {
        let newest = self.frames.back().map(|(t, _)| *t).unwrap_or(now);
        self.frames.iter().map(move |(t, f)| {
            let alpha = if *t >= newest || keep <= 0.0 {
                1.0
            } else {
                // linear is fine and stays visible longer than a curve does
                (1.0 - ((newest - t) / keep) as f32).clamp(MIN_ALPHA, 1.0)
            };
            (f.as_slice(), alpha)
        })
    }

    pub fn clear(&mut self) {
        self.frames.clear();
    }
}

/// dont let the trail grow without bound on a fast feed
const MAX_FRAMES: usize = 40;
/// the faintest a frame gets before it drops off entirely. below this its
/// invisible anyway and just costs a draw
const MIN_ALPHA: f32 = 0.06;

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(v: u8) -> Arc<Vec<u8>> {
        Arc::new(vec![v; 4])
    }

    #[test]
    fn the_same_frame_twice_only_counts_once() {
        let mut t = Trail::default();
        let f = frame(1);
        t.push(0.0, Some(&f), 1.0);
        t.push(0.1, Some(&f), 1.0);
        t.push(0.2, Some(&f), 1.0);
        assert_eq!(t.frames.len(), 1, "redrawing is not a new frame");
    }

    #[test]
    fn old_frames_fall_off_the_back() {
        let mut t = Trail::default();
        for i in 0..5 {
            t.push(i as f64 * 0.5, Some(&frame(i)), 1.0);
        }
        // keeping 1 second at half a second a frame leaves about three
        assert!(t.frames.len() <= 3, "got {}", t.frames.len());
    }

    #[test]
    fn the_newest_is_never_faded() {
        let mut t = Trail::default();
        t.push(0.0, Some(&frame(1)), 10.0);
        t.push(1.0, Some(&frame(2)), 10.0);
        let all: Vec<f32> = t.iter(1.0, 10.0).map(|(_, a)| a).collect();
        assert_eq!(*all.last().unwrap(), 1.0);
        assert!(all[0] < 1.0, "the older one should be faded");
    }

    #[test]
    fn no_persistence_means_everything_is_solid() {
        let mut t = Trail::default();
        t.push(0.0, Some(&frame(1)), 0.0);
        assert!(t.iter(0.0, 0.0).all(|(_, a)| a == 1.0));
    }
}
