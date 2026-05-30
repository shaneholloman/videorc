use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use chrono::Utc;
use tokio::fs;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{ChildStdin, Command};
use tokio::sync::Mutex;
use tokio::time::{Duration, sleep};
use uuid::Uuid;

use crate::devices::find_avfoundation_screen_index;
use crate::protocol::{
    CameraCorner, CameraShape, CameraSize, HealthLevel, RecordingState, RecordingStatus,
    RemuxSessionParams, RtmpPreset, RtmpSettings, StartSessionParams,
};
use crate::state::AppState;
use crate::storage::NewSession;

#[derive(Debug)]
pub struct ActiveRecording {
    pub session_id: String,
    pub pid: u32,
    pub stdin: Option<ChildStdin>,
    pub output_path: Option<PathBuf>,
    pub stream_url: Option<String>,
    pub started_at: String,
    pub mode: String,
}

impl ActiveRecording {
    pub fn status(&self, state: RecordingState, message: Option<String>) -> RecordingStatus {
        RecordingStatus {
            state,
            session_id: Some(self.session_id.clone()),
            output_path: self
                .output_path
                .as_ref()
                .map(|path| path.display().to_string()),
            stream_url: self.stream_url.clone(),
            started_at: Some(self.started_at.clone()),
            message,
        }
    }
}

pub fn default_recordings_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Movies")
        .join("Videogre")
        .join("Recordings")
}

pub fn idle_status() -> RecordingStatus {
    RecordingStatus {
        state: RecordingState::Idle,
        session_id: None,
        output_path: None,
        stream_url: None,
        started_at: None,
        message: Some("Ready to start a capture session.".to_string()),
    }
}

pub async fn start_session(state: AppState, params: StartSessionParams) -> Result<RecordingStatus> {
    if state.recording.lock().await.is_some() {
        bail!("A capture session is already running");
    }

    validate_outputs(&params)?;

    let ffmpeg_path = params
        .output
        .ffmpeg_path
        .clone()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "ffmpeg".to_string());
    let output_dir = params
        .output
        .output_directory
        .clone()
        .filter(|value| !value.trim().is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(default_recordings_dir);

    if params.output.record_enabled {
        fs::create_dir_all(&output_dir)
            .await
            .with_context(|| format!("Could not create {}", output_dir.display()))?;
    }

    let session_id = Uuid::new_v4().to_string();
    let started_at = Utc::now();
    let output_path = params.output.record_enabled.then(|| {
        output_dir.join(format!(
            "videogre-session-{}.mkv",
            started_at.format("%Y%m%d-%H%M%S")
        ))
    });
    let stream_target = params
        .output
        .stream_enabled
        .then(|| build_stream_url(&params.output.rtmp))
        .transpose()?;
    let stream_url = stream_target
        .as_ref()
        .map(|target| target.redacted_url.clone());
    let mode = output_mode(params.output.record_enabled, params.output.stream_enabled);

    state.database.create_session(&NewSession {
        id: session_id.clone(),
        title: format!("Session {}", started_at.format("%Y-%m-%d %H:%M")),
        started_at: started_at.to_rfc3339(),
        mode: mode.to_string(),
        output_path: output_path.as_ref().map(|path| path.display().to_string()),
        stream_preset: params
            .output
            .stream_enabled
            .then(|| format!("{:?}", params.output.rtmp.preset)),
        sources: params.sources.clone(),
        layout: params.layout.clone(),
        output: params.output.clone(),
    })?;
    state
        .database
        .save_setting("last_capture_session", &params)?;

    emit_foundation_health_events(&state, &session_id, &params)?;

    let capture = resolve_capture_inputs(&ffmpeg_path, &params).await;
    if matches!(capture.video, VideoInput::TestPattern) {
        emit_health_event(
            &state,
            Some(&session_id),
            HealthLevel::Warn,
            "screen-capture-fallback",
            "Using FFmpeg test pattern because a macOS screen/window source was not available.",
        )?;
    }

    let args = ffmpeg_args(
        &capture,
        &params,
        output_path.as_deref(),
        stream_target.as_ref(),
    )?;

    state.emit_event(
        "recording.status",
        RecordingStatus {
            state: RecordingState::Starting,
            session_id: Some(session_id.clone()),
            output_path: output_path.as_ref().map(|path| path.display().to_string()),
            stream_url: stream_url.clone(),
            started_at: Some(started_at.to_rfc3339()),
            message: Some(format!("Starting {mode} session.")),
        },
    );

    let mut child = Command::new(&ffmpeg_path)
        .args(&args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("Could not start {ffmpeg_path}"))?;

    let stderr = child.stderr.take();
    let stdin = child.stdin.take();
    let pid = child.id().unwrap_or_default();
    let active = ActiveRecording {
        session_id: session_id.clone(),
        pid,
        stdin,
        output_path: output_path.clone(),
        stream_url,
        started_at: started_at.to_rfc3339(),
        mode: mode.to_string(),
    };
    let running_state = if params.output.stream_enabled && !params.output.record_enabled {
        RecordingState::Streaming
    } else {
        RecordingState::Recording
    };
    let running_status = active.status(running_state, Some(format!("Running {mode} session.")));

    *state.recording.lock().await = Some(active);
    state.emit_event("recording.status", running_status.clone());

    if let Some(stderr) = stderr {
        let log_state = state.clone();
        let log_session_id = session_id.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }

                log_state.emit_log("warn", trimmed);
                if looks_like_ffmpeg_health_event(trimmed) {
                    let _ = emit_health_event(
                        &log_state,
                        Some(&log_session_id),
                        HealthLevel::Warn,
                        "ffmpeg-warning",
                        trimmed,
                    );
                }
            }
        });
    }

    tokio::spawn(monitor_session(
        state.clone(),
        child,
        session_id,
        output_path,
    ));
    Ok(running_status)
}

pub async fn stop_recording(state: AppState) -> Result<RecordingStatus> {
    let mut guard = state.recording.lock().await;
    let Some(active) = guard.as_mut() else {
        return Ok(idle_status());
    };

    let pid = active.pid;
    let output_path = active.output_path.clone();
    let session_id = active.session_id.clone();
    if let Some(mut stdin) = active.stdin.take() {
        stdin
            .write_all(b"q\n")
            .await
            .context("Could not send stop command to FFmpeg")?;
        let _ = stdin.shutdown().await;
    }

    let status = active.status(
        RecordingState::Stopping,
        Some(format!("Stopping {} session.", active.mode)),
    );
    drop(guard);

    state.emit_event("recording.status", status.clone());
    tokio::spawn(stop_fallback(state.clone(), pid, session_id, output_path));
    Ok(status)
}

pub async fn remux_session(state: AppState, params: RemuxSessionParams) -> Result<String> {
    let input = state
        .database
        .session_output_path(&params.session_id)?
        .map(PathBuf::from)
        .context("Session does not have an MKV output path")?;

    if input.extension().and_then(|value| value.to_str()) != Some("mkv") {
        bail!("Only MKV session outputs can be remuxed to MP4");
    }

    let output = input.with_extension("mp4");
    let ffmpeg_path = params
        .ffmpeg_path
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "ffmpeg".to_string());

    let status = Command::new(&ffmpeg_path)
        .args([
            "-y",
            "-hide_banner",
            "-loglevel",
            "warning",
            "-i",
            &input.display().to_string(),
            "-codec",
            "copy",
            &output.display().to_string(),
        ])
        .status()
        .await
        .with_context(|| format!("Could not start {ffmpeg_path} for MP4 remux"))?;

    if !status.success() {
        bail!("FFmpeg remux failed with {status}");
    }

    state.database.finish_session(
        &params.session_id,
        "completed",
        None,
        Some(output.display().to_string()),
    )?;
    state.emit_log("info", "Created MP4 copy for session.");
    Ok(output.display().to_string())
}

async fn stop_fallback(
    state: AppState,
    pid: u32,
    session_id: String,
    output_path: Option<PathBuf>,
) {
    if pid == 0 {
        return;
    }

    sleep(Duration::from_secs(5)).await;

    let still_running = state.recording.lock().await.as_ref().is_some_and(|active| {
        active.pid == pid && active.session_id == session_id && active.output_path == output_path
    });

    if !still_running {
        return;
    }

    state.emit_log(
        "warn",
        "FFmpeg did not stop after stdin quit command; sending SIGTERM.",
    );
    let _ = Command::new("kill")
        .arg("-TERM")
        .arg(pid.to_string())
        .status()
        .await;
}

async fn monitor_session(
    state: AppState,
    mut child: tokio::process::Child,
    session_id: String,
    output_path: Option<PathBuf>,
) {
    let status = child.wait().await;
    let mut guard = state.recording.lock().await;
    let had_active_recording = guard
        .as_ref()
        .is_some_and(|active| active.session_id == session_id);
    if had_active_recording {
        guard.take();
    }
    drop(guard);

    if !had_active_recording {
        return;
    }

    let ended_at = Utc::now().to_rfc3339();
    match status {
        Ok(exit_status) if exit_status.success() => {
            state.emit_log("info", "Capture session finalized.");
            let _ = state
                .database
                .finish_session(&session_id, "completed", Some(ended_at), None);
            state.emit_event(
                "recording.status",
                RecordingStatus {
                    state: RecordingState::Idle,
                    session_id: Some(session_id),
                    output_path: output_path.as_ref().map(|path| path.display().to_string()),
                    stream_url: None,
                    started_at: None,
                    message: Some("Capture session finalized.".to_string()),
                },
            );
        }
        Ok(exit_status) => {
            let message = format!("FFmpeg exited with {exit_status}");
            state.emit_log("error", &message);
            let _ = state
                .database
                .finish_session(&session_id, "failed", Some(ended_at), None);
            let _ = emit_health_event(
                &state,
                Some(&session_id),
                HealthLevel::Error,
                "ffmpeg-exit",
                &message,
            );
            state.emit_event(
                "recording.status",
                RecordingStatus {
                    state: RecordingState::Failed,
                    session_id: Some(session_id),
                    output_path: output_path.as_ref().map(|path| path.display().to_string()),
                    stream_url: None,
                    started_at: None,
                    message: Some(message),
                },
            );
        }
        Err(error) => {
            let message = format!("Could not wait for FFmpeg: {error}");
            state.emit_log("error", &message);
            let _ = state
                .database
                .finish_session(&session_id, "failed", Some(ended_at), None);
            let _ = emit_health_event(
                &state,
                Some(&session_id),
                HealthLevel::Error,
                "ffmpeg-wait-failed",
                &message,
            );
            state.emit_event(
                "recording.status",
                RecordingStatus {
                    state: RecordingState::Failed,
                    session_id: Some(session_id),
                    output_path: output_path.as_ref().map(|path| path.display().to_string()),
                    stream_url: None,
                    started_at: None,
                    message: Some(message),
                },
            );
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CaptureInputs {
    video: VideoInput,
    camera_index: Option<usize>,
    microphone_index: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum VideoInput {
    MacScreen { index: usize },
    TestPattern,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct StreamTarget {
    url: String,
    redacted_url: String,
}

async fn resolve_capture_inputs(ffmpeg_path: &str, params: &StartSessionParams) -> CaptureInputs {
    let selected_screen = params
        .sources
        .screen_id
        .as_deref()
        .and_then(parse_avfoundation_id);
    let camera_index = params
        .sources
        .camera_id
        .as_deref()
        .and_then(parse_avfoundation_id);
    let microphone_index = params
        .sources
        .microphone_id
        .as_deref()
        .and_then(parse_avfoundation_id);
    let detected_screen = if cfg!(target_os = "macos") {
        selected_screen.or(find_avfoundation_screen_index(ffmpeg_path).await)
    } else {
        None
    };

    CaptureInputs {
        video: detected_screen
            .map(|index| VideoInput::MacScreen { index })
            .unwrap_or(VideoInput::TestPattern),
        camera_index,
        microphone_index,
    }
}

fn ffmpeg_args(
    capture: &CaptureInputs,
    params: &StartSessionParams,
    output_path: Option<&Path>,
    stream_target: Option<&StreamTarget>,
) -> Result<Vec<String>> {
    let mut args = vec![
        "-y".to_string(),
        "-hide_banner".to_string(),
        "-loglevel".to_string(),
        "warning".to_string(),
    ];
    let mut camera_input_index = None;

    let audio_map = match capture.video {
        VideoInput::MacScreen { index } => {
            args.extend([
                "-f".to_string(),
                "avfoundation".to_string(),
                "-framerate".to_string(),
                "30".to_string(),
                "-capture_cursor".to_string(),
                "1".to_string(),
                "-i".to_string(),
                format!(
                    "{}:{}",
                    index,
                    capture
                        .microphone_index
                        .map(|value| value.to_string())
                        .unwrap_or_else(|| "none".to_string())
                ),
            ]);
            "0:a?".to_string()
        }
        VideoInput::TestPattern => {
            args.extend([
                "-f".to_string(),
                "lavfi".to_string(),
                "-i".to_string(),
                "testsrc2=size=1920x1080:rate=30".to_string(),
                "-f".to_string(),
                "lavfi".to_string(),
                "-i".to_string(),
                "sine=frequency=880:sample_rate=48000".to_string(),
            ]);
            "1:a?".to_string()
        }
    };

    if let Some(camera_index) = capture.camera_index {
        camera_input_index = Some(match capture.video {
            VideoInput::MacScreen { .. } => 1,
            VideoInput::TestPattern => 2,
        });
        args.extend([
            "-f".to_string(),
            "avfoundation".to_string(),
            "-framerate".to_string(),
            "30".to_string(),
            "-i".to_string(),
            format!("{camera_index}:none"),
        ]);
    }

    if let Some(camera_input_index) = camera_input_index {
        args.extend([
            "-filter_complex".to_string(),
            camera_overlay_filter(camera_input_index, params),
            "-map".to_string(),
            "[v]".to_string(),
        ]);
    } else {
        args.extend(["-map".to_string(), "0:v:0".to_string()]);
    }
    args.extend(["-map".to_string(), audio_map]);
    args.extend([
        "-r".to_string(),
        "30".to_string(),
        "-pix_fmt".to_string(),
        "yuv420p".to_string(),
        "-c:v".to_string(),
        "h264_videotoolbox".to_string(),
        "-b:v".to_string(),
        "6000k".to_string(),
        "-c:a".to_string(),
        "aac".to_string(),
    ]);

    match (output_path, stream_target) {
        (Some(path), Some(target)) => {
            args.extend([
                "-f".to_string(),
                "tee".to_string(),
                format!("[f=matroska]{}|[f=flv]{}", path.display(), target.url),
            ]);
        }
        (Some(path), None) => args.push(path.display().to_string()),
        (None, Some(target)) => {
            args.extend(["-f".to_string(), "flv".to_string(), target.url.clone()]);
        }
        (None, None) => bail!("At least one output target is required"),
    }

    Ok(args)
}

fn camera_overlay_filter(camera_input_index: usize, params: &StartSessionParams) -> String {
    let width = match params.layout.camera_size {
        CameraSize::Small => 260,
        CameraSize::Medium => 360,
        CameraSize::Large => 480,
    };
    let margin = params.layout.camera_margin.min(160);
    let (x, y) = match params.layout.camera_corner {
        CameraCorner::TopLeft => (format!("{margin}"), format!("{margin}")),
        CameraCorner::TopRight => (format!("W-w-{margin}"), format!("{margin}")),
        CameraCorner::BottomLeft => (format!("{margin}"), format!("H-h-{margin}")),
        CameraCorner::BottomRight => (format!("W-w-{margin}"), format!("H-h-{margin}")),
    };

    format!(
        "[{camera_input_index}:v]scale={width}:-1[cam];[0:v][cam]overlay=x={x}:y={y}:format=auto[v]"
    )
}

fn validate_outputs(params: &StartSessionParams) -> Result<()> {
    if !params.output.record_enabled && !params.output.stream_enabled {
        bail!("Enable local recording, RTMP streaming, or both");
    }

    if params.output.stream_enabled {
        build_stream_url(&params.output.rtmp)?;
    }

    Ok(())
}

fn build_stream_url(settings: &RtmpSettings) -> Result<StreamTarget> {
    let server_url = settings.server_url.trim().trim_end_matches('/');
    let stream_key = settings.stream_key.trim().trim_start_matches('/');

    if server_url.is_empty() {
        bail!("RTMP server URL is required when streaming is enabled");
    }

    if stream_key.is_empty() {
        bail!("Stream key is required when streaming is enabled");
    }

    let url = format!("{server_url}/{stream_key}");
    Ok(StreamTarget {
        url,
        redacted_url: format!("{server_url}/••••"),
    })
}

fn output_mode(record_enabled: bool, stream_enabled: bool) -> &'static str {
    match (record_enabled, stream_enabled) {
        (true, true) => "record+stream",
        (true, false) => "record",
        (false, true) => "stream",
        (false, false) => "idle",
    }
}

fn parse_avfoundation_id(id: &str) -> Option<usize> {
    id.rsplit(':').next()?.parse().ok()
}

fn emit_foundation_health_events(
    state: &AppState,
    session_id: &str,
    params: &StartSessionParams,
) -> Result<()> {
    if params.sources.microphone_id.is_none() {
        emit_health_event(
            state,
            Some(session_id),
            HealthLevel::Warn,
            "microphone-not-selected",
            "No microphone is selected for this capture session.",
        )?;
    }

    if matches!(params.layout.camera_shape, CameraShape::Circle) {
        emit_health_event(
            state,
            Some(session_id),
            HealthLevel::Info,
            "camera-shape-foundation",
            "Circle camera shape is stored in the session; the FFmpeg spike records a rectangular overlay until the native compositor lands.",
        )?;
    }

    if matches!(params.output.rtmp.preset, RtmpPreset::X) && params.output.stream_enabled {
        emit_health_event(
            state,
            Some(session_id),
            HealthLevel::Info,
            "x-rtmp-access",
            "X/Twitter streaming requires an account with live RTMP access.",
        )?;
    }

    Ok(())
}

pub fn emit_health_event(
    state: &AppState,
    session_id: Option<&str>,
    level: HealthLevel,
    code: &str,
    message: &str,
) -> Result<()> {
    let event = state
        .database
        .add_health_event(session_id, level, code, message)?;
    state.emit_event("health.event", event);
    Ok(())
}

fn looks_like_ffmpeg_health_event(line: &str) -> bool {
    let normalized = line.to_lowercase();
    normalized.contains("dropped")
        || normalized.contains("overload")
        || normalized.contains("permission")
        || normalized.contains("failed")
        || normalized.contains("connection")
}

pub type RecordingSlot = Arc<Mutex<Option<ActiveRecording>>>;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{
        CameraCorner, CameraShape, CameraSize, LayoutSettings, OutputSettings, RtmpSettings,
        SourceSelection,
    };

    fn base_params(record_enabled: bool, stream_enabled: bool) -> StartSessionParams {
        StartSessionParams {
            sources: SourceSelection {
                screen_id: Some("screen:avfoundation:3".to_string()),
                window_id: None,
                camera_id: Some("camera:avfoundation:0".to_string()),
                microphone_id: Some("microphone:avfoundation:1".to_string()),
            },
            layout: LayoutSettings {
                camera_corner: CameraCorner::BottomRight,
                camera_size: CameraSize::Medium,
                camera_shape: CameraShape::Rectangle,
                camera_margin: 32,
            },
            output: OutputSettings {
                record_enabled,
                stream_enabled,
                output_directory: None,
                ffmpeg_path: None,
                rtmp: RtmpSettings {
                    preset: RtmpPreset::YouTube,
                    server_url: "rtmp://a.rtmp.youtube.com/live2".to_string(),
                    stream_key: "abc123".to_string(),
                },
            },
        }
    }

    #[test]
    fn default_recordings_dir_uses_videogre_movies_folder() {
        let path = default_recordings_dir();
        let rendered = path.display().to_string();

        assert!(rendered.contains("Movies"));
        assert!(rendered.ends_with("Videogre/Recordings"));
    }

    #[test]
    fn shared_pipeline_uses_tee_for_dual_output() {
        let params = base_params(true, true);
        let args = ffmpeg_args(
            &CaptureInputs {
                video: VideoInput::MacScreen { index: 3 },
                camera_index: Some(0),
                microphone_index: Some(1),
            },
            &params,
            Some(Path::new("/tmp/videogre-test.mkv")),
            Some(&build_stream_url(&params.output.rtmp).unwrap()),
        )
        .unwrap();

        assert!(args.contains(&"tee".to_string()));
        assert!(args.iter().any(|arg| arg.contains("[f=matroska]")));
        assert!(args.iter().any(|arg| arg.contains("[f=flv]")));
        assert!(args.contains(&"-filter_complex".to_string()));
    }

    #[test]
    fn stream_requires_manual_key() {
        let mut params = base_params(false, true);
        params.output.rtmp.stream_key.clear();

        assert!(validate_outputs(&params).is_err());
    }

    #[test]
    fn parses_avfoundation_device_ids() {
        assert_eq!(parse_avfoundation_id("camera:avfoundation:12"), Some(12));
        assert_eq!(parse_avfoundation_id("window:native-adapter-pending"), None);
    }
}
