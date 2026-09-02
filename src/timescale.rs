//! Pitch-preserving time stretching for podcast episodes, by WSOLA
//! (waveform-similarity overlap-add). Music is bypassed at 1.0 and passes
//! through untouched; see [`crate::vis::Tapped`].

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

/// The speeds the player-bar button cycles through.
pub const SPEEDS: [f32; 8] = [0.5, 0.8, 1.0, 1.2, 1.5, 2.0, 3.0, 3.5];
pub const MIN_SPEED: f32 = 0.5;
pub const MAX_SPEED: f32 = 3.5;

/// The speed after `current` in [`SPEEDS`], stepping from the nearest one.
pub fn next_speed(current: f32) -> f32 {
    let mut nearest = 0;
    let mut closest = f32::INFINITY;
    for (index, speed) in SPEEDS.iter().enumerate() {
        let distance = (*speed - current).abs();
        if distance < closest {
            closest = distance;
            nearest = index;
        }
    }
    SPEEDS[(nearest + 1) % SPEEDS.len()]
}

/// A speed shared between the window and the audio thread, as `f32` bits.
pub type SharedSpeed = Arc<AtomicU32>;

pub fn shared_speed() -> SharedSpeed {
    Arc::new(AtomicU32::new(1.0f32.to_bits()))
}

pub fn load_speed(shared: &SharedSpeed) -> f32 {
    f32::from_bits(shared.load(Ordering::Relaxed))
}

pub fn store_speed(shared: &SharedSpeed, speed: f32) {
    shared.store(speed.to_bits(), Ordering::Relaxed);
}

/// Window length in frames, about 12 ms at 44.1 kHz.
const WINDOW: usize = 530;
const HOP: usize = WINDOW / 2;
/// How far either side of the nominal read point a matching waveform is
/// sought, about 20 ms: more than a pitch period of a low voice.
const SEEK: usize = 882;
/// Per-hop easing of a live speed change; it settles in about 60 ms.
const RAMP: f32 = 0.25;

/// A WSOLA time-stretcher over interleaved samples, fed packet by packet.
pub struct Stretch {
    channels: usize,
    ratio: f32,
    target: f32,
    window: Vec<f64>,
    input: Vec<f64>,
    tail: Vec<f64>,
    template: Vec<f32>,
    read: f64,
}

impl Stretch {
    pub fn new(channels: usize) -> Self {
        let window = (0..WINDOW)
            .map(|i| 0.5 - 0.5 * (std::f64::consts::TAU * i as f64 / WINDOW as f64).cos())
            .collect();
        Self {
            channels,
            ratio: 1.0,
            target: 1.0,
            window,
            input: Vec::new(),
            tail: vec![0.0; HOP * channels],
            template: Vec::new(),
            read: 0.0,
        }
    }

    /// Sets the speed to ease toward.
    pub fn set_ratio(&mut self, ratio: f32) {
        self.target = ratio.clamp(MIN_SPEED, MAX_SPEED);
    }

    /// Drops buffered audio and snaps to the target speed.
    pub fn reset(&mut self) {
        self.input.clear();
        self.tail.fill(0.0);
        self.template.clear();
        self.read = 0.0;
        self.ratio = self.target;
    }

    /// Settled at 1.0, so the signal can pass through untouched.
    pub fn bypassed(&self) -> bool {
        self.target == 1.0 && (self.ratio - 1.0).abs() < 1e-3
    }

    fn mono(&self, frame: usize, frames: usize) -> Vec<f32> {
        let ch = self.channels;
        (0..frames)
            .map(|i| {
                (0..ch)
                    .map(|c| self.input[(frame + i) * ch + c])
                    .sum::<f64>() as f32
            })
            .collect()
    }

    fn best_offset(&self, base: usize, frames: usize) -> usize {
        if self.template.is_empty() {
            return base;
        }
        let low = base.saturating_sub(SEEK);
        let high = (base + SEEK).min(frames - WINDOW);
        let mut best = base.min(high);
        let mut best_score = f32::NEG_INFINITY;
        for start in low..=high {
            let candidate = self.mono(start, HOP);
            let mut dot = 0.0f32;
            let mut energy = 0.0f32;
            for (a, b) in candidate.iter().zip(&self.template) {
                dot += a * b;
                energy += a * a;
            }
            let score = if energy > f32::EPSILON {
                dot / energy.sqrt()
            } else {
                0.0
            };
            if score > best_score {
                best_score = score;
                best = start;
            }
        }
        best
    }

    /// Stretches a chunk of interleaved frames, returning what is ready.
    pub fn process(&mut self, samples: &[f64]) -> Vec<f64> {
        self.input.extend_from_slice(samples);
        let ch = self.channels;
        let mut out = Vec::new();
        loop {
            let base = self.read.round() as usize;
            let frames = self.input.len() / ch;
            if frames < base + SEEK + WINDOW {
                break;
            }
            let start = self.best_offset(base, frames);
            for i in 0..HOP {
                let w = self.window[i];
                for c in 0..ch {
                    let sample = self.input[(start + i) * ch + c] * w;
                    out.push(self.tail[i * ch + c] + sample);
                }
            }
            for i in 0..HOP {
                let w = self.window[HOP + i];
                for c in 0..ch {
                    self.tail[i * ch + c] = self.input[(start + HOP + i) * ch + c] * w;
                }
            }
            self.template = self.mono(start + HOP, HOP);
            self.ratio += (self.target - self.ratio) * RAMP;
            if (self.target - self.ratio).abs() < 1e-3 {
                self.ratio = self.target;
            }
            self.read += HOP as f64 * f64::from(self.ratio);
        }
        let keep = (self.read.floor() as usize).saturating_sub(SEEK);
        if keep > 0 {
            let keep = keep.min(self.input.len() / ch);
            self.input.drain(..keep * ch);
            self.read -= keep as f64;
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: f32 = 44_100.0;

    fn tone(hz: f32, frames: usize) -> Vec<f64> {
        (0..frames)
            .flat_map(|i| {
                let s = 0.5 * (std::f32::consts::TAU * hz * i as f32 / RATE).sin();
                [f64::from(s), f64::from(s)]
            })
            .collect()
    }

    fn crossings_per_second(out: &[f64]) -> f32 {
        let left: Vec<f64> = out.iter().step_by(2).copied().collect();
        let crossings = left
            .windows(2)
            .filter(|w| (w[0] <= 0.0) != (w[1] <= 0.0))
            .count();
        crossings as f32 / (left.len() as f32 / RATE)
    }

    fn stretched(hz: f32, ratio: f32, seconds: f32) -> Vec<f64> {
        let mut stretch = Stretch::new(2);
        stretch.set_ratio(ratio);
        stretch.reset();
        let mut out = Vec::new();
        let input = tone(hz, (RATE * seconds) as usize);
        let mut at = 0;
        for size in [128, 1000, 517, 4096, 64].iter().cycle() {
            if at >= input.len() {
                break;
            }
            let end = (at + size * 2).min(input.len());
            out.extend(stretch.process(&input[at..end]));
            at = end;
        }
        out
    }

    #[test]
    fn faster_is_shorter_and_keeps_its_pitch() {
        let out = stretched(440.0, 1.5, 2.0);
        let frames = out.len() / 2;
        let expected = (RATE * 2.0 / 1.5) as usize;
        let slack = ((SEEK + WINDOW) as f32 / 1.5) as i64 + WINDOW as i64;
        assert!(
            (frames as i64 - expected as i64).abs() < slack,
            "{frames} frames, expected about {expected}"
        );
        let pitch = crossings_per_second(&out) / 2.0;
        assert!((pitch - 440.0).abs() < 15.0, "pitch came out {pitch:.0} Hz");
    }

    #[test]
    fn slower_is_longer_and_keeps_its_pitch() {
        let out = stretched(440.0, 0.5, 2.0);
        let frames = out.len() / 2;
        let expected = (RATE * 2.0 / 0.5) as usize;
        let slack = ((SEEK + WINDOW) as f32 / 0.5) as i64 + WINDOW as i64;
        assert!(
            (frames as i64 - expected as i64).abs() < slack,
            "{frames} frames, expected about {expected}"
        );
        let pitch = crossings_per_second(&out) / 2.0;
        assert!((pitch - 440.0).abs() < 15.0, "pitch came out {pitch:.0} Hz");
    }

    #[test]
    fn chunking_makes_no_difference() {
        let input = tone(440.0, 40_000);
        let mut whole = Stretch::new(2);
        whole.set_ratio(1.3);
        let whole = whole.process(&input);
        let mut piece = Stretch::new(2);
        piece.set_ratio(1.3);
        let mut out = Vec::new();
        let mut at = 0;
        for size in [2, 14, 200, 3256, 6, 1000, 2, 8000].iter().cycle() {
            if at >= input.len() {
                break;
            }
            let end = (at + size).min(input.len());
            out.extend(piece.process(&input[at..end]));
            at = end;
        }
        assert_eq!(out, whole);
    }

    #[test]
    fn the_speed_button_cycles_spotifys_set() {
        assert_eq!(next_speed(1.0), 1.2);
        assert_eq!(next_speed(3.5), 0.5);
        assert_eq!(next_speed(2.0), 3.0);
        assert_eq!(next_speed(1.45), 2.0);
    }

    #[test]
    fn a_reset_clears_the_buffered_audio_and_snaps_to_the_target() {
        let mut stretch = Stretch::new(2);
        stretch.set_ratio(1.5);
        stretch.process(&tone(440.0, 500));
        assert!(!stretch.input.is_empty());
        stretch.reset();
        assert!(stretch.input.is_empty());
        assert!(stretch.template.is_empty());
        assert_eq!(stretch.read, 0.0);
        assert!(stretch.tail.iter().all(|s| *s == 0.0));
        assert_eq!(stretch.ratio, 1.5);
    }

    #[test]
    fn a_live_speed_change_eases_in_instead_of_jumping() {
        let mut stretch = Stretch::new(2);
        stretch.set_ratio(2.0);
        stretch.process(&tone(200.0, 3_000));
        assert!(
            stretch.ratio > 1.0 && stretch.ratio < 2.0,
            "ratio jumped straight to {}",
            stretch.ratio
        );
        stretch.process(&tone(200.0, 40_000));
        assert_eq!(stretch.ratio, 2.0, "ratio never settled");
    }

    #[test]
    fn music_stays_bypassed_and_a_podcast_does_not() {
        let mut stretch = Stretch::new(2);
        assert!(stretch.bypassed());
        stretch.set_ratio(1.5);
        stretch.reset();
        assert!(!stretch.bypassed());
    }
}
