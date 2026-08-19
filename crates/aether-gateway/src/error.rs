//! Failure classification (gateway spec §13).
//!
//! Provider errors are never surfaced as a bare "API Error". Every failure is
//! classified into a [`FailureClass`] with a human-readable message, and the
//! raw API key is never included in the message.

use serde::{Deserialize, Serialize};

/// Coarse classification of a provider failure. Stable string codes are used
/// across the IPC boundary and in observability metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureClass {
    /// 401 / missing credentials.
    InvalidApiKey,
    /// 403 / expired token / insufficient scope.
    AuthenticationFailed,
    /// DNS failure or malformed base URL.
    InvalidBaseUrl,
    /// Connect timeout / connection refused on the configured endpoint.
    EndpointUnavailable,
    /// 404 on the model id.
    ModelNotFound,
    /// Model exists but is not currently serving (overloaded / disabled).
    ModelUnavailable,
    /// Provider is up but the whole service is down (5xx fleet-wide, billing).
    ProviderUnavailable,
    /// 429.
    RateLimited,
    /// Request exceeded the configured timeout.
    RequestTimeout,
    /// Network-level failure that is not DNS/connect (TLS, reset, proxy).
    NetworkFailure,
    /// 400 — malformed or rejected request body.
    InvalidRequest,
    /// The explicitly configured model cannot serve the requested operation.
    UnsupportedCapability,
    /// 5xx from an otherwise healthy endpoint.
    ServerError,
    /// Unknown provider error (kept as catch-all; detail carries the reason).
    UnknownProvider,
}

impl FailureClass {
    pub fn as_str(&self) -> &'static str {
        match self {
            FailureClass::InvalidApiKey => "invalid_api_key",
            FailureClass::AuthenticationFailed => "authentication_failed",
            FailureClass::InvalidBaseUrl => "invalid_base_url",
            FailureClass::EndpointUnavailable => "endpoint_unavailable",
            FailureClass::ModelNotFound => "model_not_found",
            FailureClass::ModelUnavailable => "model_unavailable",
            FailureClass::ProviderUnavailable => "provider_unavailable",
            FailureClass::RateLimited => "rate_limited",
            FailureClass::RequestTimeout => "request_timeout",
            FailureClass::NetworkFailure => "network_failure",
            FailureClass::InvalidRequest => "invalid_request",
            FailureClass::UnsupportedCapability => "unsupported_capability",
            FailureClass::ServerError => "server_error",
            FailureClass::UnknownProvider => "unknown_provider_error",
        }
    }

    /// Classify an HTTP (status, body) pair from a provider call.
    pub fn from_http(status: u16, body: &str) -> FailureClass {
        let lower = body.to_ascii_lowercase();
        match status {
            400 | 422 => {
                if lower.contains("model") && (lower.contains("not found") || lower.contains("does not exist")) {
                    FailureClass::ModelNotFound
                } else {
                    FailureClass::InvalidRequest
                }
            }
            401 => FailureClass::InvalidApiKey,
            403 => FailureClass::AuthenticationFailed,
            404 => FailureClass::ModelNotFound,
            408 => FailureClass::RequestTimeout,
            413 => FailureClass::InvalidRequest,
            429 => FailureClass::RateLimited,
            500..=599 => {
                if lower.contains("overloaded") || lower.contains("capacity") || lower.contains("unavailable") {
                    FailureClass::ModelUnavailable
                } else {
                    FailureClass::ServerError
                }
            }
            _ => FailureClass::UnknownProvider,
        }
    }

    /// Human-readable hint shown in the UI. Never includes secrets — call sites
    /// pass already-redacted details.
    pub fn hint(&self) -> &'static str {
        match self {
            FailureClass::InvalidApiKey => "Check the API key (environment variable) for this provider.",
            FailureClass::AuthenticationFailed => "Authentication was rejected. The key may be expired or lack permission for this model.",
            FailureClass::InvalidBaseUrl => "The base URL could not be reached. Check the URL for typos.",
            FailureClass::EndpointUnavailable => "The endpoint is not reachable right now (connection refused / DNS failure).",
            FailureClass::ModelNotFound => "The model id was not found for this provider.",
            FailureClass::ModelUnavailable => "The model exists but is not serving requests (overloaded or disabled).",
            FailureClass::ProviderUnavailable => "The provider service is down. Try again later.",
            FailureClass::RateLimited => "The provider rate-limited the request. Wait or reduce request rate.",
            FailureClass::RequestTimeout => "The request timed out before the provider answered.",
            FailureClass::NetworkFailure => "A network failure occurred (TLS error, connection reset, proxy interference).",
            FailureClass::InvalidRequest => "The provider rejected the request body.",
            FailureClass::UnsupportedCapability => "The explicitly configured model does not support the requested operation.",
            FailureClass::ServerError => "The provider returned a server error.",
            FailureClass::UnknownProvider => "An unknown provider error occurred.",
        }
    }
}

/// The gateway error type. Carries the classification plus a redacted detail.
#[derive(Debug, thiserror::Error)]
pub enum GatewayError {
    #[error("[{class}] {detail}", class = .class.as_str())]
    Provider {
        #[source]
        source: Option<Box<dyn std::error::Error + Send + Sync>>,
        class: FailureClass,
        detail: String,
    },
    #[error("no provider configured for role {0}")]
    NotConfigured(String),
    #[error("role {role}: [{class}] {detail}", class = .class.as_str())]
    CapabilityDenied {
        role: String,
        class: FailureClass,
        detail: String,
    },
    #[error("gateway cancelled: {0}")]
    Cancelled(String),
}

impl GatewayError {
    pub fn provider(class: FailureClass, detail: impl Into<String>) -> Self {
        GatewayError::Provider { source: None, class, detail: detail.into() }
    }

    pub fn class(&self) -> FailureClass {
        match self {
            GatewayError::Provider { class, .. } => *class,
            GatewayError::CapabilityDenied { class, .. } => *class,
            GatewayError::NotConfigured(_) => FailureClass::UnknownProvider,
            GatewayError::Cancelled(_) => FailureClass::RequestTimeout,
        }
    }

    /// True when the failure means the provider/model is unusable right now
    /// (as opposed to a malformed request).
    pub fn is_unavailable(&self) -> bool {
        matches!(
            self.class(),
            FailureClass::EndpointUnavailable
                | FailureClass::ProviderUnavailable
                | FailureClass::ModelUnavailable
                | FailureClass::NetworkFailure
                | FailureClass::InvalidBaseUrl
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn http_classification() {
        assert_eq!(FailureClass::from_http(401, "bad token"), FailureClass::InvalidApiKey);
        assert_eq!(FailureClass::from_http(403, "forbidden"), FailureClass::AuthenticationFailed);
        assert_eq!(
            FailureClass::from_http(404, "The model `x` does not exist"),
            FailureClass::ModelNotFound
        );
        assert_eq!(FailureClass::from_http(429, "too many"), FailureClass::RateLimited);
        assert_eq!(FailureClass::from_http(503, "overloaded"), FailureClass::ModelUnavailable);
        assert_eq!(FailureClass::from_http(500, "oops"), FailureClass::ServerError);
        assert_eq!(
            FailureClass::from_http(400, "model `y` not found"),
            FailureClass::ModelNotFound
        );
        assert_eq!(FailureClass::from_http(400, "bad json"), FailureClass::InvalidRequest);
        assert_eq!(FailureClass::from_http(408, "timeout"), FailureClass::RequestTimeout);
    }

    #[test]
    fn error_carries_class_and_message() {
        let e = GatewayError::provider(FailureClass::RateLimited, "provider X rate limited");
        assert_eq!(e.class(), FailureClass::RateLimited);
        assert!(e.to_string().contains("rate_limited"));
        assert!(FailureClass::RateLimited.hint().contains("rate-limited"));
    }

    #[test]
    fn unavailable_classification() {
        let e = GatewayError::provider(FailureClass::EndpointUnavailable, "refused");
        assert!(e.is_unavailable());
        let e2 = GatewayError::provider(FailureClass::InvalidRequest, "bad body");
        assert!(!e2.is_unavailable());
    }
}
