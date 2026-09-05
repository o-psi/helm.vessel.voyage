use super::*;
const ORIGIN: &str = "https://vessel.example";
fn challenge(key: &SigningKey) -> Challenge {
    Challenge {
        version: 2,
        id: Uuid::new_v4(),
        origin: ORIGIN.into(),
        expires_at_ms: 1000,
        server_tag: vec![0; 32],
        operation: ProofOperation::Enroll {
            transaction_id: Uuid::new_v4(),
            invitation_id: Uuid::new_v4(),
            machine_id: Uuid::new_v4(),
            public_key: key.public_key(),
        },
    }
}
fn signed(key: &SigningKey, challenge: Challenge) -> SignedChallenge {
    SignedChallenge {
        signature: key.sign(&challenge).unwrap(),
        challenge,
        new_signature: None,
    }
}
#[test]
fn proof_roundtrip_private_key_restore_and_safe_diagnostics() {
    let key = SigningKey::generate().unwrap();
    let restored = SigningKey::from_pkcs8(key.as_pkcs8()).unwrap();
    assert_eq!(key.public_key(), restored.public_key());
    let proof = signed(&key, challenge(&key));
    let bytes = serde_json::to_vec(&proof).unwrap();
    let decoded = SignedChallenge::decode(&bytes).unwrap();
    decoded.verify(&key.public_key(), ORIGIN, 1).unwrap();
    assert_eq!(decoded, proof);
    assert_eq!(
        key.sign(&proof.challenge).unwrap(),
        restored.sign(&proof.challenge).unwrap()
    );
    for text in [
        format!("{proof:?}"),
        format!("{:?}", proof.challenge),
        format!("{:?}", proof.challenge.operation),
    ] {
        assert!(!text.contains(ORIGIN));
        assert!(!text.contains(&proof.challenge.id.to_string()));
    }
    assert!(SigningKey::from_pkcs8(b"private-key-sentinel").is_err());
}
#[test]
fn every_signed_field_is_bound_and_wrong_keys_fail() {
    let key = SigningKey::generate().unwrap();
    let proof = signed(&key, challenge(&key));
    let mut variants = vec![];
    let mut p = proof.clone();
    p.challenge.version = 3;
    variants.push(p);
    let mut p = proof.clone();
    p.challenge.id = Uuid::new_v4();
    variants.push(p);
    let mut p = proof.clone();
    p.challenge.origin = "https://attacker.example".into();
    variants.push(p);
    let mut p = proof.clone();
    p.challenge.expires_at_ms += 1;
    variants.push(p);
    for field in 0..4 {
        let mut p = proof.clone();
        if let ProofOperation::Enroll {
            transaction_id,
            invitation_id,
            machine_id,
            public_key,
        } = &mut p.challenge.operation
        {
            match field {
                0 => *transaction_id = Uuid::new_v4(),
                1 => *invitation_id = Uuid::new_v4(),
                2 => *machine_id = Uuid::new_v4(),
                _ => public_key[0] ^= 1,
            }
        }
        variants.push(p);
    }
    for p in variants {
        assert!(p.verify(&key.public_key(), ORIGIN, 1).is_err());
    }
    assert!(
        proof
            .verify(&SigningKey::generate().unwrap().public_key(), ORIGIN, 1)
            .is_err()
    );
    let mut p = proof;
    p.signature[0] ^= 1;
    assert!(p.verify(&key.public_key(), ORIGIN, 1).is_err());
}
#[test]
fn rotation_requires_both_keys_for_the_same_exact_action() {
    let old = SigningKey::generate().unwrap();
    let new = SigningKey::generate().unwrap();
    let mut c = challenge(&old);
    c.operation = ProofOperation::Rotate {
        machine_id: Uuid::new_v4(),
        epoch: 4,
        transaction_id: Uuid::new_v4(),
        new_public_key: new.public_key(),
    };
    let mut p = signed(&old, c);
    assert_eq!(
        p.verify(&old.public_key(), ORIGIN, 1),
        Err(ProofError::InvalidNewSignature)
    );
    p.new_signature = Some(old.sign(&p.challenge).unwrap());
    assert!(p.verify(&old.public_key(), ORIGIN, 1).is_err());
    p.new_signature = Some(new.sign(&p.challenge).unwrap());
    p.verify(&old.public_key(), ORIGIN, 1).unwrap();
    let mut altered = p.challenge.clone();
    altered.expires_at_ms += 1;
    p.new_signature = Some(new.sign(&altered).unwrap());
    assert!(p.verify(&old.public_key(), ORIGIN, 1).is_err());
    let mut p = signed(&old, challenge(&old));
    p.new_signature = Some(new.sign(&p.challenge).unwrap());
    assert_eq!(
        p.verify(&old.public_key(), ORIGIN, 1),
        Err(ProofError::InvalidNewSignature)
    );
}
#[test]
fn expiry_ids_and_epoch_boundaries_fail_closed() {
    let key = SigningKey::generate().unwrap();
    let mut c = challenge(&key);
    c.expires_at_ms = 60_001;
    c.validate(ORIGIN, 1).unwrap();
    for now in [-1, 0, 60_001, i64::MIN, i64::MAX] {
        assert!(c.validate(ORIGIN, now).is_err());
    }
    c.id = Uuid::nil();
    assert_eq!(c.validate(ORIGIN, 1), Err(ProofError::InvalidId));
    c.id = Uuid::new_v4();
    for epoch in [0, i64::MAX as u64, u64::MAX] {
        c.operation = ProofOperation::Connect {
            machine_id: Uuid::new_v4(),
            epoch,
        };
        assert_eq!(c.validate(ORIGIN, 1), Err(ProofError::InvalidEpoch));
    }
    c.operation = ProofOperation::Connect {
        machine_id: Uuid::new_v4(),
        epoch: i64::MAX as u64 - 1,
    };
    c.validate(ORIGIN, 1).unwrap();
    c.operation = ProofOperation::Revoke {
        machine_id: Uuid::nil(),
        epoch: 1,
        transaction_id: Uuid::new_v4(),
    };
    assert_eq!(c.validate(ORIGIN, 1), Err(ProofError::InvalidId));
}
#[test]
fn malformed_duplicate_unknown_and_oversized_inputs_have_no_content_errors() {
    let key = SigningKey::generate().unwrap();
    let p = signed(&key, challenge(&key));
    let encoded = serde_json::to_string(&p).unwrap();
    for input in [
        encoded.clone() + "{}",
        encoded.replacen("\"version\":2", "\"version\":2,\"version\":2", 1),
        encoded.replacen(
            "\"type\":\"enroll\"",
            "\"type\":\"enroll\",\"type\":\"enroll\"",
            1,
        ),
        encoded.replacen(
            "\"version\":2",
            "\"unknown-secret-sentinel\":0,\"version\":2",
            1,
        ),
    ] {
        let e = SignedChallenge::decode(input.as_bytes()).unwrap_err();
        assert_eq!(e, ProofError::InvalidJson);
        assert!(!e.to_string().contains("sentinel"));
    }
    assert_eq!(
        SignedChallenge::decode(&vec![b' '; MAX_PROOF_BYTES + 1]).unwrap_err(),
        ProofError::TooLarge
    );
    for size in [0, 63, 65, MAX_PROOF_BYTES] {
        let mut bad = p.clone();
        bad.signature = vec![0; size];
        assert!(bad.verify(&key.public_key(), ORIGIN, 1).is_err());
    }
}
#[test]
fn canonical_transcript_is_key_order_independent_and_domain_separated() {
    let key = SigningKey::generate().unwrap();
    let c = challenge(&key);
    let bytes = c.signing_bytes().unwrap();
    assert!(bytes.starts_with(SIGNING_DOMAIN));
    let sorted = serde_json::to_vec(&serde_json::to_value(&c).unwrap()).unwrap();
    assert_eq!(
        Challenge::decode(&sorted).unwrap().signing_bytes().unwrap(),
        bytes
    );
    let pair = Ed25519KeyPair::from_pkcs8(key.as_pkcs8()).unwrap();
    let raw = pair
        .sign(&serde_json::to_vec(&c).unwrap())
        .as_ref()
        .to_vec();
    let p = SignedChallenge {
        challenge: c,
        signature: raw,
        new_signature: None,
    };
    assert_eq!(
        p.verify(&key.public_key(), ORIGIN, 1),
        Err(ProofError::InvalidSignature)
    );
}
