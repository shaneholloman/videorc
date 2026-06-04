use chrono::Utc;

use crate::protocol::{
    DiagnosticBottleneck, DiagnosticStats, PermissionPane, PreviewTransport, StreamHealth,
};

pub fn idle_diagnostics() -> DiagnosticStats {
    DiagnosticStats {
        session_id: None,
        target_fps: None,
        capture_fps: None,
        render_fps: None,
        skipped_frames: 0,
        dropped_frames: 0,
        encoder_speed: None,
        preview_target_fps: None,
        preview_frame_age_ms: None,
        preview_transport: PreviewTransport::Unavailable,
        preview_latency_ms: None,
        preview_dropped_frames: 0,
        mic_captured_frames: None,
        mic_dropped_frames: 0,
        device_disconnected: false,
        bottleneck: DiagnosticBottleneck::None,
        updated_at: Utc::now().to_rfc3339(),
    }
}

pub fn starting_diagnostics(session_id: &str, target_fps: u32) -> DiagnosticStats {
    DiagnosticStats {
        session_id: Some(session_id.to_string()),
        target_fps: Some(f64::from(target_fps)),
        bottleneck: DiagnosticBottleneck::Unknown,
        updated_at: Utc::now().to_rfc3339(),
        ..idle_diagnostics()
    }
}

pub fn apply_stream_health(
    mut stats: DiagnosticStats,
    health: &StreamHealth,
    target_fps: u32,
) -> DiagnosticStats {
    stats.session_id = Some(health.session_id.clone());
    stats.target_fps = Some(f64::from(target_fps));
    if let Some(fps) = health.fps {
        stats.capture_fps = Some(fps);
        stats.render_fps = Some(fps);
    }
    if let Some(dropped_frames) = health.dropped_frames {
        stats.dropped_frames = dropped_frames;
        stats.skipped_frames = dropped_frames;
    }
    stats.encoder_speed = health.speed;
    stats.bottleneck = classify_bottleneck(
        stats.capture_fps,
        stats.render_fps,
        stats.encoder_speed,
        stats.dropped_frames,
        stats.mic_dropped_frames,
        target_fps,
        stats.device_disconnected,
    );
    stats.updated_at = Utc::now().to_rfc3339();
    stats
}

pub fn apply_preview_stats(
    mut stats: DiagnosticStats,
    preview_latency_ms: Option<u64>,
    preview_dropped_frames: u64,
    preview_target_fps: Option<f64>,
    preview_transport: PreviewTransport,
) -> DiagnosticStats {
    stats.preview_latency_ms = preview_latency_ms;
    stats.preview_dropped_frames = preview_dropped_frames;
    stats.preview_target_fps = preview_target_fps;
    stats.preview_transport = preview_transport;
    if preview_dropped_frames > 0 {
        stats.bottleneck = DiagnosticBottleneck::Preview;
    }
    stats.updated_at = Utc::now().to_rfc3339();
    stats
}

pub fn apply_preview_frame_age(
    mut stats: DiagnosticStats,
    preview_frame_age_ms: u64,
) -> DiagnosticStats {
    stats.preview_frame_age_ms = Some(preview_frame_age_ms);
    stats.updated_at = Utc::now().to_rfc3339();
    stats
}

pub fn apply_audio_stats(
    mut stats: DiagnosticStats,
    captured_frames: u64,
    dropped_frames: u64,
) -> DiagnosticStats {
    stats.mic_captured_frames = Some(captured_frames);
    stats.mic_dropped_frames = dropped_frames;
    if dropped_frames > 0 {
        stats.bottleneck = DiagnosticBottleneck::Audio;
    }
    stats.updated_at = Utc::now().to_rfc3339();
    stats
}

pub fn classify_bottleneck(
    capture_fps: Option<f64>,
    render_fps: Option<f64>,
    encoder_speed: Option<f64>,
    dropped_frames: u64,
    mic_dropped_frames: u64,
    target_fps: u32,
    device_disconnected: bool,
) -> DiagnosticBottleneck {
    if device_disconnected {
        return DiagnosticBottleneck::Device;
    }
    if mic_dropped_frames > 0 {
        return DiagnosticBottleneck::Audio;
    }
    if encoder_speed.is_some_and(|speed| speed < 0.98) {
        return DiagnosticBottleneck::Encoder;
    }
    if capture_fps.is_some_and(|fps| fps < f64::from(target_fps) * 0.9) {
        return DiagnosticBottleneck::Capture;
    }
    if render_fps.is_some_and(|fps| fps < f64::from(target_fps) * 0.9) {
        return DiagnosticBottleneck::Render;
    }
    if dropped_frames > 0 {
        return DiagnosticBottleneck::Encoder;
    }
    if capture_fps.is_some() || render_fps.is_some() || encoder_speed.is_some() {
        return DiagnosticBottleneck::None;
    }

    DiagnosticBottleneck::Unknown
}

pub fn permission_pane_for_log(code: &str, message: &str) -> Option<PermissionPane> {
    let normalized = format!("{code} {message}").to_lowercase();
    if normalized.contains("screen")
        || normalized.contains("screencapture")
        || normalized.contains("screen capture")
        || normalized.contains("screen recording")
    {
        return Some(PermissionPane::ScreenRecording);
    }
    if normalized.contains("camera") || normalized.contains("video device") {
        return Some(PermissionPane::Camera);
    }
    if normalized.contains("microphone")
        || normalized.contains("mic")
        || normalized.contains("audio device")
    {
        return Some(PermissionPane::Microphone);
    }
    if normalized.contains("permission") || normalized.contains("privacy") {
        return Some(PermissionPane::Privacy);
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stream_health_updates_stats_and_classifies_encoder_lag() {
        let stats = apply_stream_health(
            idle_diagnostics(),
            &StreamHealth {
                session_id: "session".to_string(),
                fps: Some(29.7),
                dropped_frames: Some(4),
                speed: Some(0.82),
                created_at: "now".to_string(),
            },
            30,
        );

        assert_eq!(stats.session_id.as_deref(), Some("session"));
        assert_eq!(stats.capture_fps, Some(29.7));
        assert_eq!(stats.dropped_frames, 4);
        assert_eq!(stats.skipped_frames, 4);
        assert_eq!(stats.bottleneck, DiagnosticBottleneck::Encoder);
    }

    #[test]
    fn audio_drops_take_priority_over_video_stats() {
        let stats = apply_audio_stats(idle_diagnostics(), 48_000, 128);

        assert_eq!(stats.mic_captured_frames, Some(48_000));
        assert_eq!(stats.mic_dropped_frames, 128);
        assert_eq!(stats.bottleneck, DiagnosticBottleneck::Audio);
    }

    #[test]
    fn permission_logs_map_to_specific_panes() {
        assert_eq!(
            permission_pane_for_log("screen-capture-fallback", "permission denied"),
            Some(PermissionPane::ScreenRecording)
        );
        assert_eq!(
            permission_pane_for_log("camera-source-unavailable", "camera permission denied"),
            Some(PermissionPane::Camera)
        );
        assert_eq!(
            permission_pane_for_log("audio-device", "microphone permission denied"),
            Some(PermissionPane::Microphone)
        );
    }
}
