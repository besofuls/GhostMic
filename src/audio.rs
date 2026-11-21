use anyhow::{Context, Result, anyhow};
use ringbuf::HeapProd;
use ringbuf::HeapCons;
use ringbuf::traits::{Consumer, Observer, Producer};
use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use wasapi::{
    Device, DeviceEnumerator, DeviceState, Direction, SampleType, StreamMode, WaveFormat,
    initialize_mta,
};

use crate::dsp::{CAPTURE_CHANNELS, RENDER_CHANNELS, SAMPLE_RATE};

pub fn desired_wave_format(channels: u16) -> WaveFormat {
    WaveFormat::new(
        32,
        32,
        SampleType::Float,
        SAMPLE_RATE as usize,
        channels as usize,
        None,
    )
}

pub fn list_active_devices(direction: Direction) -> Result<(Vec<String>, Option<String>)> {
    initialize_mta().ok().context("COM init failed")?;
    let enumerator = DeviceEnumerator::new().context("Failed to create DeviceEnumerator")?;
    let default_name = enumerator
        .get_default_device(direction)
        .ok()
        .and_then(|d| d.get_friendlyname().ok());

    let mut names = Vec::new();
    for dev in &enumerator
        .get_device_collection(direction)
        .context("Failed to enumerate devices")?
    {
        if let Ok(dev) = dev {
            if matches!(dev.get_state(), Ok(DeviceState::Active)) {
                if let Ok(name) = dev.get_friendlyname() {
                    names.push(name);
                }
            }
        }
    }
    Ok((names, default_name))
}

pub fn print_audio_devices() -> Result<()> {
    initialize_mta().ok().context("COM init failed")?;
    let enumerator = DeviceEnumerator::new().context("Failed to create DeviceEnumerator")?;

    let default_render = enumerator
        .get_default_device(Direction::Render)
        .ok()
        .and_then(|d| d.get_friendlyname().ok());
    let default_capture = enumerator
        .get_default_device(Direction::Capture)
        .ok()
        .and_then(|d| d.get_friendlyname().ok());

    println!("=== THIET BI PHAT (Speakers / Virtual Cable Input) ===");
    for dev in &enumerator
        .get_device_collection(Direction::Render)
        .context("Failed to enumerate render devices")?
    {
        if let Ok(dev) = dev {
            if matches!(dev.get_state(), Ok(DeviceState::Active)) {
                let name = dev.get_friendlyname().unwrap_or_else(|_| "<unknown>".into());
                let mark = if default_render.as_deref() == Some(name.as_str()) {
                    " [Mac dinh]"
                } else {
                    ""
                };
                println!("  - {name}{mark}");
            }
        }
    }

    println!("\n=== THIET BI THU (Microphones / Virtual Cable Output) ===");
    for dev in &enumerator
        .get_device_collection(Direction::Capture)
        .context("Failed to enumerate capture devices")?
    {
        if let Ok(dev) = dev {
            if matches!(dev.get_state(), Ok(DeviceState::Active)) {
                let name = dev.get_friendlyname().unwrap_or_else(|_| "<unknown>".into());
                let mark = if default_capture.as_deref() == Some(name.as_str()) {
                    " [Mac dinh]"
                } else {
                    ""
                };
                println!("  - {name}{mark}");
            }
        }
    }
    Ok(())
}

pub fn find_device(direction: Direction, filter: Option<&str>) -> Result<(Device, String)> {
    let enumerator = DeviceEnumerator::new().context("Failed to create DeviceEnumerator")?;

    if let Some(substr) = filter {
        let needle = substr.to_lowercase();
        for dev in &enumerator
            .get_device_collection(direction)
            .context("Failed to enumerate devices")?
        {
            if let Ok(dev) = dev {
                if matches!(dev.get_state(), Ok(DeviceState::Active)) {
                    if let Ok(name) = dev.get_friendlyname() {
                        if name.to_lowercase().contains(&needle) {
                            return Ok((dev, name));
                        }
                    }
                }
            }
        }
        return Err(anyhow!(
            "Khong tim thay thiet bi {:?} nao khop voi '{}'",
            direction,
            substr
        ));
    }

    let dev = enumerator
        .get_default_device(direction)
        .with_context(|| format!("Khong tim thay thiet bi mac dinh {:?}", direction))?;
    let name = dev.get_friendlyname().unwrap_or_else(|_| "<unknown>".into());
    Ok((dev, name))
}

/// Captures audio from either a Microphone (Direction::Capture) or a Speaker Loopback (Direction::Render)
/// and pushes normalized f32 mono 48kHz samples into `prod`.
pub fn run_capture_loop(
    device_direction: Direction,
    device_filter: Option<String>,
    mut prod: HeapProd<f32>,
    running: Arc<AtomicBool>,
    is_loopback: bool,
) -> Result<()> {
    initialize_mta().ok().context("COM init failed")?;
    let (device, _name) = find_device(device_direction, device_filter.as_deref())?;
    let mut audio_client = device.get_iaudioclient()?;

    let channels = if is_loopback {
        RENDER_CHANNELS
    } else {
        CAPTURE_CHANNELS
    };
    let format = desired_wave_format(channels);
    let (_def_time, min_time) = audio_client.get_device_period()?;

    let mode = StreamMode::EventsShared {
        autoconvert: true,
        buffer_duration_hns: min_time,
    };

    // Note: For loopback capture, `device` is Direction::Render,
    // and initializing client with Direction::Capture enables WASAPI Loopback automatically.
    audio_client.initialize_client(&format, Direction::Capture, mode, None)?;
    let h_event = audio_client.set_get_eventhandle()?;
    let buffer_frames = audio_client.get_buffer_size()? as usize;
    let capture_client = audio_client.get_audiocaptureclient()?;

    let blockalign = format.get_blockalign() as usize; // 4 bytes for mono f32
    let mut byte_queue: VecDeque<u8> = VecDeque::with_capacity(blockalign * buffer_frames * 8);

    audio_client.start_stream()?;

    while running.load(Ordering::Relaxed) {
        // Wait up to 20ms for next audio packet.
        // Loopback devices do NOT fire events during silence, so timeout is normal!
        let _ = h_event.wait_for_event(20);

        if !running.load(Ordering::Relaxed) {
            break;
        }

        if capture_client
            .read_from_device_to_deque(&mut byte_queue)
            .is_ok()
        {
            while byte_queue.len() >= 4 {
                let b0 = byte_queue.pop_front().unwrap();
                let b1 = byte_queue.pop_front().unwrap();
                let b2 = byte_queue.pop_front().unwrap();
                let b3 = byte_queue.pop_front().unwrap();
                let sample = f32::from_le_bytes([b0, b1, b2, b3]);

                // If ring buffer is full (consumer lagging), drop oldest or ignore overflow
                let _ = prod.try_push(sample);
            }
        } else if !is_loopback {
            // Brief yield on transient capture read error
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }

    let _ = audio_client.stop_stream();
    Ok(())
}

/// Renders cleaned f32 mono 48kHz samples from `cons` into the Virtual Cable (`CABLE Input`).
pub fn run_render_loop(
    output_filter: String,
    mut cons: HeapCons<f32>,
    running: Arc<AtomicBool>,
) -> Result<()> {
    initialize_mta().ok().context("COM init failed")?;
    let (device, _name) = find_device(Direction::Render, Some(&output_filter))?;
    let mut audio_client = device.get_iaudioclient()?;

    let format = desired_wave_format(CAPTURE_CHANNELS);
    let (_def_time, min_time) = audio_client.get_device_period()?;

    let mode = StreamMode::EventsShared {
        autoconvert: true,
        buffer_duration_hns: min_time,
    };

    audio_client.initialize_client(&format, Direction::Render, mode, None)?;
    let h_event = audio_client.set_get_eventhandle()?;
    let buffer_frames = audio_client.get_buffer_size()? as usize;
    let render_client = audio_client.get_audiorenderclient()?;

    let blockalign = format.get_blockalign() as usize;
    let mut byte_queue: VecDeque<u8> = VecDeque::with_capacity(blockalign * buffer_frames * 8);

    audio_client.start_stream()?;

    while running.load(Ordering::Relaxed) {
        let avail_frames = audio_client.get_available_space_in_frames()? as usize;
        if avail_frames > 0 {
            let needed_bytes = avail_frames * blockalign;
            while byte_queue.len() < needed_bytes {
                let sample = cons.try_pop().unwrap_or(0.0);
                byte_queue.extend(sample.to_le_bytes());
            }
            let _ = render_client.write_to_device_from_deque(avail_frames, &mut byte_queue, None);
        }

        // Prevent runaway latency if consumer falls behind
        if cons.occupied_len() > SAMPLE_RATE as usize / 10 {
            let to_skip = cons.occupied_len() - (SAMPLE_RATE as usize / 50);
            cons.skip(to_skip);
        }

        let _ = h_event.wait_for_event(20);
    }

    let _ = audio_client.stop_stream();
    Ok(())
}
