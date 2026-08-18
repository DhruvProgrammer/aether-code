//! Capability matrix for a model.
//!
//! A [`ModelCapabilities`] is a set of typed [`Capability`] flags. The router
//! uses it to verify a model can fulfil the requirements of a given task
//! before selecting it.

use serde::{Deserialize, Serialize};

/// A single capability flag.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    /// Tool / function calling.
    ToolCalling,
    /// Image / vision input.
    Vision,
    /// Image input (image-specific, distinct from generic vision).
    ImageInput,
    /// Audio input.
    AudioInput,
    /// Reasoning effort / extended thinking.
    Reasoning,
    /// Structured output (JSON schema).
    StructuredOutput,
    /// JSON mode.
    JsonMode,
    /// Server-sent event streaming.
    Streaming,
    /// Parallel tool calls in a single turn.
    ParallelToolCalls,
    /// Prompt caching (provider-side cache).
    PromptCaching,
    /// Computer use.
    ComputerUse,
    /// Embeddings generation.
    Embeddings,
}

/// The full set of capabilities for a single model, plus the model's
/// numeric limits.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelCapabilities {
    pub flags: Vec<Capability>,
    pub context_window: u32,
    pub max_output_tokens: u32,
}

impl ModelCapabilities {
    pub fn new() -> Self {
        Self {
            flags: Vec::new(),
            context_window: 0,
            max_output_tokens: 0,
        }
    }

    pub fn with(mut self, c: Capability) -> Self {
        if !self.flags.contains(&c) {
            self.flags.push(c);
        }
        self
    }

    pub fn with_window(mut self, n: u32) -> Self {
        self.context_window = n;
        self
    }

    pub fn with_output(mut self, n: u32) -> Self {
        self.max_output_tokens = n;
        self
    }

    pub fn has(&self, c: Capability) -> bool {
        self.flags.contains(&c)
    }

    /// Returns true if this model satisfies ALL `required` capabilities.
    pub fn satisfies(&self, required: &[Capability]) -> bool {
        required.iter().all(|c| self.has(*c))
    }

    /// Fits the requested context size?
    pub fn fits_context(&self, tokens: u32) -> bool {
        self.context_window == 0 || tokens <= self.context_window
    }
}

impl Default for ModelCapabilities {
    fn default() -> Self {
        Self::new()
    }
}

/// Re-export alias so callers can use either name.
pub type CapabilityMatrix = ModelCapabilities;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn satisfies_all_required() {
        let caps = ModelCapabilities::new()
            .with(Capability::ToolCalling)
            .with(Capability::Streaming)
            .with(Capability::JsonMode);
        assert!(caps.satisfies(&[Capability::ToolCalling, Capability::Streaming]));
        assert!(!caps.satisfies(&[Capability::Vision]));
    }

    #[test]
    fn fits_context() {
        let caps = ModelCapabilities::new().with_window(8_000);
        assert!(caps.fits_context(8_000));
        assert!(!caps.fits_context(8_001));
        // Window of 0 means "unknown / unbounded".
        let unbounded = ModelCapabilities::new();
        assert!(unbounded.fits_context(1_000_000));
    }

    #[test]
    fn with_is_idempotent() {
        let caps = ModelCapabilities::new()
            .with(Capability::ToolCalling)
            .with(Capability::ToolCalling);
        assert_eq!(caps.flags.len(), 1);
    }
}
