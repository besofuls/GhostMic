use anyhow::{Context, Result};
use hound::{SampleFormat, WavSpec, WavWriter};
use ringbuf::HeapRb;
use ringbuf::traits::{Consumer, Observer, Producer, Split};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;
use wasapi::Direction;

use crate::audio;
use crate::dsp::{
    CAPTURE_CHANNELS, CAPTURE_FRAME_SAMPLES, DspOptions, DspProcessor, RENDER_FRAME_SAMPLES,
    SAMPLE_RATE,
};

#[derive(Clone, Debug)]
pub struct EngineConfig {
    pub speaker: Option<String>,
    pub mic: Option<String>,
    pub output: Option<String>,
    pub enable_webrtc_ns: bool,
    pub enable_agc: bool,
    pub enable_rnnoise: bool,
    pub delay_ms: i32,
    pub test_wav: Option<PathBuf>,
    pub test_seconds: u32,
}

pub struct EngineMeters {
    speaker_rms: AtomicU32,
    raw_mic_rms: AtomicU32,
    clean_mic_rms: AtomicU32,
    pub status_text: Mutex<String>,
    pub last_error: Mutex<Option<String>>,
}

impl EngineMeters {
    pub fn new() -> Self {
        Self {
            speaker_rms: AtomicU32::new(0.0f32.to_bits()),
            raw_mic_rms: AtomicU32::new(0.0f32.to_bits()),
            clean_mic_rms: AtomicU32::new(0.0f32.to_bits()),
            status_text: Mutex::new("San sang".into()),
            last_error: Mutex::new(None),
        }
    }

    #[inline]
    pub fn set_levels(&self, spk: f32, raw: f32, clean: f32) {
        self.speaker_rms.store(spk.to_bits(), Ordering::Relaxed);
        self.raw_mic_rms.store(raw.to_bits(), Ordering::Relaxed);
        self.clean_mic_rms.store(clean.to_bits(), Ordering::Relaxed);
    }

    #[inline]
    pub fn levels(&self) -> (f32, f32, f32) {
        (
            f32::from_bits(self.speaker_rms.load(Ordering::Relaxed)),
            f32::from_bits(self.raw_mic_rms.load(Ordering::Relaxed)),
            f32::from_bits(self.clean_mic_rms.load(Ordering::Relaxed)),
        )
    }

    pub fn set_status(&self, msg: impl Into<String>) {
        if let Ok(mut s) = self.status_text.lock() {
            *s = msg.into();
        }
    }

    pub fn set_error(&self, msg: impl Into<String>) {
        if let Ok(mut e) = self.last_error.lock() {
            *e = Some(msg.into());
        }
    }

    pub fn clear_error(&self) {
        if let Ok(mut e) = self.last_error.lock() {
            *e = None;
        }
    }
}

pub struct AudioEngine {
    running: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
    pub meters: Arc<EngineMeters>,
}

impl AudioEngine {
    pub fn new() -> Self {
        Self {
            running: Arc::new(AtomicBool::new(false)),
            worker: None,
            meters: Arc::new(EngineMeters::new()),
        }
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    pub fn start(&mut self, cfg: EngineConfig) -> Result<()> {
        self.stop();
        self.meters.clear_error();
        self.meters.set_levels(0.0, 0.0, 0.0);

        let running = Arc::new(AtomicBool::new(true));
        self.running = Arc::clone(&running);
        let meters = Arc::clone(&self.meters);

        let handle = thread::Builder::new()
            .name("seiren-aec-dsp".into())
            .spawn(move || {
                if let Err(err) = run_engine(cfg, Arc::clone(&running), Arc::clone(&meters)) {
                    meters.set_error(format!("{err:#}"));
                    meters.set_status("Dung do loi");
                }
                running.store(false, Ordering::SeqCst);
                meters.set_levels(0.0, 0.0, 0.0);
            })
            .context("Khong the khoi tao luong DSP")?;

        self.worker = Some(handle);
        Ok(())
    }

    pub fn stop(&mut self) {
        self.running.store(false, Ordering::SeqCst);
        if let Some(handle) = self.worker.take() {
            let _ = handle.join();
        }
        self.meters.set_levels(0.0, 0.0, 0.0);
    }
}

impl Drop for AudioEngine {
    fn drop(&mut self) {
        self.stop();
    }
}

#[inline]
fn compute_rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum_sq: f32 = samples.iter().map(|&x| x * x).sum();
    (sum_sq / samples.len() as f32).sqrt()
}

pub fn run_engine(
    cfg: EngineConfig,
    running: Arc<AtomicBool>,
    meters: Arc<EngineMeters>,
) -> Result<()> {
    let is_test_mode = cfg.test_wav.is_some();

    let (speaker_prod, mut speaker_cons) = HeapRb::<f32>::new(RENDER_FRAME_SAMPLES * 30).split();
    let (mic_prod, mut mic_cons) = HeapRb::<f32>::new(CAPTURE_FRAME_SAMPLES * 30).split();
    let (mut out_prod, out_cons) = HeapRb::<f32>::new(CAPTURE_FRAME_SAMPLES * 30).split();

    // Thread 1: Speaker Loopback (Stereo)
    let r_spk = Arc::clone(&running);
    let spk_filter = cfg.speaker.clone();
    let m_spk = Arc::clone(&meters);
    let spk_handle = thread::Builder::new()
        .name("speaker-loopback".into())
        .spawn(move || {
            if let Err(e) =
                audio::run_capture_loop(Direction::Render, spk_filter, speaker_prod, r_spk, true)
            {
                m_spk.set_error(format!("Loi luong Loa Loopback: {e:#}"));
            }
        })?;

    // Thread 2: Microphone Capture (Mono)
    let r_mic = Arc::clone(&running);
    let mic_filter = cfg.mic.clone();
    let m_mic = Arc::clone(&meters);
    let mic_handle = thread::Builder::new()
        .name("mic-capture".into())
        .spawn(move || {
            if let Err(e) =
                audio::run_capture_loop(Direction::Capture, mic_filter, mic_prod, r_mic, false)
            {
                m_mic.set_error(format!("Loi luong Mic: {e:#}"));
            }
        })?;

    // Thread 3: Virtual Cable Render (only in live mode)
    let out_handle = if !is_test_mode {
        let out_filter = cfg
            .output
            .clone()
            .context("Chua chon thiet bi Mic ao dau ra (CABLE Input)")?;
        let r_out = Arc::clone(&running);
        let m_out = Arc::clone(&meters);
        Some(
            thread::Builder::new()
                .name("virtual-mic-render".into())
                .spawn(move || {
                    if let Err(e) = audio::run_render_loop(out_filter, out_cons, r_out) {
                        m_out.set_error(format!("Loi luong Mic ao dau ra: {e:#}"));
                    }
                })?,
        )
    } else {
        None
    };

    let wav_spec = WavSpec {
        channels: CAPTURE_CHANNELS,
        sample_rate: SAMPLE_RATE,
        bits_per_sample: 32,
        sample_format: SampleFormat::Float,
    };
    let mut clean_writer = if let Some(ref wav_path) = cfg.test_wav {
        Some(
            WavWriter::create(wav_path, wav_spec)
                .with_context(|| format!("Khong the tao file {}", wav_path.display()))?,
        )
    } else {
        None
    };
    let raw_wav_path = cfg.test_wav.as_ref().map(|p| {
        let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("test");
        p.with_file_name(format!("{stem}_raw.wav"))
    });
    let mut raw_writer = if let Some(ref p) = raw_wav_path {
        Some(WavWriter::create(p, wav_spec)?)
    } else {
        None
    };

    let mut dsp = DspProcessor::new(DspOptions {
        initial_delay_ms: cfg.delay_ms,
        enable_webrtc_ns: cfg.enable_webrtc_ns,
        enable_agc: cfg.enable_agc,
        enable_rnnoise: cfg.enable_rnnoise,
    })?;

    if is_test_mode {
        meters.set_status(format!("Dang ghi thu nghiem {} giay...", cfg.test_seconds));
    } else {
        meters.set_status("Dang loc am thanh (Stereo AEC3)...");
    }

    let mut speaker_frame = [0.0f32; RENDER_FRAME_SAMPLES];
    let mut mic_frame = [0.0f32; CAPTURE_FRAME_SAMPLES];
    let mut out_frame = [0.0f32; CAPTURE_FRAME_SAMPLES];

    let max_test_frames = (cfg.test_seconds as usize) * 100;
    let mut processed_frames: usize = 0;

    while running.load(Ordering::Relaxed) {
        if mic_cons.occupied_len() < CAPTURE_FRAME_SAMPLES {
            thread::sleep(Duration::from_millis(1));
            continue;
        }

        if speaker_cons.occupied_len() > RENDER_FRAME_SAMPLES * 12 {
            let excess = speaker_cons.occupied_len() - RENDER_FRAME_SAMPLES * 3;
            speaker_cons.skip(excess);
        }

        if speaker_cons.occupied_len() > 0 && speaker_cons.occupied_len() < RENDER_FRAME_SAMPLES {
            thread::sleep(Duration::from_millis(1));
        }

        if speaker_cons.occupied_len() >= RENDER_FRAME_SAMPLES {
            let n = speaker_cons.pop_slice(&mut speaker_frame);
            if n < RENDER_FRAME_SAMPLES {
                speaker_frame[n..].fill(0.0);
            }
        } else {
            speaker_frame.fill(0.0);
        }

        let n = mic_cons.pop_slice(&mut mic_frame);
        if n < CAPTURE_FRAME_SAMPLES {
            mic_frame[n..].fill(0.0);
        }

        dsp.feed_speaker_frame(&speaker_frame)?;
        dsp.process_mic_frame(&mic_frame, &mut out_frame)?;

        // Update live VU meters (fast, lock-free)
        meters.set_levels(
            compute_rms(&speaker_frame),
            compute_rms(&mic_frame),
            compute_rms(&out_frame),
        );

        if let Some(ref mut cw) = clean_writer {
            for &s in &out_frame {
                cw.write_sample(s)?;
            }
            if let Some(ref mut rw) = raw_writer {
                for &s in &mic_frame {
                    rw.write_sample(s)?;
                }
            }
            processed_frames += 1;
            if processed_frames % 100 == 0 {
                let left = cfg.test_seconds.saturating_sub((processed_frames / 100) as u32);
                meters.set_status(format!("Dang ghi thu nghiem... con {left} giay"));
            }
            if processed_frames >= max_test_frames {
                running.store(false, Ordering::SeqCst);
                break;
            }
        } else {
            let _ = out_prod.push_slice(&out_frame);
        }
    }

    if let Some(cw) = clean_writer {
        cw.finalize()?;
    }
    if let Some(rw) = raw_writer {
        rw.finalize()?;
    }

    let _ = spk_handle.join();
    let _ = mic_handle.join();
    if let Some(h) = out_handle {
        let _ = h.join();
    }

    if let Some(ref clean_path) = cfg.test_wav {
        meters.set_status(format!("Da luu file test tai: {}", clean_path.display()));
    } else if meters.last_error.lock().ok().and_then(|e| e.clone()).is_none() {
        meters.set_status("Da dung.");
    }

    Ok(())
}
