//! Checkpoint persistence adapter: stores compaction checkpoints in the
//! session store's `kv` table, keyed per session. This guarantees session
//! isolation — Session A's checkpoint can never be read by Session B.
//!
//! rusqlite's `Connection` is `Send` but not `Sync`, so the adapter stores
//! the database path and opens a fresh connection per operation. Compaction
//! is rare, so the connection cost is negligible, and the adapter is trivially
//! `Send + Sync`.

use aether_context::{CheckpointStore, CompactionCheckpoint};
use aether_sessions::SessionStore;
use std::path::PathBuf;

const KV_KEY: &str = "compaction_checkpoint";

pub struct SessionCheckpointStore {
    db_path: PathBuf,
}

impl SessionCheckpointStore {
    pub fn new(db_path: PathBuf) -> Self {
        Self { db_path }
    }

    fn open(&self) -> Result<std::sync::Arc<SessionStore>, String> {
        SessionStore::open(&self.db_path).map_err(|e| e.to_string())
    }
}

impl CheckpointStore for SessionCheckpointStore {
    fn save_checkpoint(&self, session_id: &str, checkpoint: &CompactionCheckpoint) -> Result<(), String> {
        let json = serde_json::to_string(checkpoint).map_err(|e| e.to_string())?;
        let store = self.open()?;
        store.set_kv(session_id, KV_KEY, &json).map_err(|e| e.to_string())
    }

    fn load_checkpoint(&self, session_id: &str) -> Result<Option<CompactionCheckpoint>, String> {
        let store = self.open()?;
        match store.get_kv(session_id, KV_KEY).map_err(|e| e.to_string())? {
            Some(json) => {
                let cp: CompactionCheckpoint = serde_json::from_str(&json).map_err(|e| e.to_string())?;
                Ok(Some(cp))
            }
            None => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_path() -> PathBuf {
        std::env::temp_dir().join(format!(
            "aether-cp-{}.db",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn checkpoint_roundtrip_and_isolation() {
        let path = tmp_path();
        let store = SessionStore::open(&path).unwrap();
        let sid_a = store.new_session().unwrap();
        let sid_b = store.new_session().unwrap();
        drop(store);

        let adapter = SessionCheckpointStore::new(path);
        let mut cp = CompactionCheckpoint::default();
        cp.user_objective = "Build auth".into();
        cp.relevant_files = vec!["src/auth.ts".into()];

        adapter.save_checkpoint(&sid_a, &cp).unwrap();
        let loaded = adapter.load_checkpoint(&sid_a).unwrap().unwrap();
        assert_eq!(loaded.user_objective, "Build auth");
        assert_eq!(loaded.relevant_files, vec!["src/auth.ts"]);

        // Session B must not see Session A's checkpoint.
        assert!(adapter.load_checkpoint(&sid_b).unwrap().is_none());
    }
}
