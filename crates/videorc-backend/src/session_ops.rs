//! Library session operations (Library rewrite L3): Duplicate and Import.
//! Both do their file work off the async runtime (spawn_blocking copies) and
//! finish by writing truthful rows — sizes statted from the real files,
//! durations probed, posters extracted.

use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};

use crate::state::AppState;

const IMPORTABLE_EXTENSIONS: [&str; 5] = ["mp4", "mov", "m4v", "mkv", "webm"];

/// `Recording.mp4` → `Recording (copy).mp4`, `Recording (copy 2).mp4`, … the
/// first name that does not exist yet (pure candidate builder, tested).
pub fn duplicate_candidate_path(source: &Path, attempt: u32) -> PathBuf {
    let stem = source
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("Recording");
    let extension = source
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| format!(".{value}"))
        .unwrap_or_default();
    let suffix = if attempt == 0 {
        " (copy)".to_string()
    } else {
        format!(" (copy {})", attempt + 1)
    };
    source.with_file_name(format!("{stem}{suffix}{extension}"))
}

fn first_free_duplicate_path(source: &Path) -> PathBuf {
    for attempt in 0..100 {
        let candidate = duplicate_candidate_path(source, attempt);
        if !candidate.exists() {
            return candidate;
        }
    }
    duplicate_candidate_path(source, 100)
}

pub fn import_extension_allowed(path: &Path) -> bool {
    path.extension()
        .and_then(|value| value.to_str())
        .map(|value| value.to_ascii_lowercase())
        .is_some_and(|ext| IMPORTABLE_EXTENSIONS.contains(&ext.as_str()))
}

/// Copy-safe destination inside the output directory for an import (pure
/// candidate builder; uniqueness handled like Duplicate).
fn import_destination(output_directory: &Path, source: &Path) -> PathBuf {
    let name = source
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("Imported recording.mp4");
    let base = output_directory.join(name);
    if !base.exists() {
        return base;
    }
    first_free_duplicate_path(&base)
}

async fn probe_duration_ms(ffmpeg_path: &str, file: &Path) -> Option<i64> {
    let ffprobe = crate::ffmpeg::ffprobe_path_for(ffmpeg_path);
    let output = tokio::process::Command::new(ffprobe)
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "json",
        ])
        .arg(file)
        .output()
        .await
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let parsed: serde_json::Value = serde_json::from_slice(&output.stdout).ok()?;
    let seconds: f64 = parsed["format"]["duration"].as_str()?.parse().ok()?;
    (seconds.is_finite() && seconds > 0.0).then_some((seconds * 1000.0) as i64)
}

/// Duplicate the session's VISIBLE file + row. Returns the new session id.
pub async fn duplicate_session(state: &AppState, session_id: &str) -> Result<String> {
    let Some(facts) = state.database.session_clone_facts(session_id)? else {
        bail!("Session not found.");
    };
    let title = facts.title;
    let mp4_path = facts.mp4_path;
    let visible = mp4_path
        .clone()
        .or(facts.output_path.clone())
        .ok_or_else(|| anyhow::anyhow!("This session has no local file to duplicate."))?;
    let source = PathBuf::from(&visible);
    if !source.exists() {
        bail!("The recording file is missing on disk.");
    }
    let destination = first_free_duplicate_path(&source);
    let copy_source = source.clone();
    let copy_destination = destination.clone();
    tokio::task::spawn_blocking(move || std::fs::copy(&copy_source, &copy_destination))
        .await
        .context("Copy task failed.")?
        .with_context(|| format!("Could not copy {}", source.display()))?;

    let new_id = uuid::Uuid::new_v4().to_string();
    let new_title = format!("{} (copy)", title.trim());
    let destination_string = destination.display().to_string();
    let size = std::fs::metadata(&destination)
        .map(|metadata| metadata.len() as i64)
        .ok();
    // The copy takes the same slot the source's visible file had.
    let (new_output, new_mp4) = if mp4_path.is_some() {
        (None, Some(destination_string.clone()))
    } else {
        (Some(destination_string.clone()), None)
    };
    let inserted = state.database.clone_session_row(
        session_id,
        &new_id,
        &new_title,
        new_output.as_deref(),
        new_mp4.as_deref(),
        &chrono::Utc::now().to_rfc3339(),
        size,
    )?;
    if !inserted {
        let _ = std::fs::remove_file(&destination);
        bail!("Session row vanished while duplicating.");
    }
    // Poster: copy the source's if present, else extract lazily later.
    let source_poster = crate::posters::poster_path(session_id);
    if source_poster.exists() {
        let _ = std::fs::copy(&source_poster, crate::posters::poster_path(&new_id));
    }
    Ok(new_id)
}

/// Import a foreign recording: managed copy into the output directory, probed
/// duration, completed session row, poster. Returns the new session id.
pub async fn import_recording(
    state: &AppState,
    source_path: &str,
    output_directory: &str,
    ffmpeg_path: &str,
) -> Result<String> {
    let source = PathBuf::from(source_path);
    if !source.exists() {
        bail!("That file does not exist.");
    }
    if !import_extension_allowed(&source) {
        bail!("Only MP4, MOV, M4V, MKV, and WebM files can be imported.");
    }
    let output_dir = PathBuf::from(output_directory.trim());
    if output_directory.trim().is_empty() || !output_dir.is_dir() {
        bail!("Set a valid output directory in Settings before importing.");
    }
    let destination = import_destination(&output_dir, &source);
    let copy_source = source.clone();
    let copy_destination = destination.clone();
    tokio::task::spawn_blocking(move || std::fs::copy(&copy_source, &copy_destination))
        .await
        .context("Copy task failed.")?
        .with_context(|| format!("Could not copy {}", source.display()))?;

    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    let title = source
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("Imported recording")
        .to_string();
    let destination_string = destination.display().to_string();
    let is_mp4_family = destination
        .extension()
        .and_then(|value| value.to_str())
        .map(|ext| matches!(ext.to_ascii_lowercase().as_str(), "mp4" | "mov" | "m4v"))
        .unwrap_or(false);
    state.database.create_session(&crate::storage::NewSession {
        id: id.clone(),
        title,
        started_at: now.clone(),
        mode: "imported".to_string(),
        output_path: (!is_mp4_family).then(|| destination_string.clone()),
        container: (!is_mp4_family).then(|| {
            destination
                .extension()
                .and_then(|value| value.to_str())
                .unwrap_or("mkv")
                .to_ascii_lowercase()
        }),
        stream_preset: None,
        sources: crate::protocol::SourceSelection {
            screen_id: None,
            window_id: None,
            camera_id: None,
            microphone_id: None,
            test_pattern: false,
        },
        layout: crate::protocol::default_layout_settings(),
        // Imported files were not produced by a capture session; the output
        // config is a neutral placeholder the UI never reads for imports.
        output: crate::protocol::OutputSettings {
            record_enabled: false,
            stream_enabled: false,
            output_directory: Some(output_directory.trim().to_string()),
            ffmpeg_path: None,
            video: crate::protocol::VideoSettings {
                preset: crate::protocol::VideoPreset::Tutorial1080p30,
                width: 1920,
                height: 1080,
                fps: 30,
                bitrate_kbps: 6000,
            },
            rtmp: crate::protocol::RtmpSettings {
                preset: crate::protocol::RtmpPreset::Custom,
                server_url: String::new(),
                stream_key: String::new(),
            },
        },
    })?;
    let duration_ms = probe_duration_ms(ffmpeg_path, &destination).await;
    state.database.finish_session(
        &id,
        "completed",
        Some(now),
        is_mp4_family.then(|| destination_string.clone()),
        duration_ms,
    )?;
    crate::posters::ensure_session_poster(
        state,
        &id,
        &destination_string,
        duration_ms,
        ffmpeg_path,
    )
    .await;
    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duplicate_names_count_upward() {
        let source = Path::new("/tmp/Weekly Update.mp4");
        assert_eq!(
            duplicate_candidate_path(source, 0),
            Path::new("/tmp/Weekly Update (copy).mp4")
        );
        assert_eq!(
            duplicate_candidate_path(source, 1),
            Path::new("/tmp/Weekly Update (copy 2).mp4")
        );
        // Extension-less files survive.
        assert_eq!(
            duplicate_candidate_path(Path::new("/tmp/raw"), 0),
            Path::new("/tmp/raw (copy)")
        );
    }

    #[test]
    fn import_extension_gate() {
        assert!(import_extension_allowed(Path::new("/x/a.MP4")));
        assert!(import_extension_allowed(Path::new("/x/a.webm")));
        assert!(!import_extension_allowed(Path::new("/x/a.txt")));
        assert!(!import_extension_allowed(Path::new("/x/noext")));
    }
}
