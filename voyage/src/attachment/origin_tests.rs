use super::validate_origin;

#[test]
fn origin_normalization_preserves_only_tls_authority_or_opted_in_literal_loopback() {
    for (input, expected) in [
        ("https://EXAMPLE.test:443/", "https://example.test"),
        ("https://example.test:8443", "https://example.test:8443"),
        ("https://[::1]", "https://[::1]"),
    ] {
        assert_eq!(validate_origin(input, false).unwrap(), expected);
    }
    for input in [
        "http://127.0.0.1:8080",
        "http://127.2.3.4",
        "http://[::1]:8080",
    ] {
        assert!(validate_origin(input, false).is_err());
        assert_eq!(validate_origin(input, true).unwrap(), input);
    }
}

#[test]
fn origin_refuses_ambiguous_authority_and_every_non_origin_component() {
    for input in [
        "",
        "not a url",
        " https://example.test",
        "https://example.test ",
        "https://example.test\n",
        "https://example.test/secret",
        "https://example.test?",
        "https://example.test#",
        "https://name@example.test",
        "https://name:secret@example.test",
        "https://@example.test",
        "https://example.test\\evil",
        "ftp://example.test",
        "file:///",
        "http://localhost",
        "http://127.1",
        "http://2130706433",
        "http://0x7f000001",
        "http://192.0.2.1",
        "http://[::2]",
        "http://[broken",
    ] {
        assert!(validate_origin(input, true).is_err(), "accepted {input:?}");
    }
    assert!(validate_origin(&format!("https://{}", "a".repeat(2049)), true).is_err());
}
