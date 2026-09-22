//! Sanitization for Redfish-derived text and URI references in product models.

use std::fmt;
use std::ops::Deref;

use serde::Serialize;
use url::Url;

const REDACTED_URL: &str = "[URL_REDACTED]";
const URL_SCHEMES: &[&str] = &["https://", "http://", "ftps://", "ftp://", "file://", "//"];

/// Error text whose embedded URLs have been fully redacted.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct SanitizedErrorMessage(String);

impl SanitizedErrorMessage {
    pub fn new(value: impl AsRef<str>) -> Self {
        Self(sanitize_error_message(value.as_ref()))
    }
}

/// A URI reference without authority, credentials, port, or query data.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct SanitizedUriReference(String);

impl SanitizedUriReference {
    pub fn new(value: impl AsRef<str>) -> Self {
        Self(sanitize_uri_reference(value.as_ref()))
    }
}

macro_rules! impl_sanitized_string {
    ($type:ty) => {
        impl Deref for $type {
            type Target = str;

            fn deref(&self) -> &Self::Target {
                &self.0
            }
        }

        impl AsRef<str> for $type {
            fn as_ref(&self) -> &str {
                self
            }
        }

        impl fmt::Display for $type {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(formatter)
            }
        }

        impl From<String> for $type {
            fn from(value: String) -> Self {
                Self::new(value)
            }
        }

        impl From<&str> for $type {
            fn from(value: &str) -> Self {
                Self::new(value)
            }
        }

        impl PartialEq<str> for $type {
            fn eq(&self, other: &str) -> bool {
                self.0 == other
            }
        }

        impl PartialEq<&str> for $type {
            fn eq(&self, other: &&str) -> bool {
                self.0 == *other
            }
        }
    };
}

impl_sanitized_string!(SanitizedErrorMessage);
impl_sanitized_string!(SanitizedUriReference);

/// Remove sensitive URL components from error text exposed outside the source layer.
///
/// Full error messages can contain BMC addresses and can also trigger request
/// body security filters. Replace each embedded URL entirely; structured URI
/// references are sanitized separately to preserve resource identity.
pub fn sanitize_error_message(message: &str) -> String {
    let lower = message.to_ascii_lowercase();
    let mut output = String::with_capacity(message.len());
    let mut index = 0;

    while index < message.len() {
        let Some(scheme) = URL_SCHEMES
            .iter()
            .find(|scheme| lower[index..].starts_with(*scheme))
        else {
            let character = message[index..].chars().next().expect("character at index");
            output.push(character);
            index += character.len_utf8();
            continue;
        };

        let url_start = index;
        index += scheme.len();
        while index < message.len() {
            let character = message[index..].chars().next().expect("character at index");
            if character.is_whitespace() || matches!(character, '"' | '\'' | '<' | '>') {
                break;
            }
            index += character.len_utf8();
        }

        let (_, trailing_punctuation) = split_trailing_punctuation(&message[url_start..index]);
        output.push_str(REDACTED_URL);
        output.push_str(trailing_punctuation);
    }

    output
}

fn contains_embedded_url(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    URL_SCHEMES.iter().any(|scheme| lower.contains(scheme))
}

/// Sanitize a structured URI reference for product inventory output.
/// Preserve resource paths and fragments, but never authority or query data.
/// Invalid references are redacted rather than returned verbatim.
pub fn sanitize_uri_reference(reference: &str) -> String {
    if reference.is_empty()
        || reference
            .chars()
            .any(|c| c.is_whitespace() || c.is_control())
        || reference.contains('\\')
    {
        return REDACTED_URL.to_string();
    }
    if reference.starts_with("//") {
        return safe_uri_reference(&format!("https:{reference}"));
    }
    if Url::parse(reference).is_ok() {
        return safe_uri_reference(reference);
    }
    let (resource, fragment) = reference
        .split_once('#')
        .map_or((reference, None), |(resource, fragment)| {
            (resource, Some(fragment))
        });
    let path = resource.split('?').next().unwrap_or_default();
    // A colon in the first segment indicates a malformed absolute URI.
    if (path.is_empty() && fragment.is_none())
        || path.split('/').next().unwrap_or_default().contains(':')
        || contains_embedded_url(path)
        || fragment.is_some_and(contains_embedded_url)
    {
        return REDACTED_URL.to_string();
    }
    match fragment {
        Some(fragment) => format!("{path}#{fragment}"),
        None => path.to_string(),
    }
}

fn safe_uri_reference(url_text: &str) -> String {
    Url::parse(url_text)
        .ok()
        .filter(|url| !url.cannot_be_a_base())
        .and_then(|url| {
            let path = url.path();
            if path.is_empty()
                || path == "/"
                || contains_embedded_url(path)
                || url.fragment().is_some_and(contains_embedded_url)
            {
                return None;
            }
            Some(match url.fragment() {
                Some(fragment) => format!("{path}#{fragment}"),
                None => path.to_string(),
            })
        })
        .unwrap_or_else(|| REDACTED_URL.to_string())
}

fn split_trailing_punctuation(value: &str) -> (&str, &str) {
    let trimmed = value.trim_end_matches([')', ']', '}', ',', '.', ';', '!']);
    (trimmed, &value[trimmed.len()..])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitizes_scheme_relative_errors() {
        assert_eq!(
            sanitize_error_message("failed (//user:secret@host/path?token=x#item)"),
            "failed ([URL_REDACTED])"
        );
        assert_eq!(
            sanitize_error_message("failed //host"),
            "failed [URL_REDACTED]"
        );
    }

    #[test]
    fn redacts_urls_with_embedded_urls() {
        for input in [
            "failed https://safe/redfish/https://user:secret@evil/path",
            "failed https://safe/redfish//user:secret@evil/path",
        ] {
            assert_eq!(sanitize_error_message(input), "failed [URL_REDACTED]");
        }
    }

    #[test]
    fn sanitizes_structured_references() {
        for (input, expected) in [
            ("https://user:secret@host/path?token=x#item", "/path#item"),
            ("//user:secret@host/path?token=x#item", "/path#item"),
            ("/redfish/v1/Systems/1?token=x", "/redfish/v1/Systems/1"),
            ("Systems/1#item", "Systems/1#item"),
            ("#/Processors/0", "#/Processors/0"),
            ("//host", REDACTED_URL),
            ("https://", REDACTED_URL),
            ("//[invalid/path", REDACTED_URL),
            ("", REDACTED_URL),
            ("?token=x", REDACTED_URL),
            ("bad reference", REDACTED_URL),
            ("\\\\host/path", REDACTED_URL),
        ] {
            assert_eq!(sanitize_uri_reference(input), expected, "{input}");
        }
    }

    #[test]
    fn redacts_structured_references_with_embedded_urls() {
        for input in [
            "https://safe/redfish/https://user:secret@evil/path",
            "https://safe/redfish//user:secret@evil/path",
            "/redfish/https://user:secret@evil/path",
            "https://safe/redfish#https://user:secret@evil/path",
            "/redfish#//user:secret@evil/path",
        ] {
            assert_eq!(sanitize_uri_reference(input), REDACTED_URL, "{input}");
        }

        assert_eq!(sanitize_uri_reference("#/Processors/0"), "#/Processors/0");
    }

    #[test]
    fn sanitized_types_serialize_only_sanitized_values() {
        let message =
            SanitizedErrorMessage::new("failed //user:secret@host/redfish/v1/Systems/1?token=x");
        let reference =
            SanitizedUriReference::new("//user:secret@host/redfish/v1/Systems/1?token=x#item");

        assert_eq!(
            serde_json::to_string(&message).unwrap(),
            r#""failed [URL_REDACTED]""#
        );
        assert_eq!(
            serde_json::to_string(&reference).unwrap(),
            r#""/redfish/v1/Systems/1#item""#
        );
    }

    #[test]
    fn redacts_url_and_preserves_surrounding_error_details() {
        assert_eq!(
            sanitize_error_message(
                "BMC error - url: https://10.0.0.1/redfish/v1/Systems/1 status: 403 Forbidden"
            ),
            "BMC error - url: [URL_REDACTED] status: 403 Forbidden"
        );
    }

    #[test]
    fn sanitizes_multiple_urls_case_insensitively() {
        assert_eq!(
            sanitize_error_message(
                "HTTP://bmc/redfish failed; retry HTTPS://bmc/redfish?target=system"
            ),
            "[URL_REDACTED] failed; retry [URL_REDACTED]"
        );
    }

    #[test]
    fn removes_authority_credentials_port_query_and_fragment() {
        assert_eq!(
            sanitize_error_message(
                "https://user:secret@10.0.0.1:8443/redfish/v1/Systems/1?detail=all#result"
            ),
            "[URL_REDACTED]"
        );
    }

    #[test]
    fn preserves_text_without_urls() {
        assert_eq!(
            sanitize_error_message("connection refused"),
            "connection refused"
        );
    }

    #[test]
    fn preserves_punctuation_in_reqwest_url_suffix() {
        assert_eq!(
            sanitize_error_message(
                "HTTP status client error (404 Not Found) for url (https://10.0.0.1/redfish/v1)"
            ),
            "HTTP status client error (404 Not Found) for url ([URL_REDACTED])"
        );
    }

    #[test]
    fn fully_redacts_invalid_or_pathless_urls() {
        assert_eq!(
            sanitize_error_message("failed http:// and https://bmc"),
            "failed [URL_REDACTED] and [URL_REDACTED]"
        );
    }
}
