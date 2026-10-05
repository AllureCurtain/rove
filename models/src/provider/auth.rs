use std::fmt;

use reqwest::header::{HeaderName, HeaderValue};
use thiserror::Error;

use super::AuthStyle;

/// A value whose debug and display representations never expose its contents.
///
/// The inner value is intentionally only available through an explicit
/// crate-private accessor. Provider code should resolve secrets once and pass
/// this wrapper to transport rather than retaining raw values in profiles.
pub struct Redacted<T>(T);

impl<T> Redacted<T> {
    pub fn new(value: T) -> Self {
        Self(value)
    }

    pub(crate) fn expose(&self) -> &T {
        &self.0
    }
}

impl<T> fmt::Debug for Redacted<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("[REDACTED]")
    }
}

impl<T> fmt::Display for Redacted<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("[REDACTED]")
    }
}

impl<T: Clone> Clone for Redacted<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<T: PartialEq> PartialEq for Redacted<T> {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl<T: Eq> Eq for Redacted<T> {}

/// Authentication after a secret source has been resolved.
#[derive(Clone, PartialEq, Eq)]
pub struct ResolvedAuth {
    style: AuthStyle,
    secret: Option<Redacted<String>>,
}

impl fmt::Debug for ResolvedAuth {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ResolvedAuth")
            .field("style", &self.style)
            .field("secret", &self.secret)
            .finish()
    }
}

impl ResolvedAuth {
    pub fn new(
        style: AuthStyle,
        secret: Option<impl Into<String>>,
    ) -> Result<Self, AuthConfigurationError> {
        let secret = secret.map(|value| Redacted::new(value.into()));
        validate_auth(&style, secret.as_ref())?;
        Ok(Self { style, secret })
    }

    pub fn none() -> Self {
        Self {
            style: AuthStyle::None,
            secret: None,
        }
    }

    pub fn bearer(secret: impl Into<String>) -> Result<Self, AuthConfigurationError> {
        Self::new(AuthStyle::Bearer, Some(secret))
    }

    pub fn header(
        name: HeaderName,
        secret: impl Into<String>,
    ) -> Result<Self, AuthConfigurationError> {
        Self::new(AuthStyle::Header(name), Some(secret))
    }

    pub(crate) fn style(&self) -> &AuthStyle {
        &self.style
    }

    pub(crate) fn secret(&self) -> Option<&str> {
        self.secret.as_ref().map(|secret| secret.expose().as_str())
    }
}

/// A profile header resolved before transport. Values are always redacted in
/// diagnostics because a custom header may carry a credential.
#[derive(Clone, PartialEq, Eq)]
pub struct ResolvedHeader {
    name: HeaderName,
    value: Redacted<String>,
}

impl fmt::Debug for ResolvedHeader {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ResolvedHeader")
            .field("name", &self.name)
            .field("value", &self.value)
            .finish()
    }
}

impl ResolvedHeader {
    pub fn new(name: HeaderName, value: impl Into<String>) -> Result<Self, AuthConfigurationError> {
        let value = value.into();
        HeaderValue::from_str(&value).map_err(|_| AuthConfigurationError::InvalidHeaderValue {
            name: name.as_str().to_string(),
        })?;
        Ok(Self {
            name,
            value: Redacted::new(value),
        })
    }

    pub fn try_new(name: &str, value: impl Into<String>) -> Result<Self, AuthConfigurationError> {
        let name = HeaderName::from_bytes(name.as_bytes()).map_err(|_| {
            AuthConfigurationError::InvalidHeaderName {
                name: name.to_string(),
            }
        })?;
        Self::new(name, value)
    }

    pub(crate) fn name(&self) -> &HeaderName {
        &self.name
    }

    pub(crate) fn value(&self) -> &str {
        self.value.expose().as_str()
    }
}

#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum AuthConfigurationError {
    #[error("authentication style '{style}' requires a non-empty secret")]
    SecretRequired { style: &'static str },
    #[error("authentication style 'none' cannot carry a secret")]
    SecretNotAllowed,
    #[error("authentication secret cannot be represented as a header value")]
    InvalidSecret,
    #[error("invalid header name '{name}'")]
    InvalidHeaderName { name: String },
    #[error("header '{name}' contains an invalid value")]
    InvalidHeaderValue { name: String },
}

fn validate_auth(
    style: &AuthStyle,
    secret: Option<&Redacted<String>>,
) -> Result<(), AuthConfigurationError> {
    let secret = secret.map(|secret| secret.expose().as_str());
    match style {
        AuthStyle::None => {
            if secret.is_some() {
                return Err(AuthConfigurationError::SecretNotAllowed);
            }
        }
        AuthStyle::Bearer => {
            let Some(secret) = secret.filter(|value| !value.trim().is_empty()) else {
                return Err(AuthConfigurationError::SecretRequired { style: "bearer" });
            };
            HeaderValue::from_str(&format!("Bearer {secret}"))
                .map_err(|_| AuthConfigurationError::InvalidSecret)?;
        }
        AuthStyle::Header(name) => {
            let Some(secret) = secret.filter(|value| !value.trim().is_empty()) else {
                return Err(AuthConfigurationError::SecretRequired { style: "header" });
            };
            HeaderValue::from_str(secret).map_err(|_| AuthConfigurationError::InvalidSecret)?;
            if name.as_str().is_empty() {
                return Err(AuthConfigurationError::InvalidHeaderName {
                    name: String::new(),
                });
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use reqwest::header::HeaderName;

    use super::*;

    #[test]
    fn redacted_values_never_appear_in_debug_or_display() {
        let secret = Redacted::new("super-secret".to_string());
        assert_eq!(format!("{secret:?}"), "[REDACTED]");
        assert_eq!(secret.to_string(), "[REDACTED]");

        let auth = ResolvedAuth::bearer("super-secret").unwrap();
        let debug = format!("{auth:?}");
        assert!(!debug.contains("super-secret"));
    }

    #[test]
    fn auth_style_and_secret_combinations_are_validated() {
        assert!(ResolvedAuth::none().secret().is_none());
        assert!(matches!(
            ResolvedAuth::new(AuthStyle::None, Some("secret")),
            Err(AuthConfigurationError::SecretNotAllowed)
        ));
        assert!(matches!(
            ResolvedAuth::new(AuthStyle::Bearer, None::<String>),
            Err(AuthConfigurationError::SecretRequired { style: "bearer" })
        ));
        assert!(matches!(
            ResolvedAuth::new(AuthStyle::Bearer, Some("bad\nsecret")),
            Err(AuthConfigurationError::InvalidSecret)
        ));
    }

    #[test]
    fn custom_headers_validate_names_and_values_without_exposing_values() {
        let header = ResolvedHeader::try_new("x-tenant", "tenant-secret").unwrap();
        assert_eq!(header.name(), &HeaderName::from_static("x-tenant"));
        assert_eq!(header.value(), "tenant-secret");
        assert!(!format!("{header:?}").contains("tenant-secret"));
        assert!(matches!(
            ResolvedHeader::try_new("bad name", "value"),
            Err(AuthConfigurationError::InvalidHeaderName { .. })
        ));
        assert!(matches!(
            ResolvedHeader::try_new("x-test", "bad\nvalue"),
            Err(AuthConfigurationError::InvalidHeaderValue { .. })
        ));
    }
}
