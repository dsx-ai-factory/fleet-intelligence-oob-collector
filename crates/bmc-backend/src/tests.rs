// Copyright (c) 2026, NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use super::*;

#[test]
fn url_validation_rejects_ambiguous_or_sensitive_urls_without_echoing_them() {
    for input in [
        "relative/path",
        "ftp://example.com",
        "https://user:example-secret@example.com",
        "https://example.com?token=example-secret",
        "https://example.com#example-secret",
    ] {
        let error = BackendUrl::parse(input).expect_err(input).to_string();
        assert!(!error.contains("example-secret"));
        assert!(!error.contains(input));
    }
    for input in [
        "https://api.example.com",
        "http://127.0.0.1:8000",
        "https://[::1]:8443/prefix/",
    ] {
        BackendUrl::parse(input).expect("valid backend URL");
    }
}

#[test]
fn endpoint_modes_preserve_existing_base_path_behavior() {
    let base = BackendUrl::parse("https://example.com/prefix/").unwrap();
    assert_eq!(
        base.endpoint("/v1/agent/enroll", PathMode::Append).as_str(),
        "https://example.com/prefix/v1/agent/enroll"
    );
    assert_eq!(
        base.endpoint(
            "/v1/agent/oob_nodes/AA:BB:CC:DD:EE:FF/nonce",
            PathMode::Replace
        )
        .as_str(),
        "https://example.com/v1/agent/oob_nodes/AA:BB:CC:DD:EE:FF/nonce"
    );
}

#[test]
fn only_http_unauthorized_is_an_auth_failure() {
    for status in [400, 401, 403, 429, 500] {
        assert_eq!(
            is_auth_failure(&BackendHttpError::new(status, "example")),
            status == 401
        );
    }
    let text: Box<dyn Error> = "HTTP 401 is not a typed response".into();
    assert!(!is_auth_failure(text.as_ref()));
}

#[test]
fn error_metadata_preserves_diagnostics_and_sanitizes_urls() {
    let error = BackendHttpError::with_metadata(
        403,
        "first line\nhttps://user:example-secret@example.com/path?token=hidden",
        Some("example-proxy".into()),
        Some("request-123".into()),
    );
    let message = error.to_string();
    assert!(message.contains("http_status=403"));
    assert!(message.contains("request-123"));
    assert!(message.contains("first line"));
    assert!(message.contains("URL_REDACTED"));
    assert!(!message.contains("example-secret"));
    assert!(!message.contains('\n'));
}

/// Real HTTP fixture: validates requests and closes each connection so redirect
/// tests can distinguish following a redirect from returning its status.
async fn fixture(responses: Vec<String>) -> (Url, tokio::task::JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}/enroll", listener.local_addr().unwrap())
        .parse()
        .unwrap();
    let server = tokio::spawn(async move {
        let mut requests = Vec::new();
        for response in responses {
            let (mut socket, _) = tokio::time::timeout(Duration::from_secs(3), listener.accept())
                .await
                .unwrap()
                .unwrap();
            let mut bytes = Vec::new();
            loop {
                let mut part = [0; 4096];
                let count = socket.read(&mut part).await.unwrap();
                assert!(count > 0);
                bytes.extend_from_slice(&part[..count]);
                if let Some(end) = bytes.windows(4).position(|value| value == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&bytes[..end]);
                    let length = headers
                        .lines()
                        .filter_map(|line| line.split_once(':'))
                        .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                        .map_or(0, |(_, value)| value.trim().parse::<usize>().unwrap());
                    if bytes.len() >= end + 4 + length {
                        break;
                    }
                }
            }
            socket.write_all(response.as_bytes()).await.unwrap();
            requests.push(String::from_utf8(bytes).unwrap());
        }
        requests
    });
    (endpoint, server)
}

fn response(status: u16, headers: &str, body: &str) -> String {
    format!(
        "HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n{headers}\r\n{body}",
        body.len()
    )
}

fn local_client(policy: RedirectPolicy) -> Client {
    client_builder(Duration::from_secs(1), policy)
        .unwrap()
        .no_proxy()
        .build()
        .unwrap()
}

#[tokio::test]
async fn enrollment_preserves_auth_headers_user_agent_and_jwt_material() {
    let (endpoint, server) = fixture(vec![response(
        200,
        "",
        r#"{"jwtAssertion":"example-runtime-token"}"#,
    )])
    .await;
    let token = enroll(
        &local_client(RedirectPolicy::Default),
        &endpoint,
        "example-bootstrap-token",
    )
    .await
    .unwrap();
    assert_eq!(token.expose_secret(), "example-runtime-token");
    assert!(!format!("{token:?}").contains("example-runtime-token"));
    let requests = server.await.unwrap();
    let request = requests[0].to_ascii_lowercase();
    assert!(request.starts_with("post /enroll "));
    assert!(request.contains("authorization: bearer example-bootstrap-token"));
    assert!(request.contains(&format!("user-agent: {BACKEND_USER_AGENT}")));
}

#[tokio::test]
async fn response_status_and_metadata_remain_typed_over_http() {
    for status in [401, 403, 500] {
        let (endpoint, server) = fixture(vec![response(
            status,
            "Server: example-proxy\r\nx-request-id: example-request\r\n",
            "rejected",
        )])
        .await;
        let reply = request(&local_client(RedirectPolicy::None), Method::GET, &endpoint)
            .send()
            .await
            .unwrap();
        let error = ensure_success(reply)
            .await
            .expect_err("rejected HTTP status");
        assert_eq!(is_auth_failure(error.as_ref()), status == 401);
        assert_eq!(
            error.downcast_ref::<BackendHttpError>().unwrap().status(),
            status
        );
        assert!(error.to_string().contains("example-request"));
        server.await.unwrap();
    }
}

#[tokio::test]
async fn redirect_policies_keep_enrollment_and_attestation_behavior() {
    let redirect = response(302, "Location: /accepted\r\n", "");
    let (endpoint, server) = fixture(vec![redirect.clone(), response(200, "", "{}")]).await;
    let reply = request(
        &local_client(RedirectPolicy::Default),
        Method::GET,
        &endpoint,
    )
    .send()
    .await
    .unwrap();
    assert_eq!(reply.status(), 200);
    let requests = server.await.unwrap();
    assert!(requests[1].starts_with("GET /accepted "));

    let (endpoint, server) = fixture(vec![redirect]).await;
    let reply = request(&local_client(RedirectPolicy::None), Method::GET, &endpoint)
        .send()
        .await
        .unwrap();
    assert_eq!(reply.status(), 302);
    assert!(!is_auth_failure(
        ensure_success(reply).await.unwrap_err().as_ref()
    ));
    assert_eq!(server.await.unwrap().len(), 1);
}

#[tokio::test]
async fn enrollment_rejects_empty_jwt_and_unusable_responses() {
    for body in [r#"{"jwtAssertion":" "}"#, r#"{}"#, "not-json"] {
        let (endpoint, server) = fixture(vec![response(200, "", body)]).await;
        assert!(
            enroll(
                &local_client(RedirectPolicy::Default),
                &endpoint,
                "example-token"
            )
            .await
            .is_err()
        );
        server.await.unwrap();
    }
    assert!(http_client(Duration::ZERO, RedirectPolicy::Default).is_err());
}
