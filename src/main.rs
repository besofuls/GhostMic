#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod audio;
mod dsp;
mod engine;
mod gui;

use anyhow::{Context, Result};
use clap::Parser;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use wasapi::{Direction, initialize_mta};

use crate::engine::{EngineConfig, EngineMeters};

#[derive(Parser, Debug)]
#[command(
    name = "seiren-aec",
    version,
    about = "App sieu nhe viet bang Rust triet tieng loa lot vao mic (Stereo WASAPI Loopback + WebRTC AEC3)"
)]
struct Cli {
    /// Chay o che do dong lenh (khong mo cua so giao dien)
    #[arg(long)]
    cli: bool,

    /// Liet ke tat ca thiet bi Loa va Mic dang hoat dong tren Windows
    #[arg(long)]
    list: bool,

    /// Ten (hoac mot phan ten) cua Loa dung lam tin hieu tham chieu (mac dinh: Loa chinh cua Windows)
    #[arg(long)]
    speaker: Option<String>,

    /// Ten (hoac mot phan ten) cua Mic that (mac dinh: Mic chinh cua Windows, vd: Razer Seiren)
    #[arg(long)]
    mic: Option<String>,

    /// Ten (hoac mot phan ten) cua Mic ao dau ra (mac dinh: "CABLE Input" cua VB-Cable)
    #[arg(long, default_value = "CABLE Input")]
    output: String,

    /// Bat bo loc on WebRTC NS (mac dinh TAT de giu giong noi tu nhien nhat)
    #[arg(long)]
    ns: bool,

    /// Bat tu dong can bang am luong AGC2 (mac dinh TAT de tranh cat am)
    #[arg(long)]
    agc: bool,

    /// Bat them bo loc nhieu AI RNNoise (nnnoiseless) sau AEC3 de loc tieng go phim/quat
    #[arg(long)]
    rnnoise: bool,

    /// Do tre uoc tinh ban dau tu Loa den Mic (ms)
    #[arg(long, default_value_t = 40)]
    delay_ms: i32,

    /// Che do thu nghiem: ghi am ket qua ra file WAV (khong can VB-Cable)
    #[arg(long)]
    test_wav: Option<PathBuf>,

    /// So giay ghi am khi dung --test-wav
    #[arg(long, default_value_t = 10)]
    test_seconds: u32,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    // Default behavior (double-click or run without CLI flags): launch the native GUI
    if !cli.cli && !cli.list && cli.test_wav.is_none() {
        gui::run_gui().map_err(|e| anyhow::anyhow!("GUI error: {e}"))?;
        return Ok(());
    }

    if cli.list {
        return audio::print_audio_devices();
    }

    initialize_mta().ok().context("COM init failed")?;

    let (_, speaker_name) = audio::find_device(Direction::Render, cli.speaker.as_deref())
        .context("Khong tim thay Loa (Speaker) de lay tin hieu Loopback")?;
    let (_, mic_name) = audio::find_device(Direction::Capture, cli.mic.as_deref())
        .context("Khong tim thay Micro (Capture)")?;

    let is_test_mode = cli.test_wav.is_some();
    if !is_test_mode {
        audio::find_device(Direction::Render, Some(&cli.output))
            .with_context(|| format!("Khong tim thay thiet bi dau ra '{}'", cli.output))?;
    }

    println!("SEIREN-AEC CLI | Loa: {speaker_name} | Mic: {mic_name}");

    let running = Arc::new(AtomicBool::new(true));
    {
        let r = Arc::clone(&running);
        ctrlc::set_handler(move || {
            r.store(false, Ordering::SeqCst);
        })?;
    }

    let meters = Arc::new(EngineMeters::new());
    let cfg = EngineConfig {
        speaker: cli.speaker,
        mic: cli.mic,
        output: if is_test_mode {
            None
        } else {
            Some(cli.output)
        },
        enable_webrtc_ns: cli.ns,
        enable_agc: cli.agc,
        enable_rnnoise: cli.rnnoise,
        delay_ms: cli.delay_ms,
        test_wav: cli.test_wav,
        test_seconds: cli.test_seconds,
    };

    engine::run_engine(cfg, running, meters)
}
