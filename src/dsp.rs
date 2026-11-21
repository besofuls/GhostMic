use aec3::api::config::{EchoCanceller3Config, MaskingThresholds};
use aec3::nodes::audio::AudioFormat;
use aec3::pipelines::linear::{self, LinearPipeline};
use anyhow::{Context, Result};
use nnnoiseless::DenoiseState;

/// Standard real-time VoIP audio parameters: 48 kHz, 10 ms per frame.
/// - Speaker Loopback (Render) is STEREO (2ch = 960 samples/10ms) so AEC3 models Left & Right speakers separately!
/// - Microphone (Capture) and Output are MONO (1ch = 480 samples/10ms).
pub const SAMPLE_RATE: u32 = 48_000;
pub const RENDER_CHANNELS: u16 = 2;
pub const CAPTURE_CHANNELS: u16 = 1;
pub const RENDER_FRAME_SAMPLES: usize = 480 * RENDER_CHANNELS as usize;
pub const CAPTURE_FRAME_SAMPLES: usize = 480 * CAPTURE_CHANNELS as usize;

pub struct DspOptions {
    pub initial_delay_ms: i32,
    pub enable_webrtc_ns: bool,
    pub enable_agc: bool,
    pub enable_rnnoise: bool,
}

pub struct DspProcessor {
    pipeline: LinearPipeline,
    rnnoise: Option<Box<DenoiseState<'static>>>,
    aec_out: [f32; CAPTURE_FRAME_SAMPLES],
    rnn_in: [f32; CAPTURE_FRAME_SAMPLES],
    rnn_out: [f32; CAPTURE_FRAME_SAMPLES],
}

impl DspProcessor {
    pub fn new(opts: DspOptions) -> Result<Self> {
        let render_format = AudioFormat::ten_ms(SAMPLE_RATE, RENDER_CHANNELS);
        let capture_format = AudioFormat::ten_ms(SAMPLE_RATE, CAPTURE_CHANNELS);

        // Tune AEC3 for natural studio USB mic (Razer Seiren) + stereo speakers:
        // - Faster nearend (voice) detection so initial syllables are not clipped
        // - Longer hold duration so words don't drop out mid-sentence
        // - Conservative high-frequency suppression to preserve natural vocal clarity
        let mut cfg = EchoCanceller3Config::default();
        cfg.suppressor.dominant_nearend_detection.trigger_threshold = 6;
        cfg.suppressor.dominant_nearend_detection.hold_duration = 75;
        cfg.suppressor.conservative_hf_suppression = true;
        cfg.suppressor.nearend_tuning.mask_hf = MaskingThresholds::new(0.25, 0.45, 0.3);
        cfg.validate();

        let pipeline = linear::builder(render_format, capture_format)
            .aec3_config(cfg)
            .initial_delay_ms(opts.initial_delay_ms)
            .enable_high_pass_filter(true)
            .enable_noise_suppression(opts.enable_webrtc_ns)
            .enable_gain_controller2(opts.enable_agc)
            .enable_post_filter(false)
            .build()
            .context("Failed to initialize AEC3 linear pipeline")?;

        let rnnoise = if opts.enable_rnnoise {
            Some(DenoiseState::new())
        } else {
            None
        };

        Ok(Self {
            pipeline,
            rnnoise,
            aec_out: [0.0; CAPTURE_FRAME_SAMPLES],
            rnn_in: [0.0; CAPTURE_FRAME_SAMPLES],
            rnn_out: [0.0; CAPTURE_FRAME_SAMPLES],
        })
    }

    /// Feed a 10ms stereo (960 interleaved f32 samples) speaker reference frame into AEC3.
    #[inline]
    pub fn feed_speaker_frame(&mut self, render_frame: &[f32; RENDER_FRAME_SAMPLES]) -> Result<()> {
        self.pipeline
            .handle_render_frame(render_frame)
            .context("AEC3 handle_render_frame failed")
    }

    /// Process a 10ms mono (480 f32 samples) microphone capture frame and write cleaned audio to `out`.
    #[inline]
    pub fn process_mic_frame(
        &mut self,
        mic_frame: &[f32; CAPTURE_FRAME_SAMPLES],
        out: &mut [f32; CAPTURE_FRAME_SAMPLES],
    ) -> Result<()> {
        let produced = self
            .pipeline
            .process_capture_frame(mic_frame, &mut self.aec_out)
            .context("AEC3 process_capture_frame failed")?;

        if !produced {
            out.fill(0.0);
            return Ok(());
        }

        if let Some(ref mut rnn) = self.rnnoise {
            // RNNoise expects 16-bit PCM scale [-32768.0, 32767.0]
            for (dst, &src) in self.rnn_in.iter_mut().zip(self.aec_out.iter()) {
                *dst = (src * 32767.0).clamp(-32768.0, 32767.0);
            }
            rnn.process_frame(&mut self.rnn_out, &self.rnn_in);
            for (dst, &src) in out.iter_mut().zip(self.rnn_out.iter()) {
                *dst = (src / 32767.0).clamp(-1.0, 1.0);
            }
        } else {
            for (dst, &src) in out.iter_mut().zip(self.aec_out.iter()) {
                *dst = src.clamp(-1.0, 1.0);
            }
        }

        Ok(())
    }
}
