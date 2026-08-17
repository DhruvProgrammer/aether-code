//! `ProviderRegistry` — the in-memory source of truth for providers and models.

use crate::catalog::ModelDescriptor;
use crate::provider::ProviderDescriptor;
use crate::ProviderStatus;
use std::collections::HashMap;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum RegistryError {
    #[error("provider not found: {0}")]
    ProviderNotFound(String),
    #[error("model not found: {0}")]
    ModelNotFound(String),
    #[error("provider exists: {0}")]
    ProviderExists(String),
    #[error("provider unhealthy: {0}")]
    ProviderUnhealthy(String),
}

#[derive(Debug, Default)]
pub struct ProviderRegistry {
    providers: HashMap<String, ProviderDescriptor>,
    models: HashMap<String, ModelDescriptor>,
}

impl ProviderRegistry {
    pub fn new() -> Self { Self::default() }

    pub fn register_provider(&mut self, p: ProviderDescriptor) -> Result<(), RegistryError> {
        if self.providers.contains_key(&p.id) {
            return Err(RegistryError::ProviderExists(p.id));
        }
        for m in &p.models {
            self.models.insert(
                m.clone(),
                ModelDescriptor::new(m.clone(), p.id.clone(), crate::capabilities::CapabilityMatrix::default()),
            );
        }
        self.providers.insert(p.id.clone(), p);
        Ok(())
    }

    pub fn upsert_provider(&mut self, p: ProviderDescriptor) {
        for m in &p.models {
            self.models
                .entry(m.clone())
                .and_modify(|md| md.provider_id = p.id.clone())
                .or_insert_with(|| {
                    ModelDescriptor::new(m.clone(), p.id.clone(), crate::capabilities::CapabilityMatrix::default())
                });
        }
        self.providers.insert(p.id.clone(), p);
    }

    pub fn register_model(&mut self, m: ModelDescriptor) {
        self.models.insert(m.id.clone(), m);
    }

    pub fn provider(&self, id: &str) -> Option<&ProviderDescriptor> {
        self.providers.get(id)
    }

    pub fn provider_mut(&mut self, id: &str) -> Option<&mut ProviderDescriptor> {
        self.providers.get_mut(id)
    }

    pub fn model(&self, id: &str) -> Option<&ModelDescriptor> {
        self.models.get(id)
    }

    pub fn list_providers(&self) -> Vec<&ProviderDescriptor> {
        let mut v: Vec<_> = self.providers.values().collect();
        v.sort_by(|a, b| a.id.cmp(&b.id));
        v
    }

    pub fn list_models(&self) -> Vec<&ModelDescriptor> {
        let mut v: Vec<_> = self.models.values().collect();
        v.sort_by(|a, b| a.id.cmp(&b.id));
        v
    }

    pub fn list_models_for_provider(&self, provider_id: &str) -> Vec<&ModelDescriptor> {
        let mut v: Vec<_> = self.models.values().filter(|m| m.provider_id == provider_id).collect();
        v.sort_by(|a, b| a.id.cmp(&b.id));
        v
    }

    pub fn set_provider_status(&mut self, id: &str, status: ProviderStatus) {
        if let Some(p) = self.providers.get_mut(id) {
            p.status = status;
        }
    }

    /// Resolve a model-id to the model + the provider that hosts it.
    pub fn resolve(&self, model_id: &str) -> Option<(&ModelDescriptor, &ProviderDescriptor)> {
        let m = self.models.get(model_id)?;
        let p = self.providers.get(&m.provider_id)?;
        Some((m, p))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::ProviderDescriptor;

    #[test]
    fn register_and_resolve() {
        let mut r = ProviderRegistry::new();
        let p = ProviderDescriptor::new_openai_compatible("p1", "https://x", "KEY")
            .with_model("m1");
        r.register_provider(p).unwrap();
        let (m, p) = r.resolve("m1").unwrap();
        assert_eq!(m.id, "m1");
        assert_eq!(p.id, "p1");
    }

    #[test]
    fn duplicate_provider_is_error() {
        let mut r = ProviderRegistry::new();
        let p = ProviderDescriptor::new_openai_compatible("p1", "https://x", "KEY");
        r.register_provider(p.clone()).unwrap();
        assert!(matches!(r.register_provider(p), Err(RegistryError::ProviderExists(_))));
    }
}
