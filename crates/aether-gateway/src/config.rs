//! Gateway assembly from `config.toml` (spec §3, §17) and from the v0.17
//! provider registry with per-session role assignments.
//!
//! The single place where AETHER's three roles are bound to concrete
//! providers. Bindings come from the `[agent]` role keys (model1/model2/
//! model3) pointing into the `[models]` map, or from explicit per-session
//! role assignments referencing the provider registry. Each role gets its own
//! provider instance — no shared hidden routing.

use std::collections::HashMap;
use std::sync::Arc;

use aether_config::{Config, ProviderEntry, RoleAssignments};
use aether_models::ModelProvider;

use crate::capability::ModelCapabilities;
use crate::error::{FailureClass, GatewayError};
use crate::gateway::{GatewayConfig, ModelGateway};
use crate::role::{Role, RoleBinding};
use crate::validate::ValidateTarget;

/// Everything the runtime needs after gateway assembly: the gateway itself,
/// the provider map (for components that still talk providers directly during
/// migration), and the controller provider.
pub struct GatewayBundle {
    pub gateway: ModelGateway,
    pub providers: HashMap<String, Arc<dyn ModelProvider>>,
    pub controller: Arc<dyn ModelProvider>,
}

impl std::fmt::Debug for GatewayBundle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GatewayBundle")
            .field("installed_roles", &self.gateway.installed())
            .field("provider_keys", &self.providers.keys().collect::<Vec<_>>())
            .finish()
    }
}

impl GatewayBundle {
    /// Assemble role bindings from config and build providers.
    ///
    /// * Model 1 (executor) and Model 2 (controller) are required — a missing
    ///   binding is a configuration error that aborts startup (§17).
    /// * Model 3 (reviewer) is optional and degrades gracefully.
    pub fn from_config(cfg: &Config, gateway_config: GatewayConfig) -> Result<Self, GatewayError> {
        let mut bindings = Vec::new();

        // Model 2 — Small Controller (required).
        let (ctrl_key, _ctrl_cfg) = match cfg.agent.model2.as_deref().map(|k| (k, cfg.model(k))) {
            Some((k, Some(mc))) => (k, mc),
            _ => {
                let k = cfg.agent.controller_model.as_str();
                match cfg.model(k) {
                    Some(mc) => (k, mc),
                    None => {
                        return Err(GatewayError::CapabilityDenied {
                            role: Role::Controller.as_str().into(),
                            class: FailureClass::UnknownProvider,
                            detail: "controller model key has no [models] entry — set model2 in [agent]".into(),
                        })
                    }
                }
            }
        };
        bindings.push(RoleBinding::new(Role::Controller, ctrl_key));

        // Model 1 — Big Executor (required).
        let (exec_key, _exec_cfg) = match cfg.agent.model1.as_deref().map(|k| (k, cfg.model(k))) {
            Some((k, Some(mc))) => (k, mc),
            _ => {
                let k = cfg.agent.executor_model.as_str();
                match cfg.model(k) {
                    Some(mc) => (k, mc),
                    None => {
                        return Err(GatewayError::CapabilityDenied {
                            role: Role::Executor.as_str().into(),
                            class: FailureClass::UnknownProvider,
                            detail: "executor model key has no [models] entry — set model1 in [agent]".into(),
                        })
                    }
                }
            }
        };
        bindings.push(RoleBinding::new(Role::Executor, exec_key));

        // Model 3 — Visual Frontend Reviewer (optional).
        if let Some(key) = cfg.agent.model3.as_deref().or(cfg.agent.reviewer_model.as_deref()) {
            if let Some(_mc) = cfg.model(key) {
                bindings.push(RoleBinding::new(Role::Reviewer, key));
            }
        }

        // Build providers for every referenced key (executor, controller,
        // reviewer, plus any extra keys used by memory/embeddings).
        let mut providers: HashMap<String, Arc<dyn ModelProvider>> = HashMap::new();
        let mut model_ids: HashMap<String, String> = HashMap::new();
        let mut provider_ids: HashMap<String, String> = HashMap::new();
        let mut capabilities: HashMap<String, ModelCapabilities> = HashMap::new();
        for (key, mc) in &cfg.models {
            match aether_models::build_provider(mc) {
                Ok(p) => {
                    providers.insert(key.clone(), Arc::from(p));
                    model_ids.insert(key.clone(), mc.model.clone());
                    provider_ids.insert(key.clone(), mc.provider.clone());
                    // Undeclared capabilities → permissive defaults. Live API
                    // validation is the authoritative check (§10).
                    capabilities.insert(key.clone(), ModelCapabilities::permissive());
                }
                Err(e) => {
                    if key == ctrl_key || key == exec_key {
                        return Err(crate::gateway::classify_provider_error(e));
                    }
                    tracing::warn!("provider for model key '{key}' failed to build: {e}");
                }
            }
        }

        let gateway = ModelGateway::new(
            gateway_config,
            &bindings,
            &providers,
            &model_ids,
            &provider_ids,
            &capabilities,
        );

        let controller = providers
            .get(ctrl_key)
            .cloned()
            .ok_or_else(|| GatewayError::NotConfigured(Role::Controller.as_str().into()))?;

        Ok(Self { gateway, providers, controller })
    }

    /// Assemble role bindings from the v0.17 provider registry and a
    /// per-session [`RoleAssignments`]. This is the new architecture: the
    /// session explicitly chooses which provider/model performs each role.
    ///
    /// * Executor and Controller are required — a missing binding is a
    ///   configuration error that aborts startup.
    /// * Reviewer is optional and degrades gracefully.
    ///
    /// No routing, no fallback, no automatic selection. The user's explicit
    /// choice is executed.
    pub fn from_providers(
        registry: &[ProviderEntry],
        assignments: &RoleAssignments,
        gateway_config: GatewayConfig,
    ) -> Result<Self, GatewayError> {
        let find = |provider_id: &str, model_id: &str| -> Result<(String, aether_config::ModelConfig), GatewayError> {
            let prov = registry
                .iter()
                .find(|p| p.id == provider_id)
                .ok_or_else(|| GatewayError::CapabilityDenied {
                    role: provider_id.into(),
                    class: FailureClass::UnknownProvider,
                    detail: format!("provider '{provider_id}' not found in registry"),
                })?;
            let mc = prov.to_model_config(model_id).ok_or_else(|| GatewayError::CapabilityDenied {
                role: model_id.into(),
                class: FailureClass::UnknownProvider,
                detail: format!("model '{model_id}' not found in provider '{provider_id}'"),
            })?;
            Ok((format!("{provider_id}/{model_id}"), mc))
        };

        let mut bindings = Vec::new();
        let mut providers: HashMap<String, Arc<dyn ModelProvider>> = HashMap::new();
        let mut model_ids: HashMap<String, String> = HashMap::new();
        let mut provider_ids: HashMap<String, String> = HashMap::new();
        let mut capabilities: HashMap<String, ModelCapabilities> = HashMap::new();

        let mut install = |role: Role, key: String, mc: aether_config::ModelConfig| -> Result<(), GatewayError> {
            match aether_models::build_provider(&mc) {
                Ok(p) => {
                    providers.insert(key.clone(), Arc::from(p));
                    model_ids.insert(key.clone(), mc.model.clone());
                    provider_ids.insert(key.clone(), mc.provider.clone());
                    capabilities.insert(key.clone(), ModelCapabilities::permissive());
                    bindings.push(RoleBinding::new(role, key));
                    Ok(())
                }
                Err(e) => Err(crate::gateway::classify_provider_error(e)),
            }
        };

        // Executor (required).
        let exec = assignments.executor.as_ref().ok_or_else(|| GatewayError::CapabilityDenied {
            role: Role::Executor.as_str().into(),
            class: FailureClass::UnknownProvider,
            detail: "no executor model assigned to this session".into(),
        })?;
        let (exec_key, exec_mc) = find(&exec.provider_id, &exec.model_id)?;
        install(Role::Executor, exec_key, exec_mc)?;

        // Controller (required).
        let ctrl = assignments.controller.as_ref().ok_or_else(|| GatewayError::CapabilityDenied {
            role: Role::Controller.as_str().into(),
            class: FailureClass::UnknownProvider,
            detail: "no controller model assigned to this session".into(),
        })?;
        let (ctrl_key, ctrl_mc) = find(&ctrl.provider_id, &ctrl.model_id)?;
        install(Role::Controller, ctrl_key.clone(), ctrl_mc)?;

        // Reviewer (optional).
        if let Some(rev) = &assignments.reviewer {
            if let Ok((rev_key, rev_mc)) = find(&rev.provider_id, &rev.model_id) {
                let _ = install(Role::Reviewer, rev_key, rev_mc);
            }
        }

        let gateway = ModelGateway::new(
            gateway_config,
            &bindings,
            &providers,
            &model_ids,
            &provider_ids,
            &capabilities,
        );

        let controller = providers
            .get(&ctrl_key)
            .cloned()
            .ok_or_else(|| GatewayError::NotConfigured(Role::Controller.as_str().into()))?;

        Ok(Self { gateway, providers, controller })
    }

    /// Build a [`ValidateTarget`] for live API validation of one role's
    /// configured model. Returns None only if the role has no binding.
    pub fn validate_target(cfg: &Config, role: Role) -> Option<ValidateTarget> {
        let key = match role {
            Role::Controller => cfg
                .agent
                .model2
                .clone()
                .unwrap_or_else(|| cfg.agent.controller_model.clone()),
            Role::Executor => cfg
                .agent
                .model1
                .clone()
                .unwrap_or_else(|| cfg.agent.executor_model.clone()),
            Role::Reviewer => {
                cfg.agent.model3.clone().or(cfg.agent.reviewer_model.clone())?
            }
        };
        let mc = cfg.model(&key)?;
        Some(ValidateTarget {
            role,
            model_key: key,
            provider_id: mc.provider.clone(),
            base_url: mc.base_url.clone(),
            model_id: mc.model.clone(),
            api_key_env: mc.api_key_env.clone(),
            headers: mc.headers.clone(),
            extra_body: mc.extra_body.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aether_config::{AgentConfig, ModelConfig};

    fn test_config() -> Config {
        let mut cfg = Config::default();
        cfg.agent = AgentConfig {
            controller_model: "controller".into(),
            executor_model: "executor".into(),
            reviewer_model: Some("reviewer".into()),
            model1: Some("executor".into()),
            model2: Some("controller".into()),
            model3: Some("reviewer".into()),
            max_iterations: 30,
            loop_budget: 3,
            local_endpoint: "http://localhost:11434/v1".into(),
        };
        for (key, url) in [
            ("executor", "https://e.example/v1"),
            ("controller", "https://c.example/v1"),
            ("reviewer", "https://r.example/v1"),
        ] {
            std::env::set_var(format!("GW_BUNDLE_{key}"), "sk-test");
            cfg.models.insert(
                key.into(),
                ModelConfig {
                    provider: "openai_compatible".into(),
                    base_url: url.into(),
                    model: key.into(),
                    api_key_env: format!("GW_BUNDLE_{key}"),
                    headers: None,
                    extra_body: None,
                },
            );
        }
        cfg
    }

    #[test]
    fn from_config_builds_all_three_roles() {
        let cfg = test_config();
        let bundle = GatewayBundle::from_config(&cfg, GatewayConfig::default()).unwrap();
        assert_eq!(bundle.gateway.installed().len(), 3);
        assert!(bundle.gateway.is_configured(Role::Executor));
        assert!(bundle.gateway.is_configured(Role::Controller));
        assert!(bundle.gateway.is_configured(Role::Reviewer));
        assert_eq!(bundle.providers.len(), 3);
    }

    #[test]
    fn validate_target_resolves_role_to_config() {
        let cfg = test_config();
        let t = GatewayBundle::validate_target(&cfg, Role::Reviewer).unwrap();
        assert_eq!(t.model_key, "reviewer");
        assert_eq!(t.base_url, "https://r.example/v1");
        // Env var name only — never the key value.
        assert_eq!(t.api_key_env, "GW_BUNDLE_reviewer");
    }

    #[test]
    fn missing_executor_fails_startup() {
        let mut cfg = test_config();
        cfg.models.remove("executor");
        let err = GatewayBundle::from_config(&cfg, GatewayConfig::default()).unwrap_err();
        assert!(err.to_string().contains("executor"));
    }

    #[test]
    fn reviewer_is_optional() {
        let mut cfg = test_config();
        cfg.models.remove("reviewer");
        cfg.agent.reviewer_model = Some("reviewer".into());
        // Still assembles; reviewer just isn't installed.
        let bundle = GatewayBundle::from_config(&cfg, GatewayConfig::default()).unwrap();
        assert!(!bundle.gateway.is_configured(Role::Reviewer));
        assert!(bundle.gateway.is_configured(Role::Executor));
    }

    // ---- v0.17 from_providers tests ----

    fn test_registry() -> Vec<aether_config::ProviderEntry> {
        vec![
            aether_config::ProviderEntry {
                id: "nvidia".into(),
                display_name: "NVIDIA".into(),
                protocol: "openai_compatible".into(),
                base_url: "https://nvidia.example/v1".into(),
                api_key_env: "GW_FP_NVIDIA".into(),
                auth_type: None,
                api_key: None,
                headers: None,
                extra_body: None,
                models: vec![
                    aether_config::ModelEntry { id: "model-a".into(), display_name: "Model A".into(), vision: false, tool_calling: true, streaming: true, context_window: None, max_output_tokens: None },
                    aether_config::ModelEntry { id: "model-b".into(), display_name: "Model B".into(), vision: false, tool_calling: true, streaming: true, context_window: None, max_output_tokens: None },
                ],
            },
            aether_config::ProviderEntry {
                id: "openrouter".into(),
                display_name: "OpenRouter".into(),
                protocol: "openai_compatible".into(),
                base_url: "https://openrouter.example/v1".into(),
                api_key_env: "GW_FP_OPENROUTER".into(),
                auth_type: None,
                api_key: None,
                headers: None,
                extra_body: None,
                models: vec![
                    aether_config::ModelEntry { id: "model-c".into(), display_name: "Model C".into(), vision: false, tool_calling: true, streaming: true, context_window: None, max_output_tokens: None },
                    aether_config::ModelEntry { id: "model-d".into(), display_name: "Model D".into(), vision: false, tool_calling: true, streaming: true, context_window: None, max_output_tokens: None },
                ],
            },
        ]
    }

    #[test]
    fn from_providers_builds_explicit_session_bindings() {
        std::env::set_var("GW_FP_NVIDIA", "sk-test");
        std::env::set_var("GW_FP_OPENROUTER", "sk-test");
        let registry = test_registry();
        let assignments = aether_config::RoleAssignments {
            executor: Some(aether_config::RoleBinding { provider_id: "nvidia".into(), model_id: "model-a".into() }),
            controller: Some(aether_config::RoleBinding { provider_id: "openrouter".into(), model_id: "model-d".into() }),
            reviewer: None,
        };
        let bundle = GatewayBundle::from_providers(&registry, &assignments, GatewayConfig::default()).unwrap();
        assert!(bundle.gateway.is_configured(Role::Executor));
        assert!(bundle.gateway.is_configured(Role::Controller));
        assert!(!bundle.gateway.is_configured(Role::Reviewer));
        assert_eq!(bundle.providers.len(), 2);
    }

    #[test]
    fn from_providers_missing_executor_fails() {
        let registry = test_registry();
        let assignments = aether_config::RoleAssignments {
            executor: None,
            controller: Some(aether_config::RoleBinding { provider_id: "openrouter".into(), model_id: "model-c".into() }),
            reviewer: None,
        };
        let err = GatewayBundle::from_providers(&registry, &assignments, GatewayConfig::default()).unwrap_err();
        assert!(err.to_string().contains("executor"));
    }

    #[test]
    fn from_providers_unknown_model_fails() {
        let registry = test_registry();
        let assignments = aether_config::RoleAssignments {
            executor: Some(aether_config::RoleBinding { provider_id: "nvidia".into(), model_id: "does-not-exist".into() }),
            controller: Some(aether_config::RoleBinding { provider_id: "openrouter".into(), model_id: "model-c".into() }),
            reviewer: None,
        };
        let err = GatewayBundle::from_providers(&registry, &assignments, GatewayConfig::default()).unwrap_err();
        assert!(err.to_string().contains("does-not-exist"));
    }
}
