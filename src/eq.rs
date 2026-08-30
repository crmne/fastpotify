//! The equalizer: Winamp's ten bands, done to the sound on its way out.
//!
//! librespot has no equalizer, so this is Fastpotify's own: a peaking
//! filter per band (the textbook second-order kind) run over every sample
//! of local playback, and a preamp that only ever turns down, since the
//! app does not boost past 0 dB. The settings live behind a mutex the
//! window writes and the player's thread reads once per packet; the
//! filters are rebuilt only when something changed.

use std::sync::{Arc, Mutex};

use librespot_playback::{NUM_CHANNELS, SAMPLE_RATE};

/// The centre frequencies, Winamp's, in hertz.
pub const BANDS: [f32; 10] = [
    60.0, 170.0, 310.0, 600.0, 1000.0, 3000.0, 6000.0, 12000.0, 14000.0, 16000.0,
];
/// How far a band goes either way, in decibels.
pub const RANGE_DB: f32 = 12.0;
/// The width of each band: about an octave, so neighbours overlap a
/// little and a boost across several reads as one shape.
const Q: f32 = 1.4;
/// Bands closer to flat than this are skipped rather than run for nothing.
const FLAT: f32 = 0.05;

/// What the listener set: the switch, the preamp, and the bands, and
/// the two things Winamp's main window did to the sound as well, the
/// balance and (a lamp there, a switch here) mono. Those two apply
/// whether the equalizer is on or not.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EqSettings {
    pub on: bool,
    /// Never above zero.
    pub preamp_db: f32,
    pub bands_db: [f32; 10],
    /// -1 is all left, 1 all right.
    pub balance: f32,
    pub mono: bool,
}

impl Default for EqSettings {
    fn default() -> Self {
        Self {
            on: false,
            preamp_db: 0.0,
            bands_db: [0.0; 10],
            balance: 0.0,
            mono: false,
        }
    }
}

impl EqSettings {
    /// The same settings kept within what the equalizer can do.
    pub fn clamped(mut self) -> Self {
        self.preamp_db = self.preamp_db.clamp(-RANGE_DB, 0.0);
        for band in &mut self.bands_db {
            *band = band.clamp(-RANGE_DB, RANGE_DB);
        }
        self.balance = self.balance.clamp(-1.0, 1.0);
        self
    }

    /// The gain of each channel from the balance: the side turned away
    /// from loses, the other keeps its level, as Winamp's did.
    pub fn channel_gains(&self) -> [f64; 2] {
        let balance = f64::from(self.balance);
        [(1.0 - balance).min(1.0), (1.0 + balance).min(1.0)]
    }

    /// The response at a frequency, in decibels, with the switch on: the
    /// preamp and every band's analog prototype added up. Drawn, not
    /// played; the played response is the digital filters' and bends a
    /// little from this near the top of the band.
    pub fn response_db(&self, hz: f32) -> f32 {
        let mut db = self.preamp_db;
        for (band, gain_db) in BANDS.iter().zip(self.bands_db) {
            db += peaking_db(hz, *band, Q, gain_db);
        }
        db
    }
}

/// A named set of band gains, as Winamp shipped them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Preset {
    pub name: &'static str,
    pub bands_db: [f32; 10],
}

impl Preset {
    pub fn label_key(self) -> &'static str {
        match self.name {
            "Flat" => "eq.preset.flat",
            "Classical" => "eq.preset.classical",
            "Club" => "eq.preset.club",
            "Dance" => "eq.preset.dance",
            "Full Bass" => "eq.preset.full_bass",
            "Full Bass & Treble" => "eq.preset.full_bass_treble",
            "Full Treble" => "eq.preset.full_treble",
            "Laptop Speakers / Headphones" => "eq.preset.laptop",
            "Large Hall" => "eq.preset.large_hall",
            "Live" => "eq.preset.live",
            "Party" => "eq.preset.party",
            "Pop" => "eq.preset.pop",
            "Reggae" => "eq.preset.reggae",
            "Rock" => "eq.preset.rock",
            "Ska" => "eq.preset.ska",
            "Soft" => "eq.preset.soft",
            "Soft Rock" => "eq.preset.soft_rock",
            "Techno" => "eq.preset.techno",
            _ => "eq.preset.flat",
        }
    }
}

/// Winamp's presets, in its order.
pub const PRESETS: &[Preset] = &[
    Preset {
        name: "Flat",
        bands_db: [0.0; 10],
    },
    Preset {
        name: "Classical",
        bands_db: [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, -7.2, -7.2, -7.2, -9.6],
    },
    Preset {
        name: "Club",
        bands_db: [0.0, 0.0, 8.0, 5.6, 5.6, 5.6, 3.2, 0.0, 0.0, 0.0],
    },
    Preset {
        name: "Dance",
        bands_db: [9.6, 7.2, 2.4, 0.0, 0.0, -5.6, -7.2, -7.2, 0.0, 0.0],
    },
    Preset {
        name: "Full Bass",
        bands_db: [-8.0, 9.6, 9.6, 5.6, 1.6, -4.0, -8.0, -10.4, -11.2, -11.2],
    },
    Preset {
        name: "Full Bass & Treble",
        bands_db: [7.2, 5.6, 0.0, -7.2, -4.8, 1.6, 8.0, 11.2, 12.0, 12.0],
    },
    Preset {
        name: "Full Treble",
        bands_db: [-9.6, -9.6, -9.6, -4.0, 2.4, 11.2, 12.0, 12.0, 12.0, 12.0],
    },
    Preset {
        name: "Laptop Speakers / Headphones",
        bands_db: [4.8, 11.2, 5.6, -3.2, -2.4, 1.6, 4.8, 9.6, 12.0, 12.0],
    },
    Preset {
        name: "Large Hall",
        bands_db: [10.4, 10.4, 5.6, 5.6, 0.0, -4.8, -4.8, -4.8, 0.0, 0.0],
    },
    Preset {
        name: "Live",
        bands_db: [-4.8, 0.0, 4.0, 5.6, 5.6, 5.6, 4.0, 2.4, 2.4, 2.4],
    },
    Preset {
        name: "Party",
        bands_db: [7.2, 7.2, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 7.2, 7.2],
    },
    Preset {
        name: "Pop",
        bands_db: [-1.6, 4.8, 7.2, 8.0, 5.6, 0.0, -2.4, -2.4, -1.6, -1.6],
    },
    Preset {
        name: "Reggae",
        bands_db: [0.0, 0.0, 0.0, -5.6, 0.0, 6.4, 6.4, 0.0, 0.0, 0.0],
    },
    Preset {
        name: "Rock",
        bands_db: [8.0, 4.8, -5.6, -8.0, -3.2, 4.0, 8.8, 11.2, 11.2, 11.2],
    },
    Preset {
        name: "Ska",
        bands_db: [-2.4, -4.8, -4.0, 0.0, 4.0, 5.6, 8.8, 9.6, 11.2, 9.6],
    },
    Preset {
        name: "Soft",
        bands_db: [4.8, 1.6, 0.0, -2.4, 0.0, 4.0, 8.0, 9.6, 11.2, 12.0],
    },
    Preset {
        name: "Soft Rock",
        bands_db: [4.0, 4.0, 2.4, 0.0, -4.0, -5.6, -3.2, 0.0, 2.4, 8.8],
    },
    Preset {
        name: "Techno",
        bands_db: [8.0, 5.6, 0.0, -5.6, -4.8, 0.0, 8.0, 9.6, 9.6, 8.8],
    },
];

/// The settings as the window and the player's thread share them.
pub type SharedEq = Arc<Mutex<EqSettings>>;

pub fn shared() -> SharedEq {
    Arc::new(Mutex::new(EqSettings::default()))
}

/// A second-order section in direct form I, one per band and channel.
#[derive(Clone, Copy, Debug, Default)]
struct Biquad {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
    x1: f64,
    x2: f64,
    y1: f64,
    y2: f64,
}

impl Biquad {
    /// A peaking filter after the Audio EQ Cookbook, normalised by `a0`.
    fn peaking(hz: f32, q: f32, gain_db: f32) -> Self {
        let a = 10f64.powf(f64::from(gain_db) / 40.0);
        let w0 = std::f64::consts::TAU * f64::from(hz) / f64::from(SAMPLE_RATE);
        let (sin, cos) = w0.sin_cos();
        let alpha = sin / (2.0 * f64::from(q));
        let a0 = 1.0 + alpha / a;
        Self {
            b0: (1.0 + alpha * a) / a0,
            b1: (-2.0 * cos) / a0,
            b2: (1.0 - alpha * a) / a0,
            a1: (-2.0 * cos) / a0,
            a2: (1.0 - alpha / a) / a0,
            ..Self::default()
        }
    }

    #[inline]
    fn run(&mut self, x: f64) -> f64 {
        let y = self.b0 * x + self.b1 * self.x1 + self.b2 * self.x2
            - self.a1 * self.y1
            - self.a2 * self.y2;
        self.x2 = self.x1;
        self.x1 = x;
        self.y2 = self.y1;
        self.y1 = y;
        y
    }
}

/// The filters on the player's thread, rebuilt when the settings change.
pub struct Processor {
    shared: SharedEq,
    applied: EqSettings,
    /// One chain per channel; a band at flat is left out of the chain.
    chains: Vec<Vec<Biquad>>,
    gain: f64,
}

impl Processor {
    pub fn new(shared: SharedEq) -> Self {
        let mut processor = Self {
            shared,
            applied: EqSettings::default(),
            chains: vec![Vec::new(); NUM_CHANNELS as usize],
            gain: 1.0,
        };
        processor.rebuild();
        processor
    }

    fn rebuild(&mut self) {
        let settings = self.applied;
        self.gain = 10f64.powf(f64::from(settings.preamp_db) / 20.0);
        let chain: Vec<Biquad> = BANDS
            .iter()
            .zip(settings.bands_db)
            .filter(|(_, gain_db)| gain_db.abs() > FLAT)
            .map(|(hz, gain_db)| Biquad::peaking(*hz, Q, gain_db))
            .collect();
        self.chains = vec![chain; NUM_CHANNELS as usize];
    }

    /// Runs interleaved stereo samples through the equalizer, in place.
    pub fn process(&mut self, samples: &mut [f64]) {
        let wanted = self
            .shared
            .lock()
            .map(|settings| settings.clamped())
            .unwrap_or(self.applied);
        if wanted != self.applied {
            self.applied = wanted;
            self.rebuild();
        }
        let shaping = self.applied.on && !(self.chains[0].is_empty() && self.gain == 1.0);
        let gains = self.applied.channel_gains();
        let placing = self.applied.mono || gains != [1.0, 1.0];
        if !shaping && !placing {
            return;
        }
        let channels = self.chains.len();
        for frame in samples.chunks_exact_mut(channels) {
            if shaping {
                for (sample, chain) in frame.iter_mut().zip(self.chains.iter_mut()) {
                    let mut y = *sample * self.gain;
                    for filter in chain.iter_mut() {
                        y = filter.run(y);
                    }
                    *sample = y;
                }
            }
            if self.applied.mono && frame.len() == 2 {
                let middle = (frame[0] + frame[1]) / 2.0;
                frame[0] = middle;
                frame[1] = middle;
            }
            for (sample, gain) in frame.iter_mut().zip(gains) {
                *sample = (*sample * gain).clamp(-1.0, 1.0);
            }
        }
    }
}

/// A peaking band's gain at one frequency, from the analog prototype.
fn peaking_db(hz: f32, centre: f32, q: f32, gain_db: f32) -> f32 {
    if gain_db.abs() <= FLAT {
        return 0.0;
    }
    let a = 10f32.powf(gain_db / 40.0);
    let x = hz / centre;
    let common = (1.0 - x * x).powi(2);
    let numerator = common + (a * x / q).powi(2);
    let denominator = common + (x / (a * q)).powi(2);
    10.0 * (numerator / denominator).log10()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(hz: f32, frames: usize) -> Vec<f64> {
        (0..frames)
            .flat_map(|i| {
                let t = i as f64 / f64::from(SAMPLE_RATE);
                let sample = 0.25 * (std::f64::consts::TAU * f64::from(hz) * t).sin();
                [sample, sample]
            })
            .collect()
    }

    fn rms(samples: &[f64]) -> f64 {
        (samples.iter().map(|s| s * s).sum::<f64>() / samples.len() as f64).sqrt()
    }

    #[test]
    fn a_boosted_band_makes_its_tone_louder_and_leaves_others_alone() {
        let shared = shared();
        shared.lock().unwrap().on = true;
        shared.lock().unwrap().bands_db[4] = 12.0; // 1 kHz
        let mut processor = Processor::new(shared);
        let mut at_band = tone(1000.0, 8192);
        let before = rms(&at_band[4096..]);
        processor.process(&mut at_band);
        let after = rms(&at_band[4096..]);
        let gain_db = 20.0 * (after / before).log10();
        assert!((gain_db - 12.0).abs() < 1.0, "1 kHz gained {gain_db:.1} dB");

        let mut far = tone(60.0, 8192);
        let before = rms(&far[4096..]);
        processor.process(&mut far);
        let after = rms(&far[4096..]);
        let gain_db = 20.0 * (after / before).log10();
        assert!(gain_db.abs() < 1.0, "60 Hz moved {gain_db:.1} dB");
    }

    #[test]
    fn off_or_flat_changes_nothing_and_the_preamp_only_cuts() {
        let shared = shared();
        let mut processor = Processor::new(shared.clone());
        let original = tone(440.0, 1024);
        let mut samples = original.clone();
        processor.process(&mut samples);
        assert_eq!(samples, original);

        shared.lock().unwrap().on = true;
        let mut samples = original.clone();
        processor.process(&mut samples);
        assert_eq!(samples, original);

        shared.lock().unwrap().preamp_db = 6.0;
        let mut samples = original.clone();
        processor.process(&mut samples);
        assert_eq!(samples, original, "a preamp above zero was applied");

        shared.lock().unwrap().preamp_db = -6.0;
        let mut samples = original.clone();
        processor.process(&mut samples);
        let ratio = rms(&samples) / rms(&original);
        assert!((20.0 * ratio.log10() + 6.0).abs() < 0.1);
    }

    #[test]
    fn the_drawn_response_peaks_at_the_band_and_adds_the_preamp() {
        let mut settings = EqSettings::default();
        settings.bands_db[7] = 6.0; // 12 kHz
        settings.preamp_db = -3.0;
        assert!((settings.response_db(12_000.0) - 3.0).abs() < 0.1);
        assert!((settings.response_db(100.0) + 3.0).abs() < 0.2);
        let clamped = EqSettings {
            preamp_db: 4.0,
            bands_db: [20.0; 10],
            ..settings
        }
        .clamped();
        assert_eq!(clamped.preamp_db, 0.0);
        assert!(clamped.bands_db.iter().all(|band| *band == RANGE_DB));
    }

    #[test]
    fn balance_turns_one_side_down_and_mono_makes_the_sides_the_same() {
        let shared = shared();
        let mut processor = Processor::new(shared.clone());
        let mut samples = vec![0.5, -0.25, 0.5, -0.25];
        shared.lock().unwrap().balance = 0.5;
        processor.process(&mut samples);
        assert_eq!(samples, vec![0.25, -0.25, 0.25, -0.25]);

        shared.lock().unwrap().balance = 0.0;
        shared.lock().unwrap().mono = true;
        let mut samples = vec![0.5, -0.25];
        processor.process(&mut samples);
        assert_eq!(samples, vec![0.125, 0.125]);
        assert_eq!(
            EqSettings {
                balance: 3.0,
                ..EqSettings::default()
            }
            .clamped()
            .balance,
            1.0
        );
    }

    #[test]
    fn every_preset_stays_within_the_range() {
        assert_eq!(PRESETS[0].name, "Flat");
        for preset in PRESETS {
            assert!(
                preset.bands_db.iter().all(|band| band.abs() <= RANGE_DB),
                "{} leaves the range",
                preset.name
            );
        }
    }
}
