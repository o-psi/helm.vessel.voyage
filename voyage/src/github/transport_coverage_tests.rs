use super::*;
#[test]
fn http_failures_are_sanitized_and_never_prescribe_automatic_replay() {
    for (status, fragment) in [
        (200, ""),
        (201, ""),
        (204, ""),
        (301, "redirected"),
        (401, "authentication"),
        (403, "denied"),
        (404, "unavailable"),
        (410, "unavailable"),
        (422, "no automatic retry"),
        (429, "rate limited"),
        (500, "no automatic retry"),
    ] {
        let response = Response {
            status: StatusCode::from_u16(status).unwrap(),
            headers: header::HeaderMap::new(),
            bytes: b"private upstream diagnostic".to_vec(),
        };
        if status < 300 {
            check_status(&response).unwrap();
        } else {
            let error = check_status(&response).unwrap_err().to_string();
            assert!(error.contains(fragment), "{error}");
            assert!(!error.contains("private upstream"));
        }
    }
    let mut response = Response {
        status: StatusCode::TOO_MANY_REQUESTS,
        headers: header::HeaderMap::new(),
        bytes: vec![],
    };
    for (value, expected) in [
        ("30", true),
        ("86400", true),
        ("86401", false),
        ("invalid", false),
    ] {
        response
            .headers
            .insert(header::RETRY_AFTER, header::HeaderValue::from_static(value));
        assert_eq!(
            check_status(&response)
                .unwrap_err()
                .to_string()
                .contains("seconds"),
            expected
        );
    }
}
#[test]
fn json_and_credential_validation_are_bounded_without_sending_requests() {
    let response = |bytes: Vec<u8>| Response {
        status: StatusCode::OK,
        headers: header::HeaderMap::new(),
        bytes,
    };
    assert_eq!(
        response(b"{\"ok\":true}".to_vec()).json().unwrap()["ok"],
        true
    );
    assert!(
        response(b"private invalid JSON".to_vec())
            .json()
            .unwrap_err()
            .to_string()
            .contains("invalid JSON")
    );
    for token in [
        "".into(),
        "abc".into(),
        "test\nvalue".into(),
        "test value".into(),
        "x".repeat(4097),
    ] {
        assert!(Client::new(token).is_err());
    }
}

// Production callers retain the fixed HTTPS origin; fixtures use loopback only.
impl Client {
    pub(in crate::github) fn for_test(address: std::net::SocketAddr) -> Self {
        assert!(address.ip().is_loopback());
        let mut client = Self::new("offline-fixture-token".into()).unwrap();
        client.origin = reqwest::Url::parse(&format!("http://{address}/")).unwrap();
        client
    }
}
