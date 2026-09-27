//! Peak detector with optional inter-sample peak (ISP) estimation.
//!
//! ISP mode reconstructs the signal at 4x the sample rate with a windowed-sinc interpolator and
//! reports the largest reconstructed magnitude. Both modes have the same fixed delay
//! ([`DETECTOR_DELAY`]) so toggling ISP never changes the plugin's latency.

use std::f64::consts::PI;

const OVERSAMPLE: usize = 4;
const HALF_TAPS: usize = 8;
const TAPS: usize = HALF_TAPS * 2;

/// Delay (in samples) between feeding a sample and receiving its peak estimate.
pub const DETECTOR_DELAY: usize = HALF_TAPS;

pub struct PeakDetector {
    /// `history[k]` holds the input from `k` samples ago.
    history: [f32; TAPS],
    coeffs: [[f32; TAPS]; OVERSAMPLE],
    previous_interval_peak: f32,
    is_isp_enabled: bool,
}

impl PeakDetector {
    pub fn new() -> Self {
        Self {
            history: [0.0; TAPS],
            coeffs: interpolation_coeffs(),
            previous_interval_peak: 0.0,
            is_isp_enabled: true,
        }
    }

    pub fn set_isp_enabled(&mut self, is_enabled: bool) {
        self.is_isp_enabled = is_enabled;
    }

    pub fn reset(&mut self) {
        self.history = [0.0; TAPS];
        self.previous_interval_peak = 0.0;
    }

    /// Feeds one sample and returns the peak magnitude around the sample from
    /// [`DETECTOR_DELAY`] samples ago.
    pub fn process(&mut self, sample: f32) -> f32 {
        self.history.copy_within(0..TAPS - 1, 1);
        self.history[0] = sample;

        let centre = self.history[HALF_TAPS].abs();
        if !self.is_isp_enabled {
            return centre;
        }

        // Peak of the reconstructed interval [n - D, n - D + 1), combined with the previous
        // interval so the samples on both sides of an inter-sample peak get attenuated.
        let interval_peak = self.coeffs[1..]
            .iter()
            .map(|phase| dot(phase, &self.history).abs())
            .fold(centre, f32::max);
        let peak = interval_peak.max(self.previous_interval_peak);
        self.previous_interval_peak = interval_peak;
        peak
    }
}

impl Default for PeakDetector {
    fn default() -> Self {
        Self::new()
    }
}

fn dot(a: &[f32; TAPS], b: &[f32; TAPS]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

/// Blackman-windowed sinc coefficients. Phase `p` interpolates the point `p / OVERSAMPLE`
/// samples after the centre tap.
fn interpolation_coeffs() -> [[f32; TAPS]; OVERSAMPLE] {
    let window_half_width = HALF_TAPS as f64 + 0.5;
    let mut coeffs = [[0.0f32; TAPS]; OVERSAMPLE];
    for (phase, row) in coeffs.iter_mut().enumerate() {
        let frac = phase as f64 / OVERSAMPLE as f64;
        let raw: Vec<f64> = (0..TAPS)
            .map(|k| {
                let x = HALF_TAPS as f64 - k as f64 - frac;
                sinc(x) * blackman(x / window_half_width)
            })
            .collect();
        let sum: f64 = raw.iter().sum();
        for (c, r) in row.iter_mut().zip(&raw) {
            *c = (r / sum) as f32;
        }
    }
    coeffs
}

fn sinc(x: f64) -> f64 {
    if x.abs() < 1e-12 {
        1.0
    } else {
        (PI * x).sin() / (PI * x)
    }
}

/// Blackman window over `x` in [-1, 1].
fn blackman(x: f64) -> f64 {
    0.42 + 0.5 * (PI * x).cos() + 0.08 * (2.0 * PI * x).cos()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Sine at fs/4 with a 45 degree phase offset: every sample sits at ~0.707 while the
    /// true peak is 1.0.
    fn quarter_rate_sine(len: usize) -> Vec<f32> {
        (0..len)
            .map(|n| (PI / 2.0 * n as f64 + PI / 4.0).sin() as f32)
            .collect()
    }

    fn steady_state_peak(detector: &mut PeakDetector, input: &[f32]) -> f32 {
        let outputs: Vec<f32> = input.iter().map(|&s| detector.process(s)).collect();
        outputs[TAPS * 2..].iter().copied().fold(0.0, f32::max)
    }

    #[test]
    fn sample_peak_mode_reports_sample_magnitude() {
        let mut detector = PeakDetector::new();
        detector.set_isp_enabled(false);

        let peak = steady_state_peak(&mut detector, &quarter_rate_sine(256));

        assert!((peak - 0.7071).abs() < 1e-3, "peak was {peak}");
    }

    #[test]
    fn isp_mode_finds_inter_sample_peak() {
        let mut detector = PeakDetector::new();

        let peak = steady_state_peak(&mut detector, &quarter_rate_sine(256));

        assert!((peak - 1.0).abs() < 0.02, "peak was {peak}");
    }

    #[test]
    fn both_modes_share_the_same_delay() {
        for is_isp in [false, true] {
            let mut detector = PeakDetector::new();
            detector.set_isp_enabled(is_isp);
            let mut impulse = vec![0.0f32; 32];
            impulse[0] = 1.0;

            let outputs: Vec<f32> = impulse.iter().map(|&s| detector.process(s)).collect();
            let first_full = outputs.iter().position(|&p| p >= 0.999).unwrap();

            assert_eq!(first_full, DETECTOR_DELAY, "isp={is_isp}");
        }
    }

    #[test]
    fn phase_zero_coefficients_are_identity() {
        let coeffs = interpolation_coeffs();

        for (k, c) in coeffs[0].iter().enumerate() {
            let expected = if k == HALF_TAPS { 1.0 } else { 0.0 };
            assert!((c - expected).abs() < 1e-6, "tap {k} = {c}");
        }
    }
}
