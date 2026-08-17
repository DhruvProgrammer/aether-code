//! Capability matrix — what a model can and cannot do.

use serde::{Deserialize, Serialize};

/// Single capability flag.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    ToolCalling,
    Vision,
    ImageInput,
    AudioInput,
    Reasoning,
    StructuredOutput,
    JsonMode,
    Streaming,
    ParallelToolCalls,
    PromptCaching,
    ComputerUse,
    Embeddings,
}

impl Capability {
    pub fn label(self) -> &'static str {
        match self {
            Self::ToolCalling => "tool_calling",
            Self::Vision => "vision",
            Self::ImageInput => "image_input",
            Self::AudioInput => "audio_input",
            Self::Reasoning => "reasoning",
            Self::StructuredOutput => "structured_output",
            Self::JsonMode => "json_mode",
            Self::Streaming => "streaming",
            Self::ParallelToolCalls => "parallel_tool_calls",
            Self::PromptCaching => "prompt_caching",
            Self::ComputerUse => "computer_use",
            Self::Embeddings => "embeddings",
        }
    }
}

/// Capability vector for a model. Booleans + numeric limits.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CapabilityMatrix {
    pub flags: Vec<Capability>,
    pub context_window: u32,
    pub max_output_tokens: u32,
    pub input_cost_per_mtok: Option<f64>,
    pub output_cost_per_mtok: Option<f64>,
}

impl CapabilityMatrix {
    pub fn has(&self, c: Capability) -> bool {
        self.flags.iter().any(|x| *x == c)
    }

    pub fn with(mut self, c: Capability) -> Self {
        if !self.flags.contains(&c) {
            self.flags.push(c);
        }
        self
    }

    pub fn with_context(mut self, ctx: u32) -> Self { self.context_window = ctx; self }
    pub fn with_max_output(mut self, m: u32) -> Self { self.max_output_tokens = m; self }
    pub fn with_input_cost(mut self, c: f64) -> Self { self.input_cost_per_mtok = Some(c); self }
    pub fn with_output_cost(mut self, c: f64) -> Self { self.output_cost_per_mtok = Some(c); self }
}
