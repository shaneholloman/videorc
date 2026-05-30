use std::path::{Path, PathBuf};
use std::process::{ExitStatus, Stdio};
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use chrono::Utc;
use tokio::fs;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{ChildStdin, ChildStdout, Command};
use tokio::sync::Mutex;
use tokio::time::{Duration, sleep, timeout};
use uuid::Uuid;

use crate::devices::find_avfoundation_screen_index;
use crate::ffmpeg::resolve_ffmpeg_path;
use crate::protocol::{
    AudioTrack, AudioTrackSource, CameraCorner, CameraFit, CameraShape, CameraSize, HealthLevel,
    PreviewLiveParams, PreviewLiveSource, PreviewLiveState, PreviewLiveStatus, PreviewSnapshot,
    PreviewSnapshotParams, RecordingState, RecordingStatus, RemuxSessionParams, RtmpPreset,
    RtmpSettings, StartSessionParams, StreamHealth, VideoPreset, VideoSettings,
};
use crate::state::AppState;
use crate::storage::{NewSession, default_preview_dir};

const PREVIEW_SNAPSHOT_TIMEOUT: Duration = Duration::from_secs(5);
const LIVE_PREVIEW_WIDTH: u32 = 1280;
const LIVE_PREVIEW_HEIGHT: u32 = 720;
const LIVE_PREVIEW_FPS: u32 = 12;
const STOP_FINALIZE_TIMEOUT: Duration = Duration::from_secs(12);
const STOP_TERM_DELAY: Duration = Duration::from_secs(3);
const STOP_KILL_DELAY: Duration = Duration::from_secs(3);
const SHUTDOWN_GRACE_DELAY: Duration = Duration::from_millis(1200);
const CAPTURE_AUDIO_FILTER: &str = "aresample=async=1:first_pts=0,volume=24dB,alimiter=limit=0.95";

#[derive(Debug)]
pub struct ActiveRecording {
    pub session_id: String,
    pub pid: u32,
    pub stdin: Option<ChildStdin>,
    pub output_path: Option<PathBuf>,
    pub stream_url: Option<String>,
    pub started_at: String,
    pub mode: String,
    pub audio_tracks: Vec<AudioTrack>,
    pub stop_requested: bool,
}

#[derive(Debug)]
pub struct LivePreviewState {
    pub status: PreviewLiveStatus,
    pub desired_params: Option<PreviewLiveParams>,
    pub idle_process: Option<ActiveLivePreview>,
}

#[derive(Debug)]
pub struct ActiveLivePreview {
    pub pid: u32,
    pub stdin: Option<ChildStdin>,
    pub first_frame_received: bool,
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
            audio_tracks: self.audio_tracks.clone(),
            message,
        }
    }
}

pub fn initial_live_preview_state() -> LivePreviewState {
    LivePreviewState {
        status: unavailable_live_preview_status(None),
        desired_params: None,
        idle_process: None,
    }
}

pub fn default_recordings_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Movies")
        .join("Videorc")
        .join("Recordings")
}

fn default_video_settings() -> VideoSettings {
    VideoSettings {
        preset: VideoPreset::Tutorial1440p30,
        width: 2560,
        height: 1440,
        fps: 30,
        bitrate_kbps: 8000,
    }
}

pub fn idle_status() -> RecordingStatus {
    RecordingStatus {
        state: RecordingState::Idle,
        session_id: None,
        output_path: None,
        stream_url: None,
        started_at: None,
        audio_tracks: Vec::new(),
        message: Some("Ready to start a capture session.".to_string()),
    }
}

pub async fn start_session(state: AppState, params: StartSessionParams) -> Result<RecordingStatus> {
    if state.recording.lock().await.is_some() {
        bail!("A capture session is already running");
    }

    validate_outputs(&params)?;

    let ffmpeg_path = resolve_ffmpeg_path(params.output.ffmpeg_path.clone());
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
            "videorc-session-{}.mkv",
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
    if params.output.record_enabled {
        emit_disk_space_health_event(&state, &session_id, &output_dir).await?;
    }

    stop_idle_live_preview_for_recording(state.clone()).await;

    let capture = resolve_capture_inputs(&ffmpeg_path, &params).await;
    let audio_tracks = capture_audio_tracks(&capture);
    if matches!(capture.video, VideoInput::TestPattern) {
        emit_health_event(
            &state,
            Some(&session_id),
            HealthLevel::Warn,
            "screen-capture-fallback",
            "Using FFmpeg test pattern because a macOS screen/window source was not available.",
        )?;
    }
    emit_audio_track_health_events(&state, &session_id, &params, &audio_tracks)?;

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
            audio_tracks: audio_tracks.clone(),
            message: Some(format!("Starting {mode} session.")),
        },
    );

    let mut child = ffmpeg_command(&ffmpeg_path)
        .args(&args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("Could not start {ffmpeg_path}"))?;

    let stderr = child.stderr.take();
    let stdout = child.stdout.take();
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
        audio_tracks,
        stop_requested: false,
    };
    let running_state = if params.output.stream_enabled && !params.output.record_enabled {
        RecordingState::Streaming
    } else {
        RecordingState::Recording
    };
    let running_status = active.status(running_state, Some(format!("Running {mode} session.")));

    *state.recording.lock().await = Some(active);
    state.emit_event("recording.status", running_status.clone());
    publish_recording_live_preview_status(&state, None).await;
    if let Some(stdout) = stdout {
        tokio::spawn(publish_preview_stdout(state.clone(), None, stdout));
    }

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
                if let Some(stream_health) = parse_ffmpeg_stream_health(&log_session_id, trimmed) {
                    if stream_health.dropped_frames.unwrap_or_default() > 0 {
                        let _ = emit_health_event(
                            &log_state,
                            Some(&log_session_id),
                            HealthLevel::Warn,
                            "stream-dropped-frames",
                            &format!(
                                "FFmpeg reports {} dropped frames.",
                                stream_health.dropped_frames.unwrap_or_default()
                            ),
                        );
                    }
                    log_state.emit_event("stream.health", stream_health);
                }
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
    let mut final_status_events = state.events.subscribe();
    let mut guard = state.recording.lock().await;
    let Some(active) = guard.as_mut() else {
        return Ok(idle_status());
    };

    let pid = active.pid;
    let output_path = active.output_path.clone();
    let session_id = active.session_id.clone();
    let wait_session_id = session_id.clone();
    let mut force_stop_now = false;
    active.stop_requested = true;
    if let Some(mut stdin) = active.stdin.take() {
        stdin
            .write_all(b"q\n")
            .await
            .context("Could not send stop command to FFmpeg")?;
        let _ = stdin.shutdown().await;
    } else {
        force_stop_now = true;
    }

    let status = active.status(
        RecordingState::Stopping,
        Some(if force_stop_now {
            format!("Force stopping {} session.", active.mode)
        } else {
            format!("Stopping {} session.", active.mode)
        }),
    );
    drop(guard);

    state.emit_event("recording.status", status.clone());
    if force_stop_now {
        state.emit_log("warn", "Stop requested again; sending SIGTERM to FFmpeg.");
        let _ = send_process_signal(pid, "TERM").await;
        tokio::spawn(stop_kill_fallback(
            state.clone(),
            pid,
            session_id,
            output_path,
        ));
    } else {
        tokio::spawn(stop_fallback(state.clone(), pid, session_id, output_path));
    }

    Ok(
        wait_for_final_recording_status(&mut final_status_events, &wait_session_id)
            .await
            .unwrap_or(status),
    )
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
    let ffmpeg_path = resolve_ffmpeg_path(params.ffmpeg_path);

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

pub async fn create_preview_snapshot(
    state: AppState,
    params: PreviewSnapshotParams,
) -> Result<PreviewSnapshot> {
    let ffmpeg_path = resolve_ffmpeg_path(params.ffmpeg_path.clone());
    let preview_id = Uuid::new_v4().to_string();
    let preview_dir = default_preview_dir();
    fs::create_dir_all(&preview_dir)
        .await
        .with_context(|| format!("Could not create {}", preview_dir.display()))?;

    let output_path = preview_file_path(&preview_id);
    let session_params = StartSessionParams {
        sources: params.sources,
        layout: params.layout,
        output: crate::protocol::OutputSettings {
            record_enabled: true,
            stream_enabled: false,
            output_directory: None,
            ffmpeg_path: Some(ffmpeg_path.clone()),
            video: default_video_settings(),
            rtmp: RtmpSettings {
                preset: RtmpPreset::Custom,
                server_url: "rtmp://preview.invalid/live".to_string(),
                stream_key: "preview".to_string(),
            },
        },
    };
    let mut capture = resolve_capture_inputs(&ffmpeg_path, &session_params).await;
    capture.microphone_index = None;
    let args = preview_ffmpeg_args(&capture, &session_params, &output_path)?;
    let output = run_preview_command(&ffmpeg_path, &args).await?;

    if !output.status.success() {
        let message = String::from_utf8_lossy(&output.stderr).trim().to_string();
        bail!(
            "Preview snapshot failed{}",
            if message.is_empty() {
                String::new()
            } else {
                format!(": {message}")
            }
        );
    }

    let snapshot = PreviewSnapshot {
        id: preview_id.clone(),
        url: format!(
            "http://127.0.0.1:{}/preview/{}?token={}",
            state.port, preview_id, state.token
        ),
        created_at: Utc::now().to_rfc3339(),
    };
    state.emit_event("preview.updated", &snapshot);
    Ok(snapshot)
}

#[derive(Debug)]
struct PreviewCommandOutput {
    status: ExitStatus,
    stderr: Vec<u8>,
}

async fn run_preview_command(ffmpeg_path: &str, args: &[String]) -> Result<PreviewCommandOutput> {
    run_preview_command_with_timeout(ffmpeg_path, args, PREVIEW_SNAPSHOT_TIMEOUT).await
}

async fn run_preview_command_with_timeout(
    ffmpeg_path: &str,
    args: &[String],
    preview_timeout: Duration,
) -> Result<PreviewCommandOutput> {
    let mut child = ffmpeg_command(ffmpeg_path)
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("Could not start {ffmpeg_path} for preview"))?;

    let stderr = child.stderr.take();
    let stderr_task = tokio::spawn(async move {
        let mut bytes = Vec::new();
        if let Some(mut stderr) = stderr {
            let _ = stderr.read_to_end(&mut bytes).await;
        }
        bytes
    });

    let status = match timeout(preview_timeout, child.wait()).await {
        Ok(status) => {
            status.with_context(|| format!("Could not wait for {ffmpeg_path} preview"))?
        }
        Err(_) => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            let stderr = stderr_task.await.unwrap_or_default();
            let message = String::from_utf8_lossy(&stderr).trim().to_string();
            bail!(
                "Preview snapshot timed out after {} seconds{}",
                preview_timeout.as_secs(),
                if message.is_empty() {
                    String::new()
                } else {
                    format!(": {message}")
                }
            );
        }
    };
    let stderr = stderr_task.await.unwrap_or_default();

    Ok(PreviewCommandOutput { status, stderr })
}

pub fn preview_file_path(preview_id: &str) -> PathBuf {
    default_preview_dir().join(format!("{preview_id}.jpg"))
}

pub async fn start_live_preview(
    state: AppState,
    params: PreviewLiveParams,
) -> Result<PreviewLiveStatus> {
    if state.recording.lock().await.is_some() {
        let status = recording_live_preview_status(&state, None);
        {
            let mut guard = state.live_preview.lock().await;
            guard.desired_params = Some(params);
            guard.status = status.clone();
        }
        state.emit_event("preview.live.status", status.clone());
        return Ok(status);
    }

    if let Some(status) = reusable_idle_live_preview_status(&state, &params).await {
        return Ok(status);
    }

    start_idle_live_preview(state, params, PreviewLiveState::Connecting).await
}

pub async fn stop_live_preview(state: AppState) -> Result<PreviewLiveStatus> {
    let process = {
        let mut guard = state.live_preview.lock().await;
        let process = guard.idle_process.take();
        guard.desired_params = None;
        guard.status = unavailable_live_preview_status(Some("Live preview stopped.".to_string()));
        process
    };
    stop_live_preview_process(process).await;

    let status = live_preview_status(&state).await;
    state.emit_event("preview.live.status", status.clone());
    Ok(status)
}

pub async fn live_preview_status(state: &AppState) -> PreviewLiveStatus {
    state.live_preview.lock().await.status.clone()
}

async fn reusable_idle_live_preview_status(
    state: &AppState,
    params: &PreviewLiveParams,
) -> Option<PreviewLiveStatus> {
    let guard = state.live_preview.lock().await;
    if should_reuse_idle_live_preview(&guard, params) {
        return Some(guard.status.clone());
    }

    None
}

fn should_reuse_idle_live_preview(preview: &LivePreviewState, params: &PreviewLiveParams) -> bool {
    preview.idle_process.is_some()
        && preview.desired_params.as_ref() == Some(params)
        && matches!(
            preview.status.state,
            PreviewLiveState::Connecting | PreviewLiveState::Live | PreviewLiveState::Reconnecting
        )
}

pub fn subscribe_live_preview_frames(
    state: &AppState,
) -> tokio::sync::broadcast::Receiver<Vec<u8>> {
    state.preview_frames.subscribe()
}

fn ffmpeg_command(ffmpeg_path: &str) -> Command {
    let mut command = Command::new(ffmpeg_path);
    command.kill_on_drop(true);
    command
}

pub async fn shutdown_capture_processes(state: AppState) {
    let idle_process = {
        let mut guard = state.live_preview.lock().await;
        guard.desired_params = None;
        guard.status = unavailable_live_preview_status(Some(
            "Backend is shutting down; live preview stopped.".to_string(),
        ));
        guard.idle_process.take()
    };
    stop_live_preview_process(idle_process).await;

    let recording = {
        let mut guard = state.recording.lock().await;
        guard.take()
    };
    stop_recording_process_for_shutdown(recording).await;
}

async fn start_idle_live_preview(
    state: AppState,
    params: PreviewLiveParams,
    starting_state: PreviewLiveState,
) -> Result<PreviewLiveStatus> {
    let old_process = {
        let mut guard = state.live_preview.lock().await;
        let old_process = guard.idle_process.take();
        guard.desired_params = Some(params.clone());
        guard.status = PreviewLiveStatus {
            state: starting_state,
            source: PreviewLiveSource::IdlePreview,
            url: Some(live_preview_url(&state)),
            message: Some("Starting live preview.".to_string()),
        };
        old_process
    };
    state.emit_event("preview.live.status", live_preview_status(&state).await);
    stop_live_preview_process(old_process).await;

    let ffmpeg_path = resolve_ffmpeg_path(params.ffmpeg_path.clone());
    let session_params = live_preview_session_params(params, ffmpeg_path.clone());
    let mut capture = resolve_capture_inputs(&ffmpeg_path, &session_params).await;
    capture.microphone_index = None;
    let args = live_preview_ffmpeg_args(&capture, &session_params)?;

    let mut command = ffmpeg_command(&ffmpeg_path);
    command
        .args(&args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            let status = unavailable_live_preview_status(Some(format!(
                "Could not start {ffmpeg_path} for live preview: {error}"
            )));
            {
                let mut guard = state.live_preview.lock().await;
                guard.status = status.clone();
                guard.idle_process = None;
            }
            state.emit_event("preview.live.status", status.clone());
            return Ok(status);
        }
    };

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let stdin = child.stdin.take();
    let pid = child.id().unwrap_or_default();

    if let Some(stdout) = stdout {
        tokio::spawn(publish_preview_stdout(state.clone(), Some(pid), stdout));
    }
    if let Some(stderr) = stderr {
        tokio::spawn(log_live_preview_stderr(state.clone(), stderr));
    }

    {
        let mut guard = state.live_preview.lock().await;
        guard.idle_process = Some(ActiveLivePreview {
            pid,
            stdin,
            first_frame_received: false,
        });
    }
    let status = live_preview_status(&state).await;
    state.emit_event("preview.live.status", status.clone());
    tokio::spawn(watch_idle_live_preview_first_frame(state.clone(), pid));
    tokio::spawn(monitor_idle_live_preview(state.clone(), child, pid));
    Ok(status)
}

fn live_preview_session_params(
    params: PreviewLiveParams,
    ffmpeg_path: String,
) -> StartSessionParams {
    StartSessionParams {
        sources: params.sources,
        layout: params.layout,
        output: crate::protocol::OutputSettings {
            record_enabled: true,
            stream_enabled: false,
            output_directory: None,
            ffmpeg_path: Some(ffmpeg_path),
            video: params.video.unwrap_or_else(default_video_settings),
            rtmp: RtmpSettings {
                preset: RtmpPreset::Custom,
                server_url: "rtmp://preview.invalid/live".to_string(),
                stream_key: "preview".to_string(),
            },
        },
    }
}

async fn stop_idle_live_preview_for_recording(state: AppState) {
    let process = {
        let mut guard = state.live_preview.lock().await;
        let process = guard.idle_process.take();
        if guard.desired_params.is_some() || process.is_some() {
            guard.status = PreviewLiveStatus {
                state: PreviewLiveState::Connecting,
                source: PreviewLiveSource::RecordingSession,
                url: Some(live_preview_url(&state)),
                message: Some("Switching preview to the recording session.".to_string()),
            };
        }
        process
    };
    if process.is_some() {
        state.emit_event("preview.live.status", live_preview_status(&state).await);
    }
    stop_live_preview_process(process).await;
}

async fn publish_recording_live_preview_status(state: &AppState, message: Option<String>) {
    let status = recording_live_preview_status(state, message);
    {
        let mut guard = state.live_preview.lock().await;
        guard.status = status.clone();
    }
    state.emit_event("preview.live.status", status);
}

async fn restart_idle_live_preview_if_desired(state: AppState) {
    let desired_params = {
        let mut guard = state.live_preview.lock().await;
        let desired_params = guard.desired_params.clone();
        if desired_params.is_some() {
            guard.status = PreviewLiveStatus {
                state: PreviewLiveState::Reconnecting,
                source: PreviewLiveSource::IdlePreview,
                url: Some(live_preview_url(&state)),
                message: Some("Restarting idle live preview.".to_string()),
            };
        } else {
            guard.status =
                unavailable_live_preview_status(Some("No live preview requested.".to_string()));
        }
        desired_params
    };
    state.emit_event("preview.live.status", live_preview_status(&state).await);

    if let Some(params) = desired_params {
        let _ = start_idle_live_preview(state, params, PreviewLiveState::Reconnecting).await;
    }
}

async fn monitor_idle_live_preview(state: AppState, mut child: tokio::process::Child, pid: u32) {
    let status = child.wait().await;
    let mut should_emit = false;
    {
        let mut guard = state.live_preview.lock().await;
        let matches_active = guard
            .idle_process
            .as_ref()
            .is_some_and(|process| process.pid == pid);
        if matches_active {
            guard.idle_process = None;
            let message = match status {
                Ok(exit_status) if exit_status.success() => "Live preview stopped.".to_string(),
                Ok(exit_status) => format!("Live preview exited with {exit_status}."),
                Err(error) => format!("Could not wait for live preview: {error}."),
            };
            guard.status = unavailable_live_preview_status(Some(message));
            should_emit = true;
        }
    }

    if should_emit {
        state.emit_event("preview.live.status", live_preview_status(&state).await);
    }
}

async fn publish_preview_stdout(state: AppState, idle_pid: Option<u32>, mut stdout: ChildStdout) {
    let mut buffer = [0_u8; 16 * 1024];
    loop {
        match stdout.read(&mut buffer).await {
            Ok(0) => break,
            Ok(read) => {
                if let Some(pid) = idle_pid
                    && buffer[..read]
                        .windows(2)
                        .any(|window| window == [0xff, 0xd8])
                {
                    mark_idle_live_preview_frame_received(&state, pid).await;
                }
                let _ = state.preview_frames.send(buffer[..read].to_vec());
            }
            Err(error) => {
                state.emit_log("warn", format!("Live preview stream read failed: {error}"));
                break;
            }
        }
    }
}

async fn mark_idle_live_preview_frame_received(state: &AppState, pid: u32) {
    let mut should_emit = false;
    {
        let mut guard = state.live_preview.lock().await;
        let should_mark_live = match guard.idle_process.as_mut() {
            Some(process) if process.pid == pid && !process.first_frame_received => {
                process.first_frame_received = true;
                true
            }
            _ => false,
        };
        if should_mark_live {
            guard.status = PreviewLiveStatus {
                state: PreviewLiveState::Live,
                source: PreviewLiveSource::IdlePreview,
                url: Some(live_preview_url(state)),
                message: Some("Live preview is receiving frames.".to_string()),
            };
            should_emit = true;
        }
    }

    if should_emit {
        state.emit_event("preview.live.status", live_preview_status(state).await);
    }
}

async fn watch_idle_live_preview_first_frame(state: AppState, pid: u32) {
    sleep(Duration::from_secs(6)).await;
    let process = {
        let mut guard = state.live_preview.lock().await;
        match guard.idle_process.as_ref() {
            Some(process) if process.pid == pid && !process.first_frame_received => {
                guard.status = unavailable_live_preview_status(Some(
                    "Live preview did not receive video frames. Check macOS screen/camera permissions or select another source."
                        .to_string(),
                ));
                guard.idle_process.take()
            }
            _ => None,
        }
    };

    if process.is_some() {
        state.emit_event("preview.live.status", live_preview_status(&state).await);
        stop_live_preview_process(process).await;
    }
}

async fn log_live_preview_stderr(state: AppState, stderr: tokio::process::ChildStderr) {
    let mut lines = BufReader::new(stderr).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        state.emit_log("warn", trimmed);
    }
}

async fn stop_live_preview_process(process: Option<ActiveLivePreview>) {
    let Some(mut process) = process else {
        return;
    };

    if let Some(mut stdin) = process.stdin.take() {
        let _ = stdin.write_all(b"q\n").await;
        let _ = stdin.shutdown().await;
    }

    if process.pid != 0 {
        if wait_for_process_exit(process.pid, Duration::from_secs(2)).await {
            return;
        }
        let _ = send_process_signal(process.pid, "TERM").await;
        if wait_for_process_exit(process.pid, Duration::from_secs(2)).await {
            return;
        }
        let _ = send_process_signal(process.pid, "KILL").await;
        let _ = wait_for_process_exit(process.pid, Duration::from_secs(1)).await;
    }
}

async fn stop_recording_process_for_shutdown(recording: Option<ActiveRecording>) {
    let Some(mut recording) = recording else {
        return;
    };

    if let Some(mut stdin) = recording.stdin.take() {
        let _ = stdin.write_all(b"q\n").await;
        let _ = stdin.shutdown().await;
    }

    if recording.pid == 0 {
        return;
    }

    sleep(SHUTDOWN_GRACE_DELAY).await;
    if !process_is_running(recording.pid).await {
        return;
    }

    let _ = send_process_signal(recording.pid, "TERM").await;
    sleep(SHUTDOWN_GRACE_DELAY).await;
    if process_is_running(recording.pid).await {
        let _ = send_process_signal(recording.pid, "KILL").await;
    }
}

fn recording_live_preview_status(state: &AppState, message: Option<String>) -> PreviewLiveStatus {
    PreviewLiveStatus {
        state: PreviewLiveState::Live,
        source: PreviewLiveSource::RecordingSession,
        url: Some(live_preview_url(state)),
        message: Some(message.unwrap_or_else(|| {
            "Live preview is following the active recording session.".to_string()
        })),
    }
}

fn unavailable_live_preview_status(message: Option<String>) -> PreviewLiveStatus {
    PreviewLiveStatus {
        state: PreviewLiveState::Unavailable,
        source: PreviewLiveSource::Unavailable,
        url: None,
        message,
    }
}

fn live_preview_url(state: &AppState) -> String {
    format!(
        "http://127.0.0.1:{}/preview/live.mjpeg?token={}",
        state.port, state.token
    )
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

    sleep(STOP_TERM_DELAY).await;

    if !recording_matches(&state, pid, &session_id, &output_path).await {
        return;
    }

    state.emit_log(
        "warn",
        "FFmpeg did not stop promptly after stdin quit command; sending SIGTERM.",
    );
    let _ = send_process_signal(pid, "TERM").await;
    stop_kill_fallback(state, pid, session_id, output_path).await;
}

async fn stop_kill_fallback(
    state: AppState,
    pid: u32,
    session_id: String,
    output_path: Option<PathBuf>,
) {
    sleep(STOP_KILL_DELAY).await;

    if !recording_matches(&state, pid, &session_id, &output_path).await {
        return;
    }

    state.emit_log(
        "warn",
        "FFmpeg did not stop after SIGTERM; sending SIGKILL.",
    );
    let _ = send_process_signal(pid, "KILL").await;
}

async fn recording_matches(
    state: &AppState,
    pid: u32,
    session_id: &str,
    output_path: &Option<PathBuf>,
) -> bool {
    state.recording.lock().await.as_ref().is_some_and(|active| {
        active.pid == pid && active.session_id == session_id && &active.output_path == output_path
    })
}

async fn wait_for_final_recording_status(
    events: &mut tokio::sync::broadcast::Receiver<crate::protocol::ServerEvent>,
    session_id: &str,
) -> Option<RecordingStatus> {
    timeout(STOP_FINALIZE_TIMEOUT, async {
        loop {
            match events.recv().await {
                Ok(event) if event.event == "recording.status" => {
                    let Ok(status) = serde_json::from_value::<RecordingStatus>(event.payload)
                    else {
                        continue;
                    };
                    if status.session_id.as_deref() == Some(session_id)
                        && matches!(status.state, RecordingState::Idle | RecordingState::Failed)
                    {
                        return Some(status);
                    }
                }
                Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return None,
            }
        }
    })
    .await
    .ok()
    .flatten()
}

async fn send_process_signal(pid: u32, signal: &str) -> Result<()> {
    Command::new("kill")
        .arg(format!("-{signal}"))
        .arg(pid.to_string())
        .status()
        .await
        .with_context(|| format!("Could not send SIG{signal} to FFmpeg"))?;
    Ok(())
}

async fn process_is_running(pid: u32) -> bool {
    Command::new("kill")
        .arg("-0")
        .arg(pid.to_string())
        .status()
        .await
        .is_ok_and(|status| status.success())
}

async fn wait_for_process_exit(pid: u32, wait: Duration) -> bool {
    timeout(wait, async {
        while process_is_running(pid).await {
            sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .is_ok()
}

async fn monitor_session(
    state: AppState,
    mut child: tokio::process::Child,
    session_id: String,
    output_path: Option<PathBuf>,
) {
    let status = child.wait().await;
    let mut guard = state.recording.lock().await;
    let active_recording = guard
        .as_ref()
        .filter(|active| active.session_id == session_id)
        .map(|active| active.stop_requested);
    let had_active_recording = active_recording.is_some();
    let stop_requested = active_recording.unwrap_or(false);
    if had_active_recording {
        guard.take();
    }
    drop(guard);

    if !had_active_recording {
        return;
    }

    let ended_at = Utc::now().to_rfc3339();
    match status {
        Ok(exit_status) if exit_status.success() || stop_requested => {
            let message = if exit_status.success() {
                "Capture session finalized.".to_string()
            } else {
                format!("Capture session finalized after stop signal ({exit_status}).")
            };
            state.emit_log(
                if exit_status.success() {
                    "info"
                } else {
                    "warn"
                },
                &message,
            );
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
                    audio_tracks: Vec::new(),
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
                    audio_tracks: Vec::new(),
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
                    audio_tracks: Vec::new(),
                    message: Some(message),
                },
            );
        }
    }

    restart_idle_live_preview_if_desired(state).await;
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
    let selected_screen = (!params.sources.test_pattern)
        .then(|| {
            params
                .sources
                .screen_id
                .as_deref()
                .and_then(parse_avfoundation_id)
        })
        .flatten();
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
    let detected_screen = if cfg!(target_os = "macos") && !params.sources.test_pattern {
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
        "-stats".to_string(),
        "-stats_period".to_string(),
        "2".to_string(),
        "-progress".to_string(),
        "pipe:2".to_string(),
    ];
    let input_layout = append_input_args(&mut args, capture, true, &params.output.video);
    let filter = recording_video_filter(input_layout.camera_input_index, params, true);

    args.extend([
        "-filter_complex".to_string(),
        filter,
        "-map".to_string(),
        "[v_main]".to_string(),
    ]);
    append_audio_output_args(&mut args, &input_layout);
    args.extend([
        "-r".to_string(),
        params.output.video.fps.to_string(),
        "-pix_fmt".to_string(),
        "yuv420p".to_string(),
        "-c:v".to_string(),
        "h264_videotoolbox".to_string(),
        "-b:v".to_string(),
        format!("{}k", params.output.video.bitrate_kbps),
        "-maxrate".to_string(),
        format!("{}k", params.output.video.bitrate_kbps),
        "-bufsize".to_string(),
        format!("{}k", params.output.video.bitrate_kbps.saturating_mul(2)),
    ]);
    append_audio_encoding_args(&mut args, &input_layout, stream_target.is_some());

    match (output_path, stream_target) {
        (Some(path), Some(target)) => {
            args.extend([
                "-f".to_string(),
                "tee".to_string(),
                format!(
                    "[f=matroska:onfail=abort]{}|[f=flv:onfail=ignore:flvflags=no_duration_filesize]{}",
                    path.display(),
                    target.url
                ),
            ]);
        }
        (Some(path), None) => args.push(path.display().to_string()),
        (None, Some(target)) => {
            args.extend([
                "-flvflags".to_string(),
                "no_duration_filesize".to_string(),
                "-f".to_string(),
                "flv".to_string(),
                target.url.clone(),
            ]);
        }
        (None, None) => bail!("At least one output target is required"),
    }

    args.extend(["-map".to_string(), "[preview]".to_string()]);
    append_live_preview_output_args(&mut args);

    Ok(args)
}

fn preview_ffmpeg_args(
    capture: &CaptureInputs,
    params: &StartSessionParams,
    output_path: &Path,
) -> Result<Vec<String>> {
    let mut args = vec![
        "-y".to_string(),
        "-hide_banner".to_string(),
        "-loglevel".to_string(),
        "warning".to_string(),
    ];
    let input_layout = append_input_args(&mut args, capture, false, &params.output.video);
    args.extend([
        "-filter_complex".to_string(),
        video_filter(input_layout.camera_input_index, params, true),
        "-map".to_string(),
        "[v]".to_string(),
        "-frames:v".to_string(),
        "1".to_string(),
        "-q:v".to_string(),
        "4".to_string(),
        "-update".to_string(),
        "1".to_string(),
        output_path.display().to_string(),
    ]);
    Ok(args)
}

fn live_preview_ffmpeg_args(
    capture: &CaptureInputs,
    params: &StartSessionParams,
) -> Result<Vec<String>> {
    let mut args = vec![
        "-y".to_string(),
        "-hide_banner".to_string(),
        "-loglevel".to_string(),
        "warning".to_string(),
    ];
    let input_layout = append_input_args(&mut args, capture, false, &params.output.video);
    args.extend([
        "-filter_complex".to_string(),
        live_preview_filter(input_layout.camera_input_index, params),
        "-map".to_string(),
        "[preview]".to_string(),
    ]);
    append_live_preview_output_args(&mut args);
    Ok(args)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct InputLayout {
    camera_input_index: Option<usize>,
    audio_inputs: Vec<AudioInput>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AudioInput {
    input_index: usize,
    track: AudioTrack,
}

fn append_input_args(
    args: &mut Vec<String>,
    capture: &CaptureInputs,
    include_audio: bool,
    video: &VideoSettings,
) -> InputLayout {
    let mut next_input_index = 0;
    let mut audio_inputs = Vec::new();

    match capture.video {
        VideoInput::MacScreen { index } => {
            args.extend([
                "-f".to_string(),
                "avfoundation".to_string(),
                "-framerate".to_string(),
                video.fps.to_string(),
                "-capture_cursor".to_string(),
                "1".to_string(),
                "-i".to_string(),
                format!("{index}:none"),
            ]);
            next_input_index += 1;

            if include_audio && let Some(microphone_index) = capture.microphone_index {
                args.extend([
                    "-f".to_string(),
                    "avfoundation".to_string(),
                    "-thread_queue_size".to_string(),
                    "512".to_string(),
                    "-i".to_string(),
                    format!(":{microphone_index}"),
                ]);
                audio_inputs.push(AudioInput {
                    input_index: next_input_index,
                    track: microphone_audio_track(),
                });
                next_input_index += 1;
            }
        }
        VideoInput::TestPattern => {
            args.extend([
                "-f".to_string(),
                "lavfi".to_string(),
                "-i".to_string(),
                format!(
                    "testsrc2=size={}x{}:rate={}",
                    video.width, video.height, video.fps
                ),
            ]);
            next_input_index += 1;

            if include_audio {
                if let Some(microphone_index) = capture.microphone_index {
                    args.extend([
                        "-f".to_string(),
                        "avfoundation".to_string(),
                        "-thread_queue_size".to_string(),
                        "512".to_string(),
                        "-i".to_string(),
                        format!(":{microphone_index}"),
                    ]);
                    audio_inputs.push(AudioInput {
                        input_index: next_input_index,
                        track: microphone_audio_track(),
                    });
                    next_input_index += 1;
                } else {
                    args.extend([
                        "-f".to_string(),
                        "lavfi".to_string(),
                        "-i".to_string(),
                        "sine=frequency=880:sample_rate=48000".to_string(),
                    ]);
                    audio_inputs.push(AudioInput {
                        input_index: next_input_index,
                        track: test_tone_audio_track(),
                    });
                    next_input_index += 1;
                }
            }
        }
    };

    let camera_input_index = capture.camera_index.map(|camera_index| {
        let input_index = next_input_index;
        args.extend([
            "-f".to_string(),
            "avfoundation".to_string(),
            "-framerate".to_string(),
            video.fps.to_string(),
            "-i".to_string(),
            format!("{camera_index}:none"),
        ]);
        input_index
    });

    InputLayout {
        camera_input_index,
        audio_inputs,
    }
}

fn append_audio_output_args(args: &mut Vec<String>, input_layout: &InputLayout) {
    for (track_index, audio_input) in input_layout.audio_inputs.iter().enumerate() {
        args.extend([
            "-map".to_string(),
            format!("{}:a?", audio_input.input_index),
            format!("-metadata:s:a:{track_index}"),
            format!("title={}", audio_input.track.label),
        ]);
    }
}

fn append_audio_encoding_args(args: &mut Vec<String>, input_layout: &InputLayout, streaming: bool) {
    if input_layout.audio_inputs.is_empty() {
        return;
    }

    args.extend([
        "-af".to_string(),
        CAPTURE_AUDIO_FILTER.to_string(),
        "-ar".to_string(),
        "48000".to_string(),
        "-ac".to_string(),
        "1".to_string(),
        "-c:a".to_string(),
        if streaming { "aac" } else { "pcm_s16le" }.to_string(),
    ]);

    if streaming {
        args.extend(["-b:a".to_string(), "160k".to_string()]);
    }
}

fn append_live_preview_output_args(args: &mut Vec<String>) {
    args.extend([
        "-an".to_string(),
        "-c:v".to_string(),
        "mjpeg".to_string(),
        "-q:v".to_string(),
        "6".to_string(),
        "-f".to_string(),
        "mpjpeg".to_string(),
        "-boundary_tag".to_string(),
        "videorc".to_string(),
        "pipe:1".to_string(),
    ]);
}

fn capture_audio_tracks(capture: &CaptureInputs) -> Vec<AudioTrack> {
    if capture.microphone_index.is_some() {
        return vec![microphone_audio_track()];
    }

    if matches!(capture.video, VideoInput::TestPattern) {
        return vec![test_tone_audio_track()];
    }

    Vec::new()
}

fn microphone_audio_track() -> AudioTrack {
    AudioTrack {
        id: "microphone".to_string(),
        label: "Microphone".to_string(),
        source: AudioTrackSource::Microphone,
    }
}

fn test_tone_audio_track() -> AudioTrack {
    AudioTrack {
        id: "test-tone".to_string(),
        label: "Test tone".to_string(),
        source: AudioTrackSource::TestTone,
    }
}

fn recording_video_filter(
    camera_input_index: Option<usize>,
    params: &StartSessionParams,
    include_live_preview: bool,
) -> String {
    let scene = video_filter(camera_input_index, params, false);
    if include_live_preview {
        format!(
            "{scene};[v]split=2[v_main][v_preview];[v_preview]{}[preview]",
            live_preview_scale_filter()
        )
    } else {
        format!("{scene};[v]null[v_main]")
    }
}

fn live_preview_filter(camera_input_index: Option<usize>, params: &StartSessionParams) -> String {
    format!(
        "{};[v]{}[preview]",
        video_filter(camera_input_index, params, false),
        live_preview_scale_filter()
    )
}

fn live_preview_scale_filter() -> String {
    format!(
        "scale=w={LIVE_PREVIEW_WIDTH}:h={LIVE_PREVIEW_HEIGHT}:force_original_aspect_ratio=decrease,pad={LIVE_PREVIEW_WIDTH}:{LIVE_PREVIEW_HEIGHT}:(ow-iw)/2:(oh-ih)/2,fps={LIVE_PREVIEW_FPS}"
    )
}

fn video_filter(
    camera_input_index: Option<usize>,
    params: &StartSessionParams,
    preview: bool,
) -> String {
    let base_scale = if preview {
        "scale=w=960:h=-2".to_string()
    } else {
        output_scale_filter(&params.output.video)
    };

    if let Some(camera_input_index) = camera_input_index {
        let camera = camera_chain_filter(camera_input_index, params);
        let margin = params.layout.camera_margin.min(160);
        let (x, y) = match params.layout.camera_corner {
            CameraCorner::TopLeft => (format!("{margin}"), format!("{margin}")),
            CameraCorner::TopRight => (format!("W-w-{margin}"), format!("{margin}")),
            CameraCorner::BottomLeft => (format!("{margin}"), format!("H-h-{margin}")),
            CameraCorner::BottomRight => (format!("W-w-{margin}"), format!("H-h-{margin}")),
        };
        let final_scale = if preview { ",scale=w=960:h=-2" } else { "" };

        return format!(
            "[0:v]{base_scale},fps={}[base];{camera};[base][cam]overlay=x={x}:y={y}:format=auto{final_scale}[v]",
            params.output.video.fps
        );
    }

    format!("[0:v]{base_scale},fps={}[v]", params.output.video.fps)
}

fn output_scale_filter(video: &VideoSettings) -> String {
    format!(
        "scale=w={}:h={}:force_original_aspect_ratio=decrease,pad={}:{}:(ow-iw)/2:(oh-ih)/2",
        video.width, video.height, video.width, video.height
    )
}

fn camera_chain_filter(camera_input_index: usize, params: &StartSessionParams) -> String {
    let (width, height) = camera_box_size(&params.layout.camera_size, &params.layout.camera_shape);
    let zoom = params.layout.camera_zoom.clamp(100, 200);
    let scaled_width = width * zoom / 100;
    let scaled_height = height * zoom / 100;
    let prefix = if params.layout.camera_mirror {
        format!("[{camera_input_index}:v]hflip,")
    } else {
        format!("[{camera_input_index}:v]")
    };
    let frame = match params.layout.camera_fit {
        CameraFit::Fit if zoom == 100 => format!(
            "scale={width}:{height}:force_original_aspect_ratio=decrease,pad={width}:{height}:(ow-iw)/2:(oh-ih)/2"
        ),
        CameraFit::Fit | CameraFit::Fill => format!(
            "scale={scaled_width}:{scaled_height}:force_original_aspect_ratio=increase,crop=w={width}:h={height}:x='{}':y='{}'",
            crop_offset_expr(params.layout.camera_offset_x, "iw", "ow"),
            crop_offset_expr(params.layout.camera_offset_y, "ih", "oh")
        ),
    };

    match params.layout.camera_shape {
        CameraShape::Rectangle => format!("{prefix}{frame}[cam]"),
        CameraShape::Circle => {
            let radius = width / 2;
            format!(
                "{prefix}{frame},format=rgba,geq=r='r(X,Y)':g='g(X,Y)':b='b(X,Y)':a='if(lte((X-{radius})*(X-{radius})+(Y-{radius})*(Y-{radius}),{radius}*{radius}),255,0)'[cam]"
            )
        }
    }
}

fn camera_box_size(size: &CameraSize, shape: &CameraShape) -> (u32, u32) {
    let width = match size {
        CameraSize::Small => 260,
        CameraSize::Medium => 360,
        CameraSize::Large => 480,
    };
    let height = match shape {
        CameraShape::Rectangle => (width * 9 + 8) / 16,
        CameraShape::Circle => width,
    };

    (width, height)
}

fn crop_offset_expr(offset: i32, input_size: &str, output_size: &str) -> String {
    let offset = offset.clamp(-100, 100);
    format!("({input_size}-{output_size})/2+({offset})*({input_size}-{output_size})/200")
}

fn validate_outputs(params: &StartSessionParams) -> Result<()> {
    if !params.output.record_enabled && !params.output.stream_enabled {
        bail!("Enable local recording, RTMP streaming, or both");
    }

    if params.output.stream_enabled {
        build_stream_url(&params.output.rtmp)?;
    }

    validate_video_settings(&params.output.video)?;

    Ok(())
}

fn validate_video_settings(video: &VideoSettings) -> Result<()> {
    if !(640..=3840).contains(&video.width) || !(360..=2160).contains(&video.height) {
        bail!("Video resolution must be between 640x360 and 3840x2160");
    }

    if !(24..=60).contains(&video.fps) {
        bail!("Video FPS must be between 24 and 60");
    }

    if !(1_000..=50_000).contains(&video.bitrate_kbps) {
        bail!("Video bitrate must be between 1000 and 50000 kbps");
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
            "camera-shape-circle",
            "Circle camera shape is applied with an FFmpeg alpha mask in the current preview/recording path.",
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

fn emit_audio_track_health_events(
    state: &AppState,
    session_id: &str,
    params: &StartSessionParams,
    audio_tracks: &[AudioTrack],
) -> Result<()> {
    if !params.output.record_enabled {
        return Ok(());
    }

    if audio_tracks.is_empty() {
        return Ok(());
    }

    let labels = audio_tracks
        .iter()
        .map(|track| track.label.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    emit_health_event(
        state,
        Some(session_id),
        HealthLevel::Info,
        "audio-tracks-separated",
        &format!("Local MKV will preserve separate audio track(s): {labels}."),
    )?;

    Ok(())
}

async fn emit_disk_space_health_event(
    state: &AppState,
    session_id: &str,
    output_dir: &Path,
) -> Result<()> {
    let output = Command::new("df")
        .args(["-Pk", &output_dir.display().to_string()])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .await;

    let Ok(output) = output else {
        return Ok(());
    };

    if !output.status.success() {
        return Ok(());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let Some(line) = stdout.lines().nth(1) else {
        return Ok(());
    };
    let columns = line.split_whitespace().collect::<Vec<_>>();
    let Some(available_kb) = columns.get(3).and_then(|value| value.parse::<u64>().ok()) else {
        return Ok(());
    };

    if available_kb < 1_048_576 {
        emit_health_event(
            state,
            Some(session_id),
            HealthLevel::Warn,
            "disk-space-low",
            "Less than 1 GB is available in the selected recording location.",
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

fn parse_ffmpeg_stream_health(session_id: &str, line: &str) -> Option<StreamHealth> {
    let fps = parse_stat_f64(line, "fps=");
    let dropped_frames =
        parse_stat_u64(line, "drop_frames=").or_else(|| parse_stat_u64(line, "drop="));
    let speed = parse_stat_f64(line, "speed=");

    if fps.is_none() && dropped_frames.is_none() && speed.is_none() {
        return None;
    }

    Some(StreamHealth {
        session_id: session_id.to_string(),
        fps,
        dropped_frames,
        speed,
        created_at: Utc::now().to_rfc3339(),
    })
}

fn parse_stat_f64(line: &str, label: &str) -> Option<f64> {
    stat_value(line, label)?
        .trim_end_matches('x')
        .parse::<f64>()
        .ok()
}

fn parse_stat_u64(line: &str, label: &str) -> Option<u64> {
    stat_value(line, label)?.parse::<u64>().ok()
}

fn stat_value<'a>(line: &'a str, label: &str) -> Option<&'a str> {
    let start = line.find(label)? + label.len();
    line[start..].split_whitespace().next()
}

pub type RecordingSlot = Arc<Mutex<Option<ActiveRecording>>>;
pub type LivePreviewSlot = Arc<Mutex<LivePreviewState>>;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{
        CameraCorner, CameraFit, CameraShape, CameraSize, LayoutSettings, OutputSettings,
        PreviewLiveParams, RtmpSettings, SourceSelection,
    };

    fn base_params(record_enabled: bool, stream_enabled: bool) -> StartSessionParams {
        StartSessionParams {
            sources: SourceSelection {
                screen_id: Some("screen:avfoundation:3".to_string()),
                window_id: None,
                camera_id: Some("camera:avfoundation:0".to_string()),
                microphone_id: Some("microphone:avfoundation:1".to_string()),
                test_pattern: false,
            },
            layout: LayoutSettings {
                camera_corner: CameraCorner::BottomRight,
                camera_size: CameraSize::Medium,
                camera_shape: CameraShape::Rectangle,
                camera_margin: 32,
                camera_fit: CameraFit::Fill,
                camera_mirror: false,
                camera_zoom: 100,
                camera_offset_x: 0,
                camera_offset_y: 0,
            },
            output: OutputSettings {
                record_enabled,
                stream_enabled,
                output_directory: None,
                ffmpeg_path: None,
                video: default_video_settings(),
                rtmp: RtmpSettings {
                    preset: RtmpPreset::YouTube,
                    server_url: "rtmp://a.rtmp.youtube.com/live2".to_string(),
                    stream_key: "abc123".to_string(),
                },
            },
        }
    }

    fn ffmpeg_inputs(args: &[String]) -> Vec<&str> {
        args.windows(2)
            .filter_map(|pair| (pair[0] == "-i").then_some(pair[1].as_str()))
            .collect()
    }

    fn arg_value<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
        args.windows(2)
            .find_map(|pair| (pair[0] == name).then_some(pair[1].as_str()))
    }

    #[test]
    fn default_recordings_dir_uses_videorc_movies_folder() {
        let path = default_recordings_dir();
        let rendered = path.display().to_string();

        assert!(rendered.contains("Movies"));
        assert!(rendered.ends_with("Videorc/Recordings"));
    }

    #[test]
    fn live_preview_initial_status_is_unavailable() {
        let state = initial_live_preview_state();

        assert_eq!(state.status.state, PreviewLiveState::Unavailable);
        assert_eq!(state.status.source, PreviewLiveSource::Unavailable);
        assert!(state.status.url.is_none());
    }

    #[test]
    fn same_connecting_idle_preview_is_reused() {
        let params = PreviewLiveParams {
            sources: base_params(true, false).sources,
            layout: base_params(true, false).layout,
            ffmpeg_path: None,
            video: Some(default_video_settings()),
        };
        let state = LivePreviewState {
            status: PreviewLiveStatus {
                state: PreviewLiveState::Connecting,
                source: PreviewLiveSource::IdlePreview,
                url: Some("http://127.0.0.1:1234/preview/live.mjpeg?token=test".to_string()),
                message: Some("Starting live preview.".to_string()),
            },
            desired_params: Some(params.clone()),
            idle_process: Some(ActiveLivePreview {
                pid: 123,
                stdin: None,
                first_frame_received: false,
            }),
        };

        assert!(should_reuse_idle_live_preview(&state, &params));
    }

    #[test]
    fn unavailable_idle_preview_is_not_reused() {
        let params = PreviewLiveParams {
            sources: base_params(true, false).sources,
            layout: base_params(true, false).layout,
            ffmpeg_path: None,
            video: Some(default_video_settings()),
        };
        let state = LivePreviewState {
            status: unavailable_live_preview_status(Some("No frames.".to_string())),
            desired_params: Some(params.clone()),
            idle_process: Some(ActiveLivePreview {
                pid: 123,
                stdin: None,
                first_frame_received: false,
            }),
        };

        assert!(!should_reuse_idle_live_preview(&state, &params));
    }

    #[tokio::test]
    async fn stop_waits_for_final_recording_status_event() {
        let (events, _) = tokio::sync::broadcast::channel(8);
        let mut receiver = events.subscribe();
        let session_id = "session";

        events
            .send(crate::protocol::ServerEvent::new(
                "recording.status",
                RecordingStatus {
                    state: RecordingState::Stopping,
                    session_id: Some(session_id.to_string()),
                    output_path: None,
                    stream_url: None,
                    started_at: None,
                    audio_tracks: Vec::new(),
                    message: Some("Stopping.".to_string()),
                },
            ))
            .unwrap();
        events
            .send(crate::protocol::ServerEvent::new(
                "recording.status",
                RecordingStatus {
                    state: RecordingState::Idle,
                    session_id: Some(session_id.to_string()),
                    output_path: Some("/tmp/videorc-test.mkv".to_string()),
                    stream_url: None,
                    started_at: None,
                    audio_tracks: Vec::new(),
                    message: Some("Capture session finalized.".to_string()),
                },
            ))
            .unwrap();

        let status = wait_for_final_recording_status(&mut receiver, session_id)
            .await
            .unwrap();

        assert!(matches!(status.state, RecordingState::Idle));
        assert_eq!(status.output_path.as_deref(), Some("/tmp/videorc-test.mkv"));
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
            Some(Path::new("/tmp/videorc-test.mkv")),
            Some(&build_stream_url(&params.output.rtmp).unwrap()),
        )
        .unwrap();

        assert!(args.contains(&"tee".to_string()));
        assert!(args.iter().any(|arg| arg.contains("[f=matroska")));
        assert!(args.iter().any(|arg| arg.contains("[f=flv")));
        assert!(args.contains(&"-filter_complex".to_string()));
        assert_eq!(ffmpeg_inputs(&args), vec!["3:none", ":1", "0:none"]);
        assert!(args.iter().any(|arg| arg == "1:a?"));
        assert!(args.iter().any(|arg| arg.contains("[2:v]")));
        assert!(args.iter().any(|arg| arg == "title=Microphone"));
        assert_eq!(arg_value(&args, "-af"), Some(CAPTURE_AUDIO_FILTER));
        assert_eq!(arg_value(&args, "-ar"), Some("48000"));
        assert_eq!(arg_value(&args, "-ac"), Some("1"));
        assert_eq!(arg_value(&args, "-c:a"), Some("aac"));
        assert_eq!(arg_value(&args, "-b:a"), Some("160k"));
        assert!(args.contains(&"8000k".to_string()));
        assert!(args.iter().any(|arg| arg.contains("pad=2560:1440")));
        assert!(args.contains(&"pipe:2".to_string()));
        assert!(args.contains(&"pipe:1".to_string()));
        assert!(args.iter().any(|arg| arg.contains("[v]split=2")));
        assert!(args.iter().any(|arg| arg == "[preview]"));
    }

    #[test]
    fn mac_recording_uses_dedicated_microphone_audio_input() {
        let params = base_params(true, false);
        let args = ffmpeg_args(
            &CaptureInputs {
                video: VideoInput::MacScreen { index: 3 },
                camera_index: None,
                microphone_index: Some(1),
            },
            &params,
            Some(Path::new("/tmp/videorc-test.mkv")),
            None,
        )
        .unwrap();

        assert_eq!(ffmpeg_inputs(&args), vec!["3:none", ":1"]);
        assert!(args.iter().any(|arg| arg == "1:a?"));
        assert!(args.iter().any(|arg| arg == "-metadata:s:a:0"));
        assert!(args.iter().any(|arg| arg == "title=Microphone"));
        assert_eq!(arg_value(&args, "-af"), Some(CAPTURE_AUDIO_FILTER));
        assert_eq!(arg_value(&args, "-ar"), Some("48000"));
        assert_eq!(arg_value(&args, "-ac"), Some("1"));
        assert_eq!(arg_value(&args, "-c:a"), Some("pcm_s16le"));
        assert_eq!(arg_value(&args, "-b:a"), None);
    }

    #[test]
    fn mac_recording_without_mic_is_video_only() {
        let params = base_params(true, false);
        let args = ffmpeg_args(
            &CaptureInputs {
                video: VideoInput::MacScreen { index: 3 },
                camera_index: None,
                microphone_index: None,
            },
            &params,
            Some(Path::new("/tmp/videorc-test.mkv")),
            None,
        )
        .unwrap();

        assert_eq!(ffmpeg_inputs(&args), vec!["3:none"]);
        assert!(!args.iter().any(|arg| arg.ends_with(":a?")));
        assert_eq!(arg_value(&args, "-af"), None);
        assert_eq!(arg_value(&args, "-c:a"), None);
        assert!(args.contains(&"pipe:1".to_string()));
    }

    #[test]
    fn recording_command_includes_muted_mjpeg_live_preview_output() {
        let params = base_params(true, false);
        let args = ffmpeg_args(
            &CaptureInputs {
                video: VideoInput::MacScreen { index: 3 },
                camera_index: None,
                microphone_index: Some(1),
            },
            &params,
            Some(Path::new("/tmp/videorc-test.mkv")),
            None,
        )
        .unwrap();

        assert!(args.iter().any(|arg| arg.contains("[v]split=2")));
        assert!(args.iter().any(|arg| arg.contains("pad=1280:720")));
        assert!(args.iter().any(|arg| arg == "-an"));
        assert!(args.iter().any(|arg| arg == "mjpeg"));
        assert!(args.iter().any(|arg| arg == "mpjpeg"));
        assert!(args.iter().any(|arg| arg == "videorc"));
        assert!(args.iter().any(|arg| arg == "pipe:1"));
    }

    #[test]
    fn idle_live_preview_command_is_video_only() {
        let params = base_params(true, false);
        let args = live_preview_ffmpeg_args(
            &CaptureInputs {
                video: VideoInput::MacScreen { index: 3 },
                camera_index: Some(0),
                microphone_index: Some(1),
            },
            &params,
        )
        .unwrap();

        assert_eq!(ffmpeg_inputs(&args), vec!["3:none", "0:none"]);
        assert!(!args.iter().any(|arg| arg.ends_with(":a?")));
        assert!(args.iter().any(|arg| arg.contains("pad=1280:720")));
        assert!(args.iter().any(|arg| arg == "[preview]"));
        assert!(args.iter().any(|arg| arg == "pipe:1"));
    }

    #[test]
    fn test_pattern_uses_mic_when_selected_otherwise_test_tone() {
        let params = base_params(true, false);
        let with_mic = ffmpeg_args(
            &CaptureInputs {
                video: VideoInput::TestPattern,
                camera_index: None,
                microphone_index: Some(1),
            },
            &params,
            Some(Path::new("/tmp/videorc-test.mkv")),
            None,
        )
        .unwrap();
        let without_mic = ffmpeg_args(
            &CaptureInputs {
                video: VideoInput::TestPattern,
                camera_index: None,
                microphone_index: None,
            },
            &params,
            Some(Path::new("/tmp/videorc-test.mkv")),
            None,
        )
        .unwrap();

        assert_eq!(
            ffmpeg_inputs(&with_mic),
            vec!["testsrc2=size=2560x1440:rate=30", ":1"]
        );
        assert!(with_mic.iter().any(|arg| arg == "title=Microphone"));
        assert_eq!(arg_value(&with_mic, "-c:a"), Some("pcm_s16le"));
        assert_eq!(
            ffmpeg_inputs(&without_mic),
            vec![
                "testsrc2=size=2560x1440:rate=30",
                "sine=frequency=880:sample_rate=48000"
            ]
        );
        assert!(without_mic.iter().any(|arg| arg == "title=Test tone"));
        assert_eq!(arg_value(&without_mic, "-c:a"), Some("pcm_s16le"));
    }

    #[test]
    fn circle_camera_filter_uses_alpha_mask() {
        let mut params = base_params(true, false);
        params.layout.camera_shape = CameraShape::Circle;
        let filter = video_filter(Some(1), &params, true);

        assert!(filter.contains("format=rgba"));
        assert!(filter.contains("geq="));
        assert!(filter.contains("scale=w=960:h=-2"));
    }

    #[test]
    fn camera_filter_applies_framing_controls() {
        let mut params = base_params(true, false);
        params.layout.camera_fit = CameraFit::Fill;
        params.layout.camera_mirror = true;
        params.layout.camera_zoom = 150;
        params.layout.camera_offset_x = 40;
        params.layout.camera_offset_y = -20;
        let filter = camera_chain_filter(1, &params);

        assert!(filter.starts_with("[1:v]hflip,"));
        assert!(filter.contains("scale=540:304"));
        assert!(filter.contains("crop=w=360:h=203"));
        assert!(filter.contains("(40)*(iw-ow)/200"));
        assert!(filter.contains("(-20)*(ih-oh)/200"));
    }

    #[test]
    fn camera_fit_filter_pads_to_fixed_frame() {
        let mut params = base_params(true, false);
        params.layout.camera_fit = CameraFit::Fit;
        let filter = camera_chain_filter(1, &params);

        assert!(filter.contains("force_original_aspect_ratio=decrease"));
        assert!(filter.contains("pad=360:203"));
    }

    #[test]
    fn stream_requires_manual_key() {
        let mut params = base_params(false, true);
        params.output.rtmp.stream_key.clear();

        assert!(validate_outputs(&params).is_err());
    }

    #[test]
    fn rejects_invalid_video_settings() {
        let mut params = base_params(true, false);
        params.output.video.fps = 120;

        assert!(validate_outputs(&params).is_err());
    }

    #[tokio::test]
    async fn preview_command_times_out() {
        let args = vec!["-c".to_string(), "sleep 5".to_string()];
        let error = run_preview_command_with_timeout("sh", &args, Duration::from_millis(100))
            .await
            .unwrap_err();

        assert!(error.to_string().contains("Preview snapshot timed out"));
    }

    #[test]
    fn parses_avfoundation_device_ids() {
        assert_eq!(parse_avfoundation_id("camera:avfoundation:12"), Some(12));
        assert_eq!(parse_avfoundation_id("window:native-adapter-pending"), None);
    }

    #[test]
    fn parses_ffmpeg_stream_health() {
        let health = parse_ffmpeg_stream_health(
            "session",
            "frame=151 fps=29.97 q=-0.0 size=1024kB drop=3 speed=1.02x",
        )
        .unwrap();

        assert_eq!(health.fps, Some(29.97));
        assert_eq!(health.dropped_frames, Some(3));
        assert_eq!(health.speed, Some(1.02));

        let progress_health = parse_ffmpeg_stream_health("session", "drop_frames=7").unwrap();
        assert_eq!(progress_health.dropped_frames, Some(7));
    }
}
