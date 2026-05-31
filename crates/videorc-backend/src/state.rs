use std::sync::Arc;

use chrono::Utc;
use tokio::sync::broadcast;

use crate::diagnostics::idle_diagnostics;
use crate::protocol::{BackendLogEvent, DiagnosticStats, Scene, ServerEvent};
use crate::recording::{LivePreviewSlot, RecordingSlot, initial_live_preview_state};
use crate::scene::default_scene;
use crate::storage::Database;

const PREVIEW_FRAME_CHANNEL_CAPACITY: usize = 4;

#[derive(Clone)]
pub struct AppState {
    pub token: String,
    pub port: u16,
    pub events: broadcast::Sender<ServerEvent>,
    pub recording: RecordingSlot,
    pub live_preview: LivePreviewSlot,
    pub preview_frames: broadcast::Sender<Vec<u8>>,
    pub preview_latest_frame: Arc<tokio::sync::RwLock<Option<Vec<u8>>>>,
    pub scene: Arc<tokio::sync::Mutex<Scene>>,
    pub diagnostics: Arc<tokio::sync::Mutex<DiagnosticStats>>,
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
            live_preview: Arc::new(tokio::sync::Mutex::new(initial_live_preview_state())),
            preview_frames: broadcast::channel(PREVIEW_FRAME_CHANNEL_CAPACITY).0,
            preview_latest_frame: Arc::new(tokio::sync::RwLock::new(None)),
            scene: Arc::new(tokio::sync::Mutex::new(default_scene())),
            diagnostics: Arc::new(tokio::sync::Mutex::new(idle_diagnostics())),
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
