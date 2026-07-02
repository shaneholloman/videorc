//! Live captions: taps microphone PCM off the native audio pipeline, slices it
//! into ~3s 16kHz mono WAV chunks, transcribes each through videorc-web
//! (`/api/ai/captions/chunks` → AI Gateway grok-stt) and broadcasts transcript
//! events to renderer clients. Chunked by design (P0 spike 2026-07-02: gateway
//! realtime tokens need a Gateway API key that is not provisioned); the session
//! loop is the transport seam where a streaming socket can replace chunking.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Result, bail};
use serde::Serialize;
use tokio::sync::Mutex;
use tokio::sync::mpsc;

use crate::audio::AudioFrame;
use crate::state::AppState;
use crate::videorc_api::{CaptionChunkFailure, VideorcApiClient};

pub const CAPTION_SAMPLE_RATE: u32 = 16_000;
pub const CAPTION_CHUNK_SECONDS: f64 = 3.0;
/// Bounded frame queue between the realtime audio thread and the session task.
/// At ~93 CoreAudio callbacks/s, 256 frames ≈ 2.7s of cushion.
const TAP_CHANNEL_CAPACITY: usize = 256;
/// Consecutive transient upload failures before the session gives up.
const MAX_CONSECUTIVE_FAILURES: u32 = 5;

// ---------------------------------------------------------------------------
// Tap: the audio FIFO writer thread offers every mic frame here. Fast path is
// one relaxed atomic load when captions are off; when on, a non-blocking
// try_send that drops the frame rather than ever stalling the audio thread.
// ---------------------------------------------------------------------------

static TAP_ACTIVE: AtomicBool = AtomicBool::new(false);
static TAP: std::sync::Mutex<Option<mpsc::Sender<AudioFrame>>> = std::sync::Mutex::new(None);

pub fn offer_caption_frame(frame: &AudioFrame) {
    if !TAP_ACTIVE.load(Ordering::Relaxed) {
        return;
    }
    let Ok(guard) = TAP.try_lock() else {
        return;
    };
    if let Some(sender) = guard.as_ref() {
        let _ = sender.try_send(frame.clone());
    }
}

fn install_tap() -> mpsc::Receiver<AudioFrame> {
    let (sender, receiver) = mpsc::channel(TAP_CHANNEL_CAPACITY);
    *TAP.lock().expect("caption tap lock") = Some(sender);
    TAP_ACTIVE.store(true, Ordering::Relaxed);
    receiver
}

fn remove_tap() {
    TAP_ACTIVE.store(false, Ordering::Relaxed);
    *TAP.lock().expect("caption tap lock") = None;
}

// ---------------------------------------------------------------------------
// DSP: 48kHz interleaved f32 (mono or stereo) → 16kHz mono s16le.
// ---------------------------------------------------------------------------

/// Downmix interleaved samples to mono and decimate 3:1 (48kHz → 16kHz) with a
/// 3-sample boxcar average as a cheap anti-alias low-pass — speech-grade, which
/// is all a caption model needs. Returns an empty vec for unsupported input
/// (only 48kHz, 1–2 channels are produced by the native pipeline).
pub fn downmix_resample_to_16k_mono(samples: &[f32], channels: u16, sample_rate: u32) -> Vec<i16> {
    if sample_rate != 48_000 || !(1..=2).contains(&channels) {
        return Vec::new();
    }
    let channels = usize::from(channels);
    let mono: Vec<f32> = samples
        .chunks_exact(channels)
        .map(|frame| frame.iter().sum::<f32>() / channels as f32)
        .collect();
    mono.chunks_exact(3)
        .map(|window| {
            let value = (window[0] + window[1] + window[2]) / 3.0;
            (value.clamp(-1.0, 1.0) * f32::from(i16::MAX)) as i16
        })
        .collect()
}

/// Minimal 44-byte-header PCM WAV (16kHz mono s16le) — what the caption route
/// uploads as `audio/wav`.
pub fn encode_wav_16k_mono(samples: &[i16]) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let byte_rate = CAPTION_SAMPLE_RATE * 2;
    let mut wav = Vec::with_capacity(44 + samples.len() * 2);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data_len).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16_u32.to_le_bytes());
    wav.extend_from_slice(&1_u16.to_le_bytes()); // PCM
    wav.extend_from_slice(&1_u16.to_le_bytes()); // mono
    wav.extend_from_slice(&CAPTION_SAMPLE_RATE.to_le_bytes());
    wav.extend_from_slice(&byte_rate.to_le_bytes());
    wav.extend_from_slice(&2_u16.to_le_bytes()); // block align
    wav.extend_from_slice(&16_u16.to_le_bytes()); // bits per sample
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_len.to_le_bytes());
    for sample in samples {
        wav.extend_from_slice(&sample.to_le_bytes());
    }
    wav
}

// ---------------------------------------------------------------------------
// Chunk records: every transcribed chunk is remembered (text + word timing +
// audio offset) so the post-recording pass can render perfectly-synced
// captions. The tap only receives frames while a session's audio pipeline
// runs and those frames are already epoch-trimmed, so offsets anchor to the
// recording start. A new session restarts the audio unit (frame timestamps
// regress), which resets the anchor and the pending buffer.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, serde::Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CaptionSegment {
    pub text: String,
    pub start_second: f64,
    pub end_second: f64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CaptionChunkRecord {
    pub seq: u64,
    /// Seconds from the recording epoch to this chunk's first sample.
    pub offset_seconds: f64,
    pub duration_seconds: f64,
    pub text: String,
    /// Word timing RELATIVE TO THE CHUNK (add offset_seconds for absolute).
    pub segments: Vec<CaptionSegment>,
}

/// A frame timestamp lower than the last one means the capture pipeline
/// restarted (new session): reset the chunk anchor.
pub fn caption_anchor_should_reset(last_timestamp: Option<u64>, current: u64) -> bool {
    last_timestamp.is_some_and(|last| current < last)
}

/// Absolute cue windows (start, end, text) shared by the SRT and ASS
/// renderers: cue per chunk, timed by word segments (chunk-window fallback),
/// sorted, ends clamped to the next cue so captions never stack.
fn caption_cues(chunks: &[CaptionChunkRecord]) -> Vec<(f64, f64, String)> {
    let mut cues = Vec::with_capacity(chunks.len());
    for chunk in chunks {
        let text = chunk.text.trim();
        if text.is_empty() {
            continue;
        }
        let (start, end) = chunk_cue_window(chunk);
        cues.push((start, end, text.to_string()));
    }
    cues.sort_by(|left, right| left.0.total_cmp(&right.0));
    for index in 0..cues.len().saturating_sub(1) {
        let next_start = cues[index + 1].0;
        if cues[index].1 > next_start {
            cues[index].1 = next_start;
        }
    }
    cues
}

/// Render chunk records as SubRip.
pub fn render_srt(chunks: &[CaptionChunkRecord]) -> String {
    let mut srt = String::new();
    for (index, (start, end, text)) in caption_cues(chunks).iter().enumerate() {
        srt.push_str(&format!(
            "{}\n{} --> {}\n{}\n\n",
            index + 1,
            format_srt_timestamp(*start),
            format_srt_timestamp((*end).max(*start + 0.001)),
            text
        ));
    }
    srt
}

/// Caption text size for the burned copy (mirrors the renderer knob).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CaptionTextSize {
    S,
    #[default]
    M,
    L,
}

/// ASS is authored against a fixed reference resolution; libass scales it to
/// the actual video, so sizes stay consistent at any output.
const ASS_PLAY_RES_X: u32 = 1920;
const ASS_PLAY_RES_Y: u32 = 1080;

/// Render chunk records as an ASS track with glass-adjacent styling:
/// translucent charcoal opaque-box, near-white text, bottom/top center per the
/// position knob (square corners — the ASS limit accepted in grilling Q6).
pub fn render_ass(
    chunks: &[CaptionChunkRecord],
    position: CaptionOverlayPosition,
    text_size: CaptionTextSize,
) -> String {
    let cues = caption_cues(chunks);
    let size_factor = match text_size {
        CaptionTextSize::S => 0.8,
        CaptionTextSize::M => 1.0,
        CaptionTextSize::L => 1.25,
    };
    let font_size = ((f64::from(ASS_PLAY_RES_X) / 38.0) * size_factor).round() as u32;
    let alignment = match position {
        CaptionOverlayPosition::Bottom => 2, // bottom-center
        CaptionOverlayPosition::Top => 8,    // top-center
    };
    let margin_v = (f64::from(ASS_PLAY_RES_Y) * 0.04).round() as u32;

    let mut ass = format!(
        "[Script Info]\n\
         ScriptType: v4.00+\n\
         PlayResX: {ASS_PLAY_RES_X}\n\
         PlayResY: {ASS_PLAY_RES_Y}\n\
         WrapStyle: 0\n\
         ScaledBorderAndShadow: yes\n\
         \n\
         [V4+ Styles]\n\
         Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding\n\
         Style: VideorcCaptions,Helvetica,{font_size},&H00F5F4F4,&H00F5F4F4,&H261F1C1C,&H261F1C1C,0,0,0,0,100,100,0,0,3,10,0,{alignment},60,60,{margin_v},1\n\
         \n\
         [Events]\n\
         Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n"
    );
    for (start, end, text) in &cues {
        let escaped = text.replace('\n', "\\N").replace('{', "(").replace('}', ")");
        ass.push_str(&format!(
            "Dialogue: 0,{},{},VideorcCaptions,,0,0,0,,{}\n",
            format_ass_timestamp(*start),
            format_ass_timestamp((*end).max(*start + 0.01)),
            escaped
        ));
    }
    ass
}

fn format_ass_timestamp(seconds: f64) -> String {
    let clamped = seconds.max(0.0);
    let total_centis = (clamped * 100.0).round() as u64;
    let hours = total_centis / 360_000;
    let minutes = (total_centis % 360_000) / 6_000;
    let secs = (total_centis % 6_000) / 100;
    let centis = total_centis % 100;
    format!("{hours}:{minutes:02}:{secs:02}.{centis:02}")
}

/// `Recording.mp4` → `Recording (captioned).mp4`.
pub fn captioned_copy_path(recording: &std::path::Path) -> std::path::PathBuf {
    let stem = recording
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("recording");
    let extension = recording
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("mp4");
    recording.with_file_name(format!("{stem} (captioned).{extension}"))
}

/// Probe whether this ffmpeg build carries the libass `ass` filter.
async fn ffmpeg_supports_ass_filter(ffmpeg_path: &str) -> bool {
    match tokio::process::Command::new(ffmpeg_path)
        .args(["-hide_banner", "-filters"])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .await
    {
        Ok(output) => String::from_utf8_lossy(&output.stdout)
            .lines()
            .any(|line| line.split_whitespace().nth(1) == Some("ass")),
        Err(_) => false,
    }
}

/// Escape a path for use inside an ffmpeg filter argument (ass=...).
fn escape_ffmpeg_filter_path(path: &std::path::Path) -> String {
    path.display()
        .to_string()
        .replace('\\', "\\\\")
        .replace(':', "\\:")
        .replace('\'', "\\'")
}

fn chunk_cue_window(chunk: &CaptionChunkRecord) -> (f64, f64) {
    let first = chunk.segments.first().map(|segment| segment.start_second);
    let last = chunk.segments.last().map(|segment| segment.end_second);
    match (first, last) {
        (Some(first), Some(last)) if last > first => (
            chunk.offset_seconds + first.max(0.0),
            chunk.offset_seconds + last.min(chunk.duration_seconds.max(last)),
        ),
        _ => (
            chunk.offset_seconds,
            chunk.offset_seconds + chunk.duration_seconds,
        ),
    }
}

fn format_srt_timestamp(seconds: f64) -> String {
    let clamped = seconds.max(0.0);
    let total_millis = (clamped * 1000.0).round() as u64;
    let hours = total_millis / 3_600_000;
    let minutes = (total_millis % 3_600_000) / 60_000;
    let secs = (total_millis % 60_000) / 1000;
    let millis = total_millis % 1000;
    format!("{hours:02}:{minutes:02}:{secs:02},{millis:03}")
}

pub async fn drain_caption_chunks(state: &AppState) -> Vec<CaptionChunkRecord> {
    std::mem::take(&mut state.captions.lock().await.chunks)
}

/// Session-stop hook (recording finalize path): drain the chunks recorded
/// during this session and write the `.srt` sidecar next to the recording.
/// Returns the drained chunks so the burned-copy job can reuse them. Never
/// fails the session — problems downgrade to health warnings.
pub async fn write_caption_artifacts(
    state: &AppState,
    session_id: &str,
    recording_path: &std::path::Path,
) -> Vec<CaptionChunkRecord> {
    let chunks = drain_caption_chunks(state).await;
    if chunks.is_empty() {
        return chunks;
    }
    let srt = render_srt(&chunks);
    if srt.is_empty() {
        return chunks;
    }
    let srt_path = recording_path.with_extension("srt");
    match tokio::fs::write(&srt_path, &srt).await {
        Ok(()) => {
            let _ = crate::recording::emit_health_event(
                state,
                Some(session_id),
                crate::protocol::HealthLevel::Info,
                "captions-srt-written",
                &format!("Captions saved to {}.", srt_path.display()),
            );
        }
        Err(error) => {
            let _ = crate::recording::emit_health_event(
                state,
                Some(session_id),
                crate::protocol::HealthLevel::Warn,
                "captions-srt-failed",
                &format!("Could not write captions sidecar: {error}"),
            );
        }
    }
    chunks
}

/// Burn the aligned captions into a `(captioned)` copy of the recording via
/// the idle-aware ffmpeg coordinator (same family as the repair gates). The
/// original file is never touched; any failure degrades to SRT-only with a
/// health warning. Not restart-resumable (v1): if the app quits mid-encode,
/// the copy is simply absent while the .srt remains.
pub fn enqueue_caption_burn(
    state: AppState,
    session_id: String,
    ffmpeg_path: String,
    recording_path: std::path::PathBuf,
    chunks: Vec<CaptionChunkRecord>,
) {
    tokio::spawn(async move {
        let (position, text_size) = state.captions.lock().await.style;
        let ass = render_ass(&chunks, position, text_size);
        if ass.is_empty() || !ass.contains("Dialogue:") {
            return;
        }
        let ass_path = recording_path.with_extension("captions.ass");
        if let Err(error) = tokio::fs::write(&ass_path, &ass).await {
            let _ = crate::recording::emit_health_event(
                &state,
                Some(&session_id),
                crate::protocol::HealthLevel::Warn,
                "captions-burn-failed",
                &format!("Could not write the captions track: {error}"),
            );
            return;
        }

        // The bundled ffmpeg is a minimal LGPL build WITHOUT libass (see
        // build-ffmpeg-macos.sh) — probe for the ass filter and degrade
        // loudly rather than failing mid-encode. Dev/homebrew ffmpeg has it.
        if !ffmpeg_supports_ass_filter(&ffmpeg_path).await {
            let _ = tokio::fs::remove_file(&ass_path).await;
            let _ = crate::recording::emit_health_event(
                &state,
                Some(&session_id),
                crate::protocol::HealthLevel::Warn,
                "captions-burn-unsupported",
                "This ffmpeg build has no subtitle renderer (libass); the .srt sidecar is available and the captioned copy was skipped.",
            );
            return;
        }

        // Wait out the same idle window as the quality gates, then hold the
        // maintenance permit so the encode never competes with a capture.
        tokio::time::sleep(std::time::Duration::from_secs(30)).await;
        let maintenance = state.ffmpeg_work.begin_maintenance_when_idle().await;
        let cancel = maintenance.cancel_token();
        let output_path = captioned_copy_path(&recording_path);
        state.emit_log(
            "info",
            format!("Burning captions into {}.", output_path.display()),
        );

        let filter = format!("ass={}", escape_ffmpeg_filter_path(&ass_path));
        let spawned = tokio::process::Command::new(&ffmpeg_path)
            .arg("-y")
            .arg("-i")
            .arg(&recording_path)
            .arg("-vf")
            .arg(&filter)
            .arg("-c:a")
            .arg("copy")
            .arg(&output_path)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
        let mut child = match spawned {
            Ok(child) => child,
            Err(error) => {
                let _ = crate::recording::emit_health_event(
                    &state,
                    Some(&session_id),
                    crate::protocol::HealthLevel::Warn,
                    "captions-burn-failed",
                    &format!("Could not start ffmpeg for the captioned copy: {error}"),
                );
                return;
            }
        };

        let outcome = loop {
            if cancel.is_cancelled() {
                let _ = child.kill().await;
                break Err("capture started; captioned copy cancelled".to_string());
            }
            match child.try_wait() {
                Ok(Some(status)) if status.success() => break Ok(()),
                Ok(Some(status)) => break Err(format!("ffmpeg exited with {status}")),
                Ok(None) => tokio::time::sleep(std::time::Duration::from_millis(250)).await,
                Err(error) => break Err(format!("could not wait for ffmpeg: {error}")),
            }
        };

        match outcome {
            Ok(()) => {
                let _ = tokio::fs::remove_file(&ass_path).await;
                let _ = crate::recording::emit_health_event(
                    &state,
                    Some(&session_id),
                    crate::protocol::HealthLevel::Info,
                    "captions-burned-copy-ready",
                    &format!("Captioned copy saved to {}.", output_path.display()),
                );
            }
            Err(reason) => {
                let _ = tokio::fs::remove_file(&output_path).await;
                let _ = crate::recording::emit_health_event(
                    &state,
                    Some(&session_id),
                    crate::protocol::HealthLevel::Warn,
                    "captions-burn-failed",
                    &format!("Captioned copy was not created ({reason}); the .srt sidecar is still available."),
                );
            }
        }
    });
}

// ---------------------------------------------------------------------------
// Burn-in overlay: a pre-rendered caption bar (RGBA) the compositor composites
// into the STREAM leg. Session-transient — set/cleared by the renderer as
// captions flow; never persisted, never part of scene config. Fail-safe per
// the background rule: bad image data is rejected and the previous overlay
// (if any) stays; a session is never touched by overlay errors.
// ---------------------------------------------------------------------------

/// Max decoded dimensions / encoded bytes for one caption bar. A 4K-width
/// two-line bar is ~3840×400; these caps leave headroom without letting the
/// RPC become an arbitrary-image firehose.
const OVERLAY_MAX_WIDTH: u32 = 4096;
const OVERLAY_MAX_HEIGHT: u32 = 2048;
const OVERLAY_MAX_ENCODED_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CaptionOverlayPosition {
    Top,
    #[default]
    Bottom,
}

#[derive(Clone)]
pub struct CaptionOverlay {
    pub rgba: Arc<Vec<u8>>,
    pub width: u32,
    pub height: u32,
    pub position: CaptionOverlayPosition,
    pub revision: u64,
}

pub type CaptionOverlaySlot = Arc<std::sync::Mutex<Option<CaptionOverlay>>>;

pub fn new_caption_overlay_slot() -> CaptionOverlaySlot {
    Arc::new(std::sync::Mutex::new(None))
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptionOverlayInfo {
    pub active: bool,
    pub width: u32,
    pub height: u32,
    pub revision: u64,
}

/// Decode + validate a caption bar and install it in the overlay slot.
/// Rejects oversized or undecodable payloads without touching the current
/// overlay. Pure with respect to the slot — unit-tested directly.
pub fn install_caption_overlay(
    slot: &CaptionOverlaySlot,
    png_base64: &str,
    position: CaptionOverlayPosition,
) -> Result<CaptionOverlayInfo> {
    use base64::Engine as _;

    let encoded_len = png_base64.len();
    if encoded_len == 0 || encoded_len > (OVERLAY_MAX_ENCODED_BYTES / 3) * 4 + 4 {
        bail!("Caption overlay payload is empty or too large.");
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(png_base64.trim())
        .map_err(|_| anyhow::anyhow!("Caption overlay payload is not valid base64."))?;
    if bytes.len() > OVERLAY_MAX_ENCODED_BYTES {
        bail!("Caption overlay image is too large.");
    }
    let image = image::load_from_memory(&bytes)
        .map_err(|_| anyhow::anyhow!("Caption overlay image could not be decoded."))?
        .into_rgba8();
    let (width, height) = image.dimensions();
    if width == 0 || height == 0 || width > OVERLAY_MAX_WIDTH || height > OVERLAY_MAX_HEIGHT {
        bail!("Caption overlay dimensions are out of range ({width}x{height}).");
    }

    let mut guard = slot.lock().expect("caption overlay lock");
    let revision = guard.as_ref().map_or(1, |overlay| overlay.revision + 1);
    *guard = Some(CaptionOverlay {
        rgba: Arc::new(image.into_raw()),
        width,
        height,
        position,
        revision,
    });
    Ok(CaptionOverlayInfo {
        active: true,
        width,
        height,
        revision,
    })
}

pub fn clear_caption_overlay(slot: &CaptionOverlaySlot) -> CaptionOverlayInfo {
    let mut guard = slot.lock().expect("caption overlay lock");
    let revision = guard.as_ref().map_or(0, |overlay| overlay.revision);
    *guard = None;
    CaptionOverlayInfo {
        active: false,
        width: 0,
        height: 0,
        revision,
    }
}

pub fn current_caption_overlay(slot: &CaptionOverlaySlot) -> Option<CaptionOverlay> {
    slot.lock().expect("caption overlay lock").clone()
}

// ---------------------------------------------------------------------------
// Session state machine.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CaptionsState {
    Idle,
    Live,
    Error,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptionsStatus {
    pub state: CaptionsState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remaining_seconds: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_client_id: Option<String>,
}

impl CaptionsStatus {
    pub fn idle() -> Self {
        Self {
            state: CaptionsState::Idle,
            message: None,
            remaining_seconds: None,
            session_client_id: None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptionsUpdate {
    pub session_client_id: String,
    pub seq: u64,
    pub text: String,
    pub chunk_seconds: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remaining_seconds: Option<u64>,
}

#[derive(Default)]
pub struct CaptionsCoordinator {
    task: Option<tokio::task::JoinHandle<()>>,
    stop: Option<Arc<AtomicBool>>,
    status: Option<CaptionsStatus>,
    /// Transcribed chunks awaiting the post-recording pass (drained at
    /// session stop; cleared when a new capture pipeline starts).
    chunks: Vec<CaptionChunkRecord>,
    /// Styling knobs captured at session start for the burned copy.
    style: (CaptionOverlayPosition, CaptionTextSize),
}

/// Stash the caption style for this session (used by the burned copy's ASS).
pub async fn set_caption_session_style(
    state: &AppState,
    position: CaptionOverlayPosition,
    text_size: CaptionTextSize,
) {
    state.captions.lock().await.style = (position, text_size);
}

pub type CaptionsSlot = Arc<Mutex<CaptionsCoordinator>>;

pub fn new_captions_slot() -> CaptionsSlot {
    Arc::new(Mutex::new(CaptionsCoordinator::default()))
}

pub async fn captions_status(state: &AppState) -> CaptionsStatus {
    state
        .captions
        .lock()
        .await
        .status
        .clone()
        .unwrap_or_else(CaptionsStatus::idle)
}

fn set_status(state: &AppState, coordinator: &mut CaptionsCoordinator, status: CaptionsStatus) {
    coordinator.status = Some(status.clone());
    state.emit_event("captions.status", status);
}

/// Fire-and-forget status update from inside the session task (which cannot
/// hold the coordinator lock while the RPC handler might).
async fn publish_status(state: &AppState, status: CaptionsStatus) {
    let mut coordinator = state.captions.lock().await;
    coordinator.status = Some(status.clone());
    drop(coordinator);
    state.emit_event("captions.status", status);
}

pub async fn start_captions(state: &AppState, language: Option<String>) -> Result<CaptionsStatus> {
    let Some(bearer) = crate::account::stored_session_token() else {
        bail!("Sign in to use live captions.");
    };
    let client = VideorcApiClient::new()?;

    let mut coordinator = state.captions.lock().await;
    if let (Some(task), Some(status)) = (coordinator.task.as_ref(), coordinator.status.as_ref()) {
        if !task.is_finished() && status.state == CaptionsState::Live {
            return Ok(status.clone());
        }
    }
    if let Some(task) = coordinator.task.take() {
        task.abort();
    }
    remove_tap();

    let session_client_id = format!("captions-{}", uuid::Uuid::new_v4().simple());
    let stop = Arc::new(AtomicBool::new(false));
    let receiver = install_tap();
    let status = CaptionsStatus {
        state: CaptionsState::Live,
        message: None,
        remaining_seconds: None,
        session_client_id: Some(session_client_id.clone()),
    };
    set_status(state, &mut coordinator, status.clone());

    let task_state = state.clone();
    let task_stop = stop.clone();
    coordinator.task = Some(tokio::spawn(run_caption_session(CaptionSession {
        bearer,
        client,
        language,
        receiver,
        session_client_id,
        state: task_state,
        stop: task_stop,
    })));
    coordinator.stop = Some(stop);

    Ok(status)
}

pub async fn stop_captions(state: &AppState) -> CaptionsStatus {
    let mut coordinator = state.captions.lock().await;
    if let Some(stop) = coordinator.stop.take() {
        stop.store(true, Ordering::Relaxed);
    }
    if let Some(task) = coordinator.task.take() {
        task.abort();
    }
    remove_tap();
    let status = CaptionsStatus::idle();
    set_status(state, &mut coordinator, status.clone());
    status
}

struct CaptionSession {
    bearer: String,
    client: VideorcApiClient,
    language: Option<String>,
    receiver: mpsc::Receiver<AudioFrame>,
    session_client_id: String,
    state: AppState,
    stop: Arc<AtomicBool>,
}

async fn run_caption_session(mut session: CaptionSession) {
    let chunk_samples = (f64::from(CAPTION_SAMPLE_RATE) * CAPTION_CHUNK_SECONDS) as usize;
    let mut pcm: Vec<i16> = Vec::with_capacity(chunk_samples * 2);
    let mut seq = 0_u64;
    let mut consecutive_failures = 0_u32;
    // Recording-epoch anchoring for the post pass: mono samples consumed since
    // the current capture pipeline started (frames are already epoch-trimmed).
    let mut consumed_samples: u64 = 0;
    let mut last_frame_timestamp: Option<u64> = None;

    loop {
        if session.stop.load(Ordering::Relaxed) {
            break;
        }
        let Some(frame) = session.receiver.recv().await else {
            break; // tap removed
        };
        if caption_anchor_should_reset(last_frame_timestamp, frame.timestamp_micros) {
            // New capture pipeline (new recording): drop cross-session audio
            // and restart the offset anchor. Chunks already recorded belong to
            // the previous recording and stay until its stop path drains them.
            pcm.clear();
            consumed_samples = 0;
        }
        last_frame_timestamp = Some(frame.timestamp_micros);
        pcm.extend(downmix_resample_to_16k_mono(
            &frame.samples,
            frame.channels,
            frame.sample_rate,
        ));
        if pcm.len() < chunk_samples {
            continue;
        }

        let chunk: Vec<i16> = pcm.drain(..chunk_samples).collect();
        let wav = encode_wav_16k_mono(&chunk);
        seq += 1;
        let offset_seconds = consumed_samples as f64 / f64::from(CAPTION_SAMPLE_RATE);
        consumed_samples += chunk_samples as u64;

        match session
            .client
            .transcribe_caption_chunk(
                &session.bearer,
                &session.session_client_id,
                wav,
                session.language.as_deref(),
            )
            .await
        {
            Ok(response) => {
                consecutive_failures = 0;
                if !response.text.trim().is_empty() {
                    session.state.captions.lock().await.chunks.push(CaptionChunkRecord {
                        seq,
                        offset_seconds,
                        duration_seconds: CAPTION_CHUNK_SECONDS,
                        text: response.text.trim().to_string(),
                        segments: response.segments.clone(),
                    });
                    session.state.emit_event(
                        "captions.update",
                        CaptionsUpdate {
                            session_client_id: session.session_client_id.clone(),
                            seq,
                            text: response.text.trim().to_string(),
                            chunk_seconds: response.chunk_seconds,
                            remaining_seconds: Some(response.remaining_seconds),
                        },
                    );
                }
            }
            Err(CaptionChunkFailure::Terminal { code, message }) => {
                tracing::warn!("Live captions stopped ({code}): {message}");
                remove_tap();
                publish_status(
                    &session.state,
                    CaptionsStatus {
                        state: CaptionsState::Error,
                        message: Some(message),
                        remaining_seconds: None,
                        session_client_id: Some(session.session_client_id.clone()),
                    },
                )
                .await;
                return;
            }
            Err(CaptionChunkFailure::Transient { message }) => {
                consecutive_failures += 1;
                tracing::warn!(
                    "Live caption chunk failed ({consecutive_failures}/{MAX_CONSECUTIVE_FAILURES}): {message}"
                );
                if consecutive_failures >= MAX_CONSECUTIVE_FAILURES {
                    remove_tap();
                    publish_status(
                        &session.state,
                        CaptionsStatus {
                            state: CaptionsState::Error,
                            message: Some(
                                "Live captions stopped after repeated upload failures.".to_string(),
                            ),
                            remaining_seconds: None,
                            session_client_id: Some(session.session_client_id.clone()),
                        },
                    )
                    .await;
                    return;
                }
            }
        }
    }

    remove_tap();
    publish_status(&session.state, CaptionsStatus::idle()).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resample_decimates_48k_stereo_to_16k_mono() {
        // 6 stereo frames (12 samples) at 48kHz -> 2 mono samples at 16kHz.
        let samples: Vec<f32> = vec![
            0.3, 0.1, // frame 1 -> mono 0.2
            0.3, 0.1, // frame 2 -> mono 0.2
            0.3, 0.1, // frame 3 -> mono 0.2
            -0.6, -0.2, // frame 4 -> mono -0.4
            -0.6, -0.2, // frame 5 -> mono -0.4
            -0.6, -0.2, // frame 6 -> mono -0.4
        ];
        let output = downmix_resample_to_16k_mono(&samples, 2, 48_000);
        assert_eq!(output.len(), 2);
        assert!((f32::from(output[0]) / f32::from(i16::MAX) - 0.2).abs() < 0.001);
        assert!((f32::from(output[1]) / f32::from(i16::MAX) + 0.4).abs() < 0.001);
    }

    #[test]
    fn resample_handles_mono_input_and_clamps_overdrive() {
        let output = downmix_resample_to_16k_mono(&[2.0, 2.0, 2.0], 1, 48_000);
        assert_eq!(output, vec![i16::MAX]);
    }

    #[test]
    fn resample_rejects_unexpected_formats() {
        assert!(downmix_resample_to_16k_mono(&[0.0; 12], 2, 44_100).is_empty());
        assert!(downmix_resample_to_16k_mono(&[0.0; 12], 6, 48_000).is_empty());
    }

    #[test]
    fn wav_header_describes_16k_mono_s16le() {
        let wav = encode_wav_16k_mono(&[0, 1, -1]);
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..16], b"WAVEfmt ");
        assert_eq!(u16::from_le_bytes([wav[22], wav[23]]), 1); // channels
        assert_eq!(
            u32::from_le_bytes([wav[24], wav[25], wav[26], wav[27]]),
            16_000
        );
        assert_eq!(u16::from_le_bytes([wav[34], wav[35]]), 16); // bits/sample
        assert_eq!(u32::from_le_bytes([wav[40], wav[41], wav[42], wav[43]]), 6); // data bytes
        assert_eq!(wav.len(), 44 + 6);
    }

    fn encode_test_png(width: u32, height: u32) -> String {
        use base64::Engine as _;
        let mut png = Vec::new();
        let image = image::RgbaImage::from_pixel(width, height, image::Rgba([255, 0, 0, 128]));
        image::DynamicImage::ImageRgba8(image)
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .expect("test png encodes");
        base64::engine::general_purpose::STANDARD.encode(png)
    }

    #[test]
    fn anchor_resets_only_on_timestamp_regression() {
        assert!(!caption_anchor_should_reset(None, 0));
        assert!(!caption_anchor_should_reset(Some(10), 10));
        assert!(!caption_anchor_should_reset(Some(10), 11));
        assert!(caption_anchor_should_reset(Some(10), 3));
    }

    fn chunk(
        seq: u64,
        offset: f64,
        text: &str,
        segments: &[(&str, f64, f64)],
    ) -> CaptionChunkRecord {
        CaptionChunkRecord {
            seq,
            offset_seconds: offset,
            duration_seconds: 3.0,
            text: text.to_string(),
            segments: segments
                .iter()
                .map(|(word, start, end)| CaptionSegment {
                    text: (*word).to_string(),
                    start_second: *start,
                    end_second: *end,
                })
                .collect(),
        }
    }

    #[test]
    fn srt_uses_segment_timing_and_absolute_offsets() {
        let srt = render_srt(&[
            chunk(1, 0.0, "Hello viewers", &[("Hello", 0.10, 0.50), ("viewers", 0.60, 1.20)]),
            chunk(2, 3.0, "welcome back", &[("welcome", 0.05, 0.40), ("back", 0.50, 0.90)]),
        ]);
        assert_eq!(
            srt,
            "1\n00:00:00,100 --> 00:00:01,200\nHello viewers\n\n\
             2\n00:00:03,050 --> 00:00:03,900\nwelcome back\n\n"
        );
    }

    #[test]
    fn srt_falls_back_to_the_chunk_window_and_clamps_overlaps() {
        let srt = render_srt(&[
            // No segments: full chunk window 6.0-9.0…
            chunk(1, 6.0, "no timing here", &[]),
            // …but the next cue starts at 8.5, so the first must clamp.
            chunk(2, 8.0, "overlapping", &[("overlapping", 0.5, 1.5)]),
        ]);
        assert_eq!(
            srt,
            "1\n00:00:06,000 --> 00:00:08,500\nno timing here\n\n\
             2\n00:00:08,500 --> 00:00:09,500\noverlapping\n\n"
        );
    }

    #[test]
    fn srt_skips_empty_chunks_entirely() {
        assert_eq!(render_srt(&[chunk(1, 0.0, "   ", &[])]), "");
        assert_eq!(render_srt(&[]), "");
    }

    #[test]
    fn ass_renders_glass_adjacent_style_with_knobs() {
        let ass = render_ass(
            &[chunk(1, 3.0, "Hello viewers", &[("Hello", 0.1, 0.5), ("viewers", 0.6, 1.2)])],
            CaptionOverlayPosition::Top,
            CaptionTextSize::L,
        );
        assert!(ass.contains("PlayResX: 1920"));
        // L size = round(1920/38 * 1.25) = 63; top-center alignment = 8.
        assert!(ass.contains(",63,&H00F5F4F4,"));
        assert!(ass.contains(",8,60,60,43,1"));
        assert!(ass.contains("Dialogue: 0,0:00:03.10,0:00:04.20,VideorcCaptions,,0,0,0,,Hello viewers"));

        let bottom = render_ass(&[chunk(1, 0.0, "hi", &[])], CaptionOverlayPosition::Bottom, CaptionTextSize::M);
        assert!(bottom.contains(",2,60,60,43,1"));
        assert!(bottom.contains("Dialogue: 0,0:00:00.00,0:00:03.00,"));
    }

    #[test]
    fn ass_escapes_override_braces_and_newlines() {
        let ass = render_ass(
            &[chunk(1, 0.0, "{\\b1}bold\nnext", &[])],
            CaptionOverlayPosition::Bottom,
            CaptionTextSize::M,
        );
        assert!(ass.contains(",,(\\b1)bold\\Nnext\n"));
    }

    #[test]
    fn captioned_copy_path_appends_suffix() {
        assert_eq!(
            captioned_copy_path(std::path::Path::new("/tmp/Recording 12.mp4")),
            std::path::PathBuf::from("/tmp/Recording 12 (captioned).mp4")
        );
    }

    #[test]
    fn ffmpeg_filter_path_escaping_covers_specials() {
        assert_eq!(
            escape_ffmpeg_filter_path(std::path::Path::new("/a:b/c'd.ass")),
            "/a\\:b/c\\'d.ass"
        );
    }

    #[test]
    fn caption_segment_parses_web_camel_case() {
        let segment: CaptionSegment =
            serde_json::from_str(r#"{"text":"Hello","startSecond":0.02,"endSecond":0.42}"#)
                .expect("segment parses");
        assert_eq!(
            segment,
            CaptionSegment {
                text: "Hello".to_string(),
                start_second: 0.02,
                end_second: 0.42
            }
        );
    }

    #[test]
    fn overlay_installs_decodes_and_revs() {
        let slot = new_caption_overlay_slot();
        let info = install_caption_overlay(&slot, &encode_test_png(4, 2), CaptionOverlayPosition::Bottom)
            .expect("valid overlay installs");
        assert!(info.active);
        assert_eq!((info.width, info.height), (4, 2));
        assert_eq!(info.revision, 1);

        let overlay = current_caption_overlay(&slot).expect("overlay present");
        assert_eq!(overlay.rgba.len(), 4 * 2 * 4);
        assert_eq!(overlay.position, CaptionOverlayPosition::Bottom);

        let second =
            install_caption_overlay(&slot, &encode_test_png(6, 2), CaptionOverlayPosition::Top)
                .expect("replacement installs");
        assert_eq!(second.revision, 2);
    }

    #[test]
    fn overlay_rejects_garbage_and_keeps_previous() {
        let slot = new_caption_overlay_slot();
        install_caption_overlay(&slot, &encode_test_png(4, 2), CaptionOverlayPosition::Bottom)
            .expect("valid overlay installs");

        assert!(install_caption_overlay(&slot, "not base64!!!", CaptionOverlayPosition::Bottom).is_err());
        assert!(install_caption_overlay(&slot, "", CaptionOverlayPosition::Bottom).is_err());
        {
            use base64::Engine as _;
            let not_an_image = base64::engine::general_purpose::STANDARD.encode(b"plain bytes");
            assert!(
                install_caption_overlay(&slot, &not_an_image, CaptionOverlayPosition::Bottom)
                    .is_err()
            );
        }

        let survivor = current_caption_overlay(&slot).expect("previous overlay kept");
        assert_eq!((survivor.width, survivor.height), (4, 2));
        assert_eq!(survivor.revision, 1);
    }

    #[test]
    fn overlay_rejects_out_of_range_dimensions_and_clears() {
        let slot = new_caption_overlay_slot();
        assert!(
            install_caption_overlay(&slot, &encode_test_png(4200, 2), CaptionOverlayPosition::Top)
                .is_err()
        );

        install_caption_overlay(&slot, &encode_test_png(4, 2), CaptionOverlayPosition::Bottom)
            .expect("valid overlay installs");
        let cleared = clear_caption_overlay(&slot);
        assert!(!cleared.active);
        assert!(current_caption_overlay(&slot).is_none());
    }

    #[test]
    fn tap_offer_is_a_noop_when_inactive() {
        // Must never panic or block from the audio thread when captions are off.
        offer_caption_frame(&AudioFrame {
            timestamp_micros: 0,
            captured_at: std::time::Instant::now(),
            sample_rate: 48_000,
            channels: 2,
            samples: vec![0.0; 128],
        });
    }
}
