//! Workspace abstraction (OpenCode-inspired).
//!
//! Every file-system operation in AETHER is funnelled through the
//! `Workspace` trait so the agent core can swap implementations (local FS,
//! sandboxed FS, remote, etc.) without changing tool code. All methods
//! operate on paths **relative to the workspace root**; absolute paths
//! outside the root are rejected by every implementation.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use thiserror::Error;

/// Per-file metadata returned by [`Workspace::stat`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileMeta {
    pub size: u64,
    /// Last-modified epoch milliseconds; 0 if unknown.
    pub mtime_ms: i64,
    pub is_dir: bool,
    pub readonly: bool,
}

/// Read result with line-range metadata.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReadResult {
    pub text: String,
    /// Bytes truncated for size safety.
    pub truncated: bool,
    /// Lines truncated for safety; 0 when fully read.
    pub truncated_lines: u32,
}

/// Errors from workspace operations.
#[derive(Debug, Error)]
pub enum WorkspaceError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("path '{0}' is outside workspace root '{1}'")]
    OutsideRoot(String, String),
    #[error("path '{0}' not found")]
    NotFound(String),
    #[error("path '{0}' is binary ({1} bytes)")]
    Binary(String, u64),
    #[error("path '{0}' not UTF-8")]
    NotUtf8(String),
    #[error("old text not found in '{0}' ({1} occurrence(s) found; expected {2})")]
    ReplaceMismatch(String, usize, u32),
    #[error("unsupported: {0}")]
    Unsupported(String),
}

pub type WorkspaceResult<T> = Result<T, WorkspaceError>;

/// Workspace trait. All paths are RELATIVE to the workspace root.
#[async_trait]
pub trait Workspace: Send + Sync {
    /// The workspace root (e.g. the directory the user opened).
    fn root(&self) -> &Path;

    /// Read a UTF-8 text file with optional line range.
    async fn read(&self, rel: &Path, offset_lines: u32, max_lines: u32, max_bytes: u32) -> WorkspaceResult<ReadResult>;

    /// Write (overwrite) a UTF-8 file, creating parents.
    async fn write(&self, rel: &Path, content: &str) -> WorkspaceResult<()>;

    /// Replace a literal substring inside a file. `all_occurrences` is
    /// required to be exact: failure means the file was not what the caller
    /// expected, and we do not silently change the wrong number of sites.
    async fn replace_in_file(&self, rel: &Path, old: &str, new: &str, all_occurrences: bool) -> WorkspaceResult<()>;

    /// List a directory (non-recursive).
    async fn list(&self, rel: &Path) -> WorkspaceResult<Vec<FileEntry>>;

    /// File metadata; None if the file does not exist.
    async fn stat(&self, rel: &Path) -> WorkspaceResult<Option<FileMeta>>;

    /// Recursive file search by base name (no content).
    async fn find_files(&self, name: &str, max: usize) -> WorkspaceResult<Vec<PathBuf>>;

    /// Recursive content search (case-insensitive literal substring).
    async fn search_content(&self, query: &str, max_hits: usize) -> WorkspaceResult<Vec<SearchHit>>;
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileEntry {
    pub rel: PathBuf,
    pub meta: FileMeta,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SearchHit {
    pub rel: PathBuf,
    pub line_no: u32,
    pub line: String,
}

/// Local-filesystem implementation of [`Workspace`]. All paths are sandboxed
/// to the workspace root (no escape via `..` or absolute).
pub struct LocalWorkspace {
    root: PathBuf,
}

impl LocalWorkspace {
    pub fn new(root: PathBuf) -> WorkspaceResult<Self> {
        let canon = std::fs::canonicalize(&root)
            .map_err(WorkspaceError::from)?;
        Ok(Self { root: canon })
    }

    /// Resolve and sandbox `rel` to the root. Returns the absolute path on
    /// success; rejects escapes.
    fn resolve(&self, rel: &Path) -> WorkspaceResult<PathBuf> {
        let joined = if rel.is_absolute() { rel.to_path_buf() } else { self.root.join(rel) };
        let canon = std::fs::canonicalize(&joined).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                WorkspaceError::NotFound(rel.display().to_string())
            } else {
                WorkspaceError::Io(e)
            }
        })?;
        if !canon.starts_with(&self.root) {
            return Err(WorkspaceError::OutsideRoot(canon.display().to_string(), self.root.display().to_string()));
        }
        Ok(canon)
    }

    /// Resolve a path that may not exist yet (for write). Same sandbox.
    fn resolve_allow_missing(&self, rel: &Path) -> WorkspaceResult<PathBuf> {
        let joined = if rel.is_absolute() { rel.to_path_buf() } else { self.root.join(rel) };
        let parent = joined.parent().unwrap_or(&self.root);
        let canon_parent = std::fs::canonicalize(parent)
            .map_err(|_| WorkspaceError::NotFound(joined.display().to_string()))?;
        let canon = if joined.exists() {
            std::fs::canonicalize(&joined).map_err(WorkspaceError::from)?
        } else {
            canon_parent.join(joined.file_name().unwrap_or_default())
        };
        if !canon.starts_with(&self.root) {
            return Err(WorkspaceError::OutsideRoot(canon.display().to_string(), self.root.display().to_string()));
        }
        Ok(canon)
    }

    fn detect_mime(path: &Path) -> bool {
        if let Some(ext) = path.extension().and_then(|s| s.to_str()) {
            // Very small binary extension list. Real implementation would
            // use the `infer` crate; we keep it inline to avoid new deps.
            matches!(ext.to_ascii_lowercase().as_str(),
                "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "ico" |
                "pdf" | "zip" | "tar" | "gz" | "tgz" | "xz" | "7z" | "rar" |
                "mp3" | "mp4" | "m4a" | "wav" | "ogg" | "flac" |
                "ttf" | "otf" | "woff" | "woff2" |
                "class" | "jar" | "so" | "dll" | "dylib" | "wasm" | "bin" | "exe")
        } else {
            false
        }
    }
}

#[async_trait]
impl Workspace for LocalWorkspace {
    fn root(&self) -> &Path { &self.root }

    async fn read(&self, rel: &Path, offset_lines: u32, max_lines: u32, max_bytes: u32) -> WorkspaceResult<ReadResult> {
        let p = self.resolve(rel)?;
        if !p.is_file() {
            return Err(WorkspaceError::Unsupported(format!("not a file: {}", p.display())));
        }
        if Self::detect_mime(&p) {
            let size = std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
            return Err(WorkspaceError::Binary(p.display().to_string(), size));
        }
        let bytes = std::fs::read(&p)?;
        let text = String::from_utf8(bytes.clone())
            .map_err(|_| WorkspaceError::NotUtf8(p.display().to_string()))?;
        let lines: Vec<&str> = text.lines().collect();
        let start = (offset_lines as usize).min(lines.len());
        let end = if max_lines == 0 { lines.len() } else { (start + max_lines as usize).min(lines.len()) };
        let slice: Vec<&str> = lines[start..end].to_vec();
        let mut out = slice.join("\n");
        let mut truncated_lines = 0;
        if end < lines.len() {
            truncated_lines = (lines.len() - end) as u32;
        }
        let mut truncated = false;
        if max_bytes > 0 && out.len() > max_bytes as usize {
            out.truncate(max_bytes as usize);
            truncated = true;
        }
        Ok(ReadResult { text: out, truncated, truncated_lines })
    }

    async fn write(&self, rel: &Path, content: &str) -> WorkspaceResult<()> {
        let p = self.resolve_allow_missing(rel)?;
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&p, content)?;
        Ok(())
    }

    async fn replace_in_file(&self, rel: &Path, old: &str, new: &str, all_occurrences: bool) -> WorkspaceResult<()> {
        let p = self.resolve(rel)?;
        let text = std::fs::read_to_string(&p)?;
        let count = text.matches(old).count();
        let expected = if all_occurrences { count } else { 1 };
        if expected == 0 || (all_occurrences && count == 0) {
            return Err(WorkspaceError::ReplaceMismatch(p.display().to_string(), count, count as u32));
        }
        if all_occurrences {
            let new_text = text.replace(old, new);
            std::fs::write(&p, new_text)?;
        } else {
            // Exactly one occurrence — make sure it is exactly one, not zero or many.
            if count != 1 {
                return Err(WorkspaceError::ReplaceMismatch(p.display().to_string(), count, 1));
            }
            let new_text = text.replacen(old, new, 1);
            std::fs::write(&p, new_text)?;
        }
        Ok(())
    }

    async fn list(&self, rel: &Path) -> WorkspaceResult<Vec<FileEntry>> {
        let p = self.resolve(rel)?;
        if !p.is_dir() {
            return Err(WorkspaceError::Unsupported(format!("not a directory: {}", p.display())));
        }
        let mut out = Vec::new();
        for entry in std::fs::read_dir(&p)? {
            let e = entry?;
            let meta = match e.file_type() {
                Ok(t) if t.is_dir() => FileMeta { size: 0, mtime_ms: mtime_ms(&e.path()), is_dir: true, readonly: false },
                Ok(_t) => {
                    let md = e.metadata().map_err(WorkspaceError::from)?;
                    FileMeta {
                        size: md.len(),
                        mtime_ms: mtime_ms(&e.path()),
                        is_dir: false,
                        readonly: md.permissions().readonly(),
                    }
                },
                Err(_) => continue,
            };
            let rel_path = e.path().strip_prefix(&self.root).unwrap_or(&e.path()).to_path_buf();
            out.push(FileEntry { rel: rel_path, meta });
        }
        Ok(out)
    }

    async fn stat(&self, rel: &Path) -> WorkspaceResult<Option<FileMeta>> {
        let p = self.resolve(rel)?;
        match std::fs::metadata(&p) {
            Ok(m) => Ok(Some(FileMeta {
                size: m.len(),
                mtime_ms: mtime_ms(&p),
                is_dir: m.is_dir(),
                readonly: m.permissions().readonly(),
            })),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(WorkspaceError::Io(e)),
        }
    }

    async fn find_files(&self, name: &str, max: usize) -> WorkspaceResult<Vec<PathBuf>> {
        let mut out = Vec::new();
        walk(&self.root, &self.root, name, max, &mut out)?;
        Ok(out)
    }

    async fn search_content(&self, query: &str, max_hits: usize) -> WorkspaceResult<Vec<SearchHit>> {
        let mut out = Vec::new();
        let needle = query.to_ascii_lowercase();
        search_walk(&self.root, &self.root, &needle, max_hits, &mut out)?;
        Ok(out)
    }
}

fn mtime_ms(p: &Path) -> i64 {
    std::fs::metadata(p)
        .and_then(|m| m.modified())
        .map(|t| t.duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0))
        .unwrap_or(0)
}

fn walk(root: &Path, dir: &Path, name: &str, max: usize, out: &mut Vec<PathBuf>) -> WorkspaceResult<()> {
    if out.len() >= max {
        return Ok(());
    }
    for entry in std::fs::read_dir(dir)? {
        let e = entry?;
        let p = e.path();
        if let Some(n) = p.file_name().and_then(|s| s.to_str()) {
            if matches!(n, ".git" | "target" | "node_modules" | ".aether" | "dist" | "build") {
                continue;
            }
        }
        if p.is_dir() {
            walk(root, &p, name, max, out)?;
        } else if p.is_file() {
            if let Some(n) = p.file_name().and_then(|s| s.to_str()) {
                if n == name {
                    let rel = p.strip_prefix(root).unwrap_or(&p).to_path_buf();
                    out.push(rel);
                    if out.len() >= max { return Ok(()); }
                }
            }
        }
    }
    Ok(())
}

fn search_walk(root: &Path, dir: &Path, needle: &str, max_hits: usize, out: &mut Vec<SearchHit>) -> WorkspaceResult<()> {
    if out.len() >= max_hits {
        return Ok(());
    }
    for entry in std::fs::read_dir(dir)? {
        let e = entry?;
        let p = e.path();
        if let Some(n) = p.file_name().and_then(|s| s.to_str()) {
            if matches!(n, ".git" | "target" | "node_modules" | ".aether" | "dist" | "build") {
                continue;
            }
        }
        if p.is_dir() {
            search_walk(root, &p, needle, max_hits, out)?;
        } else if p.is_file() {
            if LocalWorkspace::detect_mime(&p) {
                continue;
            }
            if let Ok(text) = std::fs::read_to_string(&p) {
                for (i, line) in text.lines().enumerate() {
                    if line.to_ascii_lowercase().contains(needle) {
                        let rel = p.strip_prefix(root).unwrap_or(&p).to_path_buf();
                        out.push(SearchHit { rel, line_no: (i as u32) + 1, line: line.to_string() });
                        if out.len() >= max_hits { return Ok(()); }
                    }
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn make_workspace(tag: &str) -> LocalWorkspace {
        let p = std::env::temp_dir().join(format!("aether-ws-test-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).unwrap();
        LocalWorkspace::new(p).unwrap()
    }

    #[tokio::test]
    async fn read_write_replace_work() {
        let ws = make_workspace("rw");
        ws.write(std::path::Path::new("a.txt"), "hello world\nsecond line").await.unwrap();
        let r = ws.read(std::path::Path::new("a.txt"), 0, 0, 0).await.unwrap();
        assert!(r.text.contains("hello world"));
        assert!(!r.truncated);
        ws.replace_in_file(std::path::Path::new("a.txt"), "hello world", "hi there", false).await.unwrap();
        let r2 = ws.read(std::path::Path::new("a.txt"), 0, 0, 0).await.unwrap();
        assert!(r2.text.starts_with("hi there"));
    }

    #[tokio::test]
    async fn replace_requires_exact_match() {
        let ws = make_workspace("re");
        ws.write(std::path::Path::new("a.txt"), "foo foo").await.unwrap();
        // 2 occurrences but all_occurrences=false -> error
        assert!(ws.replace_in_file(std::path::Path::new("a.txt"), "foo", "bar", false).await.is_err());
        // all_occurrences=true -> succeeds
        ws.replace_in_file(std::path::Path::new("a.txt"), "foo", "bar", true).await.unwrap();
    }

    #[tokio::test]
    async fn sandbox_rejects_escape() {
        let ws = make_workspace("sb");
        ws.write(std::path::Path::new("inside.txt"), "x").await.unwrap();
        assert!(ws.read(std::path::Path::new("../escape.txt"), 0, 0, 0).await.is_err());
    }

    #[tokio::test]
    async fn binary_files_rejected() {
        let ws = make_workspace("bin");
        let p = std::path::Path::new("image.png");
        let abs = ws.root().join(p);
        fs::write(&abs, [0u8, 1, 2, 3, 0, 0]).unwrap();
        let err = ws.read(p, 0, 0, 0).await.unwrap_err();
        match err {
            WorkspaceError::Binary(_, _) => {}
            _ => panic!("expected Binary, got {err:?}"),
        }
    }

    #[tokio::test]
    async fn find_files_works() {
        let ws = make_workspace("ff");
        fs::write(ws.root().join("a.txt"), "x").unwrap();
        fs::create_dir_all(ws.root().join("sub")).unwrap();
        fs::write(ws.root().join("sub").join("a.txt"), "x").unwrap();
        let r = ws.find_files("a.txt", 10).await.unwrap();
        assert_eq!(r.len(), 2);
    }

    #[tokio::test]
    async fn search_content_case_insensitive() {
        let ws = make_workspace("sc");
        fs::write(ws.root().join("a.txt"), "Hello\nWORLD\nfoo").unwrap();
        let r = ws.search_content("world", 10).await.unwrap();
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].line_no, 2);
    }
}
