//! Model capability metadata + pre-flight checks (gateway spec §7).
//!
//! Capabilities are **informational/descriptive** — they describe what the
//! explicitly configured model supports. The gateway uses them to *validate*
//! before dispatch and to produce a clear `unsupported_capability` error. It
//! NEVER uses them to pick a different model. Live validation
//! ([`crate::validate`]) remains authoritative.

use serde::{Deserialize, Serialize};

use crate::error::{FailureClass, GatewayError};
use crate::request::Capability;
use crate::role::Role;

/// What a configured model supports. Unknowns default to the conservative
/// (declared) defaults so we never over-promise.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelCapabilities {
    pub text: bool,
    pub tool_calling: bool,
    pub vision: bool,
    pub streaming: bool,
    pub structured_output: bool,
    /// Context window in tokens (informational).
    pub context_window: Option<u32>,
    /// Max output tokens (informational).
    pub max_output_tokens: Option<u32>,
}

impl Default for ModelCapabilities {
    fn default() -> Self {
        Self {
            text: true,
            tool_calling: true,
            vision: false,
            streaming: true,
            structured_output: false,
            context_window: None,
            max_output_tokens: None,
        }
    }
}

impl ModelCapabilities {
    /// Conservatively unknown model: assume text + streaming only.
    pub fn unknown() -> Self {
        Self { tool_calling: false, ..Default::default() }
    }

    /// Undeclared capabilities → assume all common features work. Live API
    /// validation remains the authoritative check; pre-flight only rejects
    /// operations we *know* the configured model cannot serve.
    pub fn permissive() -> Self {
        Self { vision: true, structured_output: false, ..Default::default() }
    }

    pub fn supports(&self, cap: Capability) -> bool {
        match cap {
            Capability::Text => self.text,
            Capability::ToolCalling => self.tool_calling,
            Capability::Vision => self.vision,
            Capability::Streaming => self.streaming,
        }
    }
}

/// Result of the pre-flight capability check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CapabilityCheck {
    Ok,
    Denied { class: FailureClass, reason: String },
}

/// Validate that the explicitly configured model supports the requested
/// capability for `role`. Returns a denial error rather than switching models.
pub fn precheck(
    caps: &ModelCapabilities,
    required: Capability,
    role: Role,
    wants_tools: bool,
    has_images: bool,
) -> Result<(), GatewayError> {
    // Vision input requires the vision capability.
    if has_images && !caps.vision {
        return Err(GatewayError::CapabilityDenied {
            role: role.as_str().into(),
            class: FailureClass::UnsupportedCapability,
            detail: format!(
                "the configured model for {} does not support image input",
                role.display_name()
            ),
        });
    }
    // Tool calling is only required when tools are actually supplied.
    if wants_tools && required == Capability::ToolCalling && !caps.tool_calling {
        return Err(GatewayError::CapabilityDenied {
            role: role.as_str().into(),
            class: FailureClass::UnsupportedCapability,
            detail: format!(
                "the configured model for {} does not support tool calling",
                role.display_name()
            ),
        });
    }
    if required == Capability::Vision && !caps.vision {
        return Err(GatewayError::CapabilityDenied {
            role: role.as_str().into(),
            class: FailureClass::UnsupportedCapability,
            detail: format!(
                "the configured model for {} does not support vision",
                role.display_name()
            ),
        });
    }
    if !caps.supports(Capability::Text) {
        return Err(GatewayError::CapabilityDenied {
            role: role.as_str().into(),
            class: FailureClass::UnsupportedCapability,
            detail: "the configured model does not support text completion".into(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_caps_allow_text_and_tools_but_not_vision() {
        let c = ModelCapabilities::default();
        assert!(c.supports(Capability::Text));
        assert!(c.supports(Capability::ToolCalling));
        assert!(!c.supports(Capability::Vision));
    }

    #[test]
    fn precheck_passes_for_text() {
        let c = ModelCapabilities::default();
        assert!(precheck(&c, Capability::Text, Role::Controller, false, false).is_ok());
    }

    #[test]
    fn precheck_denies_vision_without_capability() {
        let c = ModelCapabilities::default(); // vision = false
        let err = precheck(&c, Capability::Text, Role::Reviewer, false, true).unwrap_err();
        assert_eq!(err.class(), FailureClass::UnsupportedCapability);
        assert!(err.to_string().contains("image input"));
    }

    #[test]
    fn precheck_denies_tool_calling_when_unsupported() {
        let c = ModelCapabilities::unknown(); // tool_calling = false
        let err = precheck(&c, Capability::ToolCalling, Role::Executor, true, false).unwrap_err();
        assert_eq!(err.class(), FailureClass::UnsupportedCapability);
    }

    #[test]
    fn cap_roundtrip() {
        let c = ModelCapabilities { vision: true, context_window: Some(128_000), ..Default::default() };
        let j = serde_json::to_string(&c).unwrap();
        let back: ModelCapabilities = serde_json::from_str(&j).unwrap();
        assert!(back.vision);
        assert_eq!(back.context_window, Some(128_000));
    }

    #[test]
    fn permissive_assumes_undeclared_ok() {
        let c = ModelCapabilities::permissive();
        assert!(c.vision);
        assert!(c.tool_calling);
        assert!(c.streaming);
        assert!(precheck(&c, Capability::Vision, Role::Reviewer, false, true).is_ok());
    }
}
