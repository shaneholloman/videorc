//! Per-platform ffmpeg capture-input construction.
//!
//! The one seam that knows which ffmpeg input device backs a Videorc source
//! on each platform: macOS today (`-f avfoundation` video, CoreAudio→FIFO or
//! `-f avfoundation` audio), with the Windows arms (`-f ddagrab` screens,
//! `-f dshow` cameras/microphones) landing here per
//! docs/windows-port-plan.md Phase 2. Output and encode argument chains stay
//! with their pipelines — only device inputs live in this module.

use std::path::PathBuf;

use crate::audio::{
    NATIVE_AUDIO_CHANNELS, NATIVE_AUDIO_FFMPEG_QUEUE_SIZE, NATIVE_AUDIO_SAMPLE_RATE,
};

pub const AVFOUNDATION_VIDEO_PIXEL_FORMAT: &str = "nv12";

/// The primary video source a recording session captures.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VideoInput {
    MacScreen { index: usize },
    MacCamera { index: usize },
    TestPattern,
}

/// The microphone source feeding a session, in preference order: the native
/// capture path (CoreAudio today, WASAPI on Windows later) writing PCM to a
/// FIFO, or ffmpeg capturing the device directly as the fallback.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MicrophoneInput {
    CoreAudio {
        device_id: u32,
        fifo_path: Option<PathBuf>,
    },
    AvFoundation {
        index: usize,
    },
}

/// Low-latency session capture input for a screen or camera device
/// (macOS: avfoundation). The cursor is only meaningful for screens.
pub fn append_avfoundation_video_input(
    args: &mut Vec<String>,
    device_index: usize,
    fps: u32,
    capture_cursor: bool,
) {
    args.extend([
        "-fflags".to_string(),
        "nobuffer".to_string(),
        "-flags".to_string(),
        "low_delay".to_string(),
        "-probesize".to_string(),
        "32".to_string(),
        "-analyzeduration".to_string(),
        "0".to_string(),
        "-thread_queue_size".to_string(),
        "16".to_string(),
        "-f".to_string(),
        "avfoundation".to_string(),
        "-pixel_format".to_string(),
        AVFOUNDATION_VIDEO_PIXEL_FORMAT.to_string(),
        "-framerate".to_string(),
        fps.to_string(),
    ]);
    if capture_cursor {
        args.extend(["-capture_cursor".to_string(), "1".to_string()]);
    }
    args.extend(["-i".to_string(), format!("{device_index}:none")]);
}

/// Appends the session's microphone input. The native-FIFO arm is
/// platform-neutral (raw f32le from a path — any native capture source can
/// feed it); the direct-device arm is the per-platform piece.
pub fn append_microphone_input(
    args: &mut Vec<String>,
    microphone: Option<&MicrophoneInput>,
    next_input_index: &mut usize,
) -> bool {
    let Some(microphone) = microphone else {
        return false;
    };

    match microphone {
        MicrophoneInput::CoreAudio {
            fifo_path: Some(fifo_path),
            ..
        } => {
            args.extend([
                "-f".to_string(),
                "f32le".to_string(),
                "-ar".to_string(),
                NATIVE_AUDIO_SAMPLE_RATE.to_string(),
                "-ac".to_string(),
                NATIVE_AUDIO_CHANNELS.to_string(),
                "-thread_queue_size".to_string(),
                NATIVE_AUDIO_FFMPEG_QUEUE_SIZE.to_string(),
                "-i".to_string(),
                fifo_path.display().to_string(),
            ]);
            *next_input_index += 1;
            true
        }
        MicrophoneInput::CoreAudio {
            fifo_path: None, ..
        } => false,
        MicrophoneInput::AvFoundation { index } => {
            args.extend([
                "-f".to_string(),
                "avfoundation".to_string(),
                "-thread_queue_size".to_string(),
                "512".to_string(),
                "-i".to_string(),
                format!(":{index}"),
            ]);
            *next_input_index += 1;
            true
        }
    }
}

pub fn microphone_channels(microphone: Option<&MicrophoneInput>) -> u16 {
    match microphone {
        Some(MicrophoneInput::CoreAudio { .. }) => NATIVE_AUDIO_CHANNELS,
        Some(MicrophoneInput::AvFoundation { .. }) => 1,
        None => 0,
    }
}

/// Live-render capture input for a camera device (macOS: avfoundation).
/// Plainer than the session input on purpose: the live render loop scales
/// and paces frames itself, so none of the low-latency input tuning applies.
pub fn append_live_avfoundation_video_input(args: &mut Vec<String>, device_index: usize, fps: u32) {
    args.extend([
        "-f".to_string(),
        "avfoundation".to_string(),
        "-framerate".to_string(),
        fps.to_string(),
        "-i".to_string(),
        format!("{device_index}:none"),
    ]);
}
