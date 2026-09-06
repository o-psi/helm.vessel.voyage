//! Inert test-only seeder for the actual offline CLI lifecycle fixture.
#[test]
fn seed_orphan_journal() {
    let Ok(root) = std::env::var("HELM_GITHUB_JOURNAL_SEED") else {
        return;
    };
    use super::{
        publication::{Action, Actor, Draft},
        repository::Object,
        store::{Owner, Store},
    };
    let workspace = std::path::PathBuf::from(root).join("deleted-workspace");
    std::fs::create_dir(&workspace).unwrap();
    let owner = Owner::new(&workspace, Some(uuid::Uuid::new_v4()), None).unwrap();
    let mut store = Store::open(Store::default_path()).unwrap();
    let mut output = Vec::new();
    for index in 0..3 {
        let operation = store
            .prepare(
                Draft {
                    object: Object::parse("https://github.com/fixture/repo/issues/1").unwrap(),
                    action: Action::Comment {
                        body: format!("Private orphan fixture {index}"),
                    },
                },
                Actor {
                    id: 7,
                    login: "fixture".into(),
                },
                "fixture-policy".into(),
                None,
                None,
                owner.clone(),
            )
            .unwrap();
        if index == 2 {
            store.begin_send(&operation).unwrap();
        }
        output.push(serde_json::json!({"id":operation.id,"digest":operation.digest}));
    }
    drop(store);
    std::fs::remove_dir(&workspace).unwrap();
    println!("SEED_JSON={}", serde_json::to_string(&output).unwrap());
}
