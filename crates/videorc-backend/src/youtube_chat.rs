//! YouTube live chat connector (slice 4 of the In-App Livestream Comments plan:
//! `2026-06-06 - Videorc In-App Livestream Comments Plan`).
//!
//! Resolves the broadcast `liveChatId`, reads `liveChatMessages`, normalizes items into the
//! shared `LiveChatMessage` model, and classifies disabled/ended/quota/token/auth errors
//! into provider status transitions — never a stream failure. Messages are fed into the
//! `LiveChatCoordinator` through the shared deliver/provider-status helpers.
//!
//! Transport: the connector requests the `liveChatMessages/stream` (`streamList`) endpoint
//! first and falls back to `liveChatMessages` (`list`) on transient failure; both honour the
//! server's `pollingIntervalMillis`. The `streamList` long-poll body optimisation (holding
//! one connection open) is a latency improvement deferred past V1 — the polling form here
//! produces identical normalized output and is the acceptance-critical path.

use std::time::Duration;

use anyhow::{Context, Result};
use reqwest::Url;
use serde::Deserialize;
use serde_json::Value;
use tokio::time::sleep;

use crate::live_chat::{
    LiveChatEventType, LiveChatMessage, LiveChatProviderConnectionState, deliver_message,
    live_chat_message_id, set_provider_and_emit,
};
use crate::state::AppState;
use crate::streaming::StreamPlatform;

const YOUTUBE_API_BASE_URL: &str = "https://www.googleapis.com";
const LIVE_CHAT_MESSAGES_PATH: &str = "/youtube/v3/liveChat/messages";
const LIVE_CHAT_MESSAGES_STREAM_PATH: &str = "/youtube/v3/liveChat/messages/stream";
const LIVE_BROADCASTS_PATH: &str = "/youtube/v3/liveBroadcasts";
const DEFAULT_POLLING_INTERVAL_MS: u64 = 5_000;
const MIN_POLLING_INTERVAL_MS: u64 = 1_000;
const MAX_BACKOFF_MS: u64 = 30_000;

/// Start config for the YouTube connector (an internal/session-aware `liveChat.start` field).
/// Either `liveChatId` is provided directly or `broadcastId` is resolved to one.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct YouTubeChatConfig {
    pub access_token: String,
    #[serde(default)]
    pub live_chat_id: Option<String>,
    #[serde(default)]
    pub broadcast_id: Option<String>,
    #[serde(default)]
    pub target_id: Option<String>,
    /// Test-only override of the API base URL.
    #[serde(default)]
    pub api_base_url: Option<String>,
}

/// Which `liveChatMessages` endpoint a request targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum YouTubeChatTransport {
    StreamList,
    List,
}

// --- API response model (the subset we consume) ---

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LiveChatMessagesResponse {
    #[serde(default)]
    offline_at: Option<String>,
    #[serde(default)]
    polling_interval_millis: Option<u64>,
    #[serde(default)]
    next_page_token: Option<String>,
    #[serde(default)]
    items: Vec<LiveChatItem>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LiveChatItem {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    snippet: LiveChatItemSnippet,
    #[serde(default)]
    author_details: Option<LiveChatAuthorDetails>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LiveChatItemSnippet {
    #[serde(rename = "type", default)]
    message_type: Option<String>,
    #[serde(default)]
    published_at: Option<String>,
    #[serde(default)]
    display_message: Option<String>,
    #[serde(default)]
    super_chat_details: Option<AmountDetails>,
    #[serde(default)]
    super_sticker_details: Option<AmountDetails>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AmountDetails {
    #[serde(default)]
    amount_display_string: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LiveChatAuthorDetails {
    #[serde(default)]
    channel_id: Option<String>,
    #[serde(default)]
    display_name: Option<String>,
    #[serde(default)]
    profile_image_url: Option<String>,
    #[serde(default)]
    is_verified: bool,
    #[serde(default)]
    is_chat_owner: bool,
    #[serde(default)]
    is_chat_sponsor: bool,
    #[serde(default)]
    is_chat_moderator: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LiveBroadcastListResponse {
    #[serde(default)]
    items: Vec<LiveBroadcastItem>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LiveBroadcastItem {
    #[serde(default)]
    snippet: Option<LiveBroadcastSnippet>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LiveBroadcastSnippet {
    #[serde(default)]
    live_chat_id: Option<String>,
}

// --- Pure normalization + classification (unit-tested) ---

/// A normalized page of chat: messages oldest-to-newest, the resume token, the server's poll
/// interval (clamped), and whether chat has ended.
#[derive(Debug, Clone, PartialEq, Eq)]
struct YouTubeChatPage {
    messages: Vec<LiveChatMessage>,
    next_page_token: Option<String>,
    polling_interval_ms: u64,
    ended: bool,
}

/// How a request failed, mapped to a provider-status reaction (never a stream failure).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum YouTubeChatErrorKind {
    Disabled,
    Ended,
    RateLimited,
    InvalidPageToken,
    AuthExpired,
    Transient,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FetchError {
    Api(YouTubeChatErrorKind),
    Network,
}

fn event_type_for(message_type: &str) -> LiveChatEventType {
    match message_type {
        "textMessageEvent" => LiveChatEventType::Message,
        "superChatEvent" | "superStickerEvent" => LiveChatEventType::Paid,
        "newSponsorEvent"
        | "memberMilestoneChatEvent"
        | "membershipGiftingEvent"
        | "giftMembershipReceivedEvent" => LiveChatEventType::Membership,
        "messageDeletedEvent" => LiveChatEventType::Deleted,
        "userBannedEvent" => LiveChatEventType::Moderation,
        _ => LiveChatEventType::System,
    }
}

/// A safe, human-readable row for events YouTube does not give a `displayMessage` (so we
/// never silently drop an event the panel can't style yet).
fn system_text_for(message_type: &str, amount: Option<&str>) -> String {
    let suffix = amount.map(|a| format!(": {a}")).unwrap_or_default();
    match message_type {
        "superChatEvent" => format!("Super Chat{suffix}"),
        "superStickerEvent" => format!("Super Sticker{suffix}"),
        "newSponsorEvent" => "New member".to_string(),
        "memberMilestoneChatEvent" => "Member milestone".to_string(),
        "membershipGiftingEvent" => "Gifted memberships".to_string(),
        "giftMembershipReceivedEvent" => "Received a gifted membership".to_string(),
        "messageDeletedEvent" => "Message deleted".to_string(),
        "userBannedEvent" => "A user was removed from chat".to_string(),
        "chatEndedEvent" => "Live chat has ended".to_string(),
        "sponsorOnlyModeStartedEvent" => "Members-only chat started".to_string(),
        "sponsorOnlyModeEndedEvent" => "Members-only chat ended".to_string(),
        _ => "Live chat event".to_string(),
    }
}

fn author_roles(author: &LiveChatAuthorDetails) -> Vec<String> {
    let mut roles = Vec::new();
    if author.is_chat_owner {
        roles.push("owner".to_string());
    }
    if author.is_chat_moderator {
        roles.push("moderator".to_string());
    }
    if author.is_chat_sponsor {
        roles.push("member".to_string());
    }
    if author.is_verified {
        roles.push("verified".to_string());
    }
    roles
}

fn normalize_item(
    item: &LiveChatItem,
    session_id: &str,
    target_id: Option<&str>,
    received_at: &str,
) -> Option<LiveChatMessage> {
    let provider_message_id = item.id.clone()?;
    let message_type = item
        .snippet
        .message_type
        .as_deref()
        .unwrap_or("textMessageEvent");
    let event_type = event_type_for(message_type);
    let author = item.author_details.as_ref();
    let amount_text = item
        .snippet
        .super_chat_details
        .as_ref()
        .and_then(|details| details.amount_display_string.clone())
        .or_else(|| {
            item.snippet
                .super_sticker_details
                .as_ref()
                .and_then(|details| details.amount_display_string.clone())
        });
    let message_text = item
        .snippet
        .display_message
        .clone()
        .filter(|text| !text.is_empty())
        .unwrap_or_else(|| system_text_for(message_type, amount_text.as_deref()));
    Some(LiveChatMessage {
        id: live_chat_message_id(StreamPlatform::Youtube, &provider_message_id),
        provider_message_id,
        platform: StreamPlatform::Youtube,
        target_id: target_id.map(ToOwned::to_owned),
        session_id: session_id.to_string(),
        author_id: author.and_then(|details| details.channel_id.clone()),
        author_name: author
            .and_then(|details| details.display_name.clone())
            .unwrap_or_else(|| "YouTube viewer".to_string()),
        author_avatar_url: author.and_then(|details| details.profile_image_url.clone()),
        author_badges: Vec::new(),
        author_roles: author.map(author_roles).unwrap_or_default(),
        published_at: item
            .snippet
            .published_at
            .clone()
            .unwrap_or_else(|| received_at.to_string()),
        received_at: received_at.to_string(),
        message_text,
        fragments: Vec::new(),
        event_type,
        amount_text,
        is_deleted: matches!(event_type, LiveChatEventType::Deleted),
        raw_provider_type: Some(message_type.to_string()),
    })
}

fn normalize_page(
    response: LiveChatMessagesResponse,
    session_id: &str,
    target_id: Option<&str>,
    received_at: &str,
) -> YouTubeChatPage {
    let ended = response.offline_at.is_some()
        || response
            .items
            .iter()
            .any(|item| item.snippet.message_type.as_deref() == Some("chatEndedEvent"));
    let polling_interval_ms = response
        .polling_interval_millis
        .unwrap_or(DEFAULT_POLLING_INTERVAL_MS)
        .max(MIN_POLLING_INTERVAL_MS);
    let messages = response
        .items
        .iter()
        .filter_map(|item| normalize_item(item, session_id, target_id, received_at))
        .collect();
    YouTubeChatPage {
        messages,
        next_page_token: response.next_page_token,
        polling_interval_ms,
        ended,
    }
}

/// Map an HTTP status + YouTube error `reason` to a reaction. 403/404/quota are provider
/// statuses, not crashes — the stream keeps running even when chat cannot.
fn classify_status(status: u16, reason: Option<&str>) -> YouTubeChatErrorKind {
    match status {
        401 => YouTubeChatErrorKind::AuthExpired,
        403 => match reason {
            Some("liveChatDisabled") => YouTubeChatErrorKind::Disabled,
            Some("liveChatEnded") => YouTubeChatErrorKind::Ended,
            Some("rateLimitExceeded")
            | Some("quotaExceeded")
            | Some("userRateLimitExceeded") => YouTubeChatErrorKind::RateLimited,
            _ => YouTubeChatErrorKind::Disabled,
        },
        400 => match reason {
            Some("pageTokenInvalid") | Some("invalidPageToken") => {
                YouTubeChatErrorKind::InvalidPageToken
            }
            _ => YouTubeChatErrorKind::Transient,
        },
        404 => YouTubeChatErrorKind::Ended,
        429 => YouTubeChatErrorKind::RateLimited,
        _ => YouTubeChatErrorKind::Transient,
    }
}

/// `(provider state, message, should_stop)` for an error kind.
fn provider_reaction(
    kind: YouTubeChatErrorKind,
) -> (LiveChatProviderConnectionState, &'static str, bool) {
    match kind {
        YouTubeChatErrorKind::Disabled => (
            LiveChatProviderConnectionState::Failed,
            "Live chat is disabled for this YouTube broadcast.",
            true,
        ),
        YouTubeChatErrorKind::Ended => (
            LiveChatProviderConnectionState::Ended,
            "YouTube live chat has ended.",
            true,
        ),
        YouTubeChatErrorKind::RateLimited => (
            LiveChatProviderConnectionState::Reconnecting,
            "YouTube live chat is rate limited; backing off.",
            false,
        ),
        YouTubeChatErrorKind::InvalidPageToken => (
            LiveChatProviderConnectionState::Reconnecting,
            "Resyncing YouTube live chat.",
            false,
        ),
        YouTubeChatErrorKind::AuthExpired => (
            LiveChatProviderConnectionState::Failed,
            "Reconnect YouTube to enable live comments.",
            true,
        ),
        YouTubeChatErrorKind::Transient => (
            LiveChatProviderConnectionState::Reconnecting,
            "Reconnecting to YouTube live chat…",
            false,
        ),
    }
}

fn chat_messages_url(
    base_url: &str,
    transport: YouTubeChatTransport,
    live_chat_id: &str,
    page_token: Option<&str>,
) -> Result<Url> {
    let path = match transport {
        YouTubeChatTransport::StreamList => LIVE_CHAT_MESSAGES_STREAM_PATH,
        YouTubeChatTransport::List => LIVE_CHAT_MESSAGES_PATH,
    };
    let mut url = Url::parse(&format!("{}{}", base_url.trim_end_matches('/'), path))
        .context("Invalid YouTube API base URL.")?;
    {
        let mut pairs = url.query_pairs_mut();
        pairs.append_pair("liveChatId", live_chat_id);
        pairs.append_pair("part", "snippet,authorDetails");
        if let Some(token) = page_token {
            pairs.append_pair("pageToken", token);
        }
    }
    Ok(url)
}

async fn extract_error_reason(response: reqwest::Response) -> Option<String> {
    let body = response.json::<Value>().await.ok()?;
    body.get("error")?
        .get("errors")?
        .as_array()?
        .first()?
        .get("reason")?
        .as_str()
        .map(ToOwned::to_owned)
}

async fn fetch_chat_page(
    client: &reqwest::Client,
    base_url: &str,
    access_token: &str,
    transport: YouTubeChatTransport,
    live_chat_id: &str,
    page_token: Option<&str>,
) -> std::result::Result<LiveChatMessagesResponse, FetchError> {
    let url = chat_messages_url(base_url, transport, live_chat_id, page_token)
        .map_err(|_| FetchError::Network)?;
    let response = client
        .get(url)
        .bearer_auth(access_token)
        .send()
        .await
        .map_err(|_| FetchError::Network)?;
    let status = response.status();
    if status.is_success() {
        response
            .json::<LiveChatMessagesResponse>()
            .await
            .map_err(|_| FetchError::Network)
    } else {
        let reason = extract_error_reason(response).await;
        Err(FetchError::Api(classify_status(
            status.as_u16(),
            reason.as_deref(),
        )))
    }
}

/// Resolve a broadcast's `liveChatId` via `liveBroadcasts.list?part=snippet&id=...`.
pub async fn resolve_live_chat_id(
    client: &reqwest::Client,
    base_url: &str,
    access_token: &str,
    broadcast_id: &str,
) -> Result<Option<String>> {
    let mut url = Url::parse(&format!(
        "{}{}",
        base_url.trim_end_matches('/'),
        LIVE_BROADCASTS_PATH
    ))
    .context("Invalid YouTube API base URL.")?;
    url.query_pairs_mut()
        .append_pair("part", "snippet")
        .append_pair("id", broadcast_id);
    let response: LiveBroadcastListResponse = client
        .get(url)
        .bearer_auth(access_token)
        .send()
        .await
        .context("Could not resolve YouTube live chat id.")?
        .error_for_status()
        .context("YouTube broadcast lookup failed.")?
        .json()
        .await
        .context("Could not parse YouTube broadcast lookup.")?;
    Ok(response
        .items
        .into_iter()
        .next()
        .and_then(|item| item.snippet)
        .and_then(|snippet| snippet.live_chat_id))
}

/// The connector task: resolve the chat id, then poll → normalize → deliver, honoring the
/// server poll interval, with backoff + status transitions on errors. Spawned by session
/// integration (and `liveChat.start` with a `youtube` config for the live smoke).
pub async fn run_youtube_chat_connector(state: AppState, session_id: String, config: YouTubeChatConfig) {
    let client = reqwest::Client::new();
    let base_url = config
        .api_base_url
        .clone()
        .unwrap_or_else(|| YOUTUBE_API_BASE_URL.to_string());
    let target_id = config.target_id.clone();

    let resolved = match config.live_chat_id.clone() {
        Some(id) => Some(id),
        None => match &config.broadcast_id {
            Some(broadcast_id) => {
                resolve_live_chat_id(&client, &base_url, &config.access_token, broadcast_id)
                    .await
                    .ok()
                    .flatten()
            }
            None => None,
        },
    };
    let Some(live_chat_id) = resolved else {
        set_provider_and_emit(
            &state,
            StreamPlatform::Youtube,
            LiveChatProviderConnectionState::Failed,
            "No live chat is available for this YouTube broadcast.",
        )
        .await;
        return;
    };

    set_provider_and_emit(
        &state,
        StreamPlatform::Youtube,
        LiveChatProviderConnectionState::Connecting,
        "Connecting to YouTube live chat…",
    )
    .await;

    let mut transport = YouTubeChatTransport::StreamList;
    let mut page_token: Option<String> = None;
    let mut backoff_ms = MIN_POLLING_INTERVAL_MS;
    let mut connected = false;

    loop {
        match fetch_chat_page(
            &client,
            &base_url,
            &config.access_token,
            transport,
            &live_chat_id,
            page_token.as_deref(),
        )
        .await
        {
            Ok(response) => {
                let now = chrono::Utc::now().to_rfc3339();
                let page = normalize_page(response, &session_id, target_id.as_deref(), &now);
                if !connected {
                    connected = true;
                    set_provider_and_emit(
                        &state,
                        StreamPlatform::Youtube,
                        LiveChatProviderConnectionState::Connected,
                        "YouTube live chat connected.",
                    )
                    .await;
                }
                for message in page.messages {
                    deliver_message(&state, message).await;
                }
                page_token = page.next_page_token;
                backoff_ms = MIN_POLLING_INTERVAL_MS;
                if page.ended {
                    set_provider_and_emit(
                        &state,
                        StreamPlatform::Youtube,
                        LiveChatProviderConnectionState::Ended,
                        "YouTube live chat has ended.",
                    )
                    .await;
                    return;
                }
                sleep(Duration::from_millis(page.polling_interval_ms)).await;
            }
            Err(error) => {
                let kind = match error {
                    FetchError::Api(kind) => kind,
                    FetchError::Network => YouTubeChatErrorKind::Transient,
                };
                let (provider_state, message, stop) = provider_reaction(kind);
                set_provider_and_emit(&state, StreamPlatform::Youtube, provider_state, message)
                    .await;
                if stop {
                    return;
                }
                if kind == YouTubeChatErrorKind::InvalidPageToken {
                    page_token = None;
                }
                // streamList may be unavailable for this broadcast — drop to list polling.
                if transport == YouTubeChatTransport::StreamList {
                    transport = YouTubeChatTransport::List;
                }
                connected = false;
                sleep(Duration::from_millis(backoff_ms)).await;
                backoff_ms = (backoff_ms.saturating_mul(2)).min(MAX_BACKOFF_MS);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use axum::extract::{OriginalUri, State};
    use axum::http::StatusCode;
    use axum::response::IntoResponse;
    use axum::routing::get;
    use axum::{Json, Router};
    use serde_json::json;
    use tokio::net::TcpListener;

    use super::*;

    fn text_response() -> LiveChatMessagesResponse {
        serde_json::from_value(json!({
            "pollingIntervalMillis": 3000,
            "nextPageToken": "tok-2",
            "items": [{
                "id": "msg-1",
                "snippet": {
                    "type": "textMessageEvent",
                    "publishedAt": "2026-06-06T10:00:00Z",
                    "displayMessage": "hello world"
                },
                "authorDetails": {
                    "channelId": "UCviewer",
                    "displayName": "Viewer One",
                    "profileImageUrl": "https://example.test/a.jpg",
                    "isChatModerator": true
                }
            }]
        }))
        .unwrap()
    }

    #[test]
    fn normalizes_text_message_with_author_roles() {
        let page = normalize_page(text_response(), "s1", Some("t1"), "2026-06-06T10:00:01Z");
        assert_eq!(page.messages.len(), 1);
        let message = &page.messages[0];
        assert_eq!(message.id, "youtube:msg-1");
        assert_eq!(message.provider_message_id, "msg-1");
        assert_eq!(message.platform, StreamPlatform::Youtube);
        assert_eq!(message.target_id.as_deref(), Some("t1"));
        assert_eq!(message.session_id, "s1");
        assert_eq!(message.author_name, "Viewer One");
        assert_eq!(message.author_id.as_deref(), Some("UCviewer"));
        assert_eq!(message.message_text, "hello world");
        assert_eq!(message.event_type, LiveChatEventType::Message);
        assert_eq!(message.author_roles, vec!["moderator".to_string()]);
        assert!(!message.is_deleted);
        // The server poll interval is preserved, and the resume token is threaded out.
        assert_eq!(page.polling_interval_ms, 3000);
        assert_eq!(page.next_page_token.as_deref(), Some("tok-2"));
        assert!(!page.ended);
    }

    #[test]
    fn normalizes_super_chat_with_amount_and_paid_type() {
        let response: LiveChatMessagesResponse = serde_json::from_value(json!({
            "items": [{
                "id": "sc-1",
                "snippet": {
                    "type": "superChatEvent",
                    "superChatDetails": { "amountDisplayString": "$5.00" }
                },
                "authorDetails": { "displayName": "Generous Fan" }
            }]
        }))
        .unwrap();
        let page = normalize_page(response, "s1", None, "now");
        let message = &page.messages[0];
        assert_eq!(message.event_type, LiveChatEventType::Paid);
        assert_eq!(message.amount_text.as_deref(), Some("$5.00"));
        // No displayMessage → a safe styled row, not an empty/dropped one.
        assert_eq!(message.message_text, "Super Chat: $5.00");
    }

    #[test]
    fn membership_and_deletion_map_to_event_types() {
        let response: LiveChatMessagesResponse = serde_json::from_value(json!({
            "items": [
                { "id": "m1", "snippet": { "type": "newSponsorEvent" } },
                { "id": "d1", "snippet": { "type": "messageDeletedEvent" } }
            ]
        }))
        .unwrap();
        let page = normalize_page(response, "s1", None, "now");
        assert_eq!(page.messages[0].event_type, LiveChatEventType::Membership);
        assert_eq!(page.messages[1].event_type, LiveChatEventType::Deleted);
        assert!(page.messages[1].is_deleted);
    }

    #[test]
    fn unknown_event_becomes_safe_system_row_not_dropped() {
        let response: LiveChatMessagesResponse = serde_json::from_value(json!({
            "items": [{ "id": "x1", "snippet": { "type": "someBrandNewEvent" } }]
        }))
        .unwrap();
        let page = normalize_page(response, "s1", None, "now");
        assert_eq!(page.messages.len(), 1);
        assert_eq!(page.messages[0].event_type, LiveChatEventType::System);
        assert!(!page.messages[0].message_text.is_empty());
    }

    #[test]
    fn offline_at_and_chat_ended_mark_page_ended() {
        let offline: LiveChatMessagesResponse =
            serde_json::from_value(json!({ "offlineAt": "2026-06-06T11:00:00Z", "items": [] }))
                .unwrap();
        assert!(normalize_page(offline, "s1", None, "now").ended);

        let ended: LiveChatMessagesResponse = serde_json::from_value(json!({
            "items": [{ "id": "e1", "snippet": { "type": "chatEndedEvent" } }]
        }))
        .unwrap();
        assert!(normalize_page(ended, "s1", None, "now").ended);
    }

    #[test]
    fn polling_interval_is_clamped_to_minimum() {
        let response: LiveChatMessagesResponse =
            serde_json::from_value(json!({ "pollingIntervalMillis": 10, "items": [] })).unwrap();
        assert_eq!(
            normalize_page(response, "s1", None, "now").polling_interval_ms,
            MIN_POLLING_INTERVAL_MS
        );
    }

    #[test]
    fn classifies_disabled_ended_quota_token_and_auth_errors() {
        assert_eq!(
            classify_status(403, Some("liveChatDisabled")),
            YouTubeChatErrorKind::Disabled
        );
        assert_eq!(
            classify_status(403, Some("liveChatEnded")),
            YouTubeChatErrorKind::Ended
        );
        assert_eq!(
            classify_status(403, Some("rateLimitExceeded")),
            YouTubeChatErrorKind::RateLimited
        );
        assert_eq!(classify_status(429, None), YouTubeChatErrorKind::RateLimited);
        assert_eq!(
            classify_status(400, Some("pageTokenInvalid")),
            YouTubeChatErrorKind::InvalidPageToken
        );
        assert_eq!(classify_status(401, None), YouTubeChatErrorKind::AuthExpired);
        assert_eq!(classify_status(503, None), YouTubeChatErrorKind::Transient);
    }

    #[test]
    fn disabled_and_ended_stop_but_rate_limit_retries() {
        assert!(provider_reaction(YouTubeChatErrorKind::Disabled).2);
        assert!(provider_reaction(YouTubeChatErrorKind::Ended).2);
        assert!(provider_reaction(YouTubeChatErrorKind::AuthExpired).2);
        assert!(!provider_reaction(YouTubeChatErrorKind::RateLimited).2);
        assert!(!provider_reaction(YouTubeChatErrorKind::Transient).2);
        assert!(!provider_reaction(YouTubeChatErrorKind::InvalidPageToken).2);
    }

    #[test]
    fn url_uses_list_vs_stream_path_and_threads_page_token() {
        let list = chat_messages_url("https://api.test", YouTubeChatTransport::List, "LC1", Some("tokA"))
            .unwrap();
        assert_eq!(list.path(), LIVE_CHAT_MESSAGES_PATH);
        let query = list.query().unwrap();
        assert!(query.contains("liveChatId=LC1"));
        assert!(query.contains("part=snippet%2CauthorDetails"));
        assert!(query.contains("pageToken=tokA"));

        let stream =
            chat_messages_url("https://api.test", YouTubeChatTransport::StreamList, "LC1", None)
                .unwrap();
        assert_eq!(stream.path(), LIVE_CHAT_MESSAGES_STREAM_PATH);
        assert!(!stream.query().unwrap().contains("pageToken"));
    }

    #[tokio::test]
    async fn resolve_live_chat_id_reads_snippet_from_broadcast() {
        async fn broadcasts() -> impl IntoResponse {
            Json(json!({ "items": [{ "snippet": { "liveChatId": "LCID-123" } }] }))
        }
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(
                listener,
                Router::new().route("/youtube/v3/liveBroadcasts", get(broadcasts)),
            )
            .await
            .unwrap();
        });

        let resolved = resolve_live_chat_id(
            &reqwest::Client::new(),
            &format!("http://{address}"),
            "token",
            "bcast-1",
        )
        .await
        .unwrap();
        assert_eq!(resolved.as_deref(), Some("LCID-123"));
    }

    #[tokio::test]
    async fn fetch_reconnect_resumes_from_stored_page_token() {
        // The mock records the pageToken it received so the test can assert the resume.
        type Seen = Arc<Mutex<Vec<String>>>;
        async fn messages(
            State(seen): State<Seen>,
            OriginalUri(uri): OriginalUri,
        ) -> impl IntoResponse {
            let query = uri.query().unwrap_or_default().to_string();
            seen.lock().unwrap().push(query.clone());
            let token = if query.contains("pageToken=resume-1") {
                "resume-2"
            } else {
                "resume-1"
            };
            Json(json!({ "nextPageToken": token, "pollingIntervalMillis": 1500, "items": [] }))
        }
        let seen: Seen = Arc::new(Mutex::new(Vec::new()));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn({
            let seen = seen.clone();
            async move {
                axum::serve(
                    listener,
                    Router::new()
                        .route("/youtube/v3/liveChat/messages/stream", get(messages))
                        .with_state(seen),
                )
                .await
                .unwrap();
            }
        });

        let client = reqwest::Client::new();
        let base_url = format!("http://{address}");
        let first = fetch_chat_page(
            &client,
            &base_url,
            "token",
            YouTubeChatTransport::StreamList,
            "LC1",
            None,
        )
        .await
        .unwrap();
        assert_eq!(first.next_page_token.as_deref(), Some("resume-1"));

        let second = fetch_chat_page(
            &client,
            &base_url,
            "token",
            YouTubeChatTransport::StreamList,
            "LC1",
            first.next_page_token.as_deref(),
        )
        .await
        .unwrap();
        assert_eq!(second.next_page_token.as_deref(), Some("resume-2"));

        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 2);
        assert!(!seen[0].contains("pageToken"));
        assert!(seen[1].contains("pageToken=resume-1"));
    }

    #[tokio::test]
    async fn disabled_chat_is_classified_not_a_hard_error() {
        async fn forbidden() -> impl IntoResponse {
            (
                StatusCode::FORBIDDEN,
                Json(json!({ "error": { "errors": [{ "reason": "liveChatDisabled" }] } })),
            )
        }
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(
                listener,
                Router::new().route("/youtube/v3/liveChat/messages/stream", get(forbidden)),
            )
            .await
            .unwrap();
        });

        let error = fetch_chat_page(
            &reqwest::Client::new(),
            &format!("http://{address}"),
            "token",
            YouTubeChatTransport::StreamList,
            "LC1",
            None,
        )
        .await
        .unwrap_err();
        assert_eq!(error, FetchError::Api(YouTubeChatErrorKind::Disabled));
    }
}
