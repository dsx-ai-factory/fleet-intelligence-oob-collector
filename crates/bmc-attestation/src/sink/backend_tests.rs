// Copyright (c) 2026, NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use super::*;
use crate::models::NodeAttestation;

const FIRST_MAC: &str = "00:11:22:33:44:55";
const SECOND_MAC: &str = "00:11:22:33:44:66";

fn batch() -> BatchAttestation {
    BatchAttestation::new(
        "2026-01-01T00:00:00Z".to_string(),
        [FIRST_MAC, SECOND_MAC]
            .into_iter()
            .map(|mac| NodeAttestation::failed(mac.into(), mac.into(), "192.0.2.1".into(), "test"))
            .collect(),
    )
}

#[test]
fn accepts_results_by_mac_in_any_order_without_node_uuids() {
    let response: BatchResponse = serde_json::from_value(serde_json::json!({
        "results": [
            {"bmcMac": SECOND_MAC, "status": "OK"},
            {"bmcMac": FIRST_MAC, "status": "OK"}
        ]
    }))
    .unwrap();
    response
        .validate(&batch())
        .expect("both MACs were accepted");
}

#[test]
fn rejects_partial_missing_duplicate_and_unexpected_results() {
    let cases = [
        (
            serde_json::json!([
                {"bmcMac": FIRST_MAC, "status": "OK"},
                {"bmcMac": SECOND_MAC, "status": "INTERNAL_ERROR"}
            ]),
            "BMC 00:11:22:33:44:66 rejected",
        ),
        (
            serde_json::json!([{ "bmcMac": FIRST_MAC, "status": "OK" }]),
            "missing result for BMC 00:11:22:33:44:66",
        ),
        (
            serde_json::json!([
                {"bmcMac": FIRST_MAC, "status": "OK"},
                {"bmcMac": SECOND_MAC, "status": "OK"},
                {"bmcMac": FIRST_MAC, "status": "OK"}
            ]),
            "duplicate result for BMC",
        ),
        (
            serde_json::json!([
                {"bmcMac": FIRST_MAC, "status": "OK"},
                {"bmcMac": SECOND_MAC, "status": "OK"},
                {"bmcMac": "00:11:22:33:44:77", "status": "OK"}
            ]),
            "unexpected BMC MAC",
        ),
        (
            serde_json::json!([{ "status": "INVALID_MAC" }]),
            "result missing BMC MAC",
        ),
        (serde_json::json!([]), "missing result for BMC"),
    ];
    for (results, expected) in cases {
        let response: BatchResponse =
            serde_json::from_value(serde_json::json!({"results": results})).unwrap();
        let error = response.validate(&batch()).expect_err(expected).to_string();
        assert!(error.contains(expected), "{error}");
    }
}

#[test]
fn upload_diagnostics_do_not_expose_backend_urls_or_messages() {
    let response: BatchResponse = serde_json::from_value(serde_json::json!({
        "results": [
            {"bmcMac": FIRST_MAC, "status": "https://user:secret@example.com/error"},
            {"bmcMac": SECOND_MAC, "status": "ERROR", "message": "private-body"}
        ]
    }))
    .unwrap();
    let error = response
        .validate(&batch())
        .expect_err("rejected")
        .to_string();
    assert!(error.contains("URL_REDACTED"));
    assert!(!error.contains("secret"));
    assert!(!error.contains("private-body"));
}

/// Exercise the real HTTP submit/deserialize path against a localhost fixture.
async fn submit_response(status: u16, body: &str) -> Result<(), Box<dyn Error>> {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let body = body.to_string();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = Vec::new();
        loop {
            let mut bytes = [0; 4096];
            let count = socket.read(&mut bytes).await.unwrap();
            assert!(count > 0, "request ended prematurely");
            request.extend_from_slice(&bytes[..count]);
            if let Some(end) = request.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
                let headers = String::from_utf8_lossy(&request[..end]);
                let length: usize = headers
                    .lines()
                    .filter_map(|line| line.split_once(':'))
                    .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                    .unwrap()
                    .1
                    .trim()
                    .parse()
                    .unwrap();
                if request.len() >= end + 4 + length {
                    break;
                }
            }
        }
        socket.write_all(format!(
            "HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()
        ).as_bytes()).await.unwrap();
        String::from_utf8(request).unwrap()
    });
    let mut client = HttpBackendClient::new(
        format!("http://{address}"),
        "/v1/agent/oob_nodes/{bmc_mac}/nonce",
        "/v1/agent/oob_nodes/attestation",
        "test-jwt",
        Duration::from_secs(2),
    )?;
    // Keep fixture traffic local even when the developer uses a system proxy.
    client.client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(2))
        .build()?;
    let result = client.submit_batch(&batch()).await;
    let request = tokio::time::timeout(Duration::from_secs(3), server)
        .await
        .expect("fixture must finish")
        .unwrap();
    assert!(request.starts_with("POST /v1/agent/oob_nodes/attestation "));
    assert!(
        request
            .to_ascii_lowercase()
            .contains("authorization: bearer test-jwt")
    );
    assert!(request.contains(FIRST_MAC));
    assert!(request.contains(SECOND_MAC));
    result
}

#[tokio::test]
async fn http_success_requires_complete_accepted_results() {
    submit_response(
        200,
        &serde_json::json!({"results": [
            {"bmcMac": SECOND_MAC, "status": "OK"},
            {"bmcMac": FIRST_MAC, "status": "OK"}
        ]})
        .to_string(),
    )
    .await
    .expect("accepted");

    for body in [
        r#"{"results":[{"bmcMac":"00:11:22:33:44:55","status":"INTERNAL_ERROR"}]}"#,
        r#"{"results":[]}"#,
        r#"{}"#,
        "not JSON",
        "",
    ] {
        assert!(submit_response(200, body).await.is_err(), "{body}");
    }
}

#[tokio::test]
async fn http_unauthorized_preserves_auth_error_for_reenrollment() {
    let error = submit_response(401, r#"{"error":"expired"}"#)
        .await
        .expect_err("JWT rejected");
    assert_eq!(
        error.downcast_ref::<BackendHttpError>().unwrap().status(),
        401
    );
}
