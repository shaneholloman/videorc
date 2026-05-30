mod devices;
mod protocol;
mod recording;
mod state;
mod storage;

use std::io::Write;
use std::process::Stdio;
use std::time::Duration;

use anyhow::Result;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};
use futures_util::{SinkExt, StreamExt};
use protocol::{
    BackendConnection, BackendHealth, ClientCommand, RecordingState, ServerEvent, ServerResponse,
    ToolStatus,
};
use recording::{idle_status, remux_session, start_session, stop_recording};
use serde::Deserialize;
use tokio::net::TcpListener;
use tokio::process::Command;
use tokio::sync::{broadcast, mpsc};
use tokio::time::timeout;
use tracing_subscriber::EnvFilter;
use uuid::Uuid;

use crate::state::AppState;
use crate::storage::Database;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::from_default_env().add_directive("videogre_backend=info".parse()?),
        )
        .with_writer(std::io::stderr)
        .init();

    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();
    let token = Uuid::new_v4().to_string();
    let (events, _) = broadcast::channel(256);
    let database = Database::open_default()?;
    let state = AppState::new(token.clone(), port, events, database);
    let app = Router::new()
        .route("/health", get(health_handler))
        .route("/ws", get(ws_handler))
        .with_state(state.clone());

    let ready = BackendConnection {
        host: "127.0.0.1".to_string(),
        port,
        token,
    };
    println!("READY {}", serde_json::to_string(&ready)?);
    std::io::stdout().flush()?;

    state.emit_log("info", "Videogre backend ready.");
    axum::serve(listener, app).await?;
    Ok(())
}

#[derive(Debug, Deserialize)]
struct WsQuery {
    token: String,
}

async fn health_handler(State(state): State<AppState>) -> Json<BackendHealth> {
    Json(backend_health(&state, "ffmpeg").await)
}

async fn ws_handler(
    State(state): State<AppState>,
    Query(query): Query<WsQuery>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    if query.token != state.token {
        return StatusCode::UNAUTHORIZED.into_response();
    }

    ws.on_upgrade(move |socket| websocket_session(socket, state))
        .into_response()
}

async fn websocket_session(socket: WebSocket, state: AppState) {
    let (mut sender, mut receiver) = socket.split();
    let mut events = state.events.subscribe();
    let (outgoing_tx, mut outgoing_rx) = mpsc::unbounded_channel::<String>();
    let event_tx = outgoing_tx.clone();

    tokio::spawn(async move {
        while let Ok(event) = events.recv().await {
            match serde_json::to_string(&event) {
                Ok(text) => {
                    if event_tx.send(text).is_err() {
                        break;
                    }
                }
                Err(error) => tracing::error!("Could not serialize event: {error}"),
            }
        }
    });

    let ready_event = ServerEvent::new(
        "backend.ready",
        BackendConnection {
            host: "127.0.0.1".to_string(),
            port: state.port,
            token: state.token.clone(),
        },
    );
    if let Ok(text) = serde_json::to_string(&ready_event) {
        let _ = sender.send(Message::Text(text.into())).await;
    }

    loop {
        tokio::select! {
            Some(text) = outgoing_rx.recv() => {
                if sender.send(Message::Text(text.into())).await.is_err() {
                    break;
                }
            }
            incoming = receiver.next() => {
                let Some(incoming) = incoming else {
                    break;
                };

                match incoming {
                    Ok(Message::Text(text)) => {
                        let response = handle_text_message(&state, text.as_str()).await;
                        match serde_json::to_string(&response) {
                            Ok(text) => {
                                if sender.send(Message::Text(text.into())).await.is_err() {
                                    break;
                                }
                            }
                            Err(error) => tracing::error!("Could not serialize response: {error}"),
                        }
                    }
                    Ok(Message::Close(_)) => break,
                    Ok(Message::Ping(payload)) => {
                        let _ = sender.send(Message::Pong(payload)).await;
                    }
                    Ok(_) => {}
                    Err(error) => {
                        tracing::warn!("WebSocket receive error: {error}");
                        break;
                    }
                }
            }
        }
    }
}

async fn handle_text_message(state: &AppState, text: &str) -> ServerResponse {
    let command = match serde_json::from_str::<ClientCommand>(text) {
        Ok(command) => command,
        Err(error) => {
            return ServerResponse::error(
                "unknown",
                "invalid-json",
                format!("Could not parse command: {error}"),
            );
        }
    };

    match command.method.as_str() {
        "health.ping" => {
            let ffmpeg_path = command
                .params
                .get("ffmpegPath")
                .and_then(|value| value.as_str())
                .filter(|value| !value.trim().is_empty())
                .unwrap_or("ffmpeg");
            ServerResponse::ok(command.id, backend_health(state, ffmpeg_path).await)
        }
        "devices.list" => {
            let ffmpeg_path = command
                .params
                .get("ffmpegPath")
                .and_then(|value| value.as_str())
                .filter(|value| !value.trim().is_empty())
                .unwrap_or("ffmpeg");
            let devices = devices::list_devices(ffmpeg_path).await;
            state.emit_event("devices.changed", &devices);
            ServerResponse::ok(command.id, devices)
        }
        "recording.start_test" => {
            match serde_json::from_value::<protocol::StartSessionParams>(command.params) {
                Ok(params) => match start_session(state.clone(), params).await {
                    Ok(status) => ServerResponse::ok(command.id, status),
                    Err(error) => ServerResponse::error(
                        command.id,
                        "recording-start-failed",
                        error.to_string(),
                    ),
                },
                Err(error) => {
                    ServerResponse::error(command.id, "invalid-params", error.to_string())
                }
            }
        }
        "session.start" => {
            match serde_json::from_value::<protocol::StartSessionParams>(command.params) {
                Ok(params) => match start_session(state.clone(), params).await {
                    Ok(status) => ServerResponse::ok(command.id, status),
                    Err(error) => {
                        ServerResponse::error(command.id, "session-start-failed", error.to_string())
                    }
                },
                Err(error) => {
                    ServerResponse::error(command.id, "invalid-params", error.to_string())
                }
            }
        }
        "session.stop" => match stop_recording(state.clone()).await {
            Ok(status) => ServerResponse::ok(command.id, status),
            Err(error) => {
                ServerResponse::error(command.id, "session-stop-failed", error.to_string())
            }
        },
        "sessions.list" => match state.database.list_sessions(20) {
            Ok(sessions) => ServerResponse::ok(command.id, sessions),
            Err(error) => {
                ServerResponse::error(command.id, "sessions-list-failed", error.to_string())
            }
        },
        "session.remux_mp4" => {
            match serde_json::from_value::<protocol::RemuxSessionParams>(command.params) {
                Ok(params) => match remux_session(state.clone(), params).await {
                    Ok(mp4_path) => {
                        ServerResponse::ok(command.id, serde_json::json!({ "mp4Path": mp4_path }))
                    }
                    Err(error) => {
                        ServerResponse::error(command.id, "remux-failed", error.to_string())
                    }
                },
                Err(error) => {
                    ServerResponse::error(command.id, "invalid-params", error.to_string())
                }
            }
        }
        "recording.stop" => match stop_recording(state.clone()).await {
            Ok(status) => ServerResponse::ok(command.id, status),
            Err(error) => {
                ServerResponse::error(command.id, "recording-stop-failed", error.to_string())
            }
        },
        "recording.status" => {
            let status = state
                .recording
                .lock()
                .await
                .as_ref()
                .map(|active| {
                    let state = if active.mode == "stream" {
                        RecordingState::Streaming
                    } else {
                        RecordingState::Recording
                    };
                    active.status(state, None)
                })
                .unwrap_or_else(idle_status);
            ServerResponse::ok(command.id, status)
        }
        method => ServerResponse::error(
            command.id,
            "unknown-method",
            format!("Unknown backend method: {method}"),
        ),
    }
}

async fn backend_health(state: &AppState, ffmpeg_path: &str) -> BackendHealth {
    BackendHealth {
        status: "ok".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        platform: std::env::consts::OS.to_string(),
        ffmpeg: ffmpeg_status(ffmpeg_path).await,
        database_path: state.database.path().display().to_string(),
    }
}

async fn ffmpeg_status(ffmpeg_path: &str) -> ToolStatus {
    let output = timeout(
        Duration::from_secs(4),
        Command::new(ffmpeg_path)
            .arg("-version")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output(),
    )
    .await;

    match output {
        Ok(Ok(output)) if output.status.success() => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            ToolStatus {
                path: ffmpeg_path.to_string(),
                available: true,
                version: stdout.lines().next().map(|line| line.to_string()),
                message: None,
            }
        }
        Ok(Ok(output)) => ToolStatus {
            path: ffmpeg_path.to_string(),
            available: false,
            version: None,
            message: Some(String::from_utf8_lossy(&output.stderr).trim().to_string()),
        },
        Ok(Err(error)) => ToolStatus {
            path: ffmpeg_path.to_string(),
            available: false,
            version: None,
            message: Some(error.to_string()),
        },
        Err(_) => ToolStatus {
            path: ffmpeg_path.to_string(),
            available: false,
            version: None,
            message: Some("Timed out while checking FFmpeg.".to_string()),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn response_shape_omits_empty_error() {
        let response = ServerResponse::ok("abc", json!({ "pong": true }));
        let value = serde_json::to_value(response).unwrap();

        assert_eq!(value["id"], "abc");
        assert_eq!(value["ok"], true);
        assert!(value.get("error").is_none());
    }
}
