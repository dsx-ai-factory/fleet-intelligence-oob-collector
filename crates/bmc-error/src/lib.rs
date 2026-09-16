//! Sanitization for error text sent to Fleet Intelligence backends.

use url::Url;

const REDACTED_URL: &str = "[URL_REDACTED]";
const URL_SCHEMES: &[&str] = &["https://", "http://", "ftps://", "ftp://", "file://", "//"];

/// Remove sensitive URL components from error text sent to a backend.
///
/// Full error messages can contain BMC addresses and can also trigger request
/// body security filters. Callers should retain the original text for local
/// diagnostics and sanitize only the backend representation. A valid URL with
/// a useful path is replaced by that path; other URLs are fully redacted.
pub fn sanitize_backend_error_message(message: &str) -> String {
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

        let (url_text, trailing_punctuation) =
            split_trailing_punctuation(&message[url_start..index]);
        output.push_str(&safe_url_path(url_text));
        output.push_str(trailing_punctuation);
    }

    output
}

fn safe_url_path(url_text: &str) -> String {
    let absolute;
    let url_text = if url_text.starts_with("//") {
        absolute = format!("https:{url_text}");
        &absolute
    } else {
        url_text
    };
    Url::parse(url_text)
        .ok()
        .filter(|url| !url.cannot_be_a_base())
        .map(|url| url.path().to_string())
        .filter(|path| !path.is_empty() && path != "/")
        .unwrap_or_else(|| REDACTED_URL.to_string())
}

/// Sanitize a structured URI reference for backend inventory output.
/// Preserve resource paths and fragments, but never authority or query data.
/// Invalid references are redacted rather than returned verbatim.
pub fn sanitize_backend_uri_reference(reference: &str) -> String {
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
            if path.is_empty() || path == "/" {
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
            sanitize_backend_error_message("failed (//user:secret@host/path?token=x#item)"),
            "failed (/path)"
        );
        assert_eq!(
            sanitize_backend_error_message("failed //host"),
            "failed [URL_REDACTED]"
        );
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
            assert_eq!(sanitize_backend_uri_reference(input), expected, "{input}");
        }
    }

    #[test]
    fn preserves_url_path_and_surrounding_error_details() {
        assert_eq!(
            sanitize_backend_error_message(
                "BMC error - url: https://10.0.0.1/redfish/v1/Systems/1 status: 403 Forbidden"
            ),
            "BMC error - url: /redfish/v1/Systems/1 status: 403 Forbidden"
        );
    }

    #[test]
    fn sanitizes_multiple_urls_case_insensitively() {
        assert_eq!(
            sanitize_backend_error_message(
                "HTTP://bmc/redfish failed; retry HTTPS://bmc/redfish?target=system"
            ),
            "/redfish failed; retry /redfish"
        );
    }

    #[test]
    fn removes_authority_credentials_port_query_and_fragment() {
        assert_eq!(
            sanitize_backend_error_message(
                "https://user:secret@10.0.0.1:8443/redfish/v1/Systems/1?detail=all#result"
            ),
            "/redfish/v1/Systems/1"
        );
    }

    #[test]
    fn preserves_text_without_urls() {
        assert_eq!(
            sanitize_backend_error_message("connection refused"),
            "connection refused"
        );
    }

    #[test]
    fn preserves_path_and_punctuation_in_reqwest_url_suffix() {
        assert_eq!(
            sanitize_backend_error_message(
                "HTTP status client error (404 Not Found) for url (https://10.0.0.1/redfish/v1)"
            ),
            "HTTP status client error (404 Not Found) for url (/redfish/v1)"
        );
    }

    #[test]
    fn fully_redacts_invalid_or_pathless_urls() {
        assert_eq!(
            sanitize_backend_error_message("failed http:// and https://bmc"),
            "failed [URL_REDACTED] and [URL_REDACTED]"
        );
    }
}
