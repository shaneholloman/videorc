use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use base64::Engine as _;
use hmac::{Hmac, Mac};
use reqwest::{Method, Url};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha1::Sha1;
use tokio::time::sleep;
use uuid::Uuid;

use crate::streaming::{
    PlatformAccount, StreamMetadataDraft, StreamPlatform, stream_platform_label,
};

type HmacSha1 = Hmac<Sha1>;

const X_LIVESTREAM_DOCS_URL: &str = "https://github.com/xdevplatform/x-livestream-sample";
const X_API_OVERVIEW_URL: &str = "https://docs.x.com/x-api/overview";
const DEFAULT_API_BASE_URL: &str = "https://api.x.com";
const DEFAULT_SOURCE_NAME: &str = "Videorc Primary Encoder";
const DEFAULT_LOCALE: &str = "en";
const DEFAULT_CHAT_OPTION: u8 = 2;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct XNativeLiveCapabilityParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct XPrepareParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct XPublishParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,
    pub source_id: String,
    pub region: String,
    #[serde(default = "default_true")]
    pub is_low_latency: bool,
    #[serde(default)]
    pub should_not_tweet: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locale: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chat_option: Option<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct XEndParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,
    pub broadcast_id: String,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum XNativeLiveCapabilityState {
    MissingCredentials,
    Ready,
    AccountMismatch,
    ApiError,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct XNativeLiveCapability {
    pub platform: StreamPlatform,
    pub state: XNativeLiveCapabilityState,
    pub native_available: bool,
    pub manual_rtmp_available: bool,
    pub oauth_connected: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_source: Option<String>,
    pub message: String,
    pub evidence: Vec<String>,
    pub docs_url: String,
    pub api_overview_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PreparedXStreamSource {
    pub platform: StreamPlatform,
    pub account_id: String,
    pub account_label: String,
    pub source_id: String,
    pub region: String,
    pub server_url: String,
    pub stream_key_secret_ref: String,
    pub stream_key_present: bool,
    pub redacted_url: String,
    pub is_stream_active: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recommended_configuration: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compatibility_info: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct XPublishResult {
    pub platform: StreamPlatform,
    pub account_id: String,
    pub source_id: String,
    pub broadcast_id: String,
    pub media_key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_id: Option<String>,
    pub share_url: String,
    pub state: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tweet_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tweet_error: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct XEndResult {
    pub platform: StreamPlatform,
    pub account_id: String,
    pub broadcast_id: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XLivestreamCredentials {
    pub consumer_key: String,
    pub consumer_secret: String,
    pub access_token: String,
    pub access_token_secret: String,
    pub user_id: String,
    pub account_label: Option<String>,
    pub credential_source: String,
}

#[derive(Debug, Clone)]
pub struct XPrepareSourceRequest {
    pub credentials: XLivestreamCredentials,
    pub account: Option<PlatformAccount>,
    pub source_name: String,
    pub api_base_url: Option<String>,
}

#[derive(Debug, Clone)]
pub struct XPublishRequest {
    pub credentials: XLivestreamCredentials,
    pub source_id: String,
    pub region: String,
    pub metadata: StreamMetadataDraft,
    pub is_low_latency: bool,
    pub should_not_tweet: bool,
    pub locale: String,
    pub chat_option: u8,
    pub api_base_url: Option<String>,
    pub poll_attempts: usize,
    pub poll_interval_ms: u64,
}

#[derive(Debug, Clone)]
pub struct XEndRequest {
    pub credentials: XLivestreamCredentials,
    pub broadcast_id: String,
    pub api_base_url: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct XRegionResponse {
    region: String,
}

#[derive(Debug, Clone, Deserialize)]
struct XSourceEnvelope {
    source: XStreamSource,
}

#[derive(Debug, Clone, Deserialize)]
struct XSourcesEnvelope {
    #[serde(default)]
    sources: Vec<XStreamSource>,
}

#[derive(Debug, Clone, Deserialize)]
struct XBroadcastEnvelope {
    broadcast: XBroadcast,
    #[serde(default)]
    share_url: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct XStreamSource {
    id: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    rtmp_region: Option<String>,
    #[serde(default)]
    rtmps_url: Option<String>,
    #[serde(default)]
    rtmp_url: Option<String>,
    #[serde(default)]
    rtmp_stream_key: Option<String>,
    #[serde(default)]
    is_stream_active: bool,
    #[serde(default)]
    recommended_configuration: Option<serde_json::Value>,
    #[serde(default)]
    compatibility_info: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize)]
struct XBroadcast {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    broadcast_id: Option<String>,
    #[serde(default)]
    media_key: Option<String>,
    #[serde(default)]
    media_id: Option<String>,
    #[serde(default)]
    share_url: Option<String>,
    #[serde(default)]
    state: Option<String>,
    #[serde(default)]
    tweet_id: Option<String>,
    #[serde(default)]
    tweet_error: Option<String>,
}

pub fn x_livestream_credentials_from_env() -> Result<Option<XLivestreamCredentials>> {
    let consumer_key = optional_env_any(&["VIDEORC_X_OAUTH1_CONSUMER_KEY", "X_CONSUMER_KEY"]);
    let consumer_secret =
        optional_env_any(&["VIDEORC_X_OAUTH1_CONSUMER_SECRET", "X_CONSUMER_SECRET"]);
    let access_token = optional_env_any(&["VIDEORC_X_OAUTH1_ACCESS_TOKEN", "X_ACCESS_TOKEN"]);
    let access_token_secret = optional_env_any(&[
        "VIDEORC_X_OAUTH1_ACCESS_TOKEN_SECRET",
        "X_ACCESS_TOKEN_SECRET",
    ]);

    let present = [
        consumer_key.is_some(),
        consumer_secret.is_some(),
        access_token.is_some(),
        access_token_secret.is_some(),
    ];
    if present.iter().all(|value| !*value) {
        return Ok(None);
    }
    if !present.iter().all(|value| *value) {
        anyhow::bail!(
            "X Livestream OAuth 1.0a credentials are incomplete. Set VIDEORC_X_OAUTH1_CONSUMER_KEY, VIDEORC_X_OAUTH1_CONSUMER_SECRET, VIDEORC_X_OAUTH1_ACCESS_TOKEN, and VIDEORC_X_OAUTH1_ACCESS_TOKEN_SECRET."
        );
    }

    let access_token = access_token.expect("presence checked above");
    let user_id = optional_env_any(&["VIDEORC_X_OAUTH1_USER_ID", "X_USER_ID"])
        .or_else(|| access_token.split_once('-').map(|(prefix, _)| prefix.to_string()))
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .context(
            "X Livestream OAuth 1.0a user id is missing. Set VIDEORC_X_OAUTH1_USER_ID or use an access token whose prefix is the numeric X user id.",
        )?;
    if !user_id.chars().all(|character| character.is_ascii_digit()) {
        anyhow::bail!("X Livestream user id must be numeric.");
    }

    Ok(Some(XLivestreamCredentials {
        consumer_key: consumer_key.expect("presence checked above"),
        consumer_secret: consumer_secret.expect("presence checked above"),
        access_token,
        access_token_secret: access_token_secret.expect("presence checked above"),
        user_id,
        account_label: optional_env_any(&["VIDEORC_X_ACCOUNT_LABEL", "X_ACCOUNT_LABEL"]),
        credential_source: if std::env::var("VIDEORC_X_OAUTH1_CONSUMER_KEY").is_ok() {
            "videorc-env".to_string()
        } else {
            "x-env".to_string()
        },
    }))
}

pub fn x_native_live_capability(
    account: Option<&PlatformAccount>,
) -> Result<XNativeLiveCapability> {
    let credentials = x_livestream_credentials_from_env()?;
    let Some(credentials) = credentials else {
        return Ok(XNativeLiveCapability {
            platform: StreamPlatform::X,
            state: XNativeLiveCapabilityState::MissingCredentials,
            native_available: false,
            manual_rtmp_available: true,
            oauth_connected: account.is_some(),
            account_id: account.map(|account| account.account_id.clone()),
            account_label: account.map(|account| account.account_label.clone()),
            credential_source: None,
            message: "X Livestream API access is approved, but OAuth 1.0a live credentials are not configured on this build.".to_string(),
            evidence: vec![
                "X Livestream management endpoints require OAuth 1.0a user-context credentials, not the existing OAuth2 PKCE token.".to_string(),
                "Set the VIDEORC_X_OAUTH1_* environment variables in the release/smoke environment to enable native source and broadcast creation.".to_string(),
                "Manual RTMP remains available only when the user explicitly chooses Manual RTMP.".to_string(),
            ],
            docs_url: X_LIVESTREAM_DOCS_URL.to_string(),
            api_overview_url: X_API_OVERVIEW_URL.to_string(),
        });
    };

    if let Some(account) = account
        && account.account_id != credentials.user_id
        && account.id != credentials.user_id
    {
        return Ok(XNativeLiveCapability {
            platform: StreamPlatform::X,
            state: XNativeLiveCapabilityState::AccountMismatch,
            native_available: false,
            manual_rtmp_available: true,
            oauth_connected: true,
            account_id: Some(account.account_id.clone()),
            account_label: Some(account.account_label.clone()),
            credential_source: Some(credentials.credential_source),
            message: "Connected X account does not match the configured X Livestream OAuth 1.0a user id.".to_string(),
            evidence: vec![
                "X rejects Livestream requests when the OAuth 1.0a token user id differs from the :user_id path.".to_string(),
                "Reconnect the matching X account or update the configured OAuth 1.0a credentials.".to_string(),
            ],
            docs_url: X_LIVESTREAM_DOCS_URL.to_string(),
            api_overview_url: X_API_OVERVIEW_URL.to_string(),
        });
    }

    Ok(XNativeLiveCapability {
        platform: StreamPlatform::X,
        state: XNativeLiveCapabilityState::Ready,
        native_available: true,
        manual_rtmp_available: true,
        oauth_connected: account.is_some(),
        account_id: Some(credentials.user_id),
        account_label: account
            .map(|account| account.account_label.clone())
            .or(credentials.account_label),
        credential_source: Some(credentials.credential_source),
        message: "X Livestream API credentials are configured. Videorc can prepare a native X source and publish broadcasts through the allow-listed API.".to_string(),
        evidence: vec![
            "OAuth 1.0a consumer key, consumer secret, access token, and token secret are present.".to_string(),
            "The numeric X user id is available for /2/users/:user_id source and broadcast paths.".to_string(),
            "RTMPS source keys will be stored in the backend secret store and redacted from UI/logs.".to_string(),
        ],
        docs_url: X_LIVESTREAM_DOCS_URL.to_string(),
        api_overview_url: X_API_OVERVIEW_URL.to_string(),
    })
}

pub fn ensure_x_native_live_available(capability: &XNativeLiveCapability) -> Result<()> {
    if capability.native_available {
        return Ok(());
    }

    anyhow::bail!("{}", capability.message)
}

pub fn select_x_account<'a>(
    accounts: &'a [PlatformAccount],
    account_id: Option<&str>,
) -> Result<Option<&'a PlatformAccount>> {
    let account = accounts.iter().find(|account| {
        account.platform == StreamPlatform::X
            && account_id.is_none_or(|account_id| {
                account.account_id == account_id || account.id == account_id
            })
    });
    if account_id.is_some() {
        account.context("No connected X OAuth account matched the requested account id.")?;
    }

    Ok(account)
}

pub async fn prepare_x_stream_source<F>(
    request: XPrepareSourceRequest,
    client: &reqwest::Client,
    mut put_secret: F,
) -> Result<PreparedXStreamSource>
where
    F: FnMut(&str, &str) -> Result<()>,
{
    let base_url = api_base_url(request.api_base_url.as_deref());
    let credentials = request.credentials;
    let region = match optional_env_any(&["VIDEORC_X_LIVESTREAM_REGION", "X_LIVESTREAM_REGION"]) {
        Some(region) => region,
        None => get_region(client, &credentials, &base_url).await?,
    };
    let source = if let Some(source_id) =
        optional_env_any(&["VIDEORC_X_LIVESTREAM_SOURCE_ID", "X_LIVESTREAM_SOURCE_ID"])
    {
        get_source(client, &credentials, &base_url, &source_id).await?
    } else {
        let sources = list_sources(client, &credentials, &base_url)
            .await
            .unwrap_or_default();
        if let Some(source) = sources.into_iter().find(|source| {
            source.name.as_deref() == Some(request.source_name.as_str())
                && source.rtmp_region.as_deref() == Some(region.as_str())
        }) {
            source
        } else {
            create_source(
                client,
                &credentials,
                &base_url,
                &request.source_name,
                &region,
            )
            .await?
        }
    };

    let source_id = source.id.trim().to_string();
    if source_id.is_empty() {
        anyhow::bail!("X stream source response did not include a source id.");
    }
    let server_url = source
        .rtmps_url
        .as_deref()
        .or(source.rtmp_url.as_deref())
        .map(str::trim)
        .filter(|url| !url.is_empty())
        .context("X stream source response did not include an RTMPS ingest URL.")?
        .to_string();
    let stream_key = source
        .rtmp_stream_key
        .as_deref()
        .map(str::trim)
        .filter(|key| !key.is_empty())
        .context("X stream source response did not include an RTMP stream key.")?
        .to_string();
    let region = source
        .rtmp_region
        .clone()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(region);
    let secret_ref = format!(
        "platform:x:{}:{}:stream-key",
        credentials.user_id,
        sanitize_secret_ref_segment(&source_id)
    );
    put_secret(&secret_ref, &stream_key).context("Could not store X RTMPS stream key.")?;

    Ok(PreparedXStreamSource {
        platform: StreamPlatform::X,
        account_id: credentials.user_id.clone(),
        account_label: request
            .account
            .map(|account| account.account_label)
            .or(credentials.account_label)
            .unwrap_or_else(|| stream_platform_label(StreamPlatform::X).to_string()),
        source_id,
        region,
        server_url,
        stream_key_secret_ref: secret_ref,
        stream_key_present: true,
        redacted_url: "rtmps://<x-ingest>/<stream-key>".to_string(),
        is_stream_active: source.is_stream_active,
        recommended_configuration: source.recommended_configuration,
        compatibility_info: source.compatibility_info,
    })
}

pub async fn publish_x_broadcast(
    request: XPublishRequest,
    client: &reqwest::Client,
) -> Result<XPublishResult> {
    let base_url = api_base_url(request.api_base_url.as_deref());
    let attempts = request.poll_attempts.max(1);
    let mut last_source = None;
    for attempt in 0..attempts {
        let source =
            get_source(client, &request.credentials, &base_url, &request.source_id).await?;
        let active = source.is_stream_active;
        last_source = Some(source);
        if active {
            break;
        }
        if attempt + 1 < attempts {
            sleep(Duration::from_millis(request.poll_interval_ms)).await;
        }
    }
    if !last_source
        .as_ref()
        .is_some_and(|source| source.is_stream_active)
    {
        anyhow::bail!("X RTMPS source did not become active before publish.");
    }

    let created = create_broadcast(
        client,
        &request.credentials,
        &base_url,
        &request.source_id,
        &request.region,
        request.is_low_latency,
    )
    .await?;
    let broadcast_id = created
        .broadcast
        .broadcast_id
        .clone()
        .or(created.broadcast.id.clone())
        .context("X broadcast create response did not include a broadcast id.")?;
    let media_key = created
        .broadcast
        .media_key
        .clone()
        .context("X broadcast create response did not include a media_key.")?;
    let share_url = created
        .share_url
        .clone()
        .or(created.broadcast.share_url.clone())
        .unwrap_or_else(|| format!("https://x.com/i/broadcasts/{broadcast_id}"));
    let title = x_title(&request.metadata);
    let published = publish_broadcast(
        client,
        &request.credentials,
        &base_url,
        PublishBroadcastStateRequest {
            broadcast_id: &broadcast_id,
            title: &title,
            should_not_tweet: request.should_not_tweet,
            locale: &request.locale,
            chat_option: request.chat_option,
        },
    )
    .await?;

    Ok(XPublishResult {
        platform: StreamPlatform::X,
        account_id: request.credentials.user_id,
        source_id: request.source_id,
        broadcast_id,
        media_key,
        media_id: created.broadcast.media_id,
        share_url,
        state: published
            .broadcast
            .state
            .unwrap_or_else(|| "RUNNING".to_string()),
        tweet_id: published.broadcast.tweet_id,
        tweet_error: published.broadcast.tweet_error,
        message: "X broadcast is live.".to_string(),
    })
}

pub async fn end_x_broadcast(request: XEndRequest, client: &reqwest::Client) -> Result<XEndResult> {
    let base_url = api_base_url(request.api_base_url.as_deref());
    end_broadcast(
        client,
        &request.credentials,
        &base_url,
        &request.broadcast_id,
    )
    .await?;
    Ok(XEndResult {
        platform: StreamPlatform::X,
        account_id: request.credentials.user_id,
        broadcast_id: request.broadcast_id,
        message: "X broadcast ended.".to_string(),
    })
}

pub fn default_source_name() -> String {
    optional_env_any(&[
        "VIDEORC_X_LIVESTREAM_SOURCE_NAME",
        "X_LIVESTREAM_SOURCE_NAME",
    ])
    .unwrap_or_else(|| DEFAULT_SOURCE_NAME.to_string())
}

pub fn default_publish_locale(value: Option<String>) -> String {
    value
        .and_then(|value| {
            let trimmed = value.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_string())
        })
        .or_else(|| optional_env_any(&["VIDEORC_X_LIVESTREAM_LOCALE", "X_LIVESTREAM_LOCALE"]))
        .unwrap_or_else(|| DEFAULT_LOCALE.to_string())
}

pub fn default_chat_option(value: Option<u8>) -> u8 {
    value.unwrap_or_else(|| {
        optional_env_any(&[
            "VIDEORC_X_LIVESTREAM_CHAT_OPTION",
            "X_LIVESTREAM_CHAT_OPTION",
        ])
        .and_then(|value| value.parse::<u8>().ok())
        .unwrap_or(DEFAULT_CHAT_OPTION)
    })
}

async fn get_region(
    client: &reqwest::Client,
    credentials: &XLivestreamCredentials,
    base_url: &str,
) -> Result<String> {
    let url = endpoint(base_url, "/2/region")?;
    let response: XRegionResponse =
        send_x_request(client, credentials, Method::GET, url, None).await?;
    Ok(response.region)
}

async fn list_sources(
    client: &reqwest::Client,
    credentials: &XLivestreamCredentials,
    base_url: &str,
) -> Result<Vec<XStreamSource>> {
    let url = endpoint(
        base_url,
        &format!("/2/users/{}/sources", credentials.user_id),
    )?;
    let response: XSourcesEnvelope =
        send_x_request(client, credentials, Method::GET, url, None).await?;
    Ok(response.sources)
}

async fn create_source(
    client: &reqwest::Client,
    credentials: &XLivestreamCredentials,
    base_url: &str,
    name: &str,
    region: &str,
) -> Result<XStreamSource> {
    let url = endpoint(
        base_url,
        &format!("/2/users/{}/sources", credentials.user_id),
    )?;
    let body = json!({ "name": name, "region": region });
    let response: XSourceEnvelope =
        send_x_request(client, credentials, Method::POST, url, Some(body)).await?;
    Ok(response.source)
}

async fn get_source(
    client: &reqwest::Client,
    credentials: &XLivestreamCredentials,
    base_url: &str,
    source_id: &str,
) -> Result<XStreamSource> {
    let url = endpoint(
        base_url,
        &format!("/2/users/{}/sources/{}", credentials.user_id, source_id),
    )?;
    let response: XSourceEnvelope =
        send_x_request(client, credentials, Method::GET, url, None).await?;
    Ok(response.source)
}

async fn create_broadcast(
    client: &reqwest::Client,
    credentials: &XLivestreamCredentials,
    base_url: &str,
    source_id: &str,
    region: &str,
    is_low_latency: bool,
) -> Result<XBroadcastEnvelope> {
    let url = endpoint(
        base_url,
        &format!("/2/users/{}/broadcasts", credentials.user_id),
    )?;
    let body = json!({
        "source_id": source_id,
        "region": region,
        "is_low_latency": is_low_latency,
    });
    send_x_request(client, credentials, Method::POST, url, Some(body)).await
}

struct PublishBroadcastStateRequest<'a> {
    broadcast_id: &'a str,
    title: &'a str,
    should_not_tweet: bool,
    locale: &'a str,
    chat_option: u8,
}

async fn publish_broadcast(
    client: &reqwest::Client,
    credentials: &XLivestreamCredentials,
    base_url: &str,
    request: PublishBroadcastStateRequest<'_>,
) -> Result<XBroadcastEnvelope> {
    let url = endpoint(
        base_url,
        &format!(
            "/2/users/{}/broadcasts/{}/state",
            credentials.user_id, request.broadcast_id
        ),
    )?;
    let body = json!({
        "state": "PUBLISH",
        "title": request.title,
        "should_not_tweet": request.should_not_tweet,
        "locale": request.locale,
        "chat_option": request.chat_option,
    });
    send_x_request(client, credentials, Method::PUT, url, Some(body)).await
}

async fn end_broadcast(
    client: &reqwest::Client,
    credentials: &XLivestreamCredentials,
    base_url: &str,
    broadcast_id: &str,
) -> Result<()> {
    let url = endpoint(
        base_url,
        &format!(
            "/2/users/{}/broadcasts/{}/state",
            credentials.user_id, broadcast_id
        ),
    )?;
    let body = json!({ "state": "END" });
    let _: serde_json::Value =
        send_x_request(client, credentials, Method::PUT, url, Some(body)).await?;
    Ok(())
}

async fn send_x_request<T: serde::de::DeserializeOwned>(
    client: &reqwest::Client,
    credentials: &XLivestreamCredentials,
    method: Method,
    url: Url,
    body: Option<serde_json::Value>,
) -> Result<T> {
    let authorization = oauth1_authorization_header(
        method.as_str(),
        url.as_str(),
        credentials,
        &oauth_nonce(),
        oauth_timestamp(),
    )?;
    let mut request = client
        .request(method, url)
        .header("Authorization", authorization);
    if let Some(body) = body {
        request = request.json(&body);
    }
    let response = request
        .send()
        .await
        .context("Could not call X Livestream API")?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        let detail = x_error_detail(&body).unwrap_or_else(|| "no error detail".to_string());
        anyhow::bail!("X Livestream API request failed with HTTP {status}: {detail}");
    }
    response
        .json::<T>()
        .await
        .context("Could not parse X Livestream API response")
}

pub fn oauth1_authorization_header(
    method: &str,
    url: &str,
    credentials: &XLivestreamCredentials,
    nonce: &str,
    timestamp: u64,
) -> Result<String> {
    let parsed = Url::parse(url).context("OAuth 1.0a request URL is invalid.")?;
    let base_url = oauth_base_url(&parsed);
    let mut params = vec![
        (
            "oauth_consumer_key".to_string(),
            credentials.consumer_key.clone(),
        ),
        ("oauth_nonce".to_string(), nonce.to_string()),
        (
            "oauth_signature_method".to_string(),
            "HMAC-SHA1".to_string(),
        ),
        ("oauth_timestamp".to_string(), timestamp.to_string()),
        ("oauth_token".to_string(), credentials.access_token.clone()),
        ("oauth_version".to_string(), "1.0".to_string()),
    ];
    for (key, value) in parsed.query_pairs() {
        params.push((key.into_owned(), value.into_owned()));
    }
    let param_string = normalized_oauth_params(&params);
    let base_string = format!(
        "{}&{}&{}",
        method.to_ascii_uppercase(),
        oauth_percent_encode(&base_url),
        oauth_percent_encode(&param_string)
    );
    let signing_key = format!(
        "{}&{}",
        oauth_percent_encode(&credentials.consumer_secret),
        oauth_percent_encode(&credentials.access_token_secret)
    );
    let mut mac = HmacSha1::new_from_slice(signing_key.as_bytes())
        .context("Could not initialize OAuth 1.0a signer.")?;
    mac.update(base_string.as_bytes());
    let signature = base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes());

    let mut header_params = vec![
        (
            "oauth_consumer_key".to_string(),
            credentials.consumer_key.clone(),
        ),
        ("oauth_nonce".to_string(), nonce.to_string()),
        (
            "oauth_signature_method".to_string(),
            "HMAC-SHA1".to_string(),
        ),
        ("oauth_timestamp".to_string(), timestamp.to_string()),
        ("oauth_token".to_string(), credentials.access_token.clone()),
        ("oauth_version".to_string(), "1.0".to_string()),
        ("oauth_signature".to_string(), signature),
    ];
    header_params.sort_by(|(left, _), (right, _)| left.cmp(right));
    let header = header_params
        .into_iter()
        .map(|(key, value)| format!(r#"{key}="{}""#, oauth_percent_encode(&value)))
        .collect::<Vec<_>>()
        .join(", ");
    Ok(format!("OAuth {header}"))
}

fn oauth_base_url(url: &Url) -> String {
    let mut base = url.clone();
    base.set_query(None);
    base.set_fragment(None);
    base.to_string()
}

fn normalized_oauth_params(params: &[(String, String)]) -> String {
    let mut encoded = params
        .iter()
        .map(|(key, value)| (oauth_percent_encode(key), oauth_percent_encode(value)))
        .collect::<Vec<_>>();
    encoded.sort();
    encoded
        .into_iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<Vec<_>>()
        .join("&")
}

pub fn oauth_percent_encode(value: &str) -> String {
    value
        .bytes()
        .flat_map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                vec![byte as char]
            }
            _ => {
                let hex = format!("%{byte:02X}");
                hex.chars().collect()
            }
        })
        .collect()
}

fn oauth_nonce() -> String {
    Uuid::new_v4().simple().to_string()
}

fn oauth_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn api_base_url(value: Option<&str>) -> String {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| optional_env_any(&["VIDEORC_X_LIVESTREAM_API_BASE_URL"]))
        .unwrap_or_else(|| DEFAULT_API_BASE_URL.to_string())
}

fn endpoint(base_url: &str, path: &str) -> Result<Url> {
    let base = Url::parse(base_url).context("X Livestream API base URL is invalid.")?;
    base.join(path)
        .with_context(|| format!("Could not build X Livestream API URL for {path}."))
}

fn optional_env_any(names: &[&str]) -> Option<String> {
    names.iter().find_map(|name| {
        std::env::var(name)
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    })
}

fn sanitize_secret_ref_segment(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                character
            } else {
                '_'
            }
        })
        .collect()
}

fn x_error_detail(body: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(body).ok()?;
    if let Some(message) = value
        .get("message")
        .and_then(|message| message.as_str())
        .map(str::trim)
        .filter(|message| !message.is_empty())
    {
        return Some(message.to_string());
    }
    if let Some(errors) = value.get("errors").and_then(|errors| errors.as_array()) {
        let joined = errors
            .iter()
            .filter_map(|error| {
                error
                    .get("message")
                    .and_then(|message| message.as_str())
                    .or_else(|| error.as_str())
            })
            .map(str::trim)
            .filter(|message| !message.is_empty())
            .map(ToOwned::to_owned)
            .collect::<Vec<_>>();
        if !joined.is_empty() {
            return Some(joined.join("; "));
        }
    }
    value
        .get("error")
        .and_then(|error| error.as_str())
        .map(str::trim)
        .filter(|message| !message.is_empty())
        .map(ToOwned::to_owned)
}

fn x_title(metadata: &StreamMetadataDraft) -> String {
    metadata
        .target_overrides
        .iter()
        .find(|target| target.platform == StreamPlatform::X && target.customize)
        .and_then(|target| (!target.title.trim().is_empty()).then(|| target.title.trim()))
        .or_else(|| (!metadata.title.trim().is_empty()).then(|| metadata.title.trim()))
        .unwrap_or("Live from Videorc")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::streaming::PlatformAccountStatus;

    fn credentials() -> XLivestreamCredentials {
        XLivestreamCredentials {
            consumer_key: "consumer".to_string(),
            consumer_secret: "consumer secret".to_string(),
            access_token: "12345-token".to_string(),
            access_token_secret: "token secret".to_string(),
            user_id: "12345".to_string(),
            account_label: Some("Videorc".to_string()),
            credential_source: "test".to_string(),
        }
    }

    #[test]
    fn oauth_percent_encoding_is_strict_rfc3986() {
        assert_eq!(
            oauth_percent_encode("!*'() space+slash/tilde~"),
            "%21%2A%27%28%29%20space%2Bslash%2Ftilde~"
        );
    }

    #[test]
    fn oauth_header_includes_query_params_but_never_json_body() {
        let header = oauth1_authorization_header(
            "POST",
            "https://api.x.com/2/users/12345/sources?pagination_token=a b",
            &credentials(),
            "nonce",
            1772031973,
        )
        .unwrap();

        assert!(header.starts_with("OAuth "));
        assert!(header.contains(r#"oauth_consumer_key="consumer""#));
        assert!(header.contains(r#"oauth_token="12345-token""#));
        assert!(header.contains("oauth_signature="));
        assert!(!header.contains("pagination_token"));
        assert!(!header.contains("source_id"));
        assert!(!header.contains("consumer secret"));
        assert!(!header.contains("token secret"));
    }

    #[test]
    fn normalized_oauth_params_sort_encoded_key_value_pairs() {
        let params = vec![
            ("b".to_string(), "two".to_string()),
            ("a".to_string(), "space value".to_string()),
            ("a".to_string(), "punct!".to_string()),
        ];

        assert_eq!(
            normalized_oauth_params(&params),
            "a=punct%21&a=space%20value&b=two"
        );
    }

    #[test]
    fn capability_reports_missing_oauth1_credentials_without_hiding_manual_rtmp() {
        let capability = x_native_live_capability(None).unwrap();

        assert_eq!(capability.platform, StreamPlatform::X);
        assert_eq!(
            capability.state,
            XNativeLiveCapabilityState::MissingCredentials
        );
        assert!(!capability.native_available);
        assert!(capability.manual_rtmp_available);
        assert!(capability.message.contains("OAuth 1.0a"));
    }

    #[test]
    fn selecting_x_account_honors_requested_provider_or_backend_id() {
        let account = PlatformAccount {
            id: "backend-id".to_string(),
            platform: StreamPlatform::X,
            account_id: "provider-id".to_string(),
            account_label: "Videorc".to_string(),
            account_handle: None,
            avatar_url: None,
            scopes: Vec::new(),
            access_token_present: true,
            refresh_token_present: false,
            stream_key_present: false,
            expires_at: None,
            connected_at: "2026-06-03T00:00:00Z".to_string(),
            updated_at: "2026-06-03T00:00:00Z".to_string(),
            status: PlatformAccountStatus::Connected,
        };
        let accounts = vec![account];

        assert_eq!(
            select_x_account(&accounts, Some("provider-id"))
                .unwrap()
                .unwrap()
                .account_id,
            "provider-id"
        );
        assert_eq!(
            select_x_account(&accounts, Some("backend-id"))
                .unwrap()
                .unwrap()
                .id,
            "backend-id"
        );
        assert!(select_x_account(&accounts, Some("missing")).is_err());
    }
}
