use std::sync::Arc;

use chrono::Utc;
use tokio::sync::broadcast;

use crate::protocol::{BackendLogEvent, ServerEvent};
use crate::recording::RecordingSlot;
use crate::storage::Database;

#[derive(Clone)]
pub struct AppState {
    pub token: String,
    pub port: u16,
    pub events: broadcast::Sender<ServerEvent>,
    pub recording: RecordingSlot,
    pub database: Database,
}

impl AppState {
    pub fn new(
        token: String,
        port: u16,
        events: broadcast::Sender<ServerEvent>,
        database: Database,
    ) -> Self {
        Self {
            token,
            port,
            events,
            recording: Arc::new(tokio::sync::Mutex::new(None)),
            database,
        }
    }

    pub fn emit_event<T: serde::Serialize>(&self, event: impl Into<String>, payload: T) {
        let _ = self.events.send(ServerEvent::new(event, payload));
    }

    pub fn emit_log(&self, level: impl Into<String>, message: impl Into<String>) {
        let payload = BackendLogEvent {
            level: level.into(),
            message: message.into(),
            timestamp: Utc::now().to_rfc3339(),
        };
        let level = payload.level.clone();
        let message = payload.message.clone();
        match level.as_str() {
            "error" => tracing::error!("{message}"),
            "warn" => tracing::warn!("{message}"),
            _ => tracing::info!("{message}"),
        }
        self.emit_event("log", payload);
    }
}
