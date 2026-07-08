//! X Livestream read-only chat connector.
//!
//! The X Livestream API document describes chat as a legacy Periscope/X handoff:
//! fetch a chat token from `api.twitter.com`, exchange it through
//! `proxsee.pscp.tv`, then connect to the returned WebSocket endpoint. This
//! module implements read access only. Sending X chat is intentionally
//! unsupported because the documented API does not include a send flow.

use std::time::Duration;

use anyhow::{Context, Result};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::time::sleep;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;
use uuid::Uuid;

use crate::live_chat::{
    LiveChatEventType, LiveChatMessage, LiveChatProviderConnectionState, deliver_message,
    live_chat_message_id, set_provider_and_emit,
};
use crate::state::AppState;
use crate::streaming::StreamPlatform;

const CHAT_STATUS_BASE_URL: &str = "https://api.twitter.com";
const CHAT_ACCESS_URL: &str = "https://proxsee.pscp.tv/api/v2/accessChatPublic";
const PERISCOPE_USER_AGENT: &str = "Twitter/m5";
const CHAT_TOKEN_ATTEMPTS: usize = 10;
const CHAT_TOKEN_RETRY_MS: u64 = 2_000;

pub const X_NATIVE_COMMENTS_AVAILABLE: bool = true;

pub const X_COMMENTS_EVIDENCE_CHECKLIST: &[&str] = &[
    "Official X Livestream API documentation covers read-only live chat token handoff.",
    "Approved X Livestream API access exists for source and broadcast lifecycle.",
    "The connector uses only documented read endpoints and does not send X chat messages.",
];

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct XChatConfig {
    pub broadcast_id: String,
    pub media_key: String,
    #[serde(default)]
    pub target_id: Option<String>,
    #[serde(default)]
    pub status_base_url: Option<String>,
    #[serde(default)]
    pub access_url: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct XChatStatusResponse {
    #[serde(default)]
    chat_token: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
struct XChatAccessResponse {
    #[serde(default)]
    endpoint: Option<String>,
    #[serde(default)]
    access_token: Option<String>,
    #[serde(default)]
    replay_endpoint: Option<String>,
    #[serde(default)]
    replay_access_token: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct XChatFrame {
    kind: i64,
    #[serde(default)]
    payload: Option<String>,
}

pub fn x_chat_message(has_x_account: bool) -> &'static str {
    if has_x_account {
        "X live chat can be read for native X broadcasts."
    } else {
        "Connect or configure X native live before reading X chat."
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct XChatReadiness {
    pub available: bool,
    pub message: String,
    pub evidence_checklist: Vec<String>,
}

pub fn x_chat_readiness(has_x_account: bool) -> XChatReadiness {
    XChatReadiness {
        available: X_NATIVE_COMMENTS_AVAILABLE,
        message: x_chat_message(has_x_account).to_string(),
        evidence_checklist: X_COMMENTS_EVIDENCE_CHECKLIST
            .iter()
            .map(|item| (*item).to_string())
            .collect(),
    }
}

pub async fn run_x_chat_connector(state: AppState, session_id: String, config: XChatConfig) {
    set_provider_and_emit(
        &state,
        StreamPlatform::X,
        LiveChatProviderConnectionState::Connecting,
        "Connecting to X live chat.",
    )
    .await;

    match run_x_chat_session(&state, &session_id, &config).await {
        Ok(()) => {
            set_provider_and_emit(
                &state,
                StreamPlatform::X,
                LiveChatProviderConnectionState::Ended,
                "X live chat ended.",
            )
            .await;
        }
        Err(error) => {
            set_provider_and_emit(
                &state,
                StreamPlatform::X,
                LiveChatProviderConnectionState::Failed,
                &format!("X live chat failed: {error}"),
            )
            .await;
        }
    }
}

async fn run_x_chat_session(
    state: &AppState,
    session_id: &str,
    config: &XChatConfig,
) -> Result<()> {
    let client = reqwest::Client::new();
    let chat_token = fetch_chat_token_with_retry(&client, config).await?;
    let access = access_chat(&client, config, &chat_token).await?;
    let endpoint = access
        .endpoint
        .or(access.replay_endpoint)
        .context("X chat access response did not include an endpoint.")?;
    let access_token = access
        .access_token
        .or(access.replay_access_token)
        .context("X chat access response did not include an access token.")?;
    let ws_url = chat_ws_url(&endpoint)?;
    let (mut ws, _response) = connect_async(&ws_url)
        .await
        .with_context(|| format!("Could not connect to X chat WebSocket {ws_url}"))?;

    ws.send(Message::Text(x_auth_frame(&access_token).into()))
        .await
        .context("Could not authenticate X chat WebSocket.")?;
    ws.send(Message::Text(
        x_subscribe_frame(&config.broadcast_id).into(),
    ))
    .await
    .context("Could not subscribe to X chat room.")?;

    set_provider_and_emit(
        state,
        StreamPlatform::X,
        LiveChatProviderConnectionState::Connected,
        "X live chat connected.",
    )
    .await;

    while let Some(message) = ws.next().await {
        let message = message.context("X chat WebSocket read failed.")?;
        let Message::Text(text) = message else {
            continue;
        };
        if let Some(chat_message) =
            parse_x_chat_message(&text, session_id, config.target_id.as_deref())
        {
            deliver_message(state, chat_message).await;
        }
    }

    Ok(())
}

async fn fetch_chat_token_with_retry(
    client: &reqwest::Client,
    config: &XChatConfig,
) -> Result<String> {
    let mut last_error = None;
    for attempt in 0..CHAT_TOKEN_ATTEMPTS {
        match fetch_chat_token(client, config).await {
            Ok(token) => return Ok(token),
            Err(error) => last_error = Some(error),
        }
        if attempt + 1 < CHAT_TOKEN_ATTEMPTS {
            sleep(Duration::from_millis(CHAT_TOKEN_RETRY_MS)).await;
        }
    }
    Err(last_error
        .unwrap_or_else(|| anyhow::anyhow!("X chat token was not available after publishing.")))
}

async fn fetch_chat_token(client: &reqwest::Client, config: &XChatConfig) -> Result<String> {
    let base = config
        .status_base_url
        .as_deref()
        .unwrap_or(CHAT_STATUS_BASE_URL)
        .trim_end_matches('/');
    let url = format!("{base}/1.1/live_video_stream/status/{}", config.media_key);
    let response = client
        .get(url)
        .header("x-periscope-user-agent", PERISCOPE_USER_AGENT)
        .send()
        .await
        .context("Could not fetch X chat token.")?;
    if !response.status().is_success() {
        let status = response.status();
        anyhow::bail!("X chat token request failed with HTTP {status}");
    }
    let body = response
        .json::<XChatStatusResponse>()
        .await
        .context("Could not parse X chat token response.")?;
    body.chat_token
        .filter(|token| !token.trim().is_empty())
        .context("X chat token is not available yet.")
}

async fn access_chat(
    client: &reqwest::Client,
    config: &XChatConfig,
    chat_token: &str,
) -> Result<XChatAccessResponse> {
    let response = client
        .post(config.access_url.as_deref().unwrap_or(CHAT_ACCESS_URL))
        .header("content-type", "application/json")
        .header("x-periscope-user-agent", PERISCOPE_USER_AGENT)
        .header("x-idempotence", Uuid::new_v4().to_string())
        .header("x-attempt", "1")
        .json(&json!({ "chat_token": chat_token }))
        .send()
        .await
        .context("Could not request X chat access.")?;
    if !response.status().is_success() {
        let status = response.status();
        anyhow::bail!("X chat access request failed with HTTP {status}");
    }
    response
        .json::<XChatAccessResponse>()
        .await
        .context("Could not parse X chat access response.")
}

fn chat_ws_url(endpoint: &str) -> Result<String> {
    let url = reqwest::Url::parse(endpoint).context("X chat endpoint URL is invalid.")?;
    let host = url
        .host_str()
        .context("X chat endpoint URL did not include a host.")?;
    Ok(format!("wss://{host}/chatapi/v1/chatnow"))
}

fn x_auth_frame(access_token: &str) -> String {
    json!({
        "kind": 3,
        "payload": json!({ "access_token": access_token }).to_string()
    })
    .to_string()
}

fn x_subscribe_frame(broadcast_id: &str) -> String {
    json!({
        "kind": 2,
        "payload": json!({
            "kind": 1,
            "payload": json!({ "room": broadcast_id }).to_string()
        }).to_string()
    })
    .to_string()
}

fn parse_x_chat_message(
    text: &str,
    session_id: &str,
    target_id: Option<&str>,
) -> Option<LiveChatMessage> {
    let frame: XChatFrame = serde_json::from_str(text).ok()?;
    if frame.kind != 1 {
        return None;
    }
    let payload = frame.payload?;
    let payload: Value = serde_json::from_str(&payload).ok()?;
    let body = payload
        .get("body")
        .and_then(|body| body.as_str())
        .and_then(|body| serde_json::from_str::<Value>(body).ok())
        .unwrap_or(payload);
    let text = body
        .get("body")
        .or_else(|| body.get("text"))
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    let username = body
        .get("username")
        .or_else(|| body.get("displayName"))
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("X viewer");
    let provider_message_id = body
        .get("uuid")
        .or_else(|| body.get("id"))
        .and_then(|value| value.as_str())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| {
            format!(
                "{}:{}:{}",
                username,
                body.get("timestamp")
                    .map(Value::to_string)
                    .unwrap_or_default(),
                text
            )
        });
    let now = chrono::Utc::now().to_rfc3339();
    Some(LiveChatMessage {
        id: live_chat_message_id(StreamPlatform::X, &provider_message_id),
        provider_message_id,
        platform: StreamPlatform::X,
        target_id: target_id.map(ToOwned::to_owned),
        session_id: session_id.to_string(),
        author_id: body
            .get("user_id")
            .or_else(|| body.get("userId"))
            .and_then(|value| value.as_str())
            .map(ToOwned::to_owned),
        author_name: username.to_string(),
        author_avatar_url: None,
        author_badges: Vec::new(),
        author_roles: Vec::new(),
        published_at: now.clone(),
        received_at: now,
        message_text: text.to_string(),
        fragments: Vec::new(),
        event_type: LiveChatEventType::Message,
        amount_text: None,
        is_deleted: false,
        raw_provider_type: Some("x-chat".to_string()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readiness_reports_available_read_only_path() {
        let readiness = x_chat_readiness(true);
        assert!(readiness.available);
        assert!(readiness.message.contains("read"));
        assert_eq!(readiness.evidence_checklist.len(), 3);
    }

    #[test]
    fn websocket_url_uses_returned_host() {
        assert_eq!(
            chat_ws_url("https://prod-chat-ancillary-eu-central-1.pscp.tv").unwrap(),
            "wss://prod-chat-ancillary-eu-central-1.pscp.tv/chatapi/v1/chatnow"
        );
    }

    #[test]
    fn frames_double_encode_payloads() {
        let auth = x_auth_frame("access");
        assert!(auth.contains(r#""kind":3"#));
        assert!(auth.contains(r#"access_token"#));

        let subscribe = x_subscribe_frame("broadcast-1");
        assert!(subscribe.contains(r#""kind":2"#));
        assert!(subscribe.contains("broadcast-1"));
    }

    #[test]
    fn parses_double_json_chat_message() {
        let body = json!({
            "body": "hello from x",
            "username": "viewer",
            "uuid": "message-1"
        });
        let frame = json!({
            "kind": 1,
            "payload": json!({ "body": body.to_string() }).to_string()
        });

        let message = parse_x_chat_message(&frame.to_string(), "session-1", Some("x"))
            .expect("message parsed");

        assert_eq!(message.platform, StreamPlatform::X);
        assert_eq!(message.provider_message_id, "message-1");
        assert_eq!(message.author_name, "viewer");
        assert_eq!(message.message_text, "hello from x");
        assert_eq!(message.target_id.as_deref(), Some("x"));
    }
}
