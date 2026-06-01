use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use chrono::Utc;
use rusqlite::{Connection, OptionalExtension, params};
use uuid::Uuid;

use crate::diagnostics::permission_pane_for_log;
use crate::protocol::{
    AiArtifact, AiArtifactKind, AiArtifactStatus, HealthEvent, HealthLevel, LayoutSettings,
    OutputSettings, SessionLogEntry, SessionSummary, SourceSelection,
};

#[derive(Clone)]
pub struct Database {
    conn: Arc<Mutex<Connection>>,
    path: PathBuf,
}

#[derive(Debug, Clone)]
pub struct NewSession {
    pub id: String,
    pub title: String,
    pub started_at: String,
    pub mode: String,
    pub output_path: Option<String>,
    pub container: Option<String>,
    pub stream_preset: Option<String>,
    pub sources: SourceSelection,
    pub layout: LayoutSettings,
    pub output: OutputSettings,
}

impl Database {
    pub fn open_default() -> Result<Self> {
        let path = default_database_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("Could not create {}", parent.display()))?;
        }

        let conn = Connection::open(&path)
            .with_context(|| format!("Could not open SQLite database {}", path.display()))?;
        let database = Self {
            conn: Arc::new(Mutex::new(conn)),
            path,
        };
        database.migrate()?;
        Ok(database)
    }

    pub fn path(&self) -> &PathBuf {
        &self.path
    }

    pub fn create_session(&self, session: &NewSession) -> Result<()> {
        let conn = self.lock()?;
        conn.execute(
            "INSERT INTO sessions (
                id, title, started_at, status, mode, output_path, container, stream_preset,
                sources_json, layout_json, output_json
            ) VALUES (?1, ?2, ?3, 'running', ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                session.id,
                session.title,
                session.started_at,
                session.mode,
                session.output_path,
                session.container,
                session.stream_preset,
                serde_json::to_string(&session.sources)?,
                serde_json::to_string(&session.layout)?,
                serde_json::to_string(&session.output)?,
            ],
        )?;
        Ok(())
    }

    pub fn finish_session(
        &self,
        session_id: &str,
        status: &str,
        ended_at: Option<String>,
        mp4_path: Option<String>,
        duration_ms: Option<i64>,
    ) -> Result<()> {
        let conn = self.lock()?;
        conn.execute(
            "UPDATE sessions
             SET status = ?2,
                 ended_at = COALESCE(?3, ended_at),
                 mp4_path = COALESCE(?4, mp4_path),
                 duration_ms = COALESCE(?5, duration_ms)
             WHERE id = ?1",
            params![session_id, status, ended_at, mp4_path, duration_ms],
        )?;
        Ok(())
    }

    pub fn session_recording_path(&self, session_id: &str) -> Result<Option<String>> {
        let conn = self.lock()?;
        let path = conn
            .query_row(
                "SELECT output_path FROM sessions WHERE id = ?1",
                params![session_id],
                |row| row.get(0),
            )
            .optional()?
            .flatten();
        Ok(path)
    }

    pub fn session_output_path(&self, session_id: &str) -> Result<Option<String>> {
        let conn = self.lock()?;
        let path = conn
            .query_row(
                "SELECT COALESCE(mp4_path, output_path) FROM sessions WHERE id = ?1",
                params![session_id],
                |row| row.get(0),
            )
            .optional()?
            .flatten();
        Ok(path)
    }

    pub fn save_ai_artifact(
        &self,
        session_id: &str,
        kind: AiArtifactKind,
        status: AiArtifactStatus,
        content: serde_json::Value,
        file_path: Option<String>,
    ) -> Result<AiArtifact> {
        let artifact = AiArtifact {
            id: Uuid::new_v4().to_string(),
            session_id: session_id.to_string(),
            kind,
            status,
            content,
            file_path,
            created_at: Utc::now().to_rfc3339(),
        };
        let conn = self.lock()?;
        conn.execute(
            "INSERT INTO ai_artifacts (
                id, session_id, kind, status, content_json, file_path, created_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                artifact.id,
                artifact.session_id,
                serde_json::to_string(&artifact.kind)?,
                serde_json::to_string(&artifact.status)?,
                serde_json::to_string(&artifact.content)?,
                artifact.file_path,
                artifact.created_at,
            ],
        )?;
        Ok(artifact)
    }

    pub fn list_ai_artifacts(&self, session_id: &str) -> Result<Vec<AiArtifact>> {
        let conn = self.lock()?;
        self.ai_artifacts_for_session_locked(&conn, session_id)
    }

    pub fn list_health_events(&self, session_id: &str) -> Result<Vec<HealthEvent>> {
        let conn = self.lock()?;
        self.health_events_for_session_locked(&conn, session_id)
    }

    pub fn add_health_event(
        &self,
        session_id: Option<&str>,
        level: HealthLevel,
        code: &str,
        message: &str,
    ) -> Result<HealthEvent> {
        let permission_pane = permission_pane_for_log(code, message);
        let event = HealthEvent {
            id: Uuid::new_v4().to_string(),
            session_id: session_id.map(str::to_string),
            level,
            code: code.to_string(),
            message: message.to_string(),
            permission_pane,
            created_at: Utc::now().to_rfc3339(),
        };
        let conn = self.lock()?;
        conn.execute(
            "INSERT INTO health_events (id, session_id, level, code, message, permission_pane, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                event.id,
                event.session_id,
                serde_json::to_string(&event.level)?,
                event.code,
                event.message,
                event
                    .permission_pane
                    .as_ref()
                    .map(serde_json::to_string)
                    .transpose()?,
                event.created_at,
            ],
        )?;
        Ok(event)
    }

    pub fn add_session_log(
        &self,
        session_id: &str,
        level: HealthLevel,
        code: &str,
        message: &str,
        source_id: Option<&str>,
    ) -> Result<SessionLogEntry> {
        let permission_pane = permission_pane_for_log(code, message);
        let entry = SessionLogEntry {
            id: Uuid::new_v4().to_string(),
            session_id: session_id.to_string(),
            level,
            code: code.to_string(),
            message: message.to_string(),
            source_id: source_id.map(str::to_string),
            permission_pane,
            created_at: Utc::now().to_rfc3339(),
        };
        let conn = self.lock()?;
        conn.execute(
            "INSERT INTO session_logs (
                id, session_id, level, code, message, source_id, permission_pane, created_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                entry.id,
                entry.session_id,
                serde_json::to_string(&entry.level)?,
                entry.code,
                entry.message,
                entry.source_id,
                entry
                    .permission_pane
                    .as_ref()
                    .map(serde_json::to_string)
                    .transpose()?,
                entry.created_at,
            ],
        )?;
        Ok(entry)
    }

    pub fn list_sessions(&self, limit: usize) -> Result<Vec<SessionSummary>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT id, title, started_at, ended_at, status, mode, output_path, mp4_path,
                    stream_preset, container, duration_ms, sources_json, layout_json
             FROM sessions
             ORDER BY started_at DESC
             LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit as i64], |row| {
            let id: String = row.get(0)?;
            let sources_json: String = row.get(11)?;
            let layout_json: String = row.get(12)?;
            Ok((
                id,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, Option<String>>(6)?,
                row.get::<_, Option<String>>(7)?,
                row.get::<_, Option<String>>(8)?,
                row.get::<_, Option<String>>(9)?,
                row.get::<_, Option<i64>>(10)?,
                sources_json,
                layout_json,
            ))
        })?;

        let mut sessions = Vec::new();
        for row in rows {
            let (
                id,
                title,
                started_at,
                ended_at,
                status,
                mode,
                output_path,
                mp4_path,
                stream_preset,
                container,
                duration_ms,
                sources_json,
                layout_json,
            ) = row?;

            sessions.push(SessionSummary {
                health_events: self.health_events_for_session_locked(&conn, &id)?,
                session_logs: self.session_logs_for_session_locked(&conn, &id)?,
                ai_artifacts: self.ai_artifacts_for_session_locked(&conn, &id)?,
                id,
                title,
                started_at,
                ended_at,
                status,
                mode,
                output_path,
                mp4_path,
                stream_preset,
                container,
                duration_ms,
                sources: serde_json::from_str(&sources_json)?,
                layout: serde_json::from_str(&layout_json)?,
            });
        }

        Ok(sessions)
    }

    pub fn save_setting<T: serde::Serialize>(&self, key: &str, value: &T) -> Result<()> {
        let conn = self.lock()?;
        conn.execute(
            "INSERT INTO app_settings (key, value_json, updated_at)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(key) DO UPDATE SET value_json = excluded.value_json, updated_at = excluded.updated_at",
            params![key, serde_json::to_string(value)?, Utc::now().to_rfc3339()],
        )?;
        Ok(())
    }

    fn migrate(&self) -> Result<()> {
        let conn = self.lock()?;
        conn.execute_batch(
            "
            PRAGMA foreign_keys = ON;

            CREATE TABLE IF NOT EXISTS sessions (
                id TEXT PRIMARY KEY,
                title TEXT NOT NULL,
                started_at TEXT NOT NULL,
                ended_at TEXT,
                status TEXT NOT NULL,
                mode TEXT NOT NULL,
                output_path TEXT,
                mp4_path TEXT,
                stream_preset TEXT,
                container TEXT,
                duration_ms INTEGER,
                sources_json TEXT NOT NULL,
                layout_json TEXT NOT NULL,
                output_json TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS health_events (
                id TEXT PRIMARY KEY,
                session_id TEXT,
                level TEXT NOT NULL,
                code TEXT NOT NULL,
                message TEXT NOT NULL,
                permission_pane TEXT,
                created_at TEXT NOT NULL,
                FOREIGN KEY(session_id) REFERENCES sessions(id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS session_logs (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                level TEXT NOT NULL,
                code TEXT NOT NULL,
                message TEXT NOT NULL,
                source_id TEXT,
                permission_pane TEXT,
                created_at TEXT NOT NULL,
                FOREIGN KEY(session_id) REFERENCES sessions(id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS ai_artifacts (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                kind TEXT NOT NULL,
                status TEXT NOT NULL,
                content_json TEXT NOT NULL,
                file_path TEXT,
                created_at TEXT NOT NULL,
                FOREIGN KEY(session_id) REFERENCES sessions(id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS app_settings (
                key TEXT PRIMARY KEY,
                value_json TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            ",
        )?;
        ensure_column(&conn, "sessions", "container", "container TEXT")?;
        ensure_column(&conn, "sessions", "duration_ms", "duration_ms INTEGER")?;
        ensure_column(
            &conn,
            "health_events",
            "permission_pane",
            "permission_pane TEXT",
        )?;
        Ok(())
    }

    fn ai_artifacts_for_session_locked(
        &self,
        conn: &Connection,
        session_id: &str,
    ) -> Result<Vec<AiArtifact>> {
        let mut stmt = conn.prepare(
            "SELECT id, session_id, kind, status, content_json, file_path, created_at
             FROM ai_artifacts
             WHERE session_id = ?1
             ORDER BY created_at ASC",
        )?;
        let rows = stmt.query_map(params![session_id], |row| {
            let kind_json: String = row.get(2)?;
            let status_json: String = row.get(3)?;
            let content_json: String = row.get(4)?;
            Ok(AiArtifact {
                id: row.get(0)?,
                session_id: row.get(1)?,
                kind: serde_json::from_str(&kind_json).unwrap_or(AiArtifactKind::Transcript),
                status: serde_json::from_str(&status_json).unwrap_or(AiArtifactStatus::Failed),
                content: serde_json::from_str(&content_json)
                    .unwrap_or_else(|_| serde_json::json!({})),
                file_path: row.get(5)?,
                created_at: row.get(6)?,
            })
        })?;

        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    fn health_events_for_session_locked(
        &self,
        conn: &Connection,
        session_id: &str,
    ) -> Result<Vec<HealthEvent>> {
        let mut stmt = conn.prepare(
            "SELECT id, session_id, level, code, message, permission_pane, created_at
             FROM health_events
             WHERE session_id = ?1
             ORDER BY created_at ASC",
        )?;
        let rows = stmt.query_map(params![session_id], |row| {
            let level_json: String = row.get(2)?;
            let permission_json: Option<String> = row.get(5)?;
            Ok(HealthEvent {
                id: row.get(0)?,
                session_id: row.get(1)?,
                level: serde_json::from_str(&level_json).unwrap_or(HealthLevel::Warn),
                code: row.get(3)?,
                message: row.get(4)?,
                permission_pane: permission_json
                    .as_deref()
                    .and_then(|value| serde_json::from_str(value).ok()),
                created_at: row.get(6)?,
            })
        })?;

        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    fn session_logs_for_session_locked(
        &self,
        conn: &Connection,
        session_id: &str,
    ) -> Result<Vec<SessionLogEntry>> {
        let mut stmt = conn.prepare(
            "SELECT id, session_id, level, code, message, source_id, permission_pane, created_at
             FROM session_logs
             WHERE session_id = ?1
             ORDER BY created_at ASC",
        )?;
        let rows = stmt.query_map(params![session_id], |row| {
            let level_json: String = row.get(2)?;
            let permission_json: Option<String> = row.get(6)?;
            Ok(SessionLogEntry {
                id: row.get(0)?,
                session_id: row.get(1)?,
                level: serde_json::from_str(&level_json).unwrap_or(HealthLevel::Warn),
                code: row.get(3)?,
                message: row.get(4)?,
                source_id: row.get(5)?,
                permission_pane: permission_json
                    .as_deref()
                    .and_then(|value| serde_json::from_str(value).ok()),
                created_at: row.get(7)?,
            })
        })?;

        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, Connection>> {
        self.conn
            .lock()
            .map_err(|_| anyhow::anyhow!("SQLite connection lock was poisoned"))
    }
}

fn ensure_column(conn: &Connection, table: &str, column: &str, definition: &str) -> Result<()> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let columns = stmt.query_map([], |row| row.get::<_, String>(1))?;

    for existing in columns {
        if existing? == column {
            return Ok(());
        }
    }

    conn.execute(&format!("ALTER TABLE {table} ADD COLUMN {definition}"), [])?;
    Ok(())
}

pub fn default_database_path() -> PathBuf {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));

    if cfg!(target_os = "macos") {
        home.join("Library")
            .join("Application Support")
            .join("Videorc")
            .join("videorc.sqlite3")
    } else {
        home.join(".videorc").join("videorc.sqlite3")
    }
}

pub fn default_preview_dir() -> PathBuf {
    default_database_path()
        .parent()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Previews")
}

pub fn default_artifacts_dir() -> PathBuf {
    default_database_path()
        .parent()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Artifacts")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{
        CameraCorner, CameraFit, CameraShape, CameraSize, OutputSettings, PermissionPane,
        RtmpPreset, RtmpSettings, VideoPreset, VideoSettings,
    };

    fn test_database() -> Database {
        let database = Database {
            conn: Arc::new(Mutex::new(Connection::open_in_memory().unwrap())),
            path: PathBuf::from(":memory:"),
        };
        database.migrate().unwrap();
        database
    }

    fn sample_session(id: &str) -> NewSession {
        NewSession {
            id: id.to_string(),
            title: "Test session".to_string(),
            started_at: "2026-05-31T00:00:00Z".to_string(),
            mode: "record".to_string(),
            output_path: Some("/tmp/videorc-test.mkv".to_string()),
            container: Some("mkv".to_string()),
            stream_preset: None,
            sources: SourceSelection {
                screen_id: Some("screen:avfoundation:1".to_string()),
                window_id: None,
                camera_id: None,
                microphone_id: None,
                test_pattern: false,
            },
            layout: LayoutSettings {
                camera_corner: CameraCorner::BottomRight,
                camera_size: CameraSize::Medium,
                camera_shape: CameraShape::Rectangle,
                camera_margin: 32,
                camera_fit: CameraFit::Fill,
                camera_mirror: false,
                camera_zoom: 100,
                camera_offset_x: 0,
                camera_offset_y: 0,
            },
            output: OutputSettings {
                record_enabled: true,
                stream_enabled: false,
                output_directory: None,
                ffmpeg_path: None,
                video: VideoSettings {
                    preset: VideoPreset::Tutorial1440p30,
                    width: 2560,
                    height: 1440,
                    fps: 30,
                    bitrate_kbps: 8000,
                },
                rtmp: RtmpSettings {
                    preset: RtmpPreset::Custom,
                    server_url: String::new(),
                    stream_key: String::new(),
                },
            },
        }
    }

    #[test]
    fn default_database_path_uses_application_support_on_macos() {
        let path = default_database_path();
        let rendered = path.display().to_string();

        assert!(rendered.contains("Videorc"));
        assert!(rendered.ends_with("videorc.sqlite3"));
    }

    #[test]
    fn session_payload_round_trips_through_json() {
        let layout = LayoutSettings {
            camera_corner: CameraCorner::BottomRight,
            camera_size: CameraSize::Medium,
            camera_shape: CameraShape::Circle,
            camera_margin: 32,
            camera_fit: CameraFit::Fill,
            camera_mirror: true,
            camera_zoom: 125,
            camera_offset_x: 10,
            camera_offset_y: -5,
        };
        let sources = SourceSelection {
            screen_id: Some("screen:avfoundation:1".to_string()),
            window_id: None,
            camera_id: Some("camera:avfoundation:0".to_string()),
            microphone_id: Some("microphone:avfoundation:0".to_string()),
            test_pattern: false,
        };
        let output = OutputSettings {
            record_enabled: true,
            stream_enabled: true,
            output_directory: None,
            ffmpeg_path: None,
            video: VideoSettings {
                preset: VideoPreset::Tutorial1440p30,
                width: 2560,
                height: 1440,
                fps: 30,
                bitrate_kbps: 8000,
            },
            rtmp: RtmpSettings {
                preset: RtmpPreset::YouTube,
                server_url: "rtmp://a.rtmp.youtube.com/live2".to_string(),
                stream_key: "abc".to_string(),
            },
        };

        assert!(
            serde_json::to_string(&layout)
                .unwrap()
                .contains("bottom-right")
        );
        assert!(serde_json::to_string(&sources).unwrap().contains("screen:"));
        assert!(serde_json::to_string(&output).unwrap().contains("youtube"));
    }

    #[test]
    fn session_logs_and_permission_actions_round_trip() {
        let database = test_database();
        database
            .create_session(&sample_session("session-1"))
            .unwrap();

        database
            .add_health_event(
                Some("session-1"),
                HealthLevel::Warn,
                "camera-source-unavailable",
                "Camera permission denied.",
            )
            .unwrap();
        database
            .add_session_log(
                "session-1",
                HealthLevel::Error,
                "screen-capture-fallback",
                "Screen recording permission denied.",
                Some("screen:avfoundation:1"),
            )
            .unwrap();

        let sessions = database.list_sessions(1).unwrap();
        let session = sessions.first().unwrap();

        assert_eq!(
            session.health_events[0].permission_pane,
            Some(PermissionPane::Camera)
        );
        assert_eq!(
            session.session_logs[0].permission_pane,
            Some(PermissionPane::ScreenRecording)
        );
        assert_eq!(
            session.session_logs[0].source_id.as_deref(),
            Some("screen:avfoundation:1")
        );
    }

    #[test]
    fn session_output_path_prefers_mp4_export_when_available() {
        let database = test_database();
        database
            .create_session(&sample_session("session-1"))
            .unwrap();

        assert_eq!(
            database
                .session_recording_path("session-1")
                .unwrap()
                .as_deref(),
            Some("/tmp/videorc-test.mkv")
        );
        assert_eq!(
            database
                .session_output_path("session-1")
                .unwrap()
                .as_deref(),
            Some("/tmp/videorc-test.mkv")
        );

        database
            .finish_session(
                "session-1",
                "completed",
                None,
                Some("/tmp/videorc-test.mp4".to_string()),
                None,
            )
            .unwrap();

        assert_eq!(
            database
                .session_recording_path("session-1")
                .unwrap()
                .as_deref(),
            Some("/tmp/videorc-test.mkv")
        );
        assert_eq!(
            database
                .session_output_path("session-1")
                .unwrap()
                .as_deref(),
            Some("/tmp/videorc-test.mp4")
        );
    }
}
