use super::*;
#[test]
fn legacy_prefixes_and_binary_plaintext_remain_unchanged() {
    for length in 0..MAGIC.len() {
        let bytes = &MAGIC[..length];
        assert!(!encrypted(bytes));
        assert_eq!(open(b"fixture-purpose", bytes).unwrap(), bytes);
    }
    for bytes in [b"\0\xff\x80legacy".as_slice(), b"plaintext".as_slice()] {
        assert!(!encrypted(bytes));
        assert_eq!(open(b"purpose", bytes).unwrap(), bytes);
    }
    assert!(encrypted(MAGIC));
    assert!(open_with(&[0; 32], b"purpose", MAGIC).is_err());
}
#[test]
fn envelopes_cover_empty_binary_and_large_payloads_without_mutating_inputs() {
    let key = [19; 32];
    for plaintext in [vec![], vec![0, 255, 128, 1], vec![42; 131072]] {
        let envelope = seal_with(&key, b"fixture\0purpose", &plaintext).unwrap();
        let saved = envelope.clone();
        assert!(encrypted(&envelope));
        assert_eq!(envelope.len(), OVERHEAD + plaintext.len());
        assert_eq!(
            open_with(&key, b"fixture\0purpose", &envelope).unwrap(),
            plaintext
        );
        assert!(open_with(&key, b"fixture", &envelope).is_err());
        assert_eq!(envelope, saved);
        let mut appended = envelope.clone();
        appended.push(0);
        assert!(open_with(&key, b"fixture\0purpose", &appended).is_err());
    }
}
#[test]
fn minimum_sized_forgery_and_wrong_magic_are_rejected() {
    let mut forged = MAGIC.to_vec();
    forged.resize(OVERHEAD, 0);
    assert!(
        open_with(&[0; 32], b"", &forged)
            .unwrap_err()
            .to_string()
            .contains("damaged")
    );
    forged[0] ^= 1;
    assert!(
        open_with(&[0; 32], b"", &forged)
            .unwrap_err()
            .to_string()
            .contains("invalid encrypted")
    );
}
