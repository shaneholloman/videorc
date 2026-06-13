use std::path::{Path, PathBuf};
use std::process::Stdio;

use anyhow::{Context, Result, bail};
use reqwest::multipart;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::fs;
use tokio::process::Command;
use tokio::time::{Duration, timeout};

use crate::entitlements;
use crate::ffmpeg::resolve_ffmpeg_path;
use crate::protocol::{
    AiArtifact, AiArtifactKind, AiArtifactStatus, AiWorkflowResult, EntitlementsSnapshot,
    ExportPublishPackParams, ExportPublishPackResult, FeatureId, HealthEvent, HealthLevel,
    RunAiWorkflowParams,
};
use crate::recording::emit_health_event;
use crate::state::AppState;
use crate::storage::default_artifacts_dir;

const OPENAI_TRANSCRIPTIONS_URL: &str = "https://api.openai.com/v1/audio/transcriptions";
const OPENAI_RESPONSES_URL: &str = "https://api.openai.com/v1/responses";
const OPENAI_AUDIO_UPLOAD_LIMIT_BYTES: u64 = 25 * 1024 * 1024;
const TRANSCRIPTION_CHUNK_SECONDS: u64 = 30 * 60;

pub async fn run_ai_workflow(
    state: AppState,
    params: RunAiWorkflowParams,
) -> Result<AiWorkflowResult> {
    let input_path = state
        .database
        .session_output_path(&params.session_id)?
        .map(PathBuf::from)
        .context("Session does not have a local recording output")?;

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

    if let Err(error) = validate_cloud_ai_entitlement(&entitlements::current_entitlements()) {
        artifacts.push(state.database.save_ai_artifact(
            &params.session_id,
            AiArtifactKind::Transcript,
            AiArtifactStatus::Failed,
            json!({
                "message": error.to_string(),
                "entitlement": "cloud-ai",
            }),
            None,
        )?);
        emit_health_event(
            &state,
            Some(&params.session_id),
            HealthLevel::Warn,
            "cloud-ai-premium-required",
            "Audio was extracted locally. Cloud AI did not run because the current entitlement does not include cloud AI.",
        )?;
        emit_ai_artifacts_changed(&state, &params.session_id)?;
        return Ok(AiWorkflowResult {
            session_id: params.session_id,
            audio_path: audio_path.display().to_string(),
            artifacts,
        });
    }

    let api_key = match std::env::var("OPENAI_API_KEY") {
        Ok(value) if !value.trim().is_empty() => value,
        _ => {
            artifacts.push(state.database.save_ai_artifact(
                &params.session_id,
                AiArtifactKind::Transcript,
                AiArtifactStatus::Failed,
                json!({
                    "message": "OPENAI_API_KEY is not configured, so cloud transcription could not run.",
                }),
                None,
            )?);
            emit_health_event(
                &state,
                Some(&params.session_id),
                HealthLevel::Warn,
                "openai-api-key-missing",
                "Set OPENAI_API_KEY before running cloud transcription.",
            )?;
            emit_ai_artifacts_changed(&state, &params.session_id)?;
            return Ok(AiWorkflowResult {
                session_id: params.session_id,
                audio_path: audio_path.display().to_string(),
                artifacts,
            });
        }
    };

    let client = reqwest::Client::new();
    let transcription_inputs = match transcription_audio_inputs(
        &ffmpeg_path,
        &audio_path,
        &artifact_dir,
    )
    .await
    {
        Ok(inputs) => inputs,
        Err(error) => {
            artifacts.push(state.database.save_ai_artifact(
                &params.session_id,
                AiArtifactKind::Transcript,
                AiArtifactStatus::Failed,
                json!({
                    "message": format!("Could not prepare audio for cloud transcription: {error}"),
                    "provider": "openai",
                    "model": transcription_model(),
                }),
                None,
            )?);
            emit_health_event(
                &state,
                Some(&params.session_id),
                HealthLevel::Warn,
                "ai-transcription-prepare-failed",
                "Audio preparation for cloud transcription failed. The local recording and extracted audio are still available.",
            )?;
            emit_ai_artifacts_changed(&state, &params.session_id)?;
            return Ok(AiWorkflowResult {
                session_id: params.session_id,
                audio_path: audio_path.display().to_string(),
                artifacts,
            });
        }
    };
    let transcript = match transcribe_audio_files(&client, &api_key, &transcription_inputs).await {
        Ok(transcript) => transcript,
        Err(error) => {
            artifacts.push(state.database.save_ai_artifact(
                &params.session_id,
                AiArtifactKind::Transcript,
                AiArtifactStatus::Failed,
                json!({
                    "message": format!("Cloud transcription failed: {error}"),
                    "provider": "openai",
                    "model": transcription_model(),
                }),
                None,
            )?);
            emit_health_event(
                &state,
                Some(&params.session_id),
                HealthLevel::Warn,
                "ai-transcription-failed",
                "Cloud transcription failed. The local recording and extracted audio are still available.",
            )?;
            emit_ai_artifacts_changed(&state, &params.session_id)?;
            return Ok(AiWorkflowResult {
                session_id: params.session_id,
                audio_path: audio_path.display().to_string(),
                artifacts,
            });
        }
    };
    artifacts.push(state.database.save_ai_artifact(
        &params.session_id,
        AiArtifactKind::Transcript,
        AiArtifactStatus::Ready,
        json!({
            "text": transcript,
            "provider": "openai",
            "model": transcription_model(),
            "chunked": transcription_inputs.len() > 1,
            "chunkCount": transcription_inputs.len(),
            "uploadLimitBytes": OPENAI_AUDIO_UPLOAD_LIMIT_BYTES,
        }),
        None,
    )?);

    let publish_pack = match summarize_and_chapter(&client, &api_key, &transcript).await {
        Ok(publish_pack) => publish_pack,
        Err(error) => {
            artifacts.push(state.database.save_ai_artifact(
                &params.session_id,
                AiArtifactKind::Summary,
                AiArtifactStatus::Failed,
                json!({
                    "message": format!("Summary and chapter generation failed: {error}"),
                    "provider": "openai",
                    "model": text_model(),
                }),
                None,
            )?);
            emit_health_event(
                &state,
                Some(&params.session_id),
                HealthLevel::Warn,
                "ai-publish-pack-failed",
                "Transcript was saved, but summary and chapter generation failed.",
            )?;
            emit_ai_artifacts_changed(&state, &params.session_id)?;
            return Ok(AiWorkflowResult {
                session_id: params.session_id,
                audio_path: audio_path.display().to_string(),
                artifacts,
            });
        }
    };
    artifacts.push(state.database.save_ai_artifact(
        &params.session_id,
        AiArtifactKind::TitleDescription,
        AiArtifactStatus::Ready,
        json!({
            "title": publish_pack.title,
            "description": publish_pack.description,
            "provider": "openai",
            "model": text_model(),
        }),
        None,
    )?);
    artifacts.push(state.database.save_ai_artifact(
        &params.session_id,
        AiArtifactKind::Summary,
        AiArtifactStatus::Ready,
        json!({
            "text": publish_pack.summary,
            "provider": "openai",
            "model": text_model(),
        }),
        None,
    )?);
    artifacts.push(state.database.save_ai_artifact(
        &params.session_id,
        AiArtifactKind::Chapters,
        AiArtifactStatus::Ready,
        json!({
            "chapters": publish_pack.chapters,
            "provider": "openai",
            "model": text_model(),
        }),
        None,
    )?);

    let health_events = state.database.list_health_events(&params.session_id)?;
    match generate_creator_intelligence(&client, &api_key, &transcript, &health_events).await {
        Ok(intelligence) => {
            artifacts.extend(save_creator_intelligence_artifacts(
                &state,
                &params.session_id,
                intelligence,
            )?);
        }
        Err(error) => {
            artifacts.push(state.database.save_ai_artifact(
                &params.session_id,
                AiArtifactKind::HealthAssistant,
                AiArtifactStatus::Failed,
                json!({
                    "message": format!("Advanced creator intelligence failed: {error}"),
                    "provider": "openai",
                    "model": text_model(),
                }),
                None,
            )?);
            emit_health_event(
                &state,
                Some(&params.session_id),
                HealthLevel::Warn,
                "ai-creator-intelligence-failed",
                "Publish pack was saved, but advanced creator intelligence failed.",
            )?;
        }
    }

    emit_ai_artifacts_changed(&state, &params.session_id)?;

    Ok(AiWorkflowResult {
        session_id: params.session_id,
        audio_path: audio_path.display().to_string(),
        artifacts,
    })
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

fn validate_cloud_ai_entitlement(snapshot: &EntitlementsSnapshot) -> Result<()> {
    entitlements::require_feature(snapshot, FeatureId::CloudAi)
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

    let output = timeout(Duration::from_secs(20 * 60), command.output())
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

async fn transcription_audio_inputs(
    ffmpeg_path: &str,
    audio_path: &Path,
    artifact_dir: &Path,
) -> Result<Vec<PathBuf>> {
    let audio_size = fs::metadata(audio_path)
        .await
        .with_context(|| format!("Could not inspect {}", audio_path.display()))?
        .len();
    if !needs_transcription_chunking(audio_size) {
        return Ok(vec![audio_path.to_path_buf()]);
    }

    let chunk_dir = artifact_dir.join("transcription-chunks");
    match fs::remove_dir_all(&chunk_dir).await {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(error).with_context(|| format!("Could not clear {}", chunk_dir.display()));
        }
    }
    fs::create_dir_all(&chunk_dir)
        .await
        .with_context(|| format!("Could not create {}", chunk_dir.display()))?;

    let audio_arg = audio_path.display().to_string();
    let segment_seconds = TRANSCRIPTION_CHUNK_SECONDS.to_string();
    let chunk_pattern = chunk_dir.join("chunk-%03d.m4a");
    let chunk_pattern_arg = chunk_pattern.display().to_string();
    let mut command = Command::new(ffmpeg_path);
    command
        .args([
            "-y",
            "-hide_banner",
            "-loglevel",
            "warning",
            "-i",
            &audio_arg,
            "-vn",
            "-ac",
            "1",
            "-ar",
            "16000",
            "-c:a",
            "aac",
            "-b:a",
            "64k",
            "-f",
            "segment",
            "-segment_time",
            &segment_seconds,
            "-reset_timestamps",
            "1",
            &chunk_pattern_arg,
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::piped());

    let output = timeout(Duration::from_secs(20 * 60), command.output())
        .await
        .context("FFmpeg transcription audio chunking timed out")?
        .with_context(|| format!("Could not start {ffmpeg_path} for transcription chunking"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        bail!(
            "FFmpeg transcription chunking failed with {}{}",
            output.status,
            if stderr.is_empty() {
                String::new()
            } else {
                format!(": {stderr}")
            }
        );
    }

    let mut chunks = Vec::new();
    let mut entries = fs::read_dir(&chunk_dir)
        .await
        .with_context(|| format!("Could not read {}", chunk_dir.display()))?;
    while let Some(entry) = entries
        .next_entry()
        .await
        .with_context(|| format!("Could not read entry in {}", chunk_dir.display()))?
    {
        let path = entry.path();
        let is_m4a = path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("m4a"));
        if is_m4a {
            chunks.push(path);
        }
    }
    chunks.sort();

    if chunks.is_empty() {
        bail!("FFmpeg transcription chunking did not produce any .m4a chunks");
    }

    for chunk in &chunks {
        let chunk_size = fs::metadata(chunk)
            .await
            .with_context(|| format!("Could not inspect {}", chunk.display()))?
            .len();
        if chunk_size > OPENAI_AUDIO_UPLOAD_LIMIT_BYTES {
            bail!(
                "Transcription chunk {} is larger than the OpenAI upload limit ({} > {} bytes)",
                chunk.display(),
                chunk_size,
                OPENAI_AUDIO_UPLOAD_LIMIT_BYTES
            );
        }
    }

    Ok(chunks)
}

fn needs_transcription_chunking(audio_size: u64) -> bool {
    audio_size > OPENAI_AUDIO_UPLOAD_LIMIT_BYTES
}

async fn transcribe_audio_files(
    client: &reqwest::Client,
    api_key: &str,
    audio_paths: &[PathBuf],
) -> Result<String> {
    if audio_paths.is_empty() {
        bail!("No audio inputs were prepared for cloud transcription");
    }
    if audio_paths.len() == 1 {
        return transcribe_audio(client, api_key, &audio_paths[0]).await;
    }

    let mut transcripts = Vec::with_capacity(audio_paths.len());
    for audio_path in audio_paths {
        transcripts.push(transcribe_audio(client, api_key, audio_path).await?);
    }

    Ok(combine_transcript_chunks(&transcripts))
}

fn combine_transcript_chunks(chunks: &[String]) -> String {
    match chunks {
        [] => String::new(),
        [single] => single.clone(),
        _ => chunks
            .iter()
            .enumerate()
            .map(|(index, text)| format!("[Chunk {}]\n{}", index + 1, text.trim()))
            .collect::<Vec<_>>()
            .join("\n\n"),
    }
}

async fn transcribe_audio(
    client: &reqwest::Client,
    api_key: &str,
    audio_path: &Path,
) -> Result<String> {
    let audio = fs::read(audio_path)
        .await
        .with_context(|| format!("Could not read {}", audio_path.display()))?;
    let file_part = multipart::Part::bytes(audio)
        .file_name("videorc-audio.m4a")
        .mime_str("audio/mp4")?;
    let form = multipart::Form::new()
        .text("model", transcription_model())
        .text("response_format", "json")
        .part("file", file_part);

    let response = client
        .post(OPENAI_TRANSCRIPTIONS_URL)
        .bearer_auth(api_key)
        .multipart(form)
        .send()
        .await?
        .error_for_status()?
        .json::<OpenAiTranscriptionResponse>()
        .await?;

    Ok(response.text)
}

async fn summarize_and_chapter(
    client: &reqwest::Client,
    api_key: &str,
    transcript: &str,
) -> Result<PublishPack> {
    let prompt = format!(
        "You are helping a creator publish a recorded gaming or coding tutorial session.\n\
         Return strict JSON with keys title, description, summary, and chapters. \
         title should be one strong YouTube-style title under 80 characters. \
         description should be a concise publish-ready description. \
         chapters must be an array of objects with timestamp and title. \
         Use approximate timestamps if the transcript has no timings.\n\n\
         Transcript:\n{transcript}"
    );
    let response = client
        .post(OPENAI_RESPONSES_URL)
        .bearer_auth(api_key)
        .json(&json!({
            "model": text_model(),
            "input": prompt,
        }))
        .send()
        .await?
        .error_for_status()?
        .json::<Value>()
        .await?;
    let output_text = response
        .get("output_text")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| response_output_text(&response))
        .unwrap_or_default();

    parse_publish_pack(&output_text)
}

async fn generate_creator_intelligence(
    client: &reqwest::Client,
    api_key: &str,
    transcript: &str,
    health_events: &[HealthEvent],
) -> Result<CreatorIntelligence> {
    let health_context = if health_events.is_empty() {
        "No health events were recorded for this session.".to_string()
    } else {
        health_events
            .iter()
            .map(|event| {
                format!(
                    "- {:?} [{}] {} ({})",
                    event.level, event.code, event.message, event.created_at
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    let prompt = format!(
        "You are Videorc's creator intelligence assistant for gaming and coding/tutorial recordings.\n\
         Return strict JSON with keys highlights, smartZoom, noiseCleanup, silenceRemoval, and healthAssistant.\n\
         highlights: array of objects with timestamp, title, reason, suggestedUse.\n\
         smartZoom: array of objects with timestamp, action, subject, reason.\n\
         noiseCleanup: array of objects with issue, suggestion, confidence.\n\
         silenceRemoval: array of objects with timestamp, reason, editSuggestion.\n\
         healthAssistant: array of objects with level, issue, explanation, action.\n\
         Prefer concrete creator-editing advice. If signal is weak, return short conservative arrays rather than guessing.\n\n\
         Health events:\n{health_context}\n\n\
         Transcript:\n{transcript}"
    );
    let response = client
        .post(OPENAI_RESPONSES_URL)
        .bearer_auth(api_key)
        .json(&json!({
            "model": text_model(),
            "input": prompt,
        }))
        .send()
        .await?
        .error_for_status()?
        .json::<Value>()
        .await?;
    let output_text = response
        .get("output_text")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| response_output_text(&response))
        .unwrap_or_default();

    parse_creator_intelligence(&output_text)
}

fn parse_publish_pack(output_text: &str) -> Result<PublishPack> {
    if let Ok(pack) = serde_json::from_str::<PublishPack>(output_text) {
        return Ok(pack);
    }

    let Some(start) = output_text.find('{') else {
        bail!("AI response did not include JSON");
    };
    let Some(end) = output_text.rfind('}') else {
        bail!("AI response did not include complete JSON");
    };
    serde_json::from_str(&output_text[start..=end]).context("Could not parse AI publish pack JSON")
}

fn parse_creator_intelligence(output_text: &str) -> Result<CreatorIntelligence> {
    if let Ok(intelligence) = serde_json::from_str::<CreatorIntelligence>(output_text) {
        return Ok(intelligence);
    }

    let Some(start) = output_text.find('{') else {
        bail!("AI response did not include JSON");
    };
    let Some(end) = output_text.rfind('}') else {
        bail!("AI response did not include complete JSON");
    };
    serde_json::from_str(&output_text[start..=end])
        .context("Could not parse AI creator intelligence JSON")
}

fn save_creator_intelligence_artifacts(
    state: &AppState,
    session_id: &str,
    intelligence: CreatorIntelligence,
) -> Result<Vec<AiArtifact>> {
    Ok(vec![
        state.database.save_ai_artifact(
            session_id,
            AiArtifactKind::Highlights,
            AiArtifactStatus::Ready,
            json!({
                "highlights": intelligence.highlights,
                "provider": "openai",
                "model": text_model(),
            }),
            None,
        )?,
        state.database.save_ai_artifact(
            session_id,
            AiArtifactKind::SmartZoom,
            AiArtifactStatus::Ready,
            json!({
                "suggestions": intelligence.smart_zoom,
                "provider": "openai",
                "model": text_model(),
            }),
            None,
        )?,
        state.database.save_ai_artifact(
            session_id,
            AiArtifactKind::NoiseCleanup,
            AiArtifactStatus::Ready,
            json!({
                "suggestions": intelligence.noise_cleanup,
                "provider": "openai",
                "model": text_model(),
            }),
            None,
        )?,
        state.database.save_ai_artifact(
            session_id,
            AiArtifactKind::SilenceRemoval,
            AiArtifactStatus::Ready,
            json!({
                "suggestions": intelligence.silence_removal,
                "provider": "openai",
                "model": text_model(),
            }),
            None,
        )?,
        state.database.save_ai_artifact(
            session_id,
            AiArtifactKind::HealthAssistant,
            AiArtifactStatus::Ready,
            json!({
                "explanations": intelligence.health_assistant,
                "provider": "openai",
                "model": text_model(),
            }),
            None,
        )?,
    ])
}

fn response_output_text(value: &Value) -> Option<String> {
    let mut chunks = Vec::new();
    for item in value.get("output")?.as_array()? {
        for content in item.get("content")?.as_array()? {
            if let Some(text) = content.get("text").and_then(Value::as_str) {
                chunks.push(text.to_string());
            }
        }
    }
    (!chunks.is_empty()).then(|| chunks.join("\n"))
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

fn transcription_model() -> String {
    std::env::var("VIDEORC_OPENAI_TRANSCRIPTION_MODEL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "gpt-4o-mini-transcribe".to_string())
}

fn text_model() -> String {
    std::env::var("VIDEORC_OPENAI_TEXT_MODEL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "gpt-5-mini".to_string())
}

#[derive(Debug, Deserialize)]
struct OpenAiTranscriptionResponse {
    text: String,
}

#[derive(Debug, Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct PublishPack {
    title: String,
    description: String,
    summary: String,
    chapters: Vec<Chapter>,
}

#[derive(Debug, Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct Chapter {
    timestamp: String,
    title: String,
}

#[derive(Debug, Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct CreatorIntelligence {
    #[serde(default)]
    highlights: Vec<HighlightSuggestion>,
    #[serde(default)]
    smart_zoom: Vec<SmartZoomSuggestion>,
    #[serde(default)]
    noise_cleanup: Vec<NoiseCleanupSuggestion>,
    #[serde(default)]
    silence_removal: Vec<SilenceRemovalSuggestion>,
    #[serde(default)]
    health_assistant: Vec<HealthAssistantExplanation>,
}

#[derive(Debug, Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct HighlightSuggestion {
    #[serde(default)]
    timestamp: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    reason: String,
    #[serde(default)]
    suggested_use: String,
}

#[derive(Debug, Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct SmartZoomSuggestion {
    #[serde(default)]
    timestamp: String,
    #[serde(default)]
    action: String,
    #[serde(default)]
    subject: String,
    #[serde(default)]
    reason: String,
}

#[derive(Debug, Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct NoiseCleanupSuggestion {
    #[serde(default)]
    issue: String,
    #[serde(default)]
    suggestion: String,
    #[serde(default)]
    confidence: String,
}

#[derive(Debug, Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct SilenceRemovalSuggestion {
    #[serde(default)]
    timestamp: String,
    #[serde(default)]
    reason: String,
    #[serde(default)]
    edit_suggestion: String,
}

#[derive(Debug, Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct HealthAssistantExplanation {
    #[serde(default)]
    level: String,
    #[serde(default)]
    issue: String,
    #[serde(default)]
    explanation: String,
    #[serde(default)]
    action: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_json_from_model_output() {
        let pack = parse_publish_pack(
            r#"```json
{"title":"Build a CLI in Rust","description":"A quick walkthrough.","summary":"A short session.","chapters":[{"timestamp":"00:00","title":"Intro"}]}
```"#,
        )
        .unwrap();

        assert_eq!(pack.title, "Build a CLI in Rust");
        assert_eq!(pack.description, "A quick walkthrough.");
        assert_eq!(pack.summary, "A short session.");
        assert_eq!(pack.chapters[0].title, "Intro");
    }

    #[test]
    fn parses_creator_intelligence_from_model_output() {
        let intelligence = parse_creator_intelligence(
            r#"```json
{"highlights":[{"timestamp":"00:12","title":"Fix the auth bug","reason":"Clear aha moment","suggestedUse":"Short clip"}],"smartZoom":[{"timestamp":"00:18","action":"Zoom editor","subject":"diff hunk","reason":"Small code text"}],"noiseCleanup":[{"issue":"Keyboard noise","suggestion":"Apply light gate","confidence":"medium"}],"silenceRemoval":[{"timestamp":"01:02","reason":"Long pause","editSuggestion":"Trim 4 seconds"}],"healthAssistant":[{"level":"warn","issue":"Dropped frames","explanation":"Encoder fell behind","action":"Lower bitrate"}]}
```"#,
        )
        .unwrap();

        assert_eq!(intelligence.highlights[0].title, "Fix the auth bug");
        assert_eq!(intelligence.smart_zoom[0].subject, "diff hunk");
        assert_eq!(intelligence.noise_cleanup[0].issue, "Keyboard noise");
        assert_eq!(
            intelligence.silence_removal[0].edit_suggestion,
            "Trim 4 seconds"
        );
        assert_eq!(intelligence.health_assistant[0].action, "Lower bitrate");
    }

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
    fn entitlement_guard_blocks_cloud_ai_in_basic_mode() {
        let snapshot = entitlements::entitlements_from_env_value(None);
        let error = validate_cloud_ai_entitlement(&snapshot)
            .expect_err("cloud AI should require premium entitlement");

        assert!(error.to_string().contains("Premium"));
    }

    #[test]
    fn entitlement_guard_allows_cloud_ai_with_developer_override() {
        let snapshot = entitlements::entitlements_from_env_value(Some("1"));

        validate_cloud_ai_entitlement(&snapshot).unwrap();
    }

    #[test]
    fn transcription_chunking_starts_after_upload_limit() {
        assert!(!needs_transcription_chunking(
            OPENAI_AUDIO_UPLOAD_LIMIT_BYTES
        ));
        assert!(needs_transcription_chunking(
            OPENAI_AUDIO_UPLOAD_LIMIT_BYTES + 1
        ));
    }

    #[test]
    fn combines_multiple_transcript_chunks_with_order_labels() {
        let single = combine_transcript_chunks(&["leave me alone".to_string()]);
        assert_eq!(single, "leave me alone");

        let combined =
            combine_transcript_chunks(&["first chunk ".to_string(), "\nsecond chunk".to_string()]);

        assert_eq!(
            combined,
            "[Chunk 1]\nfirst chunk\n\n[Chunk 2]\nsecond chunk"
        );
    }
}
