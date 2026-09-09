//! Dynamic skill/context loading for AETHER.
//!
//! The large software-engineering skill lives as external structured data
//! (`skills/software-engineering/`). This crate loads only relevant sections
//! per task, compiles the prompt within the model's budget, and keeps the
//! permanent system prompt small (300-800 tokens).
//!
//! Pipeline: `classify` task -> `retrieve` sections -> `compile` prompt.

pub mod classify;
pub mod compile;
pub mod index;
pub mod kernel;
pub mod reference;
pub mod retrieve;

pub use classify::{classify, TaskKind};
pub use compile::{CompiledPrompt, ContextBreakdown, PromptCompiler, PromptCompilerInput};
pub use index::{SectionMeta, SkillIndex};
pub use kernel::{role_prompt, Role, SYSTEM_KERNEL};
pub use reference::{ReferenceIndex, ReferenceItem, ReferenceRetriever};
pub use retrieve::{RetrievedSection, SkillRetriever};
