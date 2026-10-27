// Copyright (c) 2026, NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use std::convert::Infallible;
use std::fmt;
use std::str::FromStr;

use serde::Deserialize;

/// Credential text with explicit access and redacted debug output.
/// Intentionally does not implement Display or Serialize. This is a logging
/// safeguard, not a guarantee that credential memory is zeroized.
#[derive(Clone, Deserialize)]
#[serde(transparent)]
pub struct SecretString(String);

impl SecretString {
    pub fn expose_secret(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for SecretString {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("[REDACTED]")
    }
}

impl From<String> for SecretString {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<&str> for SecretString {
    fn from(value: &str) -> Self {
        Self(value.to_string())
    }
}

impl FromStr for SecretString {
    type Err = Infallible;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Ok(value.into())
    }
}

impl AsRef<str> for SecretString {
    fn as_ref(&self) -> &str {
        self.expose_secret()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parsed_and_cloned_secrets_are_only_exposed_explicitly() {
        let secret: SecretString = "example-password".parse().unwrap();
        let cloned = secret.clone();
        assert_eq!(cloned.expose_secret(), "example-password");
        assert_eq!(format!("{secret:?} {cloned:?}"), "[REDACTED] [REDACTED]");
        let parsed: SecretString = serde_json::from_str("\"example-jwt\"").unwrap();
        assert!(!format!("{parsed:?}").contains("example-jwt"));
    }
}
