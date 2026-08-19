//! AETHER Model Gateway.
//!
//! The single model-access layer for the whole runtime. Every component
//! (controller, executor, visual reviewer, skills, tools, plugins, context
//! manager, analysis integration) obtains LLM access **only** through this
//! gateway — never by talking to provider APIs directly.
//!
//! Architecture:
//!
//! ```text
//! AETHER Component
//!       ↓
//! Model Gateway
//!       ↓
//! Role Configuration (Model 1 / Model 2 / Model 3 — user chosen)
//!       ↓
//! Provider Adapter (OpenAI-compatible by default)
//!       ↓
//! Provider API
//! ```
//!
//! Non-goals (hard rules of this crate):
//! * NO routing — the gateway never picks a model. The user's per-role binding
//!   is used exactly as configured.
//! * NO automatic fallback — a provider failure is reported, never hidden.
//! * NO cost/latency/benchmark model selection.
//!
//! The gateway is an **abstraction layer**, not a routing engine.

pub mod role;
pub mod request;
pub mod capability;
pub mod error;
pub mod fingerprint;
pub mod validate;
pub mod store;
pub mod gateway;
pub mod config;

pub use role::{Role, RoleBinding};
pub use request::{GatewayRequest, GatewayResponse, Capability as RequestCapability};
pub use capability::{ModelCapabilities, CapabilityCheck};
pub use error::{GatewayError, FailureClass};
pub use fingerprint::{sha256_hex, fingerprint_binding, ModelRoleSnapshot};
pub use validate::{ValidationOutcome, validate_binding, ValidateTarget};
pub use store::{ValidationStore, RoleValidation};
pub use gateway::{ModelGateway, GatewayConfig, RoleProvider, classify_provider_error};
pub use config::GatewayBundle;
