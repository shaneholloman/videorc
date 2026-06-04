//! AppState-coupled orchestration for the recording-repair commands (slice 10).
//!
//! The pure analysis/repair primitives live in [`crate::repair`]; this module wires them
//! to the WebSocket command surface: it runs the blocking FFmpeg work off the async
//! runtime, emits per-file `repair.status` events the renderer subscribes to, and shapes
//! the request/response payloads. Per-recording assess / repair / restore are here; the
//! folder batch flow reuses the same primitives.

use std::path::Path;

use serde::Serialize;

use crate::ffmpeg::{ffprobe_path_for, resolve_ffmpeg_path};
use crate::ffmpeg_work::MaintenanceDeferral;
use crate::protocol::{RepairFileParams, RepairRestoreParams};
use crate::repair::{
    GateStatus, QualityExpectations, QualityIssue, QualityThresholds, QualityVerdict,
    analyze_recording, backup_path_for, gate_recording, issue_reasons, restore_from_backup,
    select_repair_plan,
};
use crate::state::AppState;

/// A read-only quality assessment of one recording (no files are modified).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileAssessment {
    pub path: String,
    pub verdict: QualityVerdict,
    pub issues: Vec<QualityIssue>,
    /// Plain-English reasons mirroring `issues`, for the UI summary.
    pub reasons: Vec<String>,
    /// Whether an FFmpeg-only repair could be attempted.
    pub repairable: bool,
    /// Whether a hidden backup already exists (so the UI can offer "restore original").
    pub has_backup: bool,
}

/// Builds the analyzer expectations from request params, defaulting to expecting audio
/// (so a lost mic is flagged) unless the caller says otherwise.
fn expectations_from(params: &RepairFileParams) -> QualityExpectations {
    QualityExpectations {
        intended_fps: params.intended_fps,
        expect_audio: params.expect_audio.unwrap_or(true),
    }
}

fn emit_status(state: &AppState, path: &str, status: &str) {
    state.emit_event(
        "repair.status",
        serde_json::json!({ "path": path, "status": status }),
    );
}

fn emit_deferred_status(state: &AppState, path: &str, deferral: MaintenanceDeferral) {
    state.emit_event(
        "repair.status",
        serde_json::json!({
            "path": path,
            "status": "deferred",
            "reason": deferral.message(),
        }),
    );
}

/// The renderer status label for a finished gate verdict.
fn gate_status_label(status: &GateStatus) -> &'static str {
    match status {
        GateStatus::Ready { .. } => "ready",
        GateStatus::Repaired { .. } => "repaired",
        GateStatus::NotHundredPercent { .. } => "not-100",
        GateStatus::Failed { .. } => "failed",
    }
}

fn gate_status_path(status: &GateStatus) -> &str {
    match status {
        GateStatus::Ready { path }
        | GateStatus::Repaired { path, .. }
        | GateStatus::NotHundredPercent { path, .. }
        | GateStatus::Failed { path, .. } => path,
    }
}

/// Assesses a single recording without modifying it: returns the verdict, issues, plain
/// reasons, whether a repair is possible, and whether a backup already exists.
pub async fn assess_file(
    state: AppState,
    params: RepairFileParams,
) -> Result<FileAssessment, String> {
    let path = params.path.clone();
    let ffmpeg_path = resolve_ffmpeg_path(params.ffmpeg_path.clone());
    let expectations = expectations_from(&params);
    let _maintenance = match state.ffmpeg_work.try_begin_maintenance() {
        Ok(permit) => permit,
        Err(deferral) => {
            emit_deferred_status(&state, &path, deferral);
            return Err(deferral.message().to_string());
        }
    };

    let probe_path = path.clone();
    let (report, repairable) = tokio::task::spawn_blocking(move || {
        let ffprobe_path = ffprobe_path_for(&ffmpeg_path);
        let (probe, report) = analyze_recording(
            &ffmpeg_path,
            &ffprobe_path,
            &probe_path,
            &QualityThresholds::default(),
            &expectations,
        )?;
        let repairable = select_repair_plan(&report, &probe, &expectations).is_some();
        Ok::<_, String>((report, repairable))
    })
    .await
    .map_err(|error| format!("assess task failed: {error}"))??;

    let has_backup = backup_path_for(Path::new(&path))
        .map(|backup| backup.exists())
        .unwrap_or(false);

    Ok(FileAssessment {
        path,
        reasons: issue_reasons(&report.issues),
        verdict: report.verdict,
        issues: report.issues,
        repairable,
        has_backup,
    })
}

/// Repairs a single recording in place (backup-then-validate). Emits `repair.status`
/// events (`checking` → `repairing` → final) so the UI can show live progress, and
/// returns the full gate verdict.
pub async fn repair_file(state: AppState, params: RepairFileParams) -> Result<GateStatus, String> {
    let path = params.path.clone();
    let ffmpeg_path = resolve_ffmpeg_path(params.ffmpeg_path.clone());
    let expectations = expectations_from(&params);
    let _maintenance = match state.ffmpeg_work.try_begin_maintenance() {
        Ok(permit) => permit,
        Err(deferral) => {
            emit_deferred_status(&state, &path, deferral);
            return Err(deferral.message().to_string());
        }
    };

    emit_status(&state, &path, "checking");
    emit_status(&state, &path, "repairing");

    let gate_path = path.clone();
    let status = tokio::task::spawn_blocking(move || {
        let ffprobe_path = ffprobe_path_for(&ffmpeg_path);
        gate_recording(
            &ffmpeg_path,
            &ffprobe_path,
            &gate_path,
            &QualityThresholds::default(),
            &expectations,
        )
    })
    .await
    .map_err(|error| format!("repair task failed: {error}"))?;

    emit_status(
        &state,
        gate_status_path(&status),
        gate_status_label(&status),
    );
    Ok(status)
}

/// Restores a recording from its hidden backup, returning whether a backup was found.
pub async fn restore_file(params: RepairRestoreParams) -> Result<bool, String> {
    let path = params.path.clone();
    tokio::task::spawn_blocking(move || {
        restore_from_backup(Path::new(&path)).map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| format!("restore task failed: {error}"))?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expectations_default_to_expecting_audio() {
        let params = RepairFileParams {
            path: "/m/a.mp4".to_string(),
            ffmpeg_path: None,
            expect_audio: None,
            intended_fps: Some(30.0),
        };
        let expectations = expectations_from(&params);
        assert!(expectations.expect_audio);
        assert_eq!(expectations.intended_fps, Some(30.0));
    }

    #[test]
    fn expectations_honor_explicit_no_audio() {
        let params = RepairFileParams {
            path: "/m/a.mp4".to_string(),
            ffmpeg_path: None,
            expect_audio: Some(false),
            intended_fps: None,
        };
        assert!(!expectations_from(&params).expect_audio);
    }

    #[test]
    fn gate_status_labels_match_renderer_statuses() {
        assert_eq!(
            gate_status_label(&GateStatus::Ready {
                path: "/m/a.mp4".to_string()
            }),
            "ready"
        );
        assert_eq!(
            gate_status_label(&GateStatus::Repaired {
                path: "/m/a.mp4".to_string(),
                interpolated: true,
            }),
            "repaired"
        );
        assert_eq!(
            gate_status_label(&GateStatus::NotHundredPercent {
                path: "/m/a.mp4".to_string(),
                reasons: vec!["bad".to_string()],
            }),
            "not-100"
        );
        assert_eq!(
            gate_status_label(&GateStatus::Failed {
                path: "/m/a.mp4".to_string(),
                reason: "boom".to_string(),
            }),
            "failed"
        );
    }
}
