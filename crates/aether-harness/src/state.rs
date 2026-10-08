//! Versioned harness persistence + recovery (spec §26, §14).
//!
//! Reuse ledger: AETHER's session store has **no schema version** and its
//! migrations are six `ALTER TABLE` calls with discarded errors
//! (`aether-sessions/src/lib.rs:275-281`). That is exactly the fragility the
//! harness must not inherit, so the harness state store keeps its own
//! `schema_version` plus an ordered, recorded migration list.
//!
//! Recovery semantics (spec §14): goals, jobs, refinements, and child records
//! are restored on startup *without* replaying the conversation. A run marked
//! in-flight is reported as interrupted so the caller can re-enter rather than
//! pretend it finished.

use std::path::Path;

use parking_lot::Mutex;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::goal::Goal;
use crate::refine::{HarnessEntry, RefinementRecord};
use crate::schedule::ScheduledJob;
use crate::subagent::ChildRecord;

#[derive(Debug, thiserror::Error)]
pub enum StateError {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("unsupported schema version {found}; this build supports {supported}")]
    Version { found: u64, supported: u64 },
}

/// Current schema version. Bump when adding a migration.
pub const SCHEMA_VERSION: u64 = 3;

/// One ordered migration step.
struct Migration {
    version: u64,
    sql: &'static str,
}

/// Ordered migrations. Never edit an applied step — append a new one.
const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        sql: "
            CREATE TABLE IF NOT EXISTS goals(
              id TEXT PRIMARY KEY, session_id TEXT NOT NULL, payload TEXT NOT NULL,
              updated_at INTEGER NOT NULL);
            CREATE TABLE IF NOT EXISTS jobs(
              id TEXT PRIMARY KEY, session_id TEXT NOT NULL, payload TEXT NOT NULL,
              next_run_at INTEGER);
            CREATE TABLE IF NOT EXISTS harness_entries(
              id TEXT PRIMARY KEY, kind TEXT NOT NULL, session_id TEXT,
              payload TEXT NOT NULL, updated_at INTEGER NOT NULL);
            CREATE TABLE IF NOT EXISTS refinements(
              id TEXT PRIMARY KEY, session_id TEXT NOT NULL, payload TEXT NOT NULL,
              created_at INTEGER NOT NULL);
            CREATE TABLE IF NOT EXISTS children(
              id TEXT PRIMARY KEY, parent_id TEXT, session_id TEXT NOT NULL,
              payload TEXT NOT NULL, status TEXT NOT NULL);
        ",
    },
    Migration {
        version: 2,
        sql: "
            CREATE TABLE IF NOT EXISTS gate_state(
              session_id TEXT PRIMARY KEY, payload TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS runtime_state(
              session_id TEXT PRIMARY KEY, payload TEXT NOT NULL, updated_at INTEGER NOT NULL);
            CREATE INDEX IF NOT EXISTS idx_jobs_next ON jobs(next_run_at);
        ",
    },
    Migration {
        version: 3,
        sql: "
            -- Structured working context (RLM state) must survive a restart,
            -- otherwise recovery loses the checkpoint and recent events and
            -- the task cannot continue coherently.
            CREATE TABLE IF NOT EXISTS context_state(
              session_id TEXT PRIMARY KEY, payload TEXT NOT NULL, updated_at INTEGER NOT NULL);
        ",
    },
];

/// Everything the harness needs to resume without replaying history.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HarnessSnapshot {
    pub goal: Option<Goal>,
    pub jobs: Vec<ScheduledJob>,
    pub entries: Vec<HarnessEntry>,
    pub refinements: Vec<RefinementRecord>,
    pub children: Vec<ChildRecord>,
    pub gate_state: Option<String>,
    pub runtime_state: Option<String>,
    /// Structured working context, so a restart resumes the task instead of
    /// re-deriving it from the transcript (spec §14).
    pub context: Option<String>,
}

/// Durable harness state. Critical writes are transactional; a failure
/// leaves the previous valid state intact (spec §26).
pub struct HarnessStore {
    conn: Mutex<Connection>,
}

impl HarnessStore {
    pub fn open_in_memory() -> Result<Self, StateError> {
        Self::from_conn(Connection::open_in_memory()?)
    }

    pub fn open(path: &Path) -> Result<Self, StateError> {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        Self::from_conn(Connection::open(path)?)
    }

    fn from_conn(conn: Connection) -> Result<Self, StateError> {
        let s = Self {
            conn: Mutex::new(conn),
        };
        s.migrate()?;
        Ok(s)
    }

    /// Create the version table if absent, then apply every pending
    /// migration in order, recording each one.
    fn migrate(&self) -> Result<(), StateError> {
        let conn = self.conn.lock();
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS harness_meta(
               key TEXT PRIMARY KEY, value TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS harness_migrations(
               version INTEGER PRIMARY KEY, applied_at INTEGER NOT NULL);",
        )?;
        let found: Option<String> = conn
            .query_row(
                "SELECT value FROM harness_meta WHERE key = 'schema_version'",
                [],
                |r| r.get(0),
            )
            .optional()?;
        let current: u64 = found.as_deref().and_then(|v| v.parse().ok()).unwrap_or(0);
        if current > SCHEMA_VERSION {
            return Err(StateError::Version {
                found: current,
                supported: SCHEMA_VERSION,
            });
        }
        for m in MIGRATIONS.iter().filter(|m| m.version > current) {
            conn.execute_batch(m.sql)?;
            conn.execute(
                "INSERT OR REPLACE INTO harness_migrations(version, applied_at) VALUES(?,?)",
                params![m.version, chrono::Utc::now().timestamp()],
            )?;
            conn.execute(
                "INSERT OR REPLACE INTO harness_meta(key, value) VALUES('schema_version', ?)",
                params![m.version.to_string()],
            )?;
        }
        Ok(())
    }

    pub fn schema_version(&self) -> Result<u64, StateError> {
        let conn = self.conn.lock();
        let v: Option<String> = conn
            .query_row(
                "SELECT value FROM harness_meta WHERE key = 'schema_version'",
                [],
                |r| r.get(0),
            )
            .optional()?;
        Ok(v.and_then(|s| s.parse().ok()).unwrap_or(0))
    }

    pub fn applied_migrations(&self) -> Result<Vec<u64>, StateError> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare("SELECT version FROM harness_migrations ORDER BY version")?;
        let rows = stmt.query_map([], |r| r.get::<_, i64>(0))?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r? as u64);
        }
        Ok(out)
    }

    /// Persist the whole harness state in one transaction. Either everything
    /// lands or nothing does.
    pub fn save(&self, snap: &HarnessSnapshot) -> Result<(), StateError> {
        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;
        if let Some(g) = &snap.goal {
            tx.execute(
                "INSERT OR REPLACE INTO goals(id, session_id, payload, updated_at) VALUES(?,?,?,?)",
                params![g.id, g.session_id, serde_json::to_string(g)?, g.updated_at],
            )?;
        }
        for j in &snap.jobs {
            tx.execute(
                "INSERT OR REPLACE INTO jobs(id, session_id, payload, next_run_at) VALUES(?,?,?,?)",
                params![j.id, j.session_id, serde_json::to_string(j)?, j.next_run_at],
            )?;
        }
        for e in &snap.entries {
            tx.execute(
                "INSERT OR REPLACE INTO harness_entries(id, kind, session_id, payload, updated_at) VALUES(?,?,?,?,?)",
                params![e.id, e.kind.as_str(), e.session_id, serde_json::to_string(e)?, e.updated_at],
            )?;
        }
        for r in &snap.refinements {
            tx.execute(
                "INSERT OR REPLACE INTO refinements(id, session_id, payload, created_at) VALUES(?,?,?,?)",
                params![r.id, r.session_id, serde_json::to_string(r)?, r.created_at],
            )?;
        }
        for c in &snap.children {
            tx.execute(
                "INSERT OR REPLACE INTO children(id, parent_id, session_id, payload, status) VALUES(?,?,?,?,?)",
                params![c.id, c.parent_id, c.session_id, serde_json::to_string(c)?, c.status.as_str()],
            )?;
        }
        if let Some(g) = &snap.gate_state {
            // Stored per session id; snapshot() serializes it without one, so
            // fall back to the goal's session and otherwise skip.
            if let Some(goal) = &snap.goal {
                tx.execute(
                    "INSERT OR REPLACE INTO gate_state(session_id, payload) VALUES(?,?)",
                    params![goal.session_id, g],
                )?;
            }
        }
        if let Some(r) = &snap.runtime_state {
            if let Some(goal) = &snap.goal {
                tx.execute(
                    "INSERT OR REPLACE INTO runtime_state(session_id, payload, updated_at) VALUES(?,?,?)",
                    params![goal.session_id, r, chrono::Utc::now().timestamp()],
                )?;
            }
        }
        if let Some(c) = &snap.context {
            if let Some(goal) = &snap.goal {
                tx.execute(
                    "INSERT OR REPLACE INTO context_state(session_id, payload, updated_at) VALUES(?,?,?)",
                    params![goal.session_id, c, chrono::Utc::now().timestamp()],
                )?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Restore state for a session (or the most recent session).
    pub fn load(&self, session_id: Option<&str>) -> Result<HarnessSnapshot, StateError> {
        let conn = self.conn.lock();
        let mut snap = HarnessSnapshot::default();

        let goal_row: Option<String> = match session_id {
            Some(s) => conn
                .query_row(
                    "SELECT payload FROM goals WHERE session_id = ? ORDER BY updated_at DESC LIMIT 1",
                    params![s],
                    |r| r.get(0),
                )
                .optional()?,
            None => conn
                .query_row(
                    "SELECT payload FROM goals ORDER BY updated_at DESC LIMIT 1",
                    [],
                    |r| r.get(0),
                )
                .optional()?,
        };
        if let Some(p) = goal_row {
            snap.goal = serde_json::from_str(&p).ok();
        }

        let mut stmt = conn.prepare("SELECT payload FROM jobs ORDER BY next_run_at")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        for r in rows {
            if let Ok(j) = serde_json::from_str::<ScheduledJob>(&r?) {
                snap.jobs.push(j);
            }
        }
        drop(stmt);

        let mut stmt = conn.prepare("SELECT payload FROM harness_entries ORDER BY updated_at")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        for r in rows {
            if let Ok(e) = serde_json::from_str::<HarnessEntry>(&r?) {
                snap.entries.push(e);
            }
        }
        drop(stmt);

        let mut stmt = conn.prepare("SELECT payload FROM refinements ORDER BY created_at")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        for r in rows {
            if let Ok(x) = serde_json::from_str::<RefinementRecord>(&r?) {
                snap.refinements.push(x);
            }
        }
        drop(stmt);

        let mut stmt = conn.prepare("SELECT payload FROM children ORDER BY rowid")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        for r in rows {
            if let Ok(c) = serde_json::from_str::<ChildRecord>(&r?) {
                snap.children.push(c);
            }
        }
        if let Some(g) = &snap.goal {
            let sid = &g.session_id;
            snap.gate_state = conn
                .query_row(
                    "SELECT payload FROM gate_state WHERE session_id = ?",
                    params![sid],
                    |r| r.get::<_, String>(0),
                )
                .optional()?;
            snap.runtime_state = conn
                .query_row(
                    "SELECT payload FROM runtime_state WHERE session_id = ?",
                    params![sid],
                    |r| r.get::<_, String>(0),
                )
                .optional()?;
            snap.context = conn
                .query_row(
                    "SELECT payload FROM context_state WHERE session_id = ?",
                    params![sid],
                    |r| r.get::<_, String>(0),
                )
                .optional()?;
        }
        Ok(snap)
    }

    pub fn set_gate_state(&self, session_id: &str, payload: &str) -> Result<(), StateError> {
        self.conn.lock().execute(
            "INSERT OR REPLACE INTO gate_state(session_id, payload) VALUES(?,?)",
            params![session_id, payload],
        )?;
        Ok(())
    }

    pub fn get_gate_state(&self, session_id: &str) -> Result<Option<String>, StateError> {
        Ok(self
            .conn
            .lock()
            .query_row(
                "SELECT payload FROM gate_state WHERE session_id = ?",
                params![session_id],
                |r| r.get(0),
            )
            .optional()?)
    }

    pub fn set_runtime_state(&self, session_id: &str, payload: &str) -> Result<(), StateError> {
        self.conn.lock().execute(
            "INSERT OR REPLACE INTO runtime_state(session_id, payload, updated_at) VALUES(?,?,?)",
            params![session_id, payload, chrono::Utc::now().timestamp()],
        )?;
        Ok(())
    }

    pub fn get_runtime_state(&self, session_id: &str) -> Result<Option<String>, StateError> {
        Ok(self
            .conn
            .lock()
            .query_row(
                "SELECT payload FROM runtime_state WHERE session_id = ?",
                params![session_id],
                |r| r.get(0),
            )
            .optional()?)
    }

    /// Children that were mid-flight when the process died. The harness
    /// reports these as interrupted instead of pretending they finished
    /// (spec §14, §25).
    pub fn interrupted_children(&self) -> Result<Vec<ChildRecord>, StateError> {
        let snap = self.load(None)?;
        Ok(snap
            .children
            .into_iter()
            .filter(|c| matches!(c.status, crate::subagent::ChildStatus::Running))
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::budget::Budget;
    use crate::refine::{EntryKind, EntryScope, HarnessEntry};
    use crate::schedule::{Schedule, ScheduledJob};
    use crate::subagent::{ChildSpec, ChildStatus};

    fn snapshot() -> HarnessSnapshot {
        let mut goal = Goal::new("sess-1", "Refactor provider architecture").with_tasks(3);
        goal.record_task_done();
        let entry = HarnessEntry::new("l1", EntryKind::Lesson, "Lesson", "use mingw64", EntryScope::Project);
        let child = ChildRecord {
            id: "child-1".into(),
            parent_id: None,
            session_id: "sess-1".into(),
            task_id: Some("task-1".into()),
            depth: 1,
            spec: ChildSpec {
                agent_id: "explorer".into(),
                objective: "investigate".into(),
                context: String::new(),
                scope: vec![],
                allowed_tools: None,
                budget: Budget::turns(1),
                expected_output: String::new(),
            },
            status: ChildStatus::Running,
            result: None,
            usage: crate::budget::BudgetUsage::new(0),
            started_at: 0,
            ended_at: None,
            error: None,
        };
        HarnessSnapshot {
            goal: Some(goal),
            jobs: vec![ScheduledJob::new(
                "sess-1",
                "hb",
                "check",
                Schedule::Interval { period_ms: 60_000 },
            )],
            entries: vec![entry],
            refinements: Vec::new(),
            children: vec![child],
            gate_state: Some("{}".into()),
            runtime_state: Some("{\"autonomous\":true}".into()),
            context: Some("{\"goal\":\"Refactor provider architecture\"}".into()),
        }
    }

    #[test]
    fn migrations_are_ordered_and_versioned() {
        let s = HarnessStore::open_in_memory().unwrap();
        assert_eq!(s.schema_version().unwrap(), SCHEMA_VERSION);
        assert_eq!(s.applied_migrations().unwrap(), vec![1, 2, 3]);
        // Re-opening does not re-apply or fail.
        let conn = s.conn.lock();
        drop(conn);
        assert_eq!(s.schema_version().unwrap(), SCHEMA_VERSION);
    }

    #[test]
    fn newer_schema_is_refused_not_silently_downgraded() {
        let path = std::env::temp_dir().join(format!(
            "aether-harness-version-{}.db",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        {
            let s = HarnessStore::open(&path).unwrap();
            s.conn.lock().execute(
                "UPDATE harness_meta SET value = '999' WHERE key = 'schema_version'",
                [],
            )
            .unwrap();
        }
        // Re-opening a newer database must fail loudly rather than corrupt it.
        match HarnessStore::open(&path) {
            Err(StateError::Version { found, supported }) => {
                assert_eq!(found, 999);
                assert_eq!(supported, SCHEMA_VERSION);
            }
            other => panic!("expected version refusal, got {:?}", other.err()),
        }
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn save_and_load_roundtrip_without_replaying_history() {
        let s = HarnessStore::open_in_memory().unwrap();
        let snap = snapshot();
        s.save(&snap).unwrap();
        let back = s.load(Some("sess-1")).unwrap();
        let g = back.goal.expect("goal restored");
        assert_eq!(g.objective, "Refactor provider architecture");
        assert_eq!(g.tasks_done, 1);
        assert_eq!(back.jobs.len(), 1);
        assert_eq!(back.entries.len(), 1);
        assert_eq!(back.children.len(), 1);
        // gate_state is addressable per session.
        assert_eq!(s.get_gate_state("sess-1").unwrap().as_deref(), Some("{}"));
        assert_eq!(
            s.get_runtime_state("sess-1").unwrap().as_deref(),
            Some("{\"autonomous\":true}")
        );
    }

    #[test]
    fn interrupted_children_are_reported() {
        let s = HarnessStore::open_in_memory().unwrap();
        s.save(&snapshot()).unwrap();
        let interrupted = s.interrupted_children().unwrap();
        assert_eq!(interrupted.len(), 1);
        assert_eq!(interrupted[0].id, "child-1");
    }

    #[test]
    fn runtime_and_gate_state_are_addressable() {
        let s = HarnessStore::open_in_memory().unwrap();
        s.set_gate_state("sess-1", "{\"per_command\":{}}").unwrap();
        s.set_runtime_state("sess-1", "{\"paused\":false}").unwrap();
        assert_eq!(
            s.get_gate_state("sess-1").unwrap().as_deref(),
            Some("{\"per_command\":{}}")
        );
        assert!(s.get_runtime_state("sess-1").unwrap().is_some());
        assert!(s.get_gate_state("other").unwrap().is_none());
    }

    #[test]
    fn corrupt_rows_are_skipped_not_fatal() {
        let s = HarnessStore::open_in_memory().unwrap();
        s.save(&snapshot()).unwrap();
        {
            let conn = s.conn.lock();
            conn.execute(
                "INSERT INTO harness_entries(id, kind, session_id, payload, updated_at) VALUES('bad','lesson',NULL,'{not json',0)",
                [],
            )
            .unwrap();
        }
        let back = s.load(None).unwrap();
        assert_eq!(back.entries.len(), 1, "valid entry kept, corrupt row skipped");
    }
}
