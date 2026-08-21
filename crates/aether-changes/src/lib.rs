//! AETHER realtime workspace change tracker.
//!
//! Watches the filesystem for actual modifications and computes Git-aware change
//! counts. The workspace is the source of truth — never the LLM.
//!
//! Architecture:
//! ```text
//! Filesystem watcher ──► debounce (250ms) ──► Git status/diff ──► WorkspaceChanges DTO ──► Tauri event
//! ```

use anyhow::Result;
use chrono::Utc;
use notify::{Config, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use parking_lot::Mutex;
use std::time::{Duration, Instant};

fn now_ts() -> String {
    Utc::now().to_rfc3339()
}

// ---------------------------------------------------------------------------
// DTOs
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "UPPERCASE")]
pub enum FileStatus {
    #[serde(rename = "M")]
    Modified,
    #[serde(rename = "A")]
    Added,
    #[serde(rename = "D")]
    Deleted,
    #[serde(rename = "R")]
    Renamed,
    #[serde(rename = "U")]
    Untracked,
}

impl FileStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            FileStatus::Modified => "M",
            FileStatus::Added => "A",
            FileStatus::Deleted => "D",
            FileStatus::Renamed => "R",
            FileStatus::Untracked => "U",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileChange {
    pub path: String,
    pub status: FileStatus,
    #[serde(default)]
    pub additions: u32,
    #[serde(default)]
    pub deletions: u32,
    #[serde(default)]
    pub staged: bool,
    #[serde(default)]
    pub renamed_from: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceChanges {
    pub workspace_id: String,
    pub workspace_path: String,
    pub total_files: usize,
    pub additions: u32,
    pub deletions: u32,
    pub files: Vec<FileChange>,
    pub timestamp: String,
    pub is_git: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileDiff {
    pub path: String,
    pub diff: String,
    pub additions: u32,
    pub deletions: u32,
}

// ---------------------------------------------------------------------------
// Git-aware computation
// ---------------------------------------------------------------------------

fn is_git_repo(path: &Path) -> bool {
    // Fast check for .git dir, fallback to git rev-parse.
    if path.join(".git").exists() {
        return true;
    }
    std::process::Command::new("git")
        .arg("rev-parse")
        .arg("--is-inside-work-tree")
        .current_dir(path)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn run_git(path: &Path, args: &[&str]) -> Option<String> {
    let out = std::process::Command::new("git")
        .args(args)
        .current_dir(path)
        .output()
        .ok()?;
    if !out.status.success() && !args.contains(&"status") {
        // For status we still want output even on non-zero? but git status returns 0 even with changes.
        // For diff, non-zero may mean no diff.
        // We'll return stdout even on failure for parsing.
    }
    Some(String::from_utf8_lossy(&out.stdout).to_string())
}

fn parse_numstat(output: &str) -> HashMap<String, (u32, u32)> {
    let mut map = HashMap::new();
    for line in output.lines() {
        // format: <add>\t<del>\t<path>  (or old -> new for renames)
        let parts: Vec<&str> = line.split('\t').collect();
        if parts.len() < 3 {
            continue;
        }
        let additions = parts[0].parse::<u32>().unwrap_or(0);
        let deletions = parts[1].parse::<u32>().unwrap_or(0);
        let raw_path = parts[2];
        // For renames, git shows "old => new" or "old\tnew"? In numstat, it's "old\000new" or "old => new" depending.
        // We'll handle " => " case: take the new path after " => "
        let path = if raw_path.contains(" => ") {
            raw_path.split(" => ").last().unwrap_or(raw_path).to_string()
        } else if raw_path.contains('\0') {
            raw_path.split('\0').last().unwrap_or(raw_path).to_string()
        } else {
            raw_path.to_string()
        };
        map.insert(path, (additions, deletions));
    }
    map
}

fn count_lines(path: &Path, workspace: &Path) -> u32 {
    let full = workspace.join(path);
    std::fs::read_to_string(&full)
        .map(|s| s.lines().count() as u32)
        .unwrap_or(0)
}

/// Compute authoritative workspace changes. Never trusts LLM claims.
pub fn compute_changes(workspace_path: &Path, workspace_id: &str) -> WorkspaceChanges {
    let is_git = is_git_repo(workspace_path);
    if !is_git {
        // Non-Git: we cannot compute diff; return empty but caller may overlay watcher-driven set.
        // For now return empty; watcher will maintain its own pending set.
        return WorkspaceChanges {
            workspace_id: workspace_id.to_string(),
            workspace_path: workspace_path.display().to_string(),
            total_files: 0,
            additions: 0,
            deletions: 0,
            files: vec![],
            timestamp: now_ts(),
            is_git: false,
        };
    }

    let status_out = run_git(workspace_path, &["status", "--porcelain", "-uall"]).unwrap_or_default();
    let numstat_unstaged = run_git(workspace_path, &["diff", "--numstat"]).unwrap_or_default();
    let numstat_staged = run_git(workspace_path, &["diff", "--cached", "--numstat"]).unwrap_or_default();
    let numstat_head = if status_out.is_empty() {
        String::new()
    } else {
        // Combined view for accurate per-file counts across staged+unstaged
        run_git(workspace_path, &["diff", "--numstat", "HEAD"]).unwrap_or_default()
    };

    let map_unstaged = parse_numstat(&numstat_unstaged);
    let map_staged = parse_numstat(&numstat_staged);
    let map_head = parse_numstat(&numstat_head);

    let mut files: Vec<FileChange> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    for line in status_out.lines() {
        if line.len() < 3 {
            continue;
        }
        let staged_c = line.chars().nth(0).unwrap_or(' ');
        let unstaged_c = line.chars().nth(1).unwrap_or(' ');
        let raw_path = line[3..].trim().to_string();

        // Handle renames: "R  old -> new" or "R old -> new" with staged/unstaged
        let (status, path, renamed_from) = if raw_path.contains(" -> ") {
            let parts: Vec<&str> = raw_path.split(" -> ").collect();
            let old = parts[0].to_string();
            let new = parts[1].to_string();
            (FileStatus::Renamed, new, Some(old))
        } else {
            let s = match (staged_c, unstaged_c) {
                ('?', '?') => FileStatus::Untracked,
                ('A', _) | (_, 'A') => FileStatus::Added,
                ('D', _) | (_, 'D') => FileStatus::Deleted,
                ('R', _) | (_, 'R') => FileStatus::Renamed,
                _ => FileStatus::Modified,
            };
            (s, raw_path.clone(), None)
        };

        if seen.contains(&path) {
            continue;
        }
        seen.insert(path.clone());

        // Prefer HEAD numstat, then staged, then unstaged
        let (add, del) = map_head
            .get(&path)
            .or_else(|| map_staged.get(&path))
            .or_else(|| map_unstaged.get(&path))
            .copied()
            .unwrap_or((0, 0));

        let (add, del) = if status == FileStatus::Untracked && add == 0 && del == 0 {
            // For untracked files, count lines as additions
            (count_lines(Path::new(&path), workspace_path), 0)
        } else {
            (add, del)
        };

        let staged = staged_c != ' ' && staged_c != '?' && staged_c != '!';

        files.push(FileChange {
            path: path.clone(),
            status,
            additions: add,
            deletions: del,
            staged,
            renamed_from,
        });
    }

    // Also include any numstat entries not in status (e.g., staged renames may have different path key)
    // But status already covers tracked changes; numstat adds diff stats for those.

    // Sort for stable UI
    files.sort_by(|a, b| a.path.cmp(&b.path));

    let total_files = files.len();
    let additions: u32 = files.iter().map(|f| f.additions).sum();
    let deletions: u32 = files.iter().map(|f| f.deletions).sum();

    WorkspaceChanges {
        workspace_id: workspace_id.to_string(),
        workspace_path: workspace_path.display().to_string(),
        total_files,
        additions,
        deletions,
        files,
        timestamp: now_ts(),
        is_git,
    }
}

/// Get diff for a single file. Returns empty string if file not found or not a git repo.
pub fn get_file_diff(workspace_path: &Path, file_path: &str) -> Result<FileDiff, String> {
    if !is_git_repo(workspace_path) {
        return Err("not a git repository".into());
    }
    // Try unstaged diff first, then staged, then HEAD
    let candidates = [
        vec!["diff", "--", file_path],
        vec!["diff", "--cached", "--", file_path],
        vec!["diff", "HEAD", "--", file_path],
    ];
    let mut diff = String::new();
    for args in candidates {
        if let Some(out) = run_git(workspace_path, &args) {
            if !out.trim().is_empty() {
                diff = out;
                break;
            }
        }
    }
    if diff.is_empty() {
        // For untracked files, show file content as added diff
        let full = workspace_path.join(file_path);
        if full.exists() {
            if let Ok(content) = std::fs::read_to_string(&full) {
                let lines = content.lines().count();
                let header = format!("--- /dev/null\n+++ b/{}\n@@ -0,0 +1,{} @@\n", file_path, lines);
                let body: String = content.lines().map(|l| format!("+{l}\n")).collect();
                diff = header + &body;
                return Ok(FileDiff {
                    path: file_path.to_string(),
                    additions: lines as u32,
                    deletions: 0,
                    diff,
                });
            }
        }
        // If file is deleted, git diff HEAD should have shown it; if still empty, return not found
        return Err(format!("no diff for {file_path}"));
    }
    // Count additions/deletions from diff
    let mut additions = 0u32;
    let mut deletions = 0u32;
    for line in diff.lines() {
        if line.starts_with("+++") || line.starts_with("---") || line.starts_with("@@") {
            continue;
        }
        if line.starts_with('+') {
            additions += 1;
        } else if line.starts_with('-') {
            deletions += 1;
        }
    }
    Ok(FileDiff {
        path: file_path.to_string(),
        diff,
        additions,
        deletions,
    })
}

// ---------------------------------------------------------------------------
// Watcher
// ---------------------------------------------------------------------------

fn should_ignore(path: &Path) -> bool {
    let s = path.to_string_lossy();
    // Ignore common large/derived dirs and our own data
    for pat in [".git", "target", "node_modules", ".aether", "dist", ".next", "__pycache__"] {
        if s.contains(&format!("/{pat}/")) || s.ends_with(&format!("/{pat}")) || s.contains(&format!("\\{pat}\\")) {
            return true;
        }
    }
    // Ignore temp files
    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
        if name.ends_with('~') || name.starts_with('.') && name.len() > 1 && name.contains("swp") {
            return true;
        }
    }
    false
}

#[derive(Debug)]
pub struct WatchHandle {
    _watcher: RecommendedWatcher,
    stop: Arc<std::sync::atomic::AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Drop for WatchHandle {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
        if let Some(h) = self.thread.take() {
            let _ = h.join();
        }
    }
}

/// Manager for per-workspace watchers. Keeps one watcher per workspace_id.
pub struct ChangeWatcherManager {
    watchers: Mutex<HashMap<String, WatchHandle>>,
}

impl ChangeWatcherManager {
    pub fn new() -> Self {
        Self {
            watchers: Mutex::new(HashMap::new()),
        }
    }

    /// Start watching a workspace. If already watching, restarts. Emits via `emit` callback.
    /// Debounce 250ms, coalesces rapid events.
    pub fn watch<F>(&self, workspace_path: PathBuf, workspace_id: String, emit: F) -> Result<()>
    where
        F: Fn(WorkspaceChanges) + Send + Sync + 'static,
    {
        self.stop(&workspace_id);

        if !workspace_path.exists() {
            anyhow::bail!("workspace path does not exist: {}", workspace_path.display());
        }

        let emit = Arc::new(emit);
        // Channel for filesystem events
        let (tx, rx) = std::sync::mpsc::channel::<Result<notify::Event, notify::Error>>();

        let mut watcher = RecommendedWatcher::new(tx, Config::default())?;
        watcher.watch(&workspace_path, RecursiveMode::Recursive)?;

        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let stop_clone = stop.clone();
        let wp_clone = workspace_path.clone();
        let wid_clone = workspace_id.clone();
        let emit_clone = emit.clone();

        // Emit initial state immediately
        let initial = compute_changes(&workspace_path, &workspace_id);
        emit_clone(initial);

        let thread = std::thread::spawn(move || {
            let mut pending_paths: HashSet<PathBuf> = HashSet::new();
            let mut last_event = Instant::now();
            let debounce = Duration::from_millis(280);
            let emit_debounced = |pending: &mut HashSet<PathBuf>, wp: &Path, wid: &str| {
                if pending.is_empty() {
                    return;
                }
                let changes = if is_git_repo(wp) {
                    compute_changes(wp, wid)
                } else {
                    // Non-Git: build from pending filesystem events.
                    let mut files: Vec<FileChange> = Vec::new();
                    let mut seen = HashSet::new();
                    for abs in pending.iter() {
                        let rel = abs.strip_prefix(wp).unwrap_or(abs).to_string_lossy().replace('\\', "/");
                        if rel.is_empty() || should_ignore(Path::new(&rel)) {
                            continue;
                        }
                        if !seen.insert(rel.clone()) {
                            continue;
                        }
                        let status = if !abs.exists() {
                            FileStatus::Deleted
                        } else if abs.is_dir() {
                            continue;
                        } else {
                            FileStatus::Modified
                        };
                        files.push(FileChange {
                            path: rel,
                            status,
                            additions: 0,
                            deletions: 0,
                            staged: false,
                            renamed_from: None,
                        });
                    }
                    files.sort_by(|a, b| a.path.cmp(&b.path));
                    let total = files.len();
                    WorkspaceChanges {
                        workspace_id: wid.to_string(),
                        workspace_path: wp.display().to_string(),
                        total_files: total,
                        additions: 0,
                        deletions: 0,
                        files,
                        timestamp: now_ts(),
                        is_git: false,
                    }
                };
                emit_clone(changes);
                pending.clear();
            };
            loop {
                if stop_clone.load(std::sync::atomic::Ordering::Relaxed) {
                    break;
                }
                if !wp_clone.exists() {
                    break;
                }
                match rx.recv_timeout(Duration::from_millis(300)) {
                    Ok(Ok(event)) => {
                        let mut relevant = false;
                        for p in &event.paths {
                            if should_ignore(p) {
                                continue;
                            }
                            match event.kind {
                                EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_) => {
                                    relevant = true;
                                    pending_paths.insert(p.clone());
                                }
                                _ => {
                                    if !event.paths.is_empty() {
                                        relevant = true;
                                        for pp in &event.paths {
                                            pending_paths.insert(pp.clone());
                                        }
                                    }
                                }
                            }
                        }
                        if relevant {
                            last_event = Instant::now();
                        }
                    }
                    Ok(Err(e)) => {
                        eprintln!("aether-changes watcher error: {e}");
                    }
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                        if !pending_paths.is_empty() && last_event.elapsed() >= debounce {
                            emit_debounced(&mut pending_paths, &wp_clone, &wid_clone);
                        }
                    }
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                }
                if !pending_paths.is_empty() && last_event.elapsed() >= debounce {
                    emit_debounced(&mut pending_paths, &wp_clone, &wid_clone);
                }
            }
        });

        let handle = WatchHandle {
            _watcher: watcher,
            stop,
            thread: Some(thread),
        };
        self.watchers.lock().insert(workspace_id, handle);
        Ok(())
    }

    pub fn stop(&self, workspace_id: &str) {
        if let Some(mut h) = self.watchers.lock().remove(workspace_id) {
            h.stop.store(true, std::sync::atomic::Ordering::Relaxed);
            if let Some(th) = h.thread.take() {
                let _ = th.join();
            }
        }
    }

    pub fn stop_all(&self) {
        let mut m = self.watchers.lock();
        for (_, mut h) in m.drain() {
            h.stop.store(true, std::sync::atomic::Ordering::Relaxed);
            if let Some(th) = h.thread.take() {
                let _ = th.join();
            }
        }
    }

    pub fn is_watching(&self, workspace_id: &str) -> bool {
        self.watchers.lock().contains_key(workspace_id)
    }
}

impl Default for ChangeWatcherManager {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::process::Command;

    fn tmp_workspace(tag: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("aether-changes-test-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).unwrap();
        p
    }

    fn init_git_repo(p: &Path) {
        Command::new("git").arg("init").current_dir(p).output().unwrap();
        Command::new("git").arg("config").arg("user.email").arg("test@test.com").current_dir(p).output().unwrap();
        Command::new("git").arg("config").arg("user.name").arg("Test").current_dir(p).output().unwrap();
    }

    #[test]
    fn compute_changes_empty_repo() {
        let ws = tmp_workspace("empty");
        init_git_repo(&ws);
        let changes = compute_changes(&ws, "wid-empty");
        assert_eq!(changes.total_files, 0);
        assert_eq!(changes.additions, 0);
        let _ = fs::remove_dir_all(&ws);
    }

    #[test]
    fn modified_tracked_file_detected() {
        let ws = tmp_workspace("modified");
        init_git_repo(&ws);
        fs::write(ws.join("foo.txt"), "line1\nline2\n").unwrap();
        Command::new("git").args(["add", "foo.txt"]).current_dir(&ws).output().unwrap();
        Command::new("git").args(["commit", "-m", "init"]).current_dir(&ws).output().unwrap();
        // Modify
        fs::write(ws.join("foo.txt"), "line1\nline2\nline3\n").unwrap();
        let changes = compute_changes(&ws, "wid-mod");
        assert_eq!(changes.total_files, 1);
        assert!(changes.files.iter().any(|f| f.path == "foo.txt"));
        assert!(changes.additions >= 1);
        let _ = fs::remove_dir_all(&ws);
    }

    #[test]
    fn untracked_file_detected() {
        let ws = tmp_workspace("untracked");
        init_git_repo(&ws);
        fs::write(ws.join("new.txt"), "hello\nworld\n").unwrap();
        let changes = compute_changes(&ws, "wid-untracked");
        assert_eq!(changes.total_files, 1);
        assert_eq!(changes.files[0].status, FileStatus::Untracked);
        assert_eq!(changes.files[0].additions, 2);
        let _ = fs::remove_dir_all(&ws);
    }

    #[test]
    fn deleted_file_detected() {
        let ws = tmp_workspace("deleted");
        init_git_repo(&ws);
        fs::write(ws.join("del.txt"), "to delete\n").unwrap();
        Command::new("git").args(["add", "del.txt"]).current_dir(&ws).output().unwrap();
        Command::new("git").args(["commit", "-m", "add"]).current_dir(&ws).output().unwrap();
        fs::remove_file(ws.join("del.txt")).unwrap();
        let changes = compute_changes(&ws, "wid-del");
        assert_eq!(changes.total_files, 1);
        assert_eq!(changes.files[0].status, FileStatus::Deleted);
        let _ = fs::remove_dir_all(&ws);
    }

    #[test]
    fn non_git_workspace_returns_empty() {
        let ws = tmp_workspace("non-git");
        // No git init
        fs::write(ws.join("file.txt"), "content\n").unwrap();
        let changes = compute_changes(&ws, "wid-nongit");
        assert!(!changes.is_git);
        assert_eq!(changes.total_files, 0);
        let _ = fs::remove_dir_all(&ws);
    }

    #[test]
    fn file_diff_for_modified() {
        let ws = tmp_workspace("diff-mod");
        init_git_repo(&ws);
        fs::write(ws.join("a.rs"), "fn a() {}\n").unwrap();
        Command::new("git").args(["add", "a.rs"]).current_dir(&ws).output().unwrap();
        Command::new("git").args(["commit", "-m", "a"]).current_dir(&ws).output().unwrap();
        fs::write(ws.join("a.rs"), "fn a() {}\nfn b() {}\n").unwrap();
        let diff = get_file_diff(&ws, "a.rs").unwrap();
        assert!(diff.diff.contains("+fn b()"));
        assert!(diff.additions >= 1);
        let _ = fs::remove_dir_all(&ws);
    }

    #[test]
    fn file_diff_for_untracked() {
        let ws = tmp_workspace("diff-untracked");
        init_git_repo(&ws);
        fs::write(ws.join("new.rs"), "fn x() {}\n").unwrap();
        let diff = get_file_diff(&ws, "new.rs").unwrap();
        assert!(diff.diff.contains("+fn x()"));
        let _ = fs::remove_dir_all(&ws);
    }

    #[test]
    fn is_git_detection() {
        let ws = tmp_workspace("isgit");
        assert!(!is_git_repo(&ws));
        init_git_repo(&ws);
        assert!(is_git_repo(&ws));
        let _ = fs::remove_dir_all(&ws);
    }

    #[test]
    fn should_ignore_filters() {
        assert!(should_ignore(Path::new("/tmp/proj/.git/config")));
        assert!(should_ignore(Path::new("/tmp/proj/target/debug/app")));
        assert!(should_ignore(Path::new("/tmp/proj/node_modules/pkg")));
        assert!(!should_ignore(Path::new("/tmp/proj/src/main.rs")));
    }

    #[test]
    fn workspace_changes_serialization() {
        let wc = WorkspaceChanges {
            workspace_id: "wid".into(),
            workspace_path: "/tmp/ws".into(),
            total_files: 1,
            additions: 5,
            deletions: 2,
            files: vec![FileChange {
                path: "src/main.rs".into(),
                status: FileStatus::Modified,
                additions: 5,
                deletions: 2,
                staged: false,
                renamed_from: None,
            }],
            timestamp: now_ts(),
            is_git: true,
        };
        let json = serde_json::to_string(&wc).unwrap();
        let back: WorkspaceChanges = serde_json::from_str(&json).unwrap();
        assert_eq!(back.total_files, 1);
    }
}
