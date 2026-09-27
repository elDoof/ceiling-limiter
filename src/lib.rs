//! Ceiling Limiter: a transparent look-ahead brickwall limiter.
//!
//! Controls:
//! - Threshold: drives the signal into the limiter (lower = louder, more limiting).
//! - Output: the output ceiling.
//! - Fader Link (GUI): moving Threshold moves Output by the same amount, for level-matched
//!   comparison.
//! - ISP Detection: limit inter-sample (true) peaks instead of sample peaks.

use atomic_float::AtomicF32;
use nih_plug::prelude::*;
use nih_plug_egui::EguiState;
use std::sync::atomic::Ordering;
use std::sync::Arc;

mod editor;
pub mod limiter;
mod min_queue;
mod true_peak;
mod widgets;

use limiter::Limiter;

pub const FADER_MIN_DB: f32 = -30.0;
pub const FADER_MAX_DB: f32 = 0.0;
const PARAM_SMOOTHING_MS: f32 = 20.0;
const MAX_CHANNELS: usize = 2;

pub struct CeilingLimiter {
    params: Arc<CeilingParams>,
    limiter: Limiter,
    /// Deepest gain reduction (dB, positive) since the GUI last consumed it.
    gain_reduction_db: Arc<AtomicF32>,
}

#[derive(Params)]
pub struct CeilingParams {
    #[persist = "editor-state"]
    editor_state: Arc<EguiState>,

    #[id = "threshold"]
    pub threshold: FloatParam,

    #[id = "output"]
    pub output: FloatParam,

    #[id = "link"]
    pub fader_link: BoolParam,

    #[id = "isp"]
    pub isp_detection: BoolParam,
}

fn fader_param(name: &str) -> FloatParam {
    FloatParam::new(
        name,
        0.0,
        FloatRange::Linear {
            min: FADER_MIN_DB,
            max: FADER_MAX_DB,
        },
    )
    .with_step_size(0.01)
    .with_unit(" dB")
    .with_value_to_string(formatters::v2s_f32_rounded(2))
    .with_smoother(SmoothingStyle::Linear(PARAM_SMOOTHING_MS))
}

impl Default for CeilingParams {
    fn default() -> Self {
        Self {
            editor_state: editor::default_state(),
            threshold: fader_param("Threshold"),
            output: fader_param("Output"),
            fader_link: BoolParam::new("Fader Link", false).non_automatable(),
            isp_detection: BoolParam::new("ISP Detection", true),
        }
    }
}

impl Default for CeilingLimiter {
    fn default() -> Self {
        Self {
            params: Arc::new(CeilingParams::default()),
            limiter: Limiter::new(MAX_CHANNELS, 48_000.0),
            gain_reduction_db: Arc::new(AtomicF32::new(0.0)),
        }
    }
}

impl Plugin for CeilingLimiter {
    const NAME: &'static str = "Ceiling Limiter";
    const VENDOR: &'static str = "Ceiling";
    const URL: &'static str = "";
    const EMAIL: &'static str = "";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[
        AudioIOLayout {
            main_input_channels: NonZeroU32::new(2),
            main_output_channels: NonZeroU32::new(2),
            ..AudioIOLayout::const_default()
        },
        AudioIOLayout {
            main_input_channels: NonZeroU32::new(1),
            main_output_channels: NonZeroU32::new(1),
            ..AudioIOLayout::const_default()
        },
    ];

    const SAMPLE_ACCURATE_AUTOMATION: bool = true;

    type SysExMessage = ();
    type BackgroundTask = ();

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn editor(&mut self, _async_executor: AsyncExecutor<Self>) -> Option<Box<dyn Editor>> {
        editor::create(self.params.clone(), self.gain_reduction_db.clone())
    }

    fn initialize(
        &mut self,
        _audio_io_layout: &AudioIOLayout,
        buffer_config: &BufferConfig,
        context: &mut impl InitContext<Self>,
    ) -> bool {
        self.limiter = Limiter::new(MAX_CHANNELS, buffer_config.sample_rate);
        context.set_latency_samples(self.limiter.latency_samples());
        true
    }

    fn reset(&mut self) {
        self.limiter.reset();
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        _context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        self.limiter
            .set_isp_enabled(self.params.isp_detection.value());

        let mut frame = [0.0f32; MAX_CHANNELS];
        let mut min_gain = 1.0f32;
        for mut channel_samples in buffer.iter_samples() {
            let channels = channel_samples.len().min(MAX_CHANNELS);
            // `ch < channels <= channel_samples.len()`, so these `unwrap`s cannot fail.
            for (ch, slot) in frame.iter_mut().enumerate().take(channels) {
                *slot = *channel_samples.get_mut(ch).unwrap();
            }

            // Threshold at -X dB means X dB of drive into a 0 dBFS limiter.
            let input_gain = util::db_to_gain(-self.params.threshold.smoothed.next());
            let output_gain = util::db_to_gain(self.params.output.smoothed.next());
            let gain = self
                .limiter
                .process_frame(&mut frame[..channels], input_gain, output_gain);
            min_gain = min_gain.min(gain);

            for (ch, value) in frame.iter().enumerate().take(channels) {
                *channel_samples.get_mut(ch).unwrap() = *value;
            }
        }

        if self.params.editor_state.is_open() {
            let reduction = -util::gain_to_db(min_gain);
            self.gain_reduction_db
                .fetch_max(reduction, Ordering::Relaxed);
        }

        ProcessStatus::Normal
    }
}

impl ClapPlugin for CeilingLimiter {
    const CLAP_ID: &'static str = "audio.ceiling.limiter";
    const CLAP_DESCRIPTION: Option<&'static str> =
        Some("Transparent look-ahead brickwall limiter with true-peak detection");
    const CLAP_MANUAL_URL: Option<&'static str> = None;
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[
        ClapFeature::AudioEffect,
        ClapFeature::Limiter,
        ClapFeature::Mastering,
        ClapFeature::Stereo,
        ClapFeature::Mono,
    ];
}

impl Vst3Plugin for CeilingLimiter {
    const VST3_CLASS_ID: [u8; 16] = *b"CeilingLimiter01";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] =
        &[Vst3SubCategory::Fx, Vst3SubCategory::Dynamics];
}

nih_export_clap!(CeilingLimiter);
nih_export_vst3!(CeilingLimiter);
