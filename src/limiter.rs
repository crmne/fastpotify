//! The one thing between the music and the speaker that says "no louder".
//!
//! A player has to stop somewhere. The equalizer works in floats and keeps
//! its boosts whole, so by the time sound arrives here a +12 dB preamp
//! really is four times the signal, and something has to decide what
//! reaches an output that only goes to one.
//!
//! The crude answer is to cut every sample off at the ceiling. It is one
//! line and it sounds terrible: a hard corner in a waveform is a burst of
//! odd harmonics reaching up past half the sample rate, which then folds
//! back down as tones that were never in the music. It is the harshest
//! distortion available and it lands on the loudest, most exposed part of
//! the song.
//!
//! So this is a limiter instead, the way a mastering chain ends:
//!
//! * It watches the loudest of the two channels and turns them both by the
//!   same amount, so a peak on the left never drags the image left.
//! * It moves in and out over milliseconds rather than per sample, which
//!   is what keeps the distortion out. Coming in is quick enough to catch
//!   a drum, going out slow enough not to breathe.
//! * It has a soft knee, so gain reduction arrives gradually instead of
//!   switching on. Below the knee it is exactly unity, so ordinary music
//!   passes through untouched and this costs nothing.
//! * The threshold sits a decibel under full scale. Sound is resampled
//!   downstream when the output does not run at 44.1 kHz, and a waveform
//!   sitting exactly on the ceiling comes out of a resampler slightly
//!   over it: the peaks between the samples are not bounded by the
//!   samples. A decibel of room is the usual allowance.
//!
//! * It looks ahead. The music is held back by a few milliseconds while
//!   the gain is worked out from sound that has not been heard yet, so by
//!   the time a drum hit arrives at the output the limiter is already
//!   turned down for it. Without this a limiter is always late by its own
//!   attack time, and the front edge of every transient goes through
//!   untouched, which is the thing it was put there to stop.
//!
//! The cost is eight milliseconds of delay on the whole stream, once,
//! which no one can hear and which the output's own buffering dwarfs. A
//! clamp still sits at the very end, but as a guarantee rather than a
//! mechanism: nothing in real music reaches it, because the gain is
//! already down before the sample gets there.

/// Where limiting is complete, under full scale. Room for the peaks
/// between samples, which a resampler downstream can turn into real ones.
///
/// This and [`KNEE_DB`] also decide where this limiter meets librespot's.
/// librespot has one of its own, for the gain volume normalisation adds,
/// and it runs inside its player: upstream of the equalizer, the preamp
/// and the volume, so it cannot see anything this crate does and is not
/// a substitute for this one. It holds its output to about -2 dBFS
/// (`normalisation_threshold_dbfs`), and unity here reaches exactly that
/// far: threshold minus half the knee is -2 dB. So with normalisation
/// on, a flat equalizer and full volume, what librespot hands over sits
/// precisely at the foot of this knee and passes untouched, and this
/// limiter only ever acts on gain added after librespot had its say.
///
/// That the two numbers meet is worth keeping. Moving either threshold
/// or the knee without looking at the other puts two limiters on the
/// same signal for no reason.
const THRESHOLD_DB: f64 = -1.0;
/// How far below the threshold gain reduction starts to come in. Under
/// this the limiter is exactly unity, so ordinary music never meets it.
const KNEE_DB: f64 = 2.0;
/// How quickly the limiter takes hold, and how slowly it lets go. Fast
/// enough for a drum hit, slow enough that a held note does not pump.
const ATTACK_MS: f64 = 1.5;
const RELEASE_MS: f64 = 100.0;
/// How far ahead the limiter reads. Comfortably more than the attack, so
/// the gain has all but arrived by the time the sound it was worked out
/// for reaches the output.
const LOOKAHEAD_MS: f64 = 8.0;

/// One-pole smoothing coefficient for a time constant in milliseconds:
/// the fraction of the distance left to travel each sample.
fn coefficient(ms: f64, sample_rate: f64) -> f64 {
    if ms <= 0.0 {
        return 1.0;
    }
    1.0 - (-1.0 / (ms / 1000.0 * sample_rate)).exp()
}

fn to_db(linear: f64) -> f64 {
    20.0 * linear.max(1e-12).log10()
}

fn from_db(db: f64) -> f64 {
    10f64.powf(db / 20.0)
}

/// How much to turn down, in decibels, a signal that is `over_db` above
/// the threshold. Zero below the knee, the whole excess above it, and a
/// quadratic curve joining the two so there is no corner.
fn reduction_db(over_db: f64) -> f64 {
    if over_db <= -KNEE_DB / 2.0 {
        0.0
    } else if over_db >= KNEE_DB / 2.0 {
        over_db
    } else {
        let above = over_db + KNEE_DB / 2.0;
        above * above / (2.0 * KNEE_DB)
    }
}

/// The limiter's memory between blocks: where the gain has got to, and
/// the sound being held back while it gets there.
pub struct Limiter {
    /// The gain being applied, as a factor. One is out of the way.
    gain: f64,
    attack: f64,
    release: f64,
    /// The frames not yet let out, oldest first. It starts full of
    /// silence, so every block leaves as long as it arrived and the whole
    /// stream is simply late by the length of this queue.
    held: std::collections::VecDeque<[f64; 2]>,
}

impl Limiter {
    pub fn new(sample_rate: f64) -> Self {
        let lookahead = ((LOOKAHEAD_MS / 1000.0 * sample_rate).round() as usize).max(1);
        Self {
            gain: 1.0,
            attack: coefficient(ATTACK_MS, sample_rate),
            release: coefficient(RELEASE_MS, sample_rate),
            held: std::collections::VecDeque::from(vec![[0.0; 2]; lookahead]),
        }
    }

    /// Holds `frames` of interleaved stereo within `full_scale`.
    ///
    /// `full_scale` is where the ceiling sits in the numbers being handed
    /// over, which is not always one. When the output applies the volume
    /// after this, a quarter volume means four times the signal reaches
    /// full scale, and the limiter has to know that or it would hold
    /// everything to a quarter of what the speaker could have had.
    pub fn process(&mut self, frames: &mut [f64], full_scale: f64) {
        if !(full_scale.is_finite() && full_scale > 0.0) {
            return;
        }
        let threshold_db = to_db(full_scale) + THRESHOLD_DB;
        let ceiling = full_scale;
        for frame in frames.chunks_mut(2) {
            // The gain is worked out from the sound arriving now, and
            // applied to the sound that arrived a few milliseconds ago.
            // That is the whole trick: by the time this frame is let out,
            // the gain has already come down for it.
            //
            // Both channels answer to the louder of them, so limiting
            // never moves the stereo image.
            let peak = frame
                .iter()
                .fold(0.0f64, |loudest, sample| loudest.max(sample.abs()));
            let target = from_db(-reduction_db(to_db(peak) - threshold_db));
            // Down quickly, up slowly: the ear forgives a slow recovery
            // and hears a slow catch as distortion.
            let coefficient = if target < self.gain {
                self.attack
            } else {
                self.release
            };
            self.gain += (target - self.gain) * coefficient;

            self.held
                .push_back([frame[0], frame.get(1).copied().unwrap_or(0.0)]);
            let due = self.held.pop_front().unwrap_or([0.0; 2]);
            for (sample, held) in frame.iter_mut().zip(due) {
                // The clamp is the guarantee, not the mechanism: the gain
                // is already down, so nothing in real music meets it.
                *sample = (held * self.gain).clamp(-ceiling, ceiling);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: f64 = 44_100.0;
    /// Frames the limiter holds back, so a test can skip past the delay
    /// to the sound that has actually been through it.
    const DELAY: usize = 353;

    fn tone(level: f64, frames: usize) -> Vec<f64> {
        (0..frames * 2).map(|_| level).collect()
    }

    fn peak(samples: &[f64]) -> f64 {
        samples
            .iter()
            .fold(0.0f64, |loudest, sample| loudest.max(sample.abs()))
    }

    /// Rule: ordinary music never meets the limiter. Below the knee the
    /// gain is exactly one, so nothing is coloured for nothing.
    #[test]
    fn quiet_sound_passes_through_untouched() {
        let mut limiter = Limiter::new(RATE);
        let mut samples = tone(0.5, DELAY * 2);
        limiter.process(&mut samples, 1.0);
        assert!(
            samples[DELAY * 2..]
                .iter()
                .all(|sample| (sample - 0.5).abs() < 1e-12),
            "half scale is well under the knee and must be left alone"
        );
    }

    /// Rule: nothing leaves louder than full scale, however hard it is
    /// pushed. A +12 dB preamp is four times the signal.
    #[test]
    fn a_loud_boost_is_held_to_full_scale() {
        let mut limiter = Limiter::new(RATE);
        let mut samples = tone(4.0, 44_100);
        limiter.process(&mut samples, 1.0);
        assert!(peak(&samples) <= 1.0, "nothing may leave above full scale");
    }

    /// Rule: the point of looking ahead. Silence, then full tilt with no
    /// warning: the gain must already be down when the first loud sample
    /// arrives, so the clamp never has a corner to cut.
    #[test]
    fn a_sudden_transient_is_caught_before_it_arrives() {
        let mut limiter = Limiter::new(RATE);
        let mut samples = tone(0.0, DELAY);
        samples.extend(tone(4.0, DELAY * 2));
        limiter.process(&mut samples, 1.0);
        // The gain closes on its target exponentially and never quite
        // arrives, so the first instant sits a fraction of a decibel over
        // the threshold. What matters is that it is nowhere near the
        // ceiling: the clamp is untouched, so there is no corner to hear.
        let loudest = peak(&samples);
        assert!(
            loudest < 0.95,
            "the gain was down in time and the clamp never fired: {loudest}"
        );
        assert!(
            loudest > from_db(THRESHOLD_DB),
            "and it really did have to limit: {loudest}"
        );
    }

    /// Rule: it settles at the threshold, not at the ceiling, so the peaks
    /// between samples that a resampler makes real still have room.
    #[test]
    fn it_settles_a_decibel_under_the_ceiling() {
        let mut limiter = Limiter::new(RATE);
        let mut samples = tone(4.0, 44_100);
        limiter.process(&mut samples, 1.0);
        let settled = peak(&samples[80_000..]);
        let wanted = from_db(THRESHOLD_DB);
        assert!(
            (settled - wanted).abs() < 0.01,
            "settled at {settled}, wanted about {wanted}"
        );
    }

    /// Rule: the ceiling is where the sound will be heard, not where it
    /// is now. When the output multiplies by a quarter afterwards, four
    /// times the signal is exactly right and must not be held back.
    #[test]
    fn a_volume_still_to_come_raises_the_ceiling() {
        let mut limiter = Limiter::new(RATE);
        let mut samples = tone(1.0, DELAY * 2);
        limiter.process(&mut samples, 4.0);
        assert!(
            samples[DELAY * 2..]
                .iter()
                .all(|sample| (sample - 1.0).abs() < 1e-12),
            "a quarter volume leaves room for four times the signal"
        );
    }

    /// Rule: both channels are turned by the same amount, so a peak on
    /// one side never drags the image towards the other.
    #[test]
    fn the_stereo_image_does_not_move() {
        let mut limiter = Limiter::new(RATE);
        // Left slams, right sits still.
        let mut samples: Vec<f64> = (0..DELAY * 4).flat_map(|_| [4.0, 0.25]).collect();
        limiter.process(&mut samples, 1.0);
        for frame in samples[DELAY * 2..].chunks(2) {
            let ratio = frame[0] / frame[1];
            assert!(
                (ratio - 16.0).abs() < 1e-9,
                "the two channels kept their proportion: {ratio}"
            );
        }
    }

    /// Rule: it lets go slowly. A loud passage followed by a quiet one
    /// must not snap back to unity and pump.
    #[test]
    fn it_lets_go_slowly() {
        let mut limiter = Limiter::new(RATE);
        limiter.process(&mut tone(4.0, 4410), 1.0);
        let held = limiter.gain;
        assert!(held < 0.3, "a four-times signal is well turned down");
        // A tenth of a second of quiet: on the way back, not all the way.
        limiter.process(&mut tone(0.1, 441), 1.0);
        assert!(limiter.gain > held, "it recovers");
        assert!(limiter.gain < 1.0, "but not all at once");
    }

    /// Rule: every block leaves exactly as long as it arrived, whatever
    /// is being held back, or the output would run short.
    #[test]
    fn a_block_leaves_as_long_as_it_arrived() {
        let mut limiter = Limiter::new(RATE);
        for frames in [1, 7, 512, 4410] {
            let mut samples = tone(0.3, frames);
            let before = samples.len();
            limiter.process(&mut samples, 1.0);
            assert_eq!(samples.len(), before, "a block of {frames} frames");
        }
    }
}
