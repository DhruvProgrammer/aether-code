//! AETHER workspace manager.
//!
//! A workspace is a folder the user works in. Sessions belong to exactly one
//! workspace. Workspaces are identified by a stable internal ID derived from
//! the canonical folder path, so two folders with the same display name never
//! collide.
//!
//! Storage: a single SQLite database at `~/.aether/workspaces.db`. Project
//! files are never copied into the database — only metadata is stored.

use anyhow::Result;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// A workspace (folder) the user has opened in AETHER.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workspace {
    pub id: String,
    /// Canonical absolute folder path.
    pub path: String,
    /// Display name (last path component by default).
    pub name: String,
    /// RFC3339 timestamp of when the workspace was first opened.
    pub created_at: String,
    /// RFC3339 timestamp of the most recent open.
    pub last_opened: String,
    /// The session that was active when the workspace was last used.
    pub last_session: Option<String>,
}

/// Filesystem-backed store of workspaces.
pub struct WorkspaceStore {
    conn: Connection,
}

impl WorkspaceStore {
    /// Open (or create) the workspace database at `path`.
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS workspaces(
                id TEXT PRIMARY KEY,
                path TEXT NOT NULL UNIQUE,
                name TEXT NOT NULL,
                created_at TEXT NOT NULL,
                last_opened TEXT NOT NULL,
                last_session TEXT
            );",
        )?;
        Ok(Self { conn })
    }

    /// Default location: `~/.aether/workspaces.db`.
    pub fn default_path() -> Result<PathBuf> {
        let home = dirs::home_dir()
            .ok_or_else(|| anyhow::anyhow!("cannot resolve home directory"))?;
        Ok(home.join(".aether").join("workspaces.db"))
    }

    /// Open a workspace by folder path. Creates it if it does not exist yet,
    /// otherwise refreshes `last_opened`. Returns the workspace.
    pub fn open_folder(&mut self, folder: &Path) -> Result<Workspace> {
        let canonical = normalize(folder)?;
        let path_str = canonical.display().to_string();
        let now = chrono::Utc::now().to_rfc3339();
        let name = canonical
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("workspace")
            .to_string();

        let existing: Option<String> = self
            .conn
            .query_row(
                "SELECT id FROM workspaces WHERE path = ?1",
                params![path_str],
                |r| r.get(0),
            )
            .ok();

        match existing {
            Some(id) => {
                self.conn.execute(
                    "UPDATE workspaces SET last_opened = ?2 WHERE id = ?1",
                    params![id, now],
                )?;
                self.get(&id)?
                    .ok_or_else(|| anyhow::anyhow!("workspace vanished after update"))
            }
            None => {
                let id = uuid::Uuid::new_v4().to_string();
                self.conn.execute(
                    "INSERT INTO workspaces(id, path, name, created_at, last_opened) VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![id, path_str, name, now, now],
                )?;
                self.get(&id)?
                    .ok_or_else(|| anyhow::anyhow!("workspace vanished after insert"))
            }
        }
    }

    /// Fetch a workspace by ID.
    pub fn get(&self, id: &str) -> Result<Option<Workspace>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, path, name, created_at, last_opened, last_session FROM workspaces WHERE id = ?1")?;
        let mut rows = stmt.query(params![id])?;
        Ok(rows.next()?.map(row_to_workspace))
    }

    /// Fetch a workspace by folder path.
    pub fn get_by_path(&self, folder: &Path) -> Result<Option<Workspace>> {
        let canonical = normalize(folder)?;
        let path_str = canonical.display().to_string();
        let mut stmt = self
            .conn
            .prepare("SELECT id, path, name, created_at, last_opened, last_session FROM workspaces WHERE path = ?1")?;
        let mut rows = stmt.query(params![path_str])?;
        Ok(rows.next()?.map(row_to_workspace))
    }

    /// List workspaces ordered by most recently opened (for the home screen).
    pub fn recent(&self, limit: usize) -> Result<Vec<Workspace>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, path, name, created_at, last_opened, last_session FROM workspaces ORDER BY last_opened DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit as i64], |r| {
            Ok(row_to_workspace_from(r))
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    /// Record which session was last active in a workspace.
    pub fn set_last_session(&self, workspace_id: &str, session_id: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE workspaces SET last_session = ?2 WHERE id = ?1",
            params![workspace_id, session_id],
        )?;
        Ok(())
    }

    /// Remove a workspace from the recent list (does not touch the folder).
    pub fn remove(&self, id: &str) -> Result<()> {
        self.conn.execute("DELETE FROM workspaces WHERE id = ?1", params![id])?;
        Ok(())
    }
}

fn row_to_workspace(r: &rusqlite::Row) -> Workspace {
    row_to_workspace_from(r)
}

fn row_to_workspace_from(r: &rusqlite::Row) -> Workspace {
    Workspace {
        id: r.get(0).unwrap_or_default(),
        path: r.get(1).unwrap_or_default(),
        name: r.get(2).unwrap_or_default(),
        created_at: r.get(3).unwrap_or_default(),
        last_opened: r.get(4).unwrap_or_default(),
        last_session: r.get(5).ok(),
    }
}

/// Canonicalize a path, falling back to absolute-ification when the folder
/// does not exist yet (canonicalize requires existence).
fn normalize(p: &Path) -> Result<PathBuf> {
    if let Ok(c) = p.canonicalize() {
        return Ok(c);
    }
    let abs = if p.is_absolute() {
        p.to_path_buf()
    } else {
        std::env::current_dir()?.join(p)
    };
    Ok(abs)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_store(name: &str) -> WorkspaceStore {
        let p = std::env::temp_dir().join(format!(
            "aether-ws-{}-{}.db",
            name,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        WorkspaceStore::open(&p).unwrap()
    }

    #[test]
    fn open_folder_creates_then_refreshes() {
        let mut s = tmp_store("open");
        let dir = std::env::temp_dir().join("aether-ws-test-folder");
        std::fs::create_dir_all(&dir).unwrap();
        let w1 = s.open_folder(&dir).unwrap();
        assert!(!w1.id.is_empty());
        assert_eq!(w1.name, "aether-ws-test-folder");
        let w2 = s.open_folder(&dir).unwrap();
        assert_eq!(w1.id, w2.id, "same folder must map to same workspace");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn recent_is_ordered_by_last_opened() {
        let mut s = tmp_store("recent");
        let a = std::env::temp_dir().join("aether-ws-a");
        let b = std::env::temp_dir().join("aether-ws-b");
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        s.open_folder(&a).unwrap();
        s.open_folder(&b).unwrap();
        s.open_folder(&a).unwrap();
        let recent = s.recent(10).unwrap();
        assert_eq!(recent.len(), 2);
        assert_eq!(recent[0].name, "aether-ws-a", "most recently opened first");
        let _ = std::fs::remove_dir_all(&a);
        let _ = std::fs::remove_dir_all(&b);
    }

    #[test]
    fn same_display_name_different_paths_are_distinct() {
        let mut s = tmp_store("distinct");
        let a = std::env::temp_dir().join("aether-ws-x").join("website");
        let b = std::env::temp_dir().join("aether-ws-y").join("website");
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        let wa = s.open_folder(&a).unwrap();
        let wb = s.open_folder(&b).unwrap();
        assert_ne!(wa.id, wb.id, "two 'website' folders must be distinct workspaces");
        let _ = std::fs::remove_dir_all(std::env::temp_dir().join("aether-ws-x"));
        let _ = std::fs::remove_dir_all(std::env::temp_dir().join("aether-ws-y"));
    }

    #[test]
    fn last_session_roundtrip() {
        let mut s = tmp_store("lastsess");
        let dir = std::env::temp_dir().join("aether-ws-ls");
        std::fs::create_dir_all(&dir).unwrap();
        let w = s.open_folder(&dir).unwrap();
        s.set_last_session(&w.id, "session-123").unwrap();
        let w2 = s.get(&w.id).unwrap().unwrap();
        assert_eq!(w2.last_session.as_deref(), Some("session-123"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
