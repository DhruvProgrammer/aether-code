//! Session / task / checkpoint store (spec §21, Phase 2). SQLite-backed, sync.
//! Deliberately separate from the `aether-mind` graph/vector store: these are
//! relational, append-only logs, not semantic memory.

use anyhow::Result;
use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Arc;

pub mod snapshot;

pub use snapshot::{
    Cursor, FileSnapshot, Snapshot, SnapshotDiff, SnapshotError, SnapshotManager, Trigger,
};

#[derive(Debug, Clone)]
pub struct SessionMeta {
    pub id: String,
    pub created_at: String,
    pub task: Option<String>,
    pub plan: Option<String>,
    pub result: Option<String>,
    /// Session display title (v0.17+ column; None for legacy rows).
    pub title: Option<String>,
}

/// A row from the `messages` table, used to seed the Executor's conversation transcript
/// on `--resume` (BUG-P1-05 regression).
#[derive(Debug, Clone)]
pub struct MessageRow {
    pub id: i64,
    pub role: String,
    pub content: String,
    pub tool_calls: Option<Vec<serde_json::Value>>,
    pub tool_call_id: Option<String>,
}

/// v0.27 part-typed message fragment. Old rows still appear as
/// `MessagePart::Text`; new rows can also be `Tool`, `Reasoning`, or
/// `Compaction`. The flat `messages.content` remains the rendered
/// fallback for backward compat.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum MessagePart {
    Text { text: String },
    Tool { tool_call_id: String, name: String, args: serde_json::Value, output: Option<String>, status: String },
    Reasoning { text: String },
    Compaction { summary: String, tokens_before: u32, tokens_after: u32 },
}

impl MessagePart {
    /// Render this part to a single string (used for legacy flat-content
    /// compatibility and tool-result display).
    pub fn render(&self) -> String {
        match self {
            MessagePart::Text { text } => text.clone(),
            MessagePart::Tool { name, args, output, status, .. } => {
                let args_str = serde_json::to_string(args).unwrap_or_default();
                let out_str = output.clone().unwrap_or_default();
                format!("[Tool {name}({args_str}) status={status}]: {out_str}")
            }
            MessagePart::Reasoning { text } => format!("[Reasoning]: {text}"),
            MessagePart::Compaction { summary, tokens_before, tokens_after } => {
                format!("[Compaction {tokens_before}->{tokens_after}]: {summary}")
            }
        }
    }

    /// Stable kind string for the `message_parts.kind` column.
    pub fn kind_str(&self) -> &'static str {
        match self {
            MessagePart::Text { .. } => "text",
            MessagePart::Tool { .. } => "tool",
            MessagePart::Reasoning { .. } => "reasoning",
            MessagePart::Compaction { .. } => "compaction",
        }
    }
}

impl SessionStore {
    /// Persist a part-typed message and return its message id. The flat
    /// `messages.content` is populated with the rendered concatenation of
    /// parts so the legacy read API still works.
    pub fn add_message_parts(&self, session_id: &str, role: &str, parts: &[MessagePart]) -> Result<i64> {
        let now = chrono::Utc::now().to_rfc3339();
        let rendered = parts.iter().map(|p| p.render()).collect::<Vec<_>>().join("\n");
        self.conn.execute(
            "INSERT INTO messages(session_id, role, content, ts) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![session_id, role, rendered, now],
        )?;
        let id = self.conn.last_insert_rowid();
        for (i, p) in parts.iter().enumerate() {
            let payload = serde_json::to_string(p).unwrap_or_else(|_| "null".to_string());
            self.conn.execute(
                "INSERT INTO message_parts(message_id, ordinal, kind, payload) VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![id, i as i64, p.kind_str(), payload],
            )?;
        }
        Ok(id)
    }

    /// Fetch the parts of a single message in original order. Returns
    /// `None` if the message does not exist; returns `Some(vec![Text{...}])`
    /// (synthesised) if the message has no parts table entries (legacy).
    pub fn get_message_parts(&self, message_id: i64) -> Result<Option<Vec<MessagePart>>> {
        // First check the message exists
        let exists: bool = self.conn.query_row(
            "SELECT 1 FROM messages WHERE id = ?1",
            rusqlite::params![message_id],
            |_r| Ok(true),
        ).optional().unwrap_or(Some(false)).unwrap_or(false);
        if !exists {
            return Ok(None);
        }
        let mut stmt = self.conn.prepare(
            "SELECT kind, payload FROM message_parts WHERE message_id = ?1 ORDER BY ordinal ASC",
        )?;
        let rows = stmt.query_map(rusqlite::params![message_id], |r| {
            let kind: String = r.get(0)?;
            let payload: String = r.get(1)?;
            Ok((kind, payload))
        })?;
        let mut out: Vec<MessagePart> = Vec::new();
        for row in rows {
            let (_kind, payload) = row?;
            // Handle two payload shapes:
            //   1. Tagged JSON:    {"Text":{"text":"..."}}  or  {"Tool":{...}}
            //   2. Bare string:    "<plain text>"           (legacy / "Text" rows)
            let p: MessagePart = match serde_json::from_str::<serde_json::Value>(&payload) {
                Ok(v) => match v {
                    serde_json::Value::Object(map) => {
                        // Tagged form: pick the only key.
                        if map.len() == 1 {
                            let (k, inner) = map.into_iter().next().unwrap();
                            match k.as_str() {
                                "Text" => {
                                    if let Some(t) = inner.get("text").and_then(|x| x.as_str()) {
                                        MessagePart::Text { text: t.to_string() }
                                    } else {
                                        MessagePart::Text { text: payload }
                                    }
                                }
                                "Reasoning" => {
                                    let t = inner.get("text").and_then(|x| x.as_str()).unwrap_or("").to_string();
                                    MessagePart::Reasoning { text: t }
                                }
                                "Compaction" => {
                                    let summary = inner.get("summary").and_then(|x| x.as_str()).unwrap_or("").to_string();
                                    let tb = inner.get("tokens_before").and_then(|x| x.as_u64()).unwrap_or(0) as u32;
                                    let ta = inner.get("tokens_after").and_then(|x| x.as_u64()).unwrap_or(0) as u32;
                                    MessagePart::Compaction { summary, tokens_before: tb, tokens_after: ta }
                                }
                                "Tool" => {
                                    let tool_call_id = inner.get("tool_call_id").and_then(|x| x.as_str()).unwrap_or("").to_string();
                                    let name = inner.get("name").and_then(|x| x.as_str()).unwrap_or("").to_string();
                                    let args = inner.get("args").cloned().unwrap_or(serde_json::Value::Null);
                                    let output = inner.get("output").and_then(|x| x.as_str()).map(|s| s.to_string());
                                    let status = inner.get("status").and_then(|x| x.as_str()).unwrap_or("ok").to_string();
                                    MessagePart::Tool { tool_call_id, name, args, output, status }
                                }
                                _ => MessagePart::Text { text: payload },
                            }
                        } else {
                            MessagePart::Text { text: payload }
                        }
                    }
                    _ => MessagePart::Text { text: payload },
                },
                Err(_) => MessagePart::Text { text: payload },
            };
            out.push(p);
        }
        if out.is_empty() {
            // Legacy fallback: render from messages.content
            let content: String = self.conn.query_row(
                "SELECT content FROM messages WHERE id = ?1",
                rusqlite::params![message_id],
                |r| r.get(0),
            )?;
            out.push(MessagePart::Text { text: content });
        }
        Ok(Some(out))
    }

    /// Crash-recovery helper: drop every message after `keep_until_message_id`
    /// in this session, and drop their parts. Used by `recovery_state` in the
    /// agent loop to resume at a safe boundary.
    pub fn truncate_after(&self, session_id: &str, keep_until_message_id: i64) -> Result<u32> {
        let n: u32 = self.conn.query_row(
            "SELECT COUNT(*) FROM messages WHERE session_id = ?1 AND id > ?2",
            rusqlite::params![session_id, keep_until_message_id],
            |r| r.get::<_, i64>(0),
        )? as u32;
        self.conn.execute(
            "DELETE FROM message_parts WHERE message_id IN (SELECT id FROM messages WHERE session_id = ?1 AND id > ?2)",
            rusqlite::params![session_id, keep_until_message_id],
        )?;
        self.conn.execute(
            "DELETE FROM messages WHERE session_id = ?1 AND id > ?2",
            rusqlite::params![session_id, keep_until_message_id],
        )?;
        Ok(n)
    }
}

pub struct SessionStore {
    conn: Connection,
}

impl SessionStore {
    pub fn open(path: &Path) -> Result<Arc<Self>> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS sessions(
                id TEXT PRIMARY KEY,
                created_at TEXT NOT NULL,
                task TEXT,
                plan TEXT,
                result TEXT
            );
            CREATE TABLE IF NOT EXISTS messages(
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id TEXT NOT NULL,
                role TEXT NOT NULL,
                content TEXT NOT NULL,
                ts TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS tool_calls(
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id TEXT NOT NULL,
                tool TEXT NOT NULL,
                args TEXT,
                output TEXT,
                ts TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS checkpoints(
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id TEXT NOT NULL,
                tool TEXT NOT NULL,
                path TEXT NOT NULL,
                before_content TEXT,
                ts TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS message_parts(
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                message_id INTEGER NOT NULL,
                ordinal INTEGER NOT NULL,
                kind TEXT NOT NULL,
                payload TEXT NOT NULL,
                FOREIGN KEY (message_id) REFERENCES messages(id) ON DELETE CASCADE
            );
            CREATE TABLE IF NOT EXISTS kv(
                session_id TEXT NOT NULL,
                key TEXT NOT NULL,
                value TEXT NOT NULL,
                PRIMARY KEY (session_id, key)
            );
            CREATE TABLE IF NOT EXISTS traces(
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id TEXT NOT NULL,
                ts TEXT NOT NULL,
                kind TEXT NOT NULL,                agent TEXT NOT NULL,
                parent TEXT,
                summary TEXT NOT NULL,
                payload TEXT
            );",
        )?;
        // BUG-P1-05: migrate older sessions DBs to add tool-call persistence columns
        // so `--resume` can restore the full conversation transcript. SQLite ignores
        // duplicate-column errors silently if we use a SELECT-count guard, but the
        // simplest robust path is to attempt ADD COLUMN and tolerate failures.
        let _ = conn.execute("ALTER TABLE messages ADD COLUMN tool_calls TEXT", []);
        let _ = conn.execute("ALTER TABLE messages ADD COLUMN tool_call_id TEXT", []);
        // v0.17: workspace + role assignment columns.
        let _ = conn.execute("ALTER TABLE sessions ADD COLUMN workspace_id TEXT", []);
        let _ = conn.execute("ALTER TABLE sessions ADD COLUMN title TEXT", []);
        let _ = conn.execute("ALTER TABLE sessions ADD COLUMN role_assignments TEXT", []);
        let _ = conn.execute("ALTER TABLE sessions ADD COLUMN updated_at TEXT", []);
        Ok(Arc::new(Self { conn }))
    }

    pub fn new_session(&self) -> Result<String> {
        let id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().to_rfc3339();
        self.conn
            .execute("INSERT INTO sessions(id, created_at) VALUES (?1, ?2)", (id.as_str(), now.as_str()))?;
        Ok(id)
    }

    /// Create a session belonging to a workspace with an optional title.
    pub fn new_session_in_workspace(&self, workspace_id: &str, title: Option<&str>) -> Result<String> {
        let id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().to_rfc3339();
        self.conn.execute(
            "INSERT INTO sessions(id, created_at, workspace_id, title, updated_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            (id.as_str(), now.as_str(), workspace_id, title, now.as_str()),
        )?;
        Ok(id)
    }

    /// Look up which workspace a session belongs to (for restore confinement).
    pub fn session_workspace_id(&self, session_id: &str) -> Result<Option<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT workspace_id FROM sessions WHERE id = ?1",
        )?;
        let mut rows = stmt.query_map((session_id,), |r| r.get::<_, Option<String>>(0))?;
        match rows.next() {
            Some(row) => Ok(row?),
            None => Ok(None),
        }
    }

    /// List sessions for a specific workspace, newest-first.
    pub fn list_by_workspace(&self, workspace_id: &str, limit: usize) -> Result<Vec<SessionMeta>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, created_at, task, plan, result, title FROM sessions WHERE workspace_id = ?1 ORDER BY COALESCE(updated_at, created_at) DESC LIMIT ?2",
        )?;
        let rows = stmt.query_map(rusqlite::params![workspace_id, limit as i64], |r| {
            Ok(SessionMeta {
                id: r.get(0)?,
                created_at: r.get(1)?,
                task: r.get(2)?,
                plan: r.get(3)?,
                result: r.get(4)?,
                title: r.get(5)?,
            })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    /// Store per-session role assignments as JSON.
    pub fn set_role_assignments(&self, session_id: &str, assignments_json: &str) -> Result<()> {
        let now = chrono::Utc::now().to_rfc3339();
        self.conn.execute(
            "UPDATE sessions SET role_assignments = ?2, updated_at = ?3 WHERE id = ?1",
            (session_id, assignments_json, now.as_str()),
        )?;
        Ok(())
    }

    /// Fetch per-session role assignments JSON.
    pub fn get_role_assignments(&self, session_id: &str) -> Result<Option<String>> {
        let result: Option<String> = self
            .conn
            .query_row(
                "SELECT role_assignments FROM sessions WHERE id = ?1",
                (session_id,),
                |r| r.get(0),
            )
            .ok()
            .flatten();
        Ok(result)
    }

    /// Update session title.
    pub fn set_title(&self, session_id: &str, title: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE sessions SET title = ?2 WHERE id = ?1",
            (session_id, title),
        )?;
        Ok(())
    }

    pub fn record_run(&self, session_id: &str, task: &str, plan: &str, result: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE sessions SET task = ?2, plan = ?3, result = ?4 WHERE id = ?1",
            (session_id, task, plan, result),
        )?;
        Ok(())
    }

    pub fn add_message(&self, session_id: &str, role: &str, content: &str) -> Result<()> {
        let now = chrono::Utc::now().to_rfc3339();
        self.conn.execute(
            "INSERT INTO messages(session_id, role, content, ts) VALUES (?1, ?2, ?3, ?4)",
            (session_id, role, content, now),
        )?;
        Ok(())
    }

    /// Return the auto-assigned rowid after a raw `INSERT INTO messages`.
    /// Caller must have just executed the insert.
    pub fn last_message_id(&self) -> Result<i64> {
        Ok(self.conn.last_insert_rowid())
    }

    /// Persist a message with optional tool-call payload. Used by the Executor to record
    /// assistant tool-call messages and tool-result messages so `--resume` can restore the
    /// full conversation transcript (BUG-P1-05 regression).
    pub fn add_message_full(
        &self,
        session_id: &str,
        role: &str,
        content: &str,
        tool_calls: Option<&[serde_json::Value]>,
        tool_call_id: Option<&str>,
    ) -> Result<()> {
        let now = chrono::Utc::now().to_rfc3339();
        let tc_json = tool_calls
            .map(|v| serde_json::to_string(v).unwrap_or_default());
        self.conn.execute(
            "INSERT INTO messages(session_id, role, content, ts, tool_calls, tool_call_id) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            (
                session_id,
                role,
                content,
                now.as_str(),
                tc_json,
                tool_call_id,
            ),
        )?;
        Ok(())
    }

    pub fn add_tool_call(&self, session_id: &str, tool: &str, args: &str, output: &str) -> Result<()> {
        let now = chrono::Utc::now().to_rfc3339();
        self.conn.execute(
            "INSERT INTO tool_calls(session_id, tool, args, output, ts) VALUES (?1, ?2, ?3, ?4, ?5)",
            (session_id, tool, args, output, now.as_str()),
        )?;
        Ok(())
    }

    /// List sessions newest-first (id, created_at). Used by `/sessions`.
    pub fn list(&self, limit: usize) -> Result<Vec<SessionMeta>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, created_at, task, plan, result, title FROM sessions ORDER BY created_at DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map((limit as i64,), |r| {
            Ok(SessionMeta {
                id: r.get(0)?,
                created_at: r.get(1)?,
                task: r.get(2)?,
                plan: r.get(3)?,
                result: r.get(4)?,
                title: r.get(5)?,
            })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    /// Fetch a single session's metadata by id (used for resume).
    pub fn get(&self, session_id: &str) -> Result<Option<SessionMeta>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, created_at, task, plan, result, title FROM sessions WHERE id = ?1",
        )?;
        let mut rows = stmt.query_map((session_id,), |r| {
            Ok(SessionMeta {
                id: r.get(0)?,
                created_at: r.get(1)?,
                task: r.get(2)?,
                plan: r.get(3)?,
                result: r.get(4)?,
                title: r.get(5)?,
            })
        })?;
        match rows.next() {
            Some(row) => Ok(Some(row?)),
            None => Ok(None),
        }
    }

    /// Record a before-state snapshot so a write can be rolled back (spec §15).
    pub fn add_checkpoint(
        &self,
        session_id: &str,
        tool: &str,
        path: &str,
        before_content: Option<&str>,
    ) -> Result<()> {
        let now = chrono::Utc::now().to_rfc3339();
        self.conn.execute(
            "INSERT INTO checkpoints(session_id, tool, path, before_content, ts) VALUES (?1, ?2, ?3, ?4, ?5)",
            (session_id, tool, path, before_content, now.as_str()),
        )?;
        Ok(())
    }

    /// Return the most recent checkpoint for a session (newest first).
    pub fn last_checkpoint(&self, session_id: &str) -> Result<Option<Checkpoint>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, tool, path, before_content, ts FROM checkpoints \
             WHERE session_id = ?1 ORDER BY id DESC LIMIT 1",
        )?;
        let mut rows = stmt.query_map((session_id,), |r| {
            Ok(Checkpoint {
                id: r.get(0)?,
                tool: r.get(1)?,
                path: r.get(2)?,
                before_content: r.get(3)?,
                ts: r.get(4)?,
            })
        })?;
        match rows.next() {
            Some(row) => Ok(Some(row?)),
            None => Ok(None),
        }
    }

    /// Persist an arbitrary key/value blob for a session (used for the engineering model).
    pub fn set_kv(&self, session_id: &str, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO kv(session_id, key, value) VALUES (?1, ?2, ?3) \
             ON CONFLICT(session_id, key) DO UPDATE SET value = ?3",
            (session_id, key, value),
        )?;
        Ok(())
    }

    /// Read a previously stored key/value blob, if any.
    pub fn get_kv(&self, session_id: &str, key: &str) -> Result<Option<String>> {
        let mut stmt = self.conn.prepare("SELECT value FROM kv WHERE session_id = ?1 AND key = ?2")?;
        let mut rows = stmt.query_map((session_id, key), |r| r.get::<_, String>(0))?;
        match rows.next() {
            Some(row) => Ok(Some(row?)),
            None => Ok(None),
        }
    }

    /// Return the most recent N messages for a session, oldest-first, used by `--resume`
    /// to seed the Executor's `messages` array (BUG-P1-05 regression: previously resume
    /// reloaded engineering state but the conversation transcript was lost).
    pub fn get_messages(&self, session_id: &str, limit: usize) -> Result<Vec<MessageRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, role, content, tool_calls, tool_call_id FROM messages \
             WHERE session_id = ?1 ORDER BY id DESC LIMIT ?2",
        )?;
        let rows = stmt.query_map((session_id, limit as i64), |r| {
            let tool_calls_json: Option<String> = r.get(3)?;
            let tool_calls = tool_calls_json
                .as_deref()
                .and_then(|s| serde_json::from_str::<Vec<serde_json::Value>>(s).ok());
            Ok(MessageRow {
                id: r.get(0)?,
                role: r.get(1)?,
                content: r.get(2)?,
                tool_calls,
                tool_call_id: r.get(4)?,
            })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        // We selected DESC for `LIMIT N recent`; flip to chronological order.
        out.reverse();
        Ok(out)
    }

    /// Record a trace event (spec §34 / Phase 6): a point-in-time record of what an agent or
    /// the loop did, for debugging, replay, and audit. `parent` links a child event to its
    /// parent (e.g. a verification agent run to the loop iteration that spawned it).
    pub fn record_trace(
        &self,
        session_id: &str,
        kind: &str,
        agent: &str,
        parent: Option<&str>,
        summary: &str,
        payload: &str,
    ) -> Result<()> {
        let now = chrono::Utc::now().to_rfc3339();
        self.conn.execute(
            "INSERT INTO traces(session_id, ts, kind, agent, parent, summary, payload) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            (session_id, now.as_str(), kind, agent, parent, summary, payload),
        )?;
        Ok(())
    }

    /// List traces newest-first for a session.
    pub fn list_traces(&self, session_id: &str, limit: usize) -> Result<Vec<Trace>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, ts, kind, agent, parent, summary, payload FROM traces \
             WHERE session_id = ?1 ORDER BY id DESC LIMIT ?2",
        )?;
        let rows = stmt.query_map((session_id, limit as i64), |r| {
            Ok(Trace {
                id: r.get(0)?,
                ts: r.get(1)?,
                kind: r.get(2)?,
                agent: r.get(3)?,
                parent: r.get(4)?,
                summary: r.get(5)?,
                payload: r.get(6)?,
            })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }
}

#[derive(Debug, Clone)]
pub struct Trace {
    pub id: i64,
    pub ts: String,
    pub kind: String,
    pub agent: String,
    pub parent: Option<String>,
    pub summary: String,
    pub payload: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn traces_record_and_list() {
        let dir = std::env::temp_dir().join(format!("aether-trace-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("sessions.db");
        let _ = std::fs::remove_file(&path);
        let store = SessionStore::open(&path).unwrap();
        let sid = store.new_session().unwrap();
        store.record_trace(&sid, "plan", "controller", None, "wrote plan", "").unwrap();
        store.record_trace(&sid, "verify", "tester", None, "tests ok", "").unwrap();
        let traces = store.list_traces(&sid, 10).unwrap();
        assert_eq!(traces.len(), 2);
        // newest-first
        assert_eq!(traces[0].kind, "verify");
        let _ = std::fs::remove_file(&path);
        let _ = std::io::stdout().flush();
    }

    fn tmp_store(tag: &str) -> (std::path::PathBuf, std::sync::Arc<SessionStore>) {
        let dir = std::env::temp_dir().join(format!("aether-sess-{tag}-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("sessions.db");
        let _ = std::fs::remove_file(&path);
        let store = SessionStore::open(&path).unwrap();
        (path, store)
    }

    #[test]
    fn messages_roundtrip_chronological() {
        let (path, store) = tmp_store("msg");
        let sid = store.new_session().unwrap();
        store.add_message(&sid, "user", "first").unwrap();
        store.add_message(&sid, "assistant", "second").unwrap();
        store.add_message(&sid, "user", "third").unwrap();
        let msgs = store.get_messages(&sid, 10).unwrap();
        assert_eq!(msgs.len(), 3);
        assert_eq!(msgs[0].content, "first");
        assert_eq!(msgs[1].content, "second");
        assert_eq!(msgs[2].content, "third");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn add_message_full_persists_tool_calls() {
        let (path, store) = tmp_store("msgfull");
        let sid = store.new_session().unwrap();
        let tcs = vec![serde_json::json!({"id":"call_1","type":"function","function":{"name":"read_file","arguments":"{}"}})];
        store.add_message_full(&sid, "assistant", "", Some(&tcs), None).unwrap();
        store.add_message_full(&sid, "tool", "[read_file]\nhi", None, Some("call_1")).unwrap();
        let msgs = store.get_messages(&sid, 10).unwrap();
        assert_eq!(msgs.len(), 2);
        assert!(msgs[0].tool_calls.is_some());
        assert_eq!(msgs[1].tool_call_id.as_deref(), Some("call_1"));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn kv_set_get_roundtrip_and_overwrite() {
        let (path, store) = tmp_store("kv");
        let sid = store.new_session().unwrap();
        assert_eq!(store.get_kv(&sid, "engineering").unwrap(), None);
        store.set_kv(&sid, "engineering", r#"{"goal":"x"}"#).unwrap();
        assert_eq!(store.get_kv(&sid, "engineering").unwrap().as_deref(), Some(r#"{"goal":"x"}"#));
        // Overwrite (upsert).
        store.set_kv(&sid, "engineering", r#"{"goal":"y"}"#).unwrap();
        assert_eq!(store.get_kv(&sid, "engineering").unwrap().as_deref(), Some(r#"{"goal":"y"}"#));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn checkpoint_roundtrip() {
        let (path, store) = tmp_store("ckpt");
        let sid = store.new_session().unwrap();
        store.add_checkpoint(&sid, "write_file", "foo.rs", Some("old content")).unwrap();
        let cp = store.last_checkpoint(&sid).unwrap().expect("checkpoint present");
        assert_eq!(cp.path, "foo.rs");
        assert_eq!(cp.before_content.as_deref(), Some("old content"));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn message_parts_roundtrip() {
        let (path, store) = tmp_store("parts");
        let sid = store.new_session().unwrap();
        let parts = vec![
            MessagePart::Reasoning { text: "thinking...".into() },
            MessagePart::Text { text: "answer".into() },
            MessagePart::Tool { tool_call_id: "c1".into(), name: "read_file".into(), args: serde_json::json!({"path":"a.txt"}), output: Some("hi".into()), status: "ok".into() },
        ];
        let id = store.add_message_parts(&sid, "assistant", &parts).unwrap();
        let got = store.get_message_parts(id).unwrap().unwrap();
        assert_eq!(got, parts);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn truncate_after_drops_messages_and_parts() {
        let (path, store) = tmp_store("trunc");
        let sid = store.new_session().unwrap();
        store.add_message(&sid, "user", "1").unwrap();
        let id1 = store.last_message_id().unwrap();
        store.add_message(&sid, "assistant", "2").unwrap();
        let id2 = store.last_message_id().unwrap();
        store.add_message(&sid, "tool", "3").unwrap();
        let id3 = store.last_message_id().unwrap();
        store.add_message_parts(&sid, "assistant", &[MessagePart::Text { text: "x".into() }]).unwrap();
        let dropped = store.truncate_after(&sid, id2).unwrap();
        assert_eq!(dropped, 2);
        let mut stmt = store.conn.prepare("SELECT id FROM messages WHERE session_id = ?1 ORDER BY id").unwrap();
        let remaining: Vec<i64> = stmt.query_map(rusqlite::params![sid], |r| r.get(0))
            .unwrap()
            .map(|x| x.unwrap())
            .collect();
        assert_eq!(remaining, vec![id1, id2]);
        // id3 was deleted; querying it should be gone
        let exists: bool = store.conn.query_row(
            "SELECT 1 FROM messages WHERE id = ?1", rusqlite::params![id3], |_| Ok(true),
        ).optional().unwrap_or(Some(false)).unwrap_or(false);
        assert!(!exists, "id3 should be deleted");
        let _ = std::fs::remove_file(&path);
    }
}

#[derive(Debug, Clone)]
pub struct Checkpoint {
    pub id: i64,
    pub tool: String,
    pub path: String,
    pub before_content: Option<String>,
    pub ts: String,
}
