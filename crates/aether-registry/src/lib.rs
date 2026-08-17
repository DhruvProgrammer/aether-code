//! `aether-registry` — provider registry, model catalog and health checker.
//!
//! The first-class replacement for the four-line provider config block in
//! v0.11. Adds:
//!   * **Provider descriptors** with full authentication, headers, env-var,
//!     base-URL, capabilities, limits, pricing, availability and live
//!     health-status fields.
//!   * **Model catalog** with the capability matrix that the controller
//!     consults when routing.
//!   * **HealthChecker** that probes a provider end-to-end before the user
//!     is allowed to mark it active.
//!   * **Registry** that the rest of the runtime consults instead of the
//!     flat `HashMap<String, ModelConfig>` it used to.
//!
//! Design contract: **no provider-specific logic leaks out of this crate**.
//! Callers ask the registry for "a provider for model X" and get back an
//! `Arc<dyn ModelProvider>`. The catalog decides which provider hosts a
//! given model. The health checker decides whether that provider is
//! currently trustworthy.

pub mod catalog;
pub mod provider;
pub mod registry;
pub mod health;
pub mod capabilities;

pub use capabilities::{Capability, CapabilityMatrix};
pub use catalog::{ModelDescriptor, ModelStatus};
pub use provider::{AuthConfig, ProviderDescriptor, ProviderStatus};
pub use registry::{ProviderRegistry, RegistryError};
pub use health::{HealthCheck, HealthChecker, HealthOutcome};
