//! Sanitization for error text sent to Fleet Intelligence backends.

use url::Url;

const REDACTED_URL: &str = "[URL_REDACTED]";
const URL_SCHEMES: &[&str] = &["https://", "http://", "ftps://", "ftp://", "file://"];

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
    Url::parse(url_text)
        .ok()
        .map(|url| url.path().to_string())
        .filter(|path| !path.is_empty() && path != "/")
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
