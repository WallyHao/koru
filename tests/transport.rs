//! Transport boundary: fixture replay, bounds, and real-transport classification.
use koru::transport::{
    FixtureTransport, Method, Request, Response, Transport, TransportErrorKind, TransportLimits,
    UreqTransport,
};

#[test]
fn fixture_replays_scripted_responses_and_records_requests() {
    let transport = FixtureTransport::new(TransportLimits::default());
    transport.push(Response::text(200, "{\"ok\":true}"));
    transport.push_error(TransportErrorKind::Timeout, "late");
    let first = transport
        .send(&Request::get("https://example.test/a").with_header("Accept", "application/json"))
        .unwrap();
    assert_eq!(first.status, 200);
    assert_eq!(first.text_body().unwrap(), "{\"ok\":true}");
    let second = transport
        .send(&Request::get("https://example.test/b"))
        .unwrap_err();
    assert_eq!(second.kind(), TransportErrorKind::Timeout);

    let requests = transport.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].method, Method::Get);
    assert_eq!(requests[0].url, "https://example.test/a");
    assert_eq!(requests[0].header("accept"), Some("application/json"));
}

#[test]
fn fixture_enforces_the_request_body_limit() {
    let transport = FixtureTransport::new(TransportLimits {
        max_request_bytes: 4,
        ..TransportLimits::default()
    });
    let error = transport
        .send(&Request::post("https://example.test/a", b"12345".to_vec()))
        .unwrap_err();
    assert_eq!(error.kind(), TransportErrorKind::InvalidRequest);
    assert!(transport.requests().is_empty());
}

#[test]
fn response_helpers_classify_status_and_encoding() {
    assert!(Response::text(200, "ok").is_success());
    assert!(!Response::text(500, "no").is_success());
    let invalid = Response {
        status: 200,
        headers: Vec::new(),
        body: vec![0xff, 0xfe],
    };
    assert_eq!(
        invalid.text_body().unwrap_err().kind(),
        TransportErrorKind::Body
    );
}

#[test]
fn real_transport_reports_a_connect_failure_without_a_network_service() {
    let transport = UreqTransport::new(TransportLimits {
        connect_timeout: std::time::Duration::from_secs(2),
        ..TransportLimits::default()
    });
    let error = transport
        .send(&Request::get("http://127.0.0.1:1/"))
        .unwrap_err();
    assert!(
        matches!(
            error.kind(),
            TransportErrorKind::Connect | TransportErrorKind::Timeout
        ),
        "unexpected kind: {:?}",
        error.kind()
    );
}
