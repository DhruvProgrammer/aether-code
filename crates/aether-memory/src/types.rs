//! Typed memory records (task-spec §11–14). `MemoryType` is a closed enum —
//! unlike the legacy free-form `kind: String`, an unknown category is a
//! compile error, not silent miscategorization.

use serde::{Deserialize, Serialize};

/// Engineering-meaningful memory categories (§12). Kept to the minimum
/// useful set; extend only with justification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryType {
    Requirement,
    Constraint,
    Decision,
    Plan,
    Task,
    Episode,
    Implementation,
    FileInsight,
    ToolResult,
    Error,
    Bug,
    Fix,
    Verification,
    Dependency,
    UserCorrection,
    ImportantFact,
    OpenQuestion,
}

impl MemoryType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Requirement => "requirement",
            Self::Constraint => "constraint",
            Self::Decision => "decision",
            Self::Plan => "plan",
            Self::Task => "task",
            Self::Episode => "episode",
            Self::Implementation => "implementation",
            Self::FileInsight => "file_insight",
            Self::ToolResult => "tool_result",
            Self::Error => "error",
            Self::Bug => "bug",
            Self::Fix => "fix",
            Self::Verification => "verification",
            Self::Dependency => "dependency",
            Self::UserCorrection => "user_correction",
            Self::ImportantFact => "important_fact",
            Self::OpenQuestion => "open_question",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        Some(match s {
            "requirement" => Self::Requirement,
            "constraint" => Self::Constraint,
            "decision" => Self::Decision,
            "plan" => Self::Plan,
            "task" => Self::Task,
            "episode" => Self::Episode,
            "implementation" => Self::Implementation,
            "file_insight" => Self::FileInsight,
            "tool_result" => Self::ToolResult,
            "error" => Self::Error,
            "bug" => Self::Bug,
            "fix" => Self::Fix,
            "verification" => Self::Verification,
            "dependency" => Self::Dependency,
            "user_correction" => Self::UserCorrection,
            "important_fact" => Self::ImportantFact,
            "open_question" => Self::OpenQuestion,
            _ => return None,
        })
    }

    pub fn all() -> &'static [MemoryType] {
        use MemoryType::*;
        &[
            Requirement,
            Constraint,
            Decision,
            Plan,
            Task,
            Episode,
            Implementation,
            FileInsight,
            ToolResult,
            Error,
            Bug,
            Fix,
            Verification,
            Dependency,
            UserCorrection,
            ImportantFact,
            OpenQuestion,
        ]
    }
}

/// Lifecycle states (§42). Raw history is never deleted; derived records
/// move through these states instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryStatus {
    Candidate,
    Active,
    Superseded,
    Stale,
    Archived,
    Deleted,
}

impl MemoryStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Candidate => "candidate",
            Self::Active => "active",
            Self::Superseded => "superseded",
            Self::Stale => "stale",
            Self::Archived => "archived",
            Self::Deleted => "deleted",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        Some(match s {
            "candidate" => Self::Candidate,
            "active" => Self::Active,
            "superseded" => Self::Superseded,
            "stale" => Self::Stale,
            "archived" => Self::Archived,
            "deleted" => Self::Deleted,
            _ => return None,
        })
    }

    /// Retrievable in normal (non-history) queries.
    pub fn is_usable(self) -> bool {
        matches!(self, Self::Candidate | Self::Active)
    }
}

/// Typed edges between records (§14).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Relation {
    Supersedes,
    SupersededBy,
    DerivedFrom,
    Contradicts,
    DependsOn,
    RelatedTo,
}

impl Relation {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Supersedes => "supersedes",
            Self::SupersededBy => "superseded_by",
            Self::DerivedFrom => "derived_from",
            Self::Contradicts => "contradicts",
            Self::DependsOn => "depends_on",
            Self::RelatedTo => "related_to",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        Some(match s {
            "supersedes" => Self::Supersedes,
            "superseded_by" => Self::SupersededBy,
            "derived_from" => Self::DerivedFrom,
            "contradicts" => Self::Contradicts,
            "depends_on" => Self::DependsOn,
            "related_to" => Self::RelatedTo,
            _ => return None,
        })
    }

    /// Inverse edge, recorded so traversal works in both directions.
    pub fn inverse(self) -> Self {
        match self {
            Self::Supersedes => Self::SupersededBy,
            Self::SupersededBy => Self::Supersedes,
            Self::DerivedFrom => Self::RelatedTo,
            Self::Contradicts => Self::Contradicts,
            Self::DependsOn => Self::RelatedTo,
            Self::RelatedTo => Self::RelatedTo,
        }
    }
}

/// One searchable memory unit. Field-for-field the schema in
/// `MEMORY_SCHEMA.md`; see that doc for the rationale of each column.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryRecord {
    pub id: String,
    pub project_id: String,
    pub session_id: Option<String>,
    pub task_id: Option<String>,
    pub mem_type: MemoryType,
    pub title: String,
    pub content: String,
    pub summary: Option<String>,
    pub source_ids: Vec<String>,
    pub files: Vec<String>,
    pub symbols: Vec<String>,
    pub tags: Vec<String>,
    pub importance: f32,
    pub confidence: f32,
    pub status: MemoryStatus,
    pub supersedes: Vec<String>,
    pub superseded_by: Vec<String>,
    pub parent_id: Option<String>,
    pub related_ids: Vec<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

impl MemoryRecord {
    pub fn new(
        project_id: impl Into<String>,
        mem_type: MemoryType,
        title: impl Into<String>,
        content: impl Into<String>,
    ) -> Self {
        let now = chrono::Utc::now().timestamp();
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            project_id: project_id.into(),
            session_id: None,
            task_id: None,
            mem_type,
            title: title.into(),
            content: content.into(),
            summary: None,
            source_ids: Vec::new(),
            files: Vec::new(),
            symbols: Vec::new(),
            tags: Vec::new(),
            importance: 0.5,
            confidence: 0.5,
            status: MemoryStatus::Active,
            supersedes: Vec::new(),
            superseded_by: Vec::new(),
            parent_id: None,
            related_ids: Vec::new(),
            created_at: now,
            updated_at: now,
        }
    }

    /// Deduplication fingerprint over normalized content (§59).
    pub fn content_hash(&self) -> String {
        let mut norm = self.content.to_lowercase();
        norm.retain(|c| !c.is_whitespace());
        format!("{:016x}", fnv1a(&norm))
    }
}

fn fnv1a(s: &str) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn type_roundtrip_covers_all() {
        for t in MemoryType::all() {
            assert_eq!(MemoryType::from_str(t.as_str()), Some(*t));
        }
        assert_eq!(MemoryType::all().len(), 17);
        assert!(MemoryType::from_str("chunk").is_none());
    }

    #[test]
    fn status_usability() {
        assert!(MemoryStatus::Active.is_usable());
        assert!(MemoryStatus::Candidate.is_usable());
        assert!(!MemoryStatus::Superseded.is_usable());
        assert!(!MemoryStatus::Stale.is_usable());
        assert!(!MemoryStatus::Deleted.is_usable());
    }

    #[test]
    fn relation_inverse_roundtrip() {
        assert_eq!(Relation::Supersedes.inverse(), Relation::SupersededBy);
        assert_eq!(
            Relation::Supersedes.inverse().inverse(),
            Relation::Supersedes
        );
    }

    #[test]
    fn hash_stable_across_whitespace() {
        let mut a = MemoryRecord::new("p", MemoryType::Decision, "t", "Use  SQLite\n now");
        let b = MemoryRecord::new("p", MemoryType::Decision, "t", "use sqlite now");
        assert_eq!(a.content_hash(), b.content_hash());
        a.content = "Use Postgres now".into();
        assert_ne!(a.content_hash(), b.content_hash());
    }
}
