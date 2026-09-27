//! Look-ahead brickwall limiter core.
//!
//! Signal flow per frame:
//! 1. Peak detection (sample peak or inter-sample peak), stereo-linked.
//! 2. Target gain `min(1, 1 / peak)`, i.e. limiting to 0 dBFS of the pre-gained signal.
//! 3. Instant attack, program-dependent release.
//! 4. Sliding minimum over the look-ahead window followed by a moving average of the same
//!    length. Because the audio is delayed by the look-ahead, the smoothed gain has fully reached
//!    each peak's target by the time that peak is output, so the ceiling is never exceeded.

use crate::min_queue::SlidingMin;
use crate::true_peak::{PeakDetector, DETECTOR_DELAY};

/// Look-ahead window length.
const LOOKAHEAD_SECONDS: f64 = 0.0015;
/// Release time for sparse peaks.
const RELEASE_FAST_SECONDS: f64 = 0.040;
/// Release time when gain reduction is sustained (dense material), to avoid pumping.
const RELEASE_SLOW_SECONDS: f64 = 0.250;
/// Averaging time used to decide how "sustained" the gain reduction is.
const SUSTAIN_AVERAGE_SECONDS: f64 = 0.500;
/// Average gain reduction (dB) at which the release is fully at its slow setting.
const SUSTAIN_FULL_DB: f32 = 6.0;
/// Re-sum the moving average after this many wraps to cancel floating point drift.
const RESUM_EVERY_WRAPS: u32 = 64;

pub struct Limiter {
    detectors: Vec<PeakDetector>,
    delay_lines: Vec<Vec<f32>>,
    delay_pos: usize,
    latency: usize,

    window: usize,
    sliding_min: SlidingMin,
    average_ring: Vec<f32>,
    average_pos: usize,
    average_sum: f64,
    wraps_since_resum: u32,

    release_gain: f32,
    average_reduction_db: f32,
    release_fast_coef: f32,
    release_slow_coef: f32,
    sustain_coef: f32,
}

impl Limiter {
    pub fn new(channels: usize, sample_rate: f32) -> Self {
        let sample_rate = f64::from(sample_rate.max(1.0));
        let window = ((sample_rate * LOOKAHEAD_SECONDS).round() as usize).max(1);
        let latency = DETECTOR_DELAY + window - 1;

        Self {
            detectors: (0..channels).map(|_| PeakDetector::new()).collect(),
            delay_lines: vec![vec![0.0; latency + 1]; channels],
            delay_pos: 0,
            latency,
            window,
            sliding_min: SlidingMin::new(window),
            average_ring: vec![1.0; window],
            average_pos: 0,
            average_sum: window as f64,
            wraps_since_resum: 0,
            release_gain: 1.0,
            average_reduction_db: 0.0,
            release_fast_coef: one_pole_coef(RELEASE_FAST_SECONDS, sample_rate),
            release_slow_coef: one_pole_coef(RELEASE_SLOW_SECONDS, sample_rate),
            sustain_coef: one_pole_coef(SUSTAIN_AVERAGE_SECONDS, sample_rate),
        }
    }

    /// Total processing delay in samples, to be reported to the host.
    pub fn latency_samples(&self) -> u32 {
        self.latency as u32
    }

    pub fn set_isp_enabled(&mut self, is_enabled: bool) {
        for detector in &mut self.detectors {
            detector.set_isp_enabled(is_enabled);
        }
    }

    pub fn reset(&mut self) {
        self.detectors.iter_mut().for_each(PeakDetector::reset);
        self.delay_lines.iter_mut().for_each(|line| line.fill(0.0));
        self.delay_pos = 0;
        self.sliding_min.reset();
        self.average_ring.fill(1.0);
        self.average_pos = 0;
        self.average_sum = self.window as f64;
        self.wraps_since_resum = 0;
        self.release_gain = 1.0;
        self.average_reduction_db = 0.0;
    }

    /// Processes one multichannel frame in place.
    ///
    /// `input_gain` is applied before limiting (the Threshold control), the signal is limited to
    /// 0 dBFS and then scaled by `output_gain` (the Output ceiling). Returns the applied limiter
    /// gain (1.0 = no reduction).
    pub fn process_frame(&mut self, frame: &mut [f32], input_gain: f32, output_gain: f32) -> f32 {
        let delay_len = self.latency + 1;
        let read_pos = (self.delay_pos + 1) % delay_len;

        let mut peak = 0.0f32;
        for ((sample, detector), line) in frame
            .iter_mut()
            .zip(&mut self.detectors)
            .zip(&mut self.delay_lines)
        {
            // NaN/Inf from upstream would pass straight through the delay line; mute it.
            let boosted = if sample.is_finite() { *sample * input_gain } else { 0.0 };
            peak = peak.max(detector.process(boosted));
            line[self.delay_pos] = boosted;
            *sample = line[read_pos];
        }
        self.delay_pos = read_pos;

        let gain = self.smoothed_gain(target_gain(peak));
        for sample in frame.iter_mut() {
            *sample = (*sample * gain).clamp(-1.0, 1.0) * output_gain;
        }
        gain
    }

    fn smoothed_gain(&mut self, target: f32) -> f32 {
        let released = self.apply_release(target);
        let held = self.sliding_min.push(released);
        self.moving_average(held)
    }

    fn apply_release(&mut self, target: f32) -> f32 {
        let reduction_db = -gain_to_db(self.release_gain);
        self.average_reduction_db =
            reduction_db + (self.average_reduction_db - reduction_db) * self.sustain_coef;

        if target < self.release_gain {
            self.release_gain = target;
        } else {
            let sustain = (self.average_reduction_db / SUSTAIN_FULL_DB).clamp(0.0, 1.0);
            let coef = self.release_fast_coef
                + (self.release_slow_coef - self.release_fast_coef) * sustain;
            self.release_gain = target + (self.release_gain - target) * coef;
        }
        self.release_gain
    }

    fn moving_average(&mut self, value: f32) -> f32 {
        self.average_sum += f64::from(value) - f64::from(self.average_ring[self.average_pos]);
        self.average_ring[self.average_pos] = value;
        self.average_pos += 1;
        if self.average_pos == self.window {
            self.average_pos = 0;
            self.wraps_since_resum += 1;
            if self.wraps_since_resum >= RESUM_EVERY_WRAPS {
                self.wraps_since_resum = 0;
                self.average_sum = self.average_ring.iter().map(|&v| f64::from(v)).sum();
            }
        }
        (self.average_sum / self.window as f64).min(1.0) as f32
    }
}

fn target_gain(peak: f32) -> f32 {
    if peak > 1.0 {
        1.0 / peak
    } else {
        1.0
    }
}

fn one_pole_coef(seconds: f64, sample_rate: f64) -> f32 {
    (-1.0 / (seconds * sample_rate)).exp() as f32
}

pub fn gain_to_db(gain: f32) -> f32 {
    20.0 * gain.max(1e-9).log10()
}

pub fn db_to_gain(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI;

    const SAMPLE_RATE: f32 = 48_000.0;

    /// Deterministic white noise in [-1, 1].
    fn noise(len: usize, seed: u32) -> Vec<f32> {
        let mut state = seed;
        (0..len)
            .map(|_| {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (state >> 8) as f32 / (1u32 << 23) as f32 - 1.0
            })
            .collect()
    }

    fn run_stereo(
        limiter: &mut Limiter,
        left: &[f32],
        right: &[f32],
        input_db: f32,
        output_db: f32,
    ) -> (Vec<f32>, Vec<f32>) {
        let (input_gain, output_gain) = (db_to_gain(input_db), db_to_gain(output_db));
        left.iter()
            .zip(right)
            .map(|(&l, &r)| {
                let mut frame = [l, r];
                limiter.process_frame(&mut frame, input_gain, output_gain);
                (frame[0], frame[1])
            })
            .unzip()
    }

    fn sample_peak(signal: &[f32]) -> f32 {
        signal.iter().fold(0.0, |acc, s| acc.max(s.abs()))
    }

    /// Independent, higher-quality true-peak measurement (8x, 64-tap windowed sinc).
    fn true_peak(signal: &[f32]) -> f32 {
        const HALF: i64 = 32;
        const OVER: usize = 8;
        let width = HALF as f64 + 1.0;
        let mut peak = sample_peak(signal);
        for n in HALF as usize..signal.len() - HALF as usize {
            for p in 1..OVER {
                let t = n as f64 + p as f64 / OVER as f64;
                let value: f64 = (-HALF..HALF)
                    .map(|k| {
                        let idx = n as i64 + k;
                        let x = t - idx as f64;
                        let w = 0.42
                            + 0.5 * (PI * x / width).cos()
                            + 0.08 * (2.0 * PI * x / width).cos();
                        let sinc = if x.abs() < 1e-12 { 1.0 } else { (PI * x).sin() / (PI * x) };
                        f64::from(signal[idx as usize]) * sinc * w
                    })
                    .sum();
                peak = peak.max(value.abs() as f32);
            }
        }
        peak
    }

    #[test]
    fn output_never_exceeds_ceiling_on_hot_noise() {
        let mut limiter = Limiter::new(2, SAMPLE_RATE);
        limiter.set_isp_enabled(false);
        let (left, right) = (noise(48_000, 1), noise(48_000, 2));

        for output_db in [0.0, -0.3, -6.0] {
            limiter.reset();
            let (l, r) = run_stereo(&mut limiter, &left, &right, 12.0, output_db);

            let ceiling = db_to_gain(output_db) * 1.000_01;
            assert!(sample_peak(&l) <= ceiling, "left {} > {}", sample_peak(&l), ceiling);
            assert!(sample_peak(&r) <= ceiling, "right {} > {}", sample_peak(&r), ceiling);
        }
    }

    #[test]
    fn ceiling_holds_without_relying_on_the_safety_clip() {
        // The gain alone must bring every delayed sample under 0 dBFS; the final clamp is only a
        // guard against float rounding.
        let mut limiter = Limiter::new(1, SAMPLE_RATE);
        limiter.set_isp_enabled(false);
        let input = noise(20_000, 7);
        let input_gain = db_to_gain(18.0);
        let latency = limiter.latency_samples() as usize;

        let gains: Vec<f32> = input
            .iter()
            .map(|&s| limiter.process_frame(&mut [s], input_gain, 1.0))
            .collect();

        for n in latency..input.len() {
            let unclipped = (input[n - latency] * input_gain * gains[n]).abs();
            assert!(unclipped <= 1.0 + 1e-5, "n={n} value={unclipped}");
        }
    }

    #[test]
    fn signal_below_threshold_passes_unchanged_after_latency() {
        let mut limiter = Limiter::new(1, SAMPLE_RATE);
        let input: Vec<f32> = noise(4_000, 3).iter().map(|s| s * 0.25).collect();
        let latency = limiter.latency_samples() as usize;

        let output: Vec<f32> = input
            .iter()
            .map(|&s| {
                let mut frame = [s];
                limiter.process_frame(&mut frame, 1.0, 1.0);
                frame[0]
            })
            .collect();

        for n in latency..input.len() {
            assert!((output[n] - input[n - latency]).abs() < 1e-6, "n={n}");
        }
    }

    #[test]
    fn lowering_threshold_adds_makeup_gain() {
        let mut limiter = Limiter::new(1, SAMPLE_RATE);
        let quiet = 0.1f32; // -20 dBFS, stays below the limit after +6 dB
        let input_gain = db_to_gain(6.0);

        let outputs: Vec<f32> = (0..2_000)
            .map(|_| {
                let mut frame = [quiet];
                limiter.process_frame(&mut frame, input_gain, 1.0);
                frame[0]
            })
            .collect();

        let settled = *outputs.last().unwrap();
        assert!((gain_to_db(settled) - gain_to_db(quiet) - 6.0).abs() < 0.01);
    }

    #[test]
    fn isp_detection_keeps_true_peak_under_ceiling() {
        // fs/4 sine at 45 degrees: samples at -3 dB of the true peak. With +3 dB of input gain
        // the samples just touch 0 dBFS while the true peak is +3 dBTP.
        let tone: Vec<f32> = (0..24_000)
            .map(|n| (PI / 2.0 * n as f64 + PI / 4.0).sin() as f32)
            .collect();
        let mut limiter = Limiter::new(2, SAMPLE_RATE);

        limiter.set_isp_enabled(true);
        let (with_isp, _) = run_stereo(&mut limiter, &tone, &tone, 3.0, 0.0);
        limiter.reset();
        limiter.set_isp_enabled(false);
        let (without_isp, _) = run_stereo(&mut limiter, &tone, &tone, 3.0, 0.0);

        let settle = 4_800;
        let tp_with = gain_to_db(true_peak(&with_isp[settle..]));
        let tp_without = gain_to_db(true_peak(&without_isp[settle..]));
        assert!(tp_with <= 0.1, "ISP on: true peak {tp_with:.2} dBTP");
        assert!(tp_without > 2.0, "ISP off should overshoot: {tp_without:.2} dBTP");
    }

    #[test]
    fn gain_recovers_after_a_transient() {
        let mut limiter = Limiter::new(1, SAMPLE_RATE);
        let mut signal = vec![0.1f32; 48_000];
        signal[1_000] = 4.0;

        let gains: Vec<f32> = signal
            .iter()
            .map(|&s| limiter.process_frame(&mut [s], 1.0, 1.0))
            .collect();

        let deepest = gains.iter().copied().fold(1.0, f32::min);
        assert!(gain_to_db(deepest) < -11.0, "deepest {deepest}");
        assert!(gains[48_000 - 1] > 0.999, "did not recover: {}", gains[48_000 - 1]);
    }

    #[test]
    fn non_finite_input_never_reaches_the_output() {
        for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let mut limiter = Limiter::new(2, SAMPLE_RATE);
            let mut left = vec![0.1f32; 2_000];
            left[500] = bad;
            let right = vec![0.1f32; 2_000];

            let (l, r) = run_stereo(&mut limiter, &left, &right, 0.0, 0.0);

            assert!(l.iter().chain(&r).all(|s| s.is_finite() && s.abs() <= 1.0), "input {bad}");
        }
    }

    #[test]
    fn latency_matches_lookahead_at_common_rates() {
        for (rate, expected_window) in [(44_100.0, 66), (48_000.0, 72), (96_000.0, 144)] {
            let limiter = Limiter::new(2, rate);
            assert_eq!(
                limiter.latency_samples() as usize,
                DETECTOR_DELAY + expected_window - 1
            );
        }
    }
}
