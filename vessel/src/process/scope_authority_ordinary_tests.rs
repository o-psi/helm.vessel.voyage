//! Ordinary binding checks and private framing; successful protected/root leases
//! remain outside this cohort. No fake root authority is minted.
use super::*;
use crate::process::test_support::Fixture;
#[test]
fn lease_identity_separates_every_retained_scope_and_never_exposes_its_inputs() {
    let f = Fixture::new();
    let r = f.registration();
    let g = f.session();
    let b = GrantBinding {
        grant_id: g.grant_id,
        principal_id: g.principal_id,
        revision: g.revision,
    };
    let socket = socket_name(&f.0);
    let id = lease_identity(&r, &b, "synthetic-epoch", &socket, false).unwrap();
    assert_eq!(
        id,
        lease_identity(&r, &b, "synthetic-epoch", &socket, false).unwrap()
    );
    for changed in 0..8 {
        let mut registration = r.clone();
        let mut binding = b.clone();
        let mut epoch = "synthetic-epoch";
        let mut target = socket.clone();
        let mut observer = false;
        match changed {
            0 => registration.session_id = Uuid::new_v4(),
            1 => registration.incarnation = Uuid::new_v4(),
            2 => binding.grant_id = Uuid::new_v4(),
            3 => binding.principal_id = Uuid::new_v4(),
            4 => binding.revision += 1,
            5 => epoch = "different-epoch",
            6 => target = guardian_socket_name(&f.0, r.session_id, r.incarnation),
            _ => observer = true,
        };
        assert_ne!(
            id,
            lease_identity(&registration, &binding, epoch, &target, observer).unwrap()
        );
    }
    assert!(!f.0.join("access/runtime-scope-leases").exists());
}
#[test]
fn ordinary_scope_binding_rejects_private_grant_changes_and_parent_revision_reuse() {
    for field in 0..7 {
        let f = Fixture::new();
        let registration = f.registration();
        let mut g = f.session();
        g.session_id = registration.session_id;
        f.save_session(&g);
        let b = GrantBinding {
            grant_id: g.grant_id,
            principal_id: g.principal_id,
            revision: g.revision,
        };
        let old = std::fs::read(store::grant_path(&f.0, g.grant_id)).unwrap();
        assert!(current_grant(&f.0, &registration, &b).is_ok());
        match field {
            0 => g.principal_id = Uuid::new_v4(),
            1 => g.revision += 1,
            2 => g.workspace = f.0.join("outside"),
            3 => g.session_id = Uuid::new_v4(),
            4 => g.rights.clear(),
            5 => g.expires_at_ms = 0,
            _ => g.revoked = true,
        };
        f.save_session(&g);
        let changed = std::fs::read(store::grant_path(&f.0, g.grant_id)).unwrap();
        assert_ne!(old, changed);
        assert!(current_grant(&f.0, &registration, &b).is_err());
        assert_eq!(
            std::fs::read(store::grant_path(&f.0, g.grant_id)).unwrap(),
            changed
        );
    }
}
#[tokio::test]
async fn private_guardian_frame_is_owner_bound_and_refuses_secret_extra_fields_without_leases() {
    let f = Fixture::new();
    let registration = f.registration();
    let request = ScopeCheck {
        schema: 1,
        session_id: registration.session_id,
        incarnation: registration.incarnation,
        token: "a".repeat(64),
        handle: ExecutionScopeHandle {
            socket_name: socket_name(&f.0),
            lease_id: Uuid::new_v4(),
            secret: "b".repeat(64),
        },
    };
    for variant in 0..3 {
        let (mut client, server) = UnixStream::pair().unwrap();
        let root = f.0.clone();
        let owner = if variant == 0 {
            Some((Uuid::new_v4(), registration.incarnation))
        } else {
            Some((registration.session_id, registration.incarnation))
        };
        let task = tokio::spawn(async move {
            connection_for(root, server, unsafe { libc::geteuid() }, owner).await
        });
        let mut value = serde_json::to_value(&request).unwrap();
        if variant == 1 {
            value["private_extra"] = serde_json::json!("synthetic-private-sentinel");
        }
        if variant == 2 {
            value["incarnation"] = serde_json::json!(Uuid::nil());
        }
        let bytes = serde_json::to_vec(&value).unwrap();
        client.write_u32(bytes.len() as u32).await.unwrap();
        client.write_all(&bytes).await.unwrap();
        client.shutdown().await.unwrap();
        let reply = tokio::time::timeout(Duration::from_secs(3), async {
            let len = client.read_u32().await.unwrap();
            let mut bytes = vec![0; len as usize];
            client.read_exact(&mut bytes).await.unwrap();
            bytes
        })
        .await
        .unwrap();
        assert_eq!(reply, br#"{"scope":null}"#);
        task.await.unwrap().unwrap();
        assert!(!f.0.join("access/runtime-scope-leases").exists());
    }
}
