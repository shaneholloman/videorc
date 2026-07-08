use std::path::{Path, PathBuf};
use std::process::Stdio;

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::fs;
use tokio::process::Command;
use tokio::time::{Duration, sleep, timeout};

use crate::account;
use crate::ffmpeg::resolve_ffmpeg_path;
use crate::process_job::output_owned_tokio;
use crate::protocol::{
    AiArtifact, AiArtifactKind, AiArtifactStatus, AiCapabilities, AiJobSnapshot, AiWorkflowResult,
    ExportPublishPackParams, ExportPublishPackResult, HealthEvent, HealthLevel,
    RunAiWorkflowParams,
};
use crate::recording::emit_health_event;
use crate::state::AppState;
use crate::storage::default_artifacts_dir;
use crate::videorc_api::{AiAudioJobRequest, AiObjectUploadRequest, VideorcApiClient};

const AI_WORKFLOW_KIND_POST_RECORDING: &str = "post-recording-publish-pack";
const AI_JOB_POLL_INTERVAL: Duration = Duration::from_secs(2);
const AI_JOB_POLL_TIMEOUT: Duration = Duration::from_secs(45 * 60);
const DESKTOP_CLIENT_VERSION: &str = concat!("videorc-desktop/", env!("CARGO_PKG_VERSION"));

struct WebAiJobContext<'a> {
    session_id: &'a str,
    client_request_id: &'a str,
    diagnostic_summary: Option<&'a str>,
    health_events: &'a [HealthEvent],
}

pub async fn run_ai_workflow(
    state: AppState,
    params: RunAiWorkflowParams,
) -> Result<AiWorkflowResult> {
    // The DB records where files were written, not whether they still exist —
    // use the first recorded path that is actually on disk, and fail with a
    // human explanation instead of FFmpeg's raw ENOENT when none are
    // (2026-07-03 report: "transcript" on a session whose file had been
    // moved/deleted surfaced "exit status 254: Error opening input").
    let candidates = state
        .database
        .session_media_candidates(&params.session_id)?;
    if candidates.is_empty() {
        anyhow::bail!("Session does not have a local recording output");
    }
    let input_path = candidates
        .iter()
        .map(PathBuf::from)
        .find(|path| path.is_file());
    let Some(input_path) = input_path else {
        anyhow::bail!(
            "The recording file for this session is missing on disk (looked for {}). It may have been moved or deleted — AI features need the original recording file.",
            candidates.join(", ")
        );
    };

    let ffmpeg_path = resolve_ffmpeg_path(params.ffmpeg_path);
    let artifact_dir = default_artifacts_dir().join(&params.session_id);
    fs::create_dir_all(&artifact_dir)
        .await
        .with_context(|| format!("Could not create {}", artifact_dir.display()))?;

    let audio_path = artifact_dir.join("audio.m4a");
    extract_audio(&ffmpeg_path, &input_path, &audio_path).await?;
    let mut artifacts = vec![state.database.save_ai_artifact(
        &params.session_id,
        AiArtifactKind::AudioExtract,
        AiArtifactStatus::Ready,
        json!({
            "privacy": "Local audio extract. Not uploaded unless cloud AI is explicitly run.",
            "sourcePath": input_path.display().to_string(),
        }),
        Some(audio_path.display().to_string()),
    )?];

    if !params.consent_to_upload_audio {
        artifacts.push(state.database.save_ai_artifact(
            &params.session_id,
            AiArtifactKind::Transcript,
            AiArtifactStatus::PendingConsent,
            json!({
                "message": "Cloud transcription is waiting for explicit consent to upload extracted audio.",
            }),
            None,
        )?);
        emit_health_event(
            &state,
            Some(&params.session_id),
            HealthLevel::Info,
            "ai-consent-required",
            "Audio was extracted locally. Cloud AI did not run because consent was not granted.",
        )?;
        emit_ai_artifacts_changed(&state, &params.session_id)?;
        return Ok(AiWorkflowResult {
            session_id: params.session_id,
            audio_path: audio_path.display().to_string(),
            artifacts,
        });
    }

    let cloud_artifacts = match run_web_ai_job(&state, &params.session_id, &audio_path).await {
        Ok(artifacts) => artifacts,
        Err(error) => {
            let message = error.to_string();
            let code = if message.contains("Sign in") {
                "cloud-ai-sign-in-required"
            } else {
                "cloud-ai-job-failed"
            };
            let event_message = if code == "cloud-ai-sign-in-required" {
                "Audio was extracted locally. Cloud AI did not run because the Videorc session is not signed in."
            } else {
                "Cloud AI failed. The local recording and extracted audio are still available."
            };
            artifacts.push(state.database.save_ai_artifact(
                &params.session_id,
                AiArtifactKind::Transcript,
                AiArtifactStatus::Failed,
                json!({
                    "message": message,
                    "provider": "videorc",
                }),
                None,
            )?);
            emit_health_event(
                &state,
                Some(&params.session_id),
                HealthLevel::Warn,
                code,
                event_message,
            )?;
            emit_ai_artifacts_changed(&state, &params.session_id)?;
            return Ok(AiWorkflowResult {
                session_id: params.session_id,
                audio_path: audio_path.display().to_string(),
                artifacts,
            });
        }
    };
    artifacts.extend(cloud_artifacts);

    emit_ai_artifacts_changed(&state, &params.session_id)?;

    Ok(AiWorkflowResult {
        session_id: params.session_id,
        audio_path: audio_path.display().to_string(),
        artifacts,
    })
}

async fn run_web_ai_job(
    state: &AppState,
    session_id: &str,
    audio_path: &Path,
) -> Result<Vec<AiArtifact>> {
    let token = account::stored_session_token().context("Sign in to use cloud AI.")?;
    let client = VideorcApiClient::new()?;
    let capabilities = client.get_ai_capabilities(&token).await?;
    validate_cloud_ai_capabilities(&capabilities)?;
    let audio_size = fs::metadata(audio_path)
        .await
        .with_context(|| format!("Could not inspect {}", audio_path.display()))?
        .len();
    let client_request_id = ai_client_request_id(session_id, audio_path).await?;
    let health_events = state.database.list_health_events(session_id)?;
    let health_events_json =
        serde_json::to_string(&health_events).context("Could not serialize AI health events.")?;
    let diagnostic_summary = diagnostic_summary_for_session(state, session_id);
    let job_context = WebAiJobContext {
        session_id,
        client_request_id: &client_request_id,
        diagnostic_summary: diagnostic_summary.as_deref(),
        health_events: &health_events,
    };
    let audio_intake_error = audio_intake_error(&capabilities, audio_size);
    let initial_job = if capabilities.features.multipart_audio_jobs_enabled
        && audio_intake_error.is_none()
    {
        client
            .create_ai_job_from_audio(
                &token,
                AiAudioJobRequest {
                    audio_path,
                    client_request_id: &client_request_id,
                    client_version: DESKTOP_CLIENT_VERSION,
                    diagnostic_summary: job_context.diagnostic_summary,
                    health_events_json: &health_events_json,
                    session_client_id: session_id,
                },
            )
            .await?
            .job
    } else if capabilities.features.object_backed_jobs_enabled && audio_intake_error.is_none() {
        create_object_backed_ai_job(&client, &token, &job_context, audio_path, audio_size).await?
    } else if capabilities.features.transcript_jobs_enabled {
        let transcript = latest_local_transcript_text(state, session_id)?.with_context(|| {
            audio_intake_error.clone().unwrap_or_else(|| {
                "No ready local transcript is available for transcript-backed AI.".to_string()
            })
        })?;
        create_transcript_backed_ai_job(&client, &token, &job_context, &transcript).await?
    } else if let Some(error) = audio_intake_error {
        bail!("{error}");
    } else {
        bail!("Videorc AI audio intake is not enabled for this account.");
    };

    let completed_job = wait_for_ai_job(
        state,
        session_id,
        &client,
        &token,
        initial_job,
        &capabilities,
    )
    .await?;
    save_completed_web_ai_artifacts(state, session_id, &completed_job)
}

fn validate_cloud_ai_capabilities(capabilities: &AiCapabilities) -> Result<()> {
    if !capabilities.entitlement.cloud_ai || !capabilities.readiness.access.cloud_ai_entitled {
        bail!("Cloud AI requires Videorc Premium.");
    }
    if capabilities.readiness.access.globally_disabled {
        bail!("Cloud AI is disabled on the Videorc server.");
    }
    if !capabilities.readiness.gateway.configured {
        bail!(
            "{}",
            capabilities
                .readiness
                .gateway
                .config_error
                .as_deref()
                .unwrap_or("Videorc AI Gateway is not configured.")
        );
    }
    if !capabilities.readiness.worker.configured {
        bail!(
            "{}",
            capabilities
                .readiness
                .worker
                .config_error
                .as_deref()
                .unwrap_or("Videorc AI worker is not configured.")
        );
    }
    if !capabilities.features.cloud_ai_enabled {
        bail!("Videorc cloud AI is not ready for this account.");
    }
    Ok(())
}

fn audio_intake_error(capabilities: &AiCapabilities, audio_size: u64) -> Option<String> {
    if !capabilities.readiness.transcription.configured {
        return Some(
            capabilities
                .readiness
                .transcription
                .config_error
                .clone()
                .unwrap_or_else(|| "Videorc cloud transcription is not configured.".to_string()),
        );
    }
    if let Some(max_audio_bytes) = capabilities.limits.max_audio_bytes
        && audio_size > max_audio_bytes
    {
        return Some(format!(
            "Recording audio is too large for configured AI intake ({} > {} bytes).",
            audio_size, max_audio_bytes
        ));
    }
    None
}

async fn create_object_backed_ai_job(
    client: &VideorcApiClient,
    token: &str,
    job_context: &WebAiJobContext<'_>,
    audio_path: &Path,
    audio_size: u64,
) -> Result<AiJobSnapshot> {
    let upload = client
        .request_ai_object_upload(
            token,
            &AiObjectUploadRequest {
                client_request_id: job_context.client_request_id,
                client_version: DESKTOP_CLIENT_VERSION,
                consent_to_upload_audio: true,
                file_name: "audio.m4a",
                mime_type: "audio/mp4",
                session_client_id: job_context.session_id,
                size_bytes: audio_size,
                workflow_kind: AI_WORKFLOW_KIND_POST_RECORDING,
            },
        )
        .await?;
    client.upload_ai_object(&upload.ticket, audio_path).await?;

    let mut job_request = upload.job_request;
    let input_json = json!({
        "diagnosticSummary": job_context.diagnostic_summary,
        "healthEvents": job_context.health_events,
    });
    let object = job_request
        .as_object_mut()
        .context("AI object upload response did not include a job request object.")?;
    object.insert("inputJson".to_string(), input_json);
    Ok(client.create_ai_job(token, &job_request).await?.job)
}

async fn create_transcript_backed_ai_job(
    client: &VideorcApiClient,
    token: &str,
    job_context: &WebAiJobContext<'_>,
    transcript: &str,
) -> Result<AiJobSnapshot> {
    let body = json!({
        "clientRequestId": job_context.client_request_id,
        "clientVersion": DESKTOP_CLIENT_VERSION,
        "inputJson": {
            "diagnosticSummary": job_context.diagnostic_summary,
            "healthEvents": job_context.health_events,
            "transcript": transcript,
        },
        "sessionClientId": job_context.session_id,
        "workflowKind": AI_WORKFLOW_KIND_POST_RECORDING,
    });
    Ok(client.create_ai_job(token, &body).await?.job)
}

async fn wait_for_ai_job(
    state: &AppState,
    session_id: &str,
    client: &VideorcApiClient,
    token: &str,
    initial_job: AiJobSnapshot,
    capabilities: &AiCapabilities,
) -> Result<AiJobSnapshot> {
    let started = tokio::time::Instant::now();
    let queued_delay = Duration::from_millis(capabilities.readiness.worker.queued_job_delay_ms);
    let running_delay = Duration::from_millis(capabilities.readiness.worker.running_job_timeout_ms);
    let mut queued_delay_reported = false;
    let mut running_delay_reported = false;
    let mut job = initial_job;
    loop {
        match job.status.as_str() {
            "completed" => return Ok(job),
            "failed" => {
                bail!(
                    "{}",
                    job.error_message
                        .as_deref()
                        .unwrap_or("Videorc AI job failed.")
                )
            }
            "cancelled" => bail!("Videorc AI job was cancelled."),
            _ => {}
        }
        let elapsed = started.elapsed();
        if job.status == "queued" && !queued_delay_reported && elapsed >= queued_delay {
            emit_health_event(
                state,
                Some(session_id),
                HealthLevel::Info,
                "cloud-ai-worker-delayed",
                "Queued - Videorc AI worker is delayed.",
            )?;
            queued_delay_reported = true;
        }
        if job.status == "running" && !running_delay_reported && elapsed >= running_delay {
            emit_health_event(
                state,
                Some(session_id),
                HealthLevel::Warn,
                "cloud-ai-worker-still-processing",
                "Still processing - Videorc AI is taking longer than expected.",
            )?;
            running_delay_reported = true;
        }
        if elapsed > AI_JOB_POLL_TIMEOUT {
            bail!("Videorc AI job timed out before completion.");
        }
        sleep(AI_JOB_POLL_INTERVAL).await;
        job = client.get_ai_job(token, &job.id).await?;
    }
}

fn save_completed_web_ai_artifacts(
    state: &AppState,
    session_id: &str,
    job: &AiJobSnapshot,
) -> Result<Vec<AiArtifact>> {
    let owner_artifacts = job
        .artifacts
        .as_ref()
        .context("Completed Videorc AI job did not include owner artifacts.")?;
    let mut saved = Vec::new();

    if let Some(transcript) = &owner_artifacts.transcript {
        saved.push(state.database.save_ai_artifact(
            session_id,
            AiArtifactKind::Transcript,
            AiArtifactStatus::Ready,
            json!({
                "jobId": job.id,
                "model": job.model,
                "provider": job.provider,
                "text": transcript.text,
                "transcription": owner_artifacts.transcription_metadata,
            }),
            None,
        )?);
    }

    let publish_pack = object_or_empty(&owner_artifacts.publish_pack);
    saved.push(state.database.save_ai_artifact(
        session_id,
        AiArtifactKind::TitleDescription,
        AiArtifactStatus::Ready,
        json!({
            "description": string_field(publish_pack, "description"),
            "jobId": job.id,
            "model": job.model,
            "provider": job.provider,
            "title": string_field(publish_pack, "title"),
        }),
        None,
    )?);
    saved.push(state.database.save_ai_artifact(
        session_id,
        AiArtifactKind::Summary,
        AiArtifactStatus::Ready,
        json!({
            "jobId": job.id,
            "model": job.model,
            "provider": job.provider,
            "text": string_field(publish_pack, "summary"),
        }),
        None,
    )?);
    saved.push(state.database.save_ai_artifact(
        session_id,
        AiArtifactKind::Chapters,
        AiArtifactStatus::Ready,
        json!({
            "chapters": value_field(publish_pack, "chapters"),
            "jobId": job.id,
            "model": job.model,
            "provider": job.provider,
        }),
        None,
    )?);

    saved.extend(save_creator_intelligence_value_artifacts(
        state,
        session_id,
        job,
        &owner_artifacts.creator_intelligence,
    )?);
    Ok(saved)
}

fn object_or_empty(value: &Value) -> &serde_json::Map<String, Value> {
    static EMPTY: std::sync::OnceLock<serde_json::Map<String, Value>> = std::sync::OnceLock::new();
    value
        .as_object()
        .unwrap_or_else(|| EMPTY.get_or_init(serde_json::Map::new))
}

fn string_field(object: &serde_json::Map<String, Value>, field: &str) -> String {
    object
        .get(field)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn value_field(object: &serde_json::Map<String, Value>, field: &str) -> Value {
    object.get(field).cloned().unwrap_or(Value::Null)
}

fn save_creator_intelligence_value_artifacts(
    state: &AppState,
    session_id: &str,
    job: &AiJobSnapshot,
    creator_intelligence: &Value,
) -> Result<Vec<AiArtifact>> {
    let intelligence = object_or_empty(creator_intelligence);
    Ok(vec![
        state.database.save_ai_artifact(
            session_id,
            AiArtifactKind::Highlights,
            AiArtifactStatus::Ready,
            json!({
                "highlights": value_field(intelligence, "highlights"),
                "jobId": job.id,
                "model": job.model,
                "provider": job.provider,
            }),
            None,
        )?,
        state.database.save_ai_artifact(
            session_id,
            AiArtifactKind::SmartZoom,
            AiArtifactStatus::Ready,
            json!({
                "jobId": job.id,
                "model": job.model,
                "provider": job.provider,
                "suggestions": value_field(intelligence, "smartZoom"),
            }),
            None,
        )?,
        state.database.save_ai_artifact(
            session_id,
            AiArtifactKind::NoiseCleanup,
            AiArtifactStatus::Ready,
            json!({
                "jobId": job.id,
                "model": job.model,
                "provider": job.provider,
                "suggestions": value_field(intelligence, "noiseCleanup"),
            }),
            None,
        )?,
        state.database.save_ai_artifact(
            session_id,
            AiArtifactKind::SilenceRemoval,
            AiArtifactStatus::Ready,
            json!({
                "jobId": job.id,
                "model": job.model,
                "provider": job.provider,
                "suggestions": value_field(intelligence, "silenceRemoval"),
            }),
            None,
        )?,
        state.database.save_ai_artifact(
            session_id,
            AiArtifactKind::HealthAssistant,
            AiArtifactStatus::Ready,
            json!({
                "explanations": value_field(intelligence, "healthAssistant"),
                "jobId": job.id,
                "model": job.model,
                "provider": job.provider,
            }),
            None,
        )?,
    ])
}

fn diagnostic_summary_for_session(state: &AppState, session_id: &str) -> Option<String> {
    state
        .database
        .list_sessions(500)
        .ok()?
        .into_iter()
        .find(|session| session.id == session_id)
        .and_then(|session| session.final_diagnostics)
        .and_then(|diagnostics| serde_json::to_string(&diagnostics).ok())
}

fn latest_local_transcript_text(state: &AppState, session_id: &str) -> Result<Option<String>> {
    Ok(state
        .database
        .list_ai_artifacts(session_id)?
        .into_iter()
        .rev()
        .find_map(|artifact| {
            if artifact.kind != AiArtifactKind::Transcript
                || artifact.status != AiArtifactStatus::Ready
            {
                return None;
            }
            content_string(&artifact, "text").filter(|text| !text.trim().is_empty())
        }))
}

async fn ai_client_request_id(session_id: &str, audio_path: &Path) -> Result<String> {
    let audio = fs::read(audio_path)
        .await
        .with_context(|| format!("Could not read {}", audio_path.display()))?;
    let hash = format!("{:x}", Sha256::digest(&audio));
    Ok(build_ai_client_request_id(session_id, &hash))
}

fn build_ai_client_request_id(session_id: &str, audio_hash: &str) -> String {
    let session_component = safe_client_request_component(session_id)
        .chars()
        .take(40)
        .collect::<String>();
    format!("desktop:{session_component}:{audio_hash}")
}

fn safe_client_request_component(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | ':' | '-') {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

pub fn list_ai_artifacts(state: &AppState, session_id: &str) -> Result<Vec<AiArtifact>> {
    state.database.list_ai_artifacts(session_id)
}

pub async fn export_publish_pack(
    state: AppState,
    params: ExportPublishPackParams,
) -> Result<ExportPublishPackResult> {
    let artifacts = state.database.list_ai_artifacts(&params.session_id)?;
    if !artifacts.iter().any(|artifact| {
        artifact.status == AiArtifactStatus::Ready && is_publish_pack_kind(&artifact.kind)
    }) {
        bail!("No ready AI artifacts are available for this session");
    }

    let artifact_dir = default_artifacts_dir().join(&params.session_id);
    fs::create_dir_all(&artifact_dir)
        .await
        .with_context(|| format!("Could not create {}", artifact_dir.display()))?;

    let markdown_path = artifact_dir.join("publish-pack.md");
    let markdown = render_publish_pack(&artifacts);
    fs::write(&markdown_path, markdown)
        .await
        .with_context(|| format!("Could not write {}", markdown_path.display()))?;

    Ok(ExportPublishPackResult {
        session_id: params.session_id,
        markdown_path: markdown_path.display().to_string(),
    })
}

fn emit_ai_artifacts_changed(state: &AppState, session_id: &str) -> Result<()> {
    state.emit_event(
        "ai.artifacts.changed",
        state.database.list_ai_artifacts(session_id)?,
    );
    Ok(())
}

async fn extract_audio(ffmpeg_path: &str, input_path: &Path, output_path: &Path) -> Result<()> {
    let mut command = Command::new(ffmpeg_path);
    command
        .args([
            "-y",
            "-hide_banner",
            "-loglevel",
            "warning",
            "-i",
            &input_path.display().to_string(),
            "-vn",
            "-ac",
            "1",
            "-ar",
            "16000",
            "-c:a",
            "aac",
            "-b:a",
            "64k",
            &output_path.display().to_string(),
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::piped());

    let output = timeout(
        Duration::from_secs(20 * 60),
        output_owned_tokio(&mut command),
    )
    .await
    .context("FFmpeg audio extraction timed out")?
    .with_context(|| format!("Could not start {ffmpeg_path} for audio extraction"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        bail!(
            "FFmpeg audio extraction failed with {}{}",
            output.status,
            if stderr.is_empty() {
                String::new()
            } else {
                format!(": {stderr}")
            }
        );
    }

    Ok(())
}

fn render_publish_pack(artifacts: &[AiArtifact]) -> String {
    let title_description = latest_ready_artifact(artifacts, AiArtifactKind::TitleDescription);
    let transcript = latest_ready_artifact(artifacts, AiArtifactKind::Transcript);
    let summary = latest_ready_artifact(artifacts, AiArtifactKind::Summary);
    let chapters = latest_ready_artifact(artifacts, AiArtifactKind::Chapters);
    let highlights = latest_ready_artifact(artifacts, AiArtifactKind::Highlights);
    let smart_zoom = latest_ready_artifact(artifacts, AiArtifactKind::SmartZoom);
    let noise_cleanup = latest_ready_artifact(artifacts, AiArtifactKind::NoiseCleanup);
    let silence_removal = latest_ready_artifact(artifacts, AiArtifactKind::SilenceRemoval);
    let health_assistant = latest_ready_artifact(artifacts, AiArtifactKind::HealthAssistant);

    let title = title_description
        .and_then(|artifact| content_string(artifact, "title"))
        .unwrap_or_else(|| "Untitled Videorc Session".to_string());
    let description = title_description
        .and_then(|artifact| content_string(artifact, "description"))
        .unwrap_or_default();
    let summary_text = summary
        .and_then(|artifact| content_string(artifact, "text"))
        .unwrap_or_default();
    let transcript_text = transcript
        .and_then(|artifact| content_string(artifact, "text"))
        .unwrap_or_default();

    let mut markdown = format!("# {title}\n\n");
    if !description.is_empty() {
        markdown.push_str("## Description\n\n");
        markdown.push_str(&description);
        markdown.push_str("\n\n");
    }
    if !summary_text.is_empty() {
        markdown.push_str("## Summary\n\n");
        markdown.push_str(&summary_text);
        markdown.push_str("\n\n");
    }
    if let Some(chapters) = chapters {
        let lines = chapter_lines(chapters);
        if !lines.is_empty() {
            markdown.push_str("## Chapters\n\n");
            for line in lines {
                markdown.push_str("- ");
                markdown.push_str(&line);
                markdown.push('\n');
            }
            markdown.push('\n');
        }
    }
    if let Some(highlights) = highlights {
        let lines = object_lines(
            highlights,
            "highlights",
            &["timestamp", "title", "reason", "suggestedUse"],
        );
        if !lines.is_empty() {
            markdown.push_str("## Highlights\n\n");
            push_markdown_list(&mut markdown, lines);
        }
    }
    if let Some(smart_zoom) = smart_zoom {
        let lines = object_lines(
            smart_zoom,
            "suggestions",
            &["timestamp", "action", "subject", "reason"],
        );
        if !lines.is_empty() {
            markdown.push_str("## Smart Zoom Notes\n\n");
            push_markdown_list(&mut markdown, lines);
        }
    }
    let cleanup_lines = noise_cleanup
        .map(|artifact| {
            object_lines(
                artifact,
                "suggestions",
                &["issue", "suggestion", "confidence"],
            )
        })
        .unwrap_or_default()
        .into_iter()
        .chain(
            silence_removal
                .map(|artifact| {
                    object_lines(
                        artifact,
                        "suggestions",
                        &["timestamp", "reason", "editSuggestion"],
                    )
                })
                .unwrap_or_default(),
        )
        .collect::<Vec<_>>();
    if !cleanup_lines.is_empty() {
        markdown.push_str("## Cleanup Suggestions\n\n");
        push_markdown_list(&mut markdown, cleanup_lines);
    }
    if let Some(health_assistant) = health_assistant {
        let lines = object_lines(
            health_assistant,
            "explanations",
            &["level", "issue", "explanation", "action"],
        );
        if !lines.is_empty() {
            markdown.push_str("## Health Assistant\n\n");
            push_markdown_list(&mut markdown, lines);
        }
    }
    if !transcript_text.is_empty() {
        markdown.push_str("## Transcript\n\n");
        markdown.push_str(&transcript_text);
        markdown.push('\n');
    }

    markdown
}

fn latest_ready_artifact(artifacts: &[AiArtifact], kind: AiArtifactKind) -> Option<&AiArtifact> {
    artifacts
        .iter()
        .rev()
        .find(|artifact| artifact.kind == kind && artifact.status == AiArtifactStatus::Ready)
}

fn is_publish_pack_kind(kind: &AiArtifactKind) -> bool {
    matches!(
        kind,
        AiArtifactKind::Transcript
            | AiArtifactKind::TitleDescription
            | AiArtifactKind::Summary
            | AiArtifactKind::Chapters
            | AiArtifactKind::Highlights
            | AiArtifactKind::SmartZoom
            | AiArtifactKind::NoiseCleanup
            | AiArtifactKind::SilenceRemoval
            | AiArtifactKind::HealthAssistant
    )
}

fn content_string(artifact: &AiArtifact, key: &str) -> Option<String> {
    artifact
        .content
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn chapter_lines(artifact: &AiArtifact) -> Vec<String> {
    artifact
        .content
        .get("chapters")
        .and_then(Value::as_array)
        .map(|chapters| {
            chapters
                .iter()
                .filter_map(|chapter| {
                    let timestamp = chapter.get("timestamp").and_then(Value::as_str)?;
                    let title = chapter.get("title").and_then(Value::as_str)?;
                    Some(format!("{timestamp} {title}"))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn object_lines(artifact: &AiArtifact, key: &str, fields: &[&str]) -> Vec<String> {
    artifact
        .content
        .get(key)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    let object = item.as_object()?;
                    let values = fields
                        .iter()
                        .filter_map(|field| object.get(*field).and_then(Value::as_str))
                        .filter(|value| !value.trim().is_empty())
                        .collect::<Vec<_>>();
                    (!values.is_empty()).then(|| values.join(" - "))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn push_markdown_list(markdown: &mut String, lines: Vec<String>) {
    for line in lines {
        markdown.push_str("- ");
        markdown.push_str(&line);
        markdown.push('\n');
    }
    markdown.push('\n');
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_publish_pack_markdown() {
        let artifacts = vec![
            AiArtifact {
                id: "1".to_string(),
                session_id: "session".to_string(),
                kind: AiArtifactKind::TitleDescription,
                status: AiArtifactStatus::Ready,
                content: json!({
                    "title": "Tutorial Session",
                    "description": "Learn the flow.",
                }),
                file_path: None,
                created_at: "2026-05-30T00:00:00Z".to_string(),
            },
            AiArtifact {
                id: "2".to_string(),
                session_id: "session".to_string(),
                kind: AiArtifactKind::Chapters,
                status: AiArtifactStatus::Ready,
                content: json!({
                    "chapters": [{"timestamp": "00:00", "title": "Intro"}],
                }),
                file_path: None,
                created_at: "2026-05-30T00:00:01Z".to_string(),
            },
            AiArtifact {
                id: "3".to_string(),
                session_id: "session".to_string(),
                kind: AiArtifactKind::Highlights,
                status: AiArtifactStatus::Ready,
                content: json!({
                    "highlights": [{"timestamp": "00:12", "title": "Aha moment", "reason": "Useful clip", "suggestedUse": "Short"}],
                }),
                file_path: None,
                created_at: "2026-05-30T00:00:02Z".to_string(),
            },
            AiArtifact {
                id: "4".to_string(),
                session_id: "session".to_string(),
                kind: AiArtifactKind::HealthAssistant,
                status: AiArtifactStatus::Ready,
                content: json!({
                    "explanations": [{"level": "warn", "issue": "Dropped frames", "explanation": "Encoder overload", "action": "Lower bitrate"}],
                }),
                file_path: None,
                created_at: "2026-05-30T00:00:03Z".to_string(),
            },
        ];

        let markdown = render_publish_pack(&artifacts);

        assert!(markdown.contains("# Tutorial Session"));
        assert!(markdown.contains("## Description"));
        assert!(markdown.contains("- 00:00 Intro"));
        assert!(markdown.contains("## Highlights"));
        assert!(markdown.contains("## Health Assistant"));
    }

    #[test]
    fn client_request_component_uses_supported_characters() {
        assert_eq!(
            safe_client_request_component("session id/with spaces"),
            "session_id_with_spaces"
        );
        assert_eq!(
            safe_client_request_component("retry:session_123.1"),
            "retry:session_123.1"
        );
    }

    #[test]
    fn client_request_id_preserves_hash_with_long_session_ids() {
        let hash = "a".repeat(64);
        let id = build_ai_client_request_id("session/with spaces/".repeat(8).as_str(), &hash);

        assert!(id.len() <= 120);
        assert!(id.ends_with(&format!(":{hash}")));
        assert!(!id.contains('/'));
        assert!(!id.contains(' '));
    }
}
