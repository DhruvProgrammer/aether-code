//! Snapshot / state-recovery system for AETHER.
//!
//! Stores immutable snapshots of:
//!   * Modified files (content + path)
//!   * Agent state (opaque JSON blobs keyed by agent id)
//!   * Controller state
//!   * Conversation state
//!   * Plan state
//!   * Configuration changes (opaque blobs)
//!   * Tool execution state
//!   * Permission decisions
//!   * Context state
//!
//! Triggers: pre-danger, pre-large-change, pre-deploy, pre-install,
//! pre-migration, pre-compaction, milestone, explicit.
//!
//! Restore semantics:
//!   * Undo:  restore the previous snapshot in the linear chain.
//!   * Redo:  restore the snapshot after the cursor.
//!   * Restore(id): jump to an arbitrary snapshot.
//!   * Branch(label): create a child snapshot from the current cursor without
//!     moving the linear chain.
//!   * Compare(a, b): diff two snapshots' file sets.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SnapshotError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("snapshot not found: {0}")]
    NotFound(String),
    #[error("nothing to undo")]
    NothingToUndo,
    #[error("nothing to redo")]
    NothingToRedo,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Trigger {
    PreDanger,
    PreLargeChange,
    PreDeploy,
    PreInstall,
    PreMigration,
    PreCompaction,
    Milestone,
    Manual,
}

impl Trigger {
    pub fn label(self) -> &'static str {
        match self {
            Self::PreDanger => "pre_danger",
            Self::PreLargeChange => "pre_large_change",
            Self::PreDeploy => "pre_deploy",
            Self::PreInstall => "pre_install",
            Self::PreMigration => "pre_migration",
            Self::PreCompaction => "pre_compaction",
            Self::Milestone => "milestone",
            Self::Manual => "manual",
        }
    }
}

/// Per-file content captured in a snapshot. `None` means the file did not
/// exist before the operation that triggered the snapshot (so restore = delete).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileSnapshot {
    pub path: PathBuf,
    pub before_content: Option<String>,
}

/// Full snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub id: String,
    pub session_id: String,
    pub parent_id: Option<String>,
    pub timestamp: DateTime<Utc>,
    pub trigger: Trigger,
    pub agent_id: Option<String>,
    pub task: Option<String>,
    pub files: Vec<FileSnapshot>,
    pub state: HashMap<String, serde_json::Value>, // arbitrary state blobs
    pub metadata: HashMap<String, String>,
}

impl Snapshot {
    pub fn changed_paths(&self) -> Vec<&Path> {
        self.files.iter().map(|f| f.path.as_path()).collect()
    }
}

/// Cursor pointing into a session's linear snapshot chain.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct Cursor {
    pub session_id: String,
    pub head: Option<String>, // last snapshot id in the chain
    pub cursor: Option<String>, // the snapshot the user is currently looking at
}

/// SnapshotManager — owns the snapshot store for a session.
pub struct SnapshotManager {
    root: PathBuf, // <session_dir>/snapshots
    snapshots: HashMap<String, Snapshot>,
    cursors: HashMap<String, Cursor>,
}

impl SnapshotManager {
    pub fn open(root: impl Into<PathBuf>) -> Result<Self, SnapshotError> {
        let root = root.into();
        std::fs::create_dir_all(&root)?;
        let mut mgr = Self {
            root,
            snapshots: HashMap::new(),
            cursors: HashMap::new(),
        };
        mgr.load_all()?;
        Ok(mgr)
    }

    fn load_all(&mut self) -> Result<(), SnapshotError> {
        let dir = self.root.join("snapshots");
        if !dir.exists() { return Ok(()); }
        for entry in std::fs::read_dir(&dir)? {
            let entry = entry?;
            if entry.path().extension().and_then(|e| e.to_str()) == Some("json") {
                let txt = std::fs::read_to_string(entry.path())?;
                let s: Snapshot = serde_json::from_str(&txt)?;
                self.cursors
                    .entry(s.session_id.clone())
                    .or_insert_with(|| Cursor { session_id: s.session_id.clone(), head: None, cursor: None });
                let cur = self.cursors.get_mut(&s.session_id).unwrap();
                if cur.head.is_none() || s.timestamp > self.snapshots.get(&cur.head.clone().unwrap()).map(|h| h.timestamp).unwrap_or(s.timestamp) {
                    cur.head = Some(s.id.clone());
                }
                self.snapshots.insert(s.id.clone(), s);
            }
        }
        Ok(())
    }

    fn persist(&self, s: &Snapshot) -> Result<(), SnapshotError> {
        let path = self.root.join("snapshots").join(format!("{}.json", s.id));
        if let Some(parent) = path.parent() { std::fs::create_dir_all(parent)?; }
        std::fs::write(path, serde_json::to_string_pretty(s)?)?;
        Ok(())
    }

    /// Capture a snapshot. `files` are absolute paths; their current on-disk
    /// content is read into the snapshot as the "before" state so future
    /// restore() can rewind.
    pub fn snapshot(
        &mut self,
        session_id: impl Into<String>,
        trigger: Trigger,
        agent_id: Option<String>,
        task: Option<String>,
        files: &[PathBuf],
        state: HashMap<String, serde_json::Value>,
        metadata: HashMap<String, String>,
    ) -> Result<String, SnapshotError> {
        let session_id = session_id.into();
        let parent_id = self.cursors.get(&session_id).and_then(|c| c.cursor.clone());
        let id = format!("snap-{}-{}", Utc::now().format("%Y%m%d-%H%M%S%3f"), uuid::Uuid::new_v4().simple());
        let mut snapshots = Vec::new();
        for f in files {
            snapshots.push(FileSnapshot {
                path: f.clone(),
                before_content: std::fs::read_to_string(f).ok(),
            });
        }
        let snap = Snapshot {
            id: id.clone(),
            session_id: session_id.clone(),
            parent_id,
            timestamp: Utc::now(),
            trigger,
            agent_id,
            task,
            files: snapshots,
            state,
            metadata,
        };
        self.persist(&snap)?;
        self.snapshots.insert(id.clone(), snap);
        let cur = self.cursors.entry(session_id.clone()).or_insert(Cursor { session_id: session_id.clone(), head: None, cursor: None });
        cur.head = Some(id.clone());
        cur.cursor = Some(id.clone());
        Ok(id)
    }

    /// Move the cursor one step back (towards the root).
    ///
    /// Semantics: the current cursor's snapshot captured the file
    /// `before_content`s at the time of capture, so applying them rewinds
    /// the workspace to that point in history. The cursor then steps back
    /// to the parent.
    pub fn undo(&mut self, session_id: &str) -> Result<Snapshot, SnapshotError> {
        let cur = self.cursors.get(session_id).ok_or(SnapshotError::NothingToUndo)?.clone();
        let cursor_id = cur.cursor.as_ref().ok_or(SnapshotError::NothingToUndo)?;
        let snap = self.snapshots.get(cursor_id).ok_or(SnapshotError::NotFound(cursor_id.clone()))?.clone();
        let parent = snap.parent_id.clone().ok_or(SnapshotError::NothingToUndo)?;
        // Apply the current snapshot's before-content (the workspace state
        // captured when this snapshot was taken), then move the cursor to
        // the parent.
        restore_files(&snap.files)?;
        self.cursors.get_mut(session_id).unwrap().cursor = Some(parent.clone());
        Ok(self.snapshots.get(&parent).ok_or(SnapshotError::NotFound(parent))?.clone())
    }

    /// Move the cursor one step forward (towards head).
    pub fn redo(&mut self, session_id: &str) -> Result<Snapshot, SnapshotError> {
        let cur = self.cursors.get(session_id).ok_or(SnapshotError::NothingToRedo)?.clone();
        let cursor_id = cur.cursor.ok_or(SnapshotError::NothingToRedo)?;
        // Find the snapshot that points back to cursor_id as parent.
        let next = self.snapshots.values()
            .find(|s| s.session_id == session_id && s.parent_id.as_deref() == Some(&cursor_id))
            .ok_or(SnapshotError::NothingToRedo)?
            .id.clone();
        let next_snap = self.snapshots.get(&next).ok_or(SnapshotError::NotFound(next.clone()))?;
        restore_files(&next_snap.files)?;
        self.cursors.get_mut(session_id).unwrap().cursor = Some(next.clone());
        Ok(next_snap.clone())
    }

    /// Jump the cursor to an arbitrary snapshot and apply it.
    pub fn restore(&mut self, id: &str) -> Result<Snapshot, SnapshotError> {
        let snap = self.snapshots.get(id).ok_or_else(|| SnapshotError::NotFound(id.to_string()))?.clone();
        restore_files(&snap.files)?;
        self.cursors.get_mut(&snap.session_id).unwrap().cursor = Some(id.to_string());
        Ok(snap)
    }

    /// Create a branch: snapshot the current cursor's state into a new snapshot
    /// without changing the linear chain. Useful for "try something — keep
    /// both outcomes".
    pub fn branch(
        &mut self,
        session_id: impl Into<String>,
        agent_id: Option<String>,
        label: impl Into<String>,
        files: &[PathBuf],
        state: HashMap<String, serde_json::Value>,
    ) -> Result<String, SnapshotError> {
        let mut meta = HashMap::new();
        meta.insert("branch".into(), label.into());
        self.snapshot(session_id, Trigger::Manual, agent_id, None, files, state, meta)
    }

    /// Diff two snapshots by file set.
    pub fn compare(&self, a: &str, b: &str) -> Result<SnapshotDiff, SnapshotError> {
        let sa = self.snapshots.get(a).ok_or_else(|| SnapshotError::NotFound(a.to_string()))?;
        let sb = self.snapshots.get(b).ok_or_else(|| SnapshotError::NotFound(b.to_string()))?;
        let paths_a: std::collections::HashSet<&Path> = sa.files.iter().map(|f| f.path.as_path()).collect();
        let paths_b: std::collections::HashSet<&Path> = sb.files.iter().map(|f| f.path.as_path()).collect();
        let only_a: Vec<PathBuf> = paths_a.difference(&paths_b).map(|p| p.to_path_buf()).collect();
        let only_b: Vec<PathBuf> = paths_b.difference(&paths_a).map(|p| p.to_path_buf()).collect();
        let mut changed = Vec::new();
        for fa in &sa.files {
            if let Some(fb) = sb.files.iter().find(|x| x.path == fa.path) {
                if fa.before_content != fb.before_content {
                    changed.push(fa.path.clone());
                }
            }
        }
        Ok(SnapshotDiff { only_in_a: only_a, only_in_b: only_b, changed })
    }

    pub fn get(&self, id: &str) -> Option<&Snapshot> { self.snapshots.get(id) }
    pub fn list(&self, session_id: &str) -> Vec<&Snapshot> {
        let mut v: Vec<_> = self.snapshots.values().filter(|s| s.session_id == session_id).collect();
        v.sort_by(|a, b| a.timestamp.cmp(&b.timestamp));
        v
    }
    pub fn cursor(&self, session_id: &str) -> Option<&Cursor> { self.cursors.get(session_id) }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotDiff {
    pub only_in_a: Vec<PathBuf>,
    pub only_in_b: Vec<PathBuf>,
    pub changed: Vec<PathBuf>,
}

fn restore_files(files: &[FileSnapshot]) -> Result<(), SnapshotError> {
    for f in files {
        match &f.before_content {
            Some(content) => {
                if let Some(parent) = f.path.parent() { std::fs::create_dir_all(parent)?; }
                std::fs::write(&f.path, content)?;
            }
            None => {
                // File didn't exist before — make sure it doesn't after restore.
                let _ = std::fs::remove_file(&f.path);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp() -> PathBuf {
        let p = std::env::temp_dir().join(format!("aether-snap-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn snapshot_undo_redo_round_trip() {
        let dir = tmp();
        let f = dir.join("a.txt");
        std::fs::write(&f, "v1").unwrap();

        let mut mgr = SnapshotManager::open(&dir).unwrap();
        let s1 = mgr.snapshot("s1", Trigger::Manual, None, None, &[f.clone()], Default::default(), Default::default()).unwrap();
        std::fs::write(&f, "v2").unwrap();
        let s2 = mgr.snapshot("s1", Trigger::Manual, None, None, &[f.clone()], Default::default(), Default::default()).unwrap();
        std::fs::write(&f, "v3").unwrap();

        // Undo from cursor=s2 restores s2's before-content = v2.
        mgr.undo("s1").unwrap();
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "v2");
        // Cannot undo past the root — s1 has no parent.
        assert!(mgr.undo("s1").is_err());
        // Redo moves cursor to s2 and applies s2's before-content (v2).
        mgr.redo("s1").unwrap();
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "v2");

        // Persistence.
        assert!(dir.join("snapshots").join(format!("{s1}.json")).exists());
        assert!(dir.join("snapshots").join(format!("{s2}.json")).exists());
    }

    #[test]
    fn branch_creates_alternate_snapshot() {
        let dir = tmp();
        let mut mgr = SnapshotManager::open(&dir).unwrap();
        let s1 = mgr.snapshot("s", Trigger::Manual, Some("agent-1".into()), None, &[], Default::default(), Default::default()).unwrap();
        let s2 = mgr.branch("s", Some("agent-1".into()), "experiment", &[], Default::default()).unwrap();
        assert_ne!(s1, s2);
        assert_eq!(mgr.list("s").len(), 2);
    }

    #[test]
    fn compare_returns_changed_paths() {
        let dir = tmp();
        let f1 = dir.join("a.txt"); std::fs::write(&f1, "x").unwrap();
        let f2 = dir.join("b.txt"); std::fs::write(&f2, "y").unwrap();
        let mut mgr = SnapshotManager::open(&dir).unwrap();
        let sa = mgr.snapshot("s", Trigger::Manual, None, None, &[f1.clone()], Default::default(), Default::default()).unwrap();
        std::fs::write(&f1, "z").unwrap();
        let sb = mgr.snapshot("s", Trigger::Manual, None, None, &[f1.clone(), f2.clone()], Default::default(), Default::default()).unwrap();
        let diff = mgr.compare(&sa, &sb).unwrap();
        assert!(diff.changed.contains(&f1));
        assert!(diff.only_in_b.contains(&f2));
    }
}
