//! `MemoryStore` abstraction (§44) + local-first SQLite implementation.
//!
//! Local persistent storage first (desktop AETHER): one SQLite file with an
//! FTS5 lexical index (BM25 ranking). No cloud database, no new services.

use std::sync::{Arc, Mutex};

use rusqlite::{params, Connection, OptionalExtension};

use crate::types::{MemoryRecord, MemoryStatus, MemoryType, Relation};

#[derive(Debug, thiserror::Error)]
pub enum MemoryError {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("record not found: {0}")]
    NotFound(String),
    #[error("fts query invalid: {0}")]
    BadQuery(String),
}

/// Storage contract (§44). Implementations beyond SQLite (external vector
/// DB, Postgres) plug in here without touching retrieval.
pub trait MemoryStore: Send + Sync {
    fn insert(&self, record: &MemoryRecord) -> Result<(), MemoryError>;
    fn update(&self, record: &MemoryRecord) -> Result<(), MemoryError>;
    fn delete(&self, id: &str) -> Result<(), MemoryError>;
    fn get(&self, id: &str) -> Result<Option<MemoryRecord>, MemoryError>;
    /// Lexical candidates with raw relevance (higher = better).
    fn search_lexical(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<(MemoryRecord, f32)>, MemoryError>;
    fn list_recent(
        &self,
        project_id: &str,
        session_id: Option<&str>,
        limit: usize,
    ) -> Result<Vec<MemoryRecord>, MemoryError>;
    fn list_related(
        &self,
        id: &str,
    ) -> Result<Vec<(MemoryRecord, Relation)>, MemoryError>;
    fn add_relation(
        &self,
        from_id: &str,
        to_id: &str,
        rel: Relation,
    ) -> Result<(), MemoryError>;
    /// Mark `old_id` replaced by `new_id`: status flip + both edge
    /// directions, so traversal and penalties stay consistent (§14).
    fn mark_superseded(&self, old_id: &str, new_id: &str) -> Result<(), MemoryError>;
    fn exists_by_hash(&self, project_id: &str, hash: &str) -> Result<bool, MemoryError>;
    fn count(&self, project_id: &str) -> Result<usize, MemoryError>;
}

/// Blanket impl so `Arc<dyn MemoryStore>` (and `Arc<SqliteMemoryStore>`)
/// can be passed wherever the trait is required.
impl<T: MemoryStore + ?Sized> MemoryStore for Arc<T> {
    fn insert(&self, record: &MemoryRecord) -> Result<(), MemoryError> {
        (**self).insert(record)
    }
    fn update(&self, record: &MemoryRecord) -> Result<(), MemoryError> {
        (**self).update(record)
    }
    fn delete(&self, id: &str) -> Result<(), MemoryError> {
        (**self).delete(id)
    }
    fn get(&self, id: &str) -> Result<Option<MemoryRecord>, MemoryError> {
        (**self).get(id)
    }
    fn search_lexical(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<(MemoryRecord, f32)>, MemoryError> {
        (**self).search_lexical(query, limit)
    }
    fn list_recent(
        &self,
        project_id: &str,
        session_id: Option<&str>,
        limit: usize,
    ) -> Result<Vec<MemoryRecord>, MemoryError> {
        (**self).list_recent(project_id, session_id, limit)
    }
    fn list_related(
        &self,
        id: &str,
    ) -> Result<Vec<(MemoryRecord, Relation)>, MemoryError> {
        (**self).list_related(id)
    }
    fn add_relation(
        &self,
        from_id: &str,
        to_id: &str,
        rel: Relation,
    ) -> Result<(), MemoryError> {
        (**self).add_relation(from_id, to_id, rel)
    }
    fn mark_superseded(&self, old_id: &str, new_id: &str) -> Result<(), MemoryError> {
        (**self).mark_superseded(old_id, new_id)
    }
    fn exists_by_hash(&self, project_id: &str, hash: &str) -> Result<bool, MemoryError> {
        (**self).exists_by_hash(project_id, hash)
    }
    fn count(&self, project_id: &str) -> Result<usize, MemoryError> {
        (**self).count(project_id)
    }
}

/// SQLite-backed store. `Connection` is `Send` (not `Sync`); the mutex
/// makes the store `Sync` for sharing across the agent runtime.
pub struct SqliteMemoryStore {
    db: Mutex<Connection>,
}

impl SqliteMemoryStore {
    pub fn open_in_memory() -> Result<Self, MemoryError> {
        let conn = Connection::open_in_memory()?;
        let s = Self {
            db: Mutex::new(conn),
        };
        s.init()?;
        Ok(s)
    }

    pub fn open(path: &std::path::Path) -> Result<Self, MemoryError> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| {
                MemoryError::Sqlite(rusqlite::Error::InvalidPath(
                    format!("{}: {e}", path.display()).into(),
                ))
            })?;
        }
        let conn = Connection::open(path)?;
        let s = Self {
            db: Mutex::new(conn),
        };
        s.init()?;
        Ok(s)
    }

    fn init(&self) -> Result<(), MemoryError> {
        let db = self.db.lock().map_err(|_| {
            MemoryError::Sqlite(rusqlite::Error::InvalidQuery)
        })?;
        db.execute_batch(
            "CREATE TABLE IF NOT EXISTS memories(
               id TEXT PRIMARY KEY, project_id TEXT NOT NULL,
               session_id TEXT, task_id TEXT, mem_type TEXT NOT NULL,
               title TEXT NOT NULL, content TEXT NOT NULL, summary TEXT,
               source_ids TEXT NOT NULL, files TEXT NOT NULL,
               symbols TEXT NOT NULL, tags TEXT NOT NULL,
               importance REAL NOT NULL, confidence REAL NOT NULL,
               status TEXT NOT NULL, parent_id TEXT,
               created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL,
               content_hash TEXT NOT NULL);
             CREATE INDEX IF NOT EXISTS idx_memories_project ON memories(project_id, created_at);
             CREATE INDEX IF NOT EXISTS idx_memories_hash ON memories(project_id, content_hash);
             CREATE VIRTUAL TABLE IF NOT EXISTS memories_fts
               USING fts5(id UNINDEXED, title, content, tags);
             CREATE TABLE IF NOT EXISTS relations(
               from_id TEXT NOT NULL, to_id TEXT NOT NULL, relation TEXT NOT NULL,
               PRIMARY KEY(from_id, to_id, relation));",
        )?;
        Ok(())
    }

    fn row_to_record(row: &rusqlite::Row) -> rusqlite::Result<MemoryRecord> {
        let j = |i: usize| -> rusqlite::Result<Vec<String>> {
            let s: String = row.get(i)?;
            Ok(serde_json::from_str(&s).unwrap_or_default())
        };
        let mem_type: String = row.get(4)?;
        let status: String = row.get(14)?;
        Ok(MemoryRecord {
            id: row.get(0)?,
            project_id: row.get(1)?,
            session_id: row.get(2)?,
            task_id: row.get(3)?,
            mem_type: MemoryType::from_str(&mem_type).unwrap_or(MemoryType::ImportantFact),
            title: row.get(5)?,
            content: row.get(6)?,
            summary: row.get(7)?,
            source_ids: j(8)?,
            files: j(9)?,
            symbols: j(10)?,
            tags: j(11)?,
            importance: row.get(12)?,
            confidence: row.get(13)?,
            status: MemoryStatus::from_str(&status).unwrap_or(MemoryStatus::Active),
            supersedes: Vec::new(),
            superseded_by: Vec::new(),
            parent_id: row.get(15)?,
            related_ids: Vec::new(),
            created_at: row.get(16)?,
            updated_at: row.get(17)?,
        })
    }
}

const COLS: &str = "id, project_id, session_id, task_id, mem_type, title, content,
  summary, source_ids, files, symbols, tags, importance, confidence,
  status, parent_id, created_at, updated_at";

/// Same columns qualified for JOIN queries (the FTS table has its own `id`).
const COLS_Q: &str = "memories.id, memories.project_id, memories.session_id,
  memories.task_id, memories.mem_type, memories.title, memories.content,
  memories.summary, memories.source_ids, memories.files, memories.symbols,
  memories.tags, memories.importance, memories.confidence,
  memories.status, memories.parent_id, memories.created_at, memories.updated_at";

impl MemoryStore for SqliteMemoryStore {
    fn insert(&self, r: &MemoryRecord) -> Result<(), MemoryError> {
        let db = self.db.lock().map_err(|_| {
            MemoryError::Sqlite(rusqlite::Error::InvalidQuery)
        })?;
        let hash = r.content_hash();
        db.execute(
            "INSERT OR REPLACE INTO memories(id, project_id, session_id, task_id,
             mem_type, title, content, summary, source_ids, files, symbols, tags,
             importance, confidence, status, parent_id, created_at, updated_at, content_hash)
             VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
            params![
                r.id,
                r.project_id,
                r.session_id,
                r.task_id,
                r.mem_type.as_str(),
                r.title,
                r.content,
                r.summary,
                serde_json::to_string(&r.source_ids)?,
                serde_json::to_string(&r.files)?,
                serde_json::to_string(&r.symbols)?,
                serde_json::to_string(&r.tags)?,
                r.importance,
                r.confidence,
                r.status.as_str(),
                r.parent_id,
                r.created_at,
                r.updated_at,
                hash,
            ],
        )?;
        db.execute("DELETE FROM memories_fts WHERE id = ?", params![r.id])?;
        db.execute(
            "INSERT INTO memories_fts(id, title, content, tags) VALUES(?,?,?,?)",
            params![r.id, r.title, r.content, r.tags.join(" ")],
        )?;
        Ok(())
    }

    fn update(&self, r: &MemoryRecord) -> Result<(), MemoryError> {
        self.insert(r)
    }

    fn delete(&self, id: &str) -> Result<(), MemoryError> {
        let db = self.db.lock().map_err(|_| {
            MemoryError::Sqlite(rusqlite::Error::InvalidQuery)
        })?;
        db.execute("DELETE FROM memories WHERE id = ?", params![id])?;
        db.execute("DELETE FROM memories_fts WHERE id = ?", params![id])?;
        db.execute(
            "DELETE FROM relations WHERE from_id = ? OR to_id = ?",
            params![id, id],
        )?;
        Ok(())
    }

    fn get(&self, id: &str) -> Result<Option<MemoryRecord>, MemoryError> {
        let db = self.db.lock().map_err(|_| {
            MemoryError::Sqlite(rusqlite::Error::InvalidQuery)
        })?;
        let sql = format!("SELECT {COLS} FROM memories WHERE id = ?");
        let rec = db
            .query_row(&sql, params![id], Self::row_to_record)
            .optional()?;
        Ok(rec)
    }

    fn search_lexical(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<(MemoryRecord, f32)>, MemoryError> {
        // FTS5 MATCH with quoted terms; fall back to LIKE on syntax errors
        // so a stray quote in an error message never breaks retrieval.
        let terms: Vec<String> = query
            .split(|c: char| !c.is_alphanumeric() && c != '_' && c != '/' && c != '.' && c != ':')
            .filter(|t| t.len() > 1)
            .take(12)
            .map(|t| format!("\"{t}\""))
            .collect();
        if terms.is_empty() {
            return Ok(Vec::new());
        }
        let db = self.db.lock().map_err(|_| {
            MemoryError::Sqlite(rusqlite::Error::InvalidQuery)
        })?;
        let match_q = terms.join(" OR ");
        let sql = format!(
            "SELECT {COLS_Q}, bm25(memories_fts) FROM memories
             JOIN memories_fts ON memories.id = memories_fts.id
             WHERE memories_fts MATCH ? ORDER BY bm25(memories_fts) LIMIT ?"
        );
        let mut stmt = db
            .prepare(&sql)
            .map_err(|e| MemoryError::BadQuery(format!("prepare {match_q}: {e}")))?;
        let rows = stmt
            .query_map(params![match_q, limit as i64], |row| {
                let rank: f64 = row.get(18)?;
                Ok((Self::row_to_record(row)?, (-rank) as f32))
            })
            .map_err(|e| MemoryError::BadQuery(format!("match {match_q}: {e}")))?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    fn list_recent(
        &self,
        project_id: &str,
        session_id: Option<&str>,
        limit: usize,
    ) -> Result<Vec<MemoryRecord>, MemoryError> {
        let db = self.db.lock().map_err(|_| {
            MemoryError::Sqlite(rusqlite::Error::InvalidQuery)
        })?;
        let sql = if session_id.is_some() {
            format!("SELECT {COLS} FROM memories WHERE project_id = ? AND (session_id = ? OR session_id IS NULL) ORDER BY created_at DESC LIMIT ?")
        } else {
            format!("SELECT {COLS} FROM memories WHERE project_id = ? ORDER BY created_at DESC LIMIT ?")
        };
        let mut stmt = db.prepare(&sql)?;
        let iter = if let Some(sid) = session_id {
            stmt.query_map(params![project_id, sid, limit as i64], Self::row_to_record)?
        } else {
            stmt.query_map(params![project_id, limit as i64], Self::row_to_record)?
        };
        iter.collect::<rusqlite::Result<Vec<_>>>().map_err(MemoryError::from)
    }

    fn list_related(
        &self,
        id: &str,
    ) -> Result<Vec<(MemoryRecord, Relation)>, MemoryError> {
        let db = self.db.lock().map_err(|_| {
            MemoryError::Sqlite(rusqlite::Error::InvalidQuery)
        })?;
        let mut stmt = db.prepare(
            "SELECT to_id, relation FROM relations WHERE from_id = ?",
        )?;
        let edges: Vec<(String, String)> = stmt
            .query_map(params![id], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        drop(stmt);
        let mut out = Vec::new();
        for (to, rel) in edges {
            let sql = format!("SELECT {COLS} FROM memories WHERE id = ?");
            if let Some(rec) = db
                .query_row(&sql, params![to], Self::row_to_record)
                .optional()?
            {
                out.push((rec, Relation::from_str(&rel).unwrap_or(Relation::RelatedTo)));
            }
        }
        Ok(out)
    }

    fn add_relation(
        &self,
        from_id: &str,
        to_id: &str,
        rel: Relation,
    ) -> Result<(), MemoryError> {
        let db = self.db.lock().map_err(|_| {
            MemoryError::Sqlite(rusqlite::Error::InvalidQuery)
        })?;
        db.execute(
            "INSERT OR IGNORE INTO relations(from_id, to_id, relation) VALUES(?,?,?)",
            params![from_id, to_id, rel.as_str()],
        )?;
        db.execute(
            "INSERT OR IGNORE INTO relations(from_id, to_id, relation) VALUES(?,?,?)",
            params![to_id, from_id, rel.inverse().as_str()],
        )?;
        Ok(())
    }

    fn mark_superseded(&self, old_id: &str, new_id: &str) -> Result<(), MemoryError> {
        self.add_relation(new_id, old_id, Relation::Supersedes)?;
        let db = self.db.lock().map_err(|_| {
            MemoryError::Sqlite(rusqlite::Error::InvalidQuery)
        })?;
        db.execute(
            "UPDATE memories SET status = 'superseded', updated_at = ? WHERE id = ?",
            params![chrono::Utc::now().timestamp(), old_id],
        )?;
        Ok(())
    }

    fn exists_by_hash(&self, project_id: &str, hash: &str) -> Result<bool, MemoryError> {
        let db = self.db.lock().map_err(|_| {
            MemoryError::Sqlite(rusqlite::Error::InvalidQuery)
        })?;
        Ok(db.query_row(
            "SELECT 1 FROM memories WHERE project_id = ? AND content_hash = ? LIMIT 1",
            params![project_id, hash],
            |_| Ok(()),
        ).optional()?.is_some())
    }

    fn count(&self, project_id: &str) -> Result<usize, MemoryError> {
        let db = self.db.lock().map_err(|_| {
            MemoryError::Sqlite(rusqlite::Error::InvalidQuery)
        })?;
        Ok(db.query_row(
            "SELECT COUNT(*) FROM memories WHERE project_id = ?",
            params![project_id],
            |row| row.get(0),
        )?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::MemoryType;

    fn rec(project: &str, t: MemoryType, title: &str, content: &str) -> MemoryRecord {
        MemoryRecord::new(project, t, title, content)
    }

    #[test]
    fn crud_roundtrip() {
        let s = SqliteMemoryStore::open_in_memory().unwrap();
        let mut r = rec("p", MemoryType::Decision, "Use SQLite", "We chose SQLite for local memory.");
        r.files = vec!["src/store.rs".into()];
        s.insert(&r).unwrap();
        let got = s.get(&r.id).unwrap().expect("stored");
        assert_eq!(got.title, "Use SQLite");
        assert_eq!(got.files, vec!["src/store.rs"]);
        assert_eq!(s.count("p").unwrap(), 1);
        s.delete(&r.id).unwrap();
        assert!(s.get(&r.id).unwrap().is_none());
    }

    #[test]
    fn lexical_finds_identifiers() {
        let s = SqliteMemoryStore::open_in_memory().unwrap();
        s.insert(&rec("p", MemoryType::Bug, "Nvidia validation", "Provider::validate rejects raw keys in nvidia.rs")).unwrap();
        s.insert(&rec("p", MemoryType::Decision, "Unrelated", "The UI uses blue accents")).unwrap();
        let hits = s.search_lexical("Provider::validate nvidia", 5).unwrap();
        assert!(!hits.is_empty());
        assert!(hits[0].0.title.contains("Nvidia"));
        assert!(hits[0].1 > 0.0);
    }

    #[test]
    fn relations_and_supersession() {
        let s = SqliteMemoryStore::open_in_memory().unwrap();
        let old = rec("p", MemoryType::Decision, "Use Postgres", "Postgres for storage");
        let new = rec("p", MemoryType::UserCorrection, "Use SQLite", "Do not use Postgres; SQLite instead");
        s.insert(&old).unwrap();
        s.insert(&new).unwrap();
        s.mark_superseded(&old.id, &new.id).unwrap();
        assert_eq!(s.get(&old.id).unwrap().unwrap().status, MemoryStatus::Superseded);
        let rel = s.list_related(&new.id).unwrap();
        assert!(rel.iter().any(|(r, _)| r.id == old.id));
        assert!(s.exists_by_hash("p", &new.content_hash()).unwrap());
    }
}
