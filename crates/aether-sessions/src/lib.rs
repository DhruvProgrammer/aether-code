//! Session / task / checkpoint store (spec §21, Phase 2). SQLite-backed, sync.
//! Deliberately separate from the `aether-mind` graph/vector store: these are
//! relational, append-only logs, not semantic memory.

use anyhow::Result;
use rusqlite::Connection;
use std::path::Path;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct SessionMeta {
    pub id: String,
    pub created_at: String,
    pub task: Option<String>,
    pub plan: Option<String>,
    pub result: Option<String>,
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
                kind TEXT NOT NULL,
                agent TEXT NOT NULL,
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
        Ok(Arc::new(Self { conn }))
    }

    pub fn new_session(&self) -> Result<String> {
        let id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().to_rfc3339();
        self.conn
            .execute("INSERT INTO sessions(id, created_at) VALUES (?1, ?2)", (id.as_str(), now.as_str()))?;
        Ok(id)
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
            (session_id, role, content, now.as_str()),
        )?;
        Ok(())
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
            "SELECT id, created_at, task, plan, result FROM sessions ORDER BY created_at DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map((limit as i64,), |r| {
            Ok(SessionMeta {
                id: r.get(0)?,
                created_at: r.get(1)?,
                task: r.get(2)?,
                plan: r.get(3)?,
                result: r.get(4)?,
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
            "SELECT id, created_at, task, plan, result FROM sessions WHERE id = ?1",
        )?;
        let mut rows = stmt.query_map((session_id,), |r| {
            Ok(SessionMeta {
                id: r.get(0)?,
                created_at: r.get(1)?,
                task: r.get(2)?,
                plan: r.get(3)?,
                result: r.get(4)?,
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
}

#[derive(Debug, Clone)]
pub struct Checkpoint {
    pub id: i64,
    pub tool: String,
    pub path: String,
    pub before_content: Option<String>,
    pub ts: String,
}
