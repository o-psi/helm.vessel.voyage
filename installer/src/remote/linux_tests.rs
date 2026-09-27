use super::*;
use crate::fixture_tests::Fixture;
const OP: &str = "10000000-0000-4000-8000-000000000001";

fn args(values: &[&str]) -> Vec<String> {
    values.iter().map(|v| (*v).into()).collect()
}
fn installation() -> Fixture {
    let f = Fixture::new();
    let install = installation_root().unwrap();
    files::private_directory(&install).unwrap();
    let current = "a".repeat(64);
    files::atomic_json(
        &install.join("transaction.json"),
        &serde_json::json!({"current":current,"pending":null}),
    )
    .unwrap();
    std::os::unix::fs::symlink(
        install.join("releases").join(&current),
        install.join("current"),
    )
    .unwrap();
    f
}
fn record(phase: &str) -> Record {
    let mut record = Record {
        operation_id: OP.into(),
        channel: "nightly".into(),
        phase: String::new(),
        message: String::new(),
        created_at: now(),
        updated_at: now(),
        current_release: "a".repeat(64),
        release_id: Some("b".repeat(64)),
        version: Some("next".into()),
        description: Some("pinned".into()),
        bin_dir: None,
        staging_root: None,
        gateways: vec![],
    };
    save(&mut record, phase, "fixture").unwrap();
    record
}

const LIST: &[&str] = &[
    "list-units",
    "--type=service",
    "--state=running",
    "--output=json",
];
fn managed_binary(f: &Fixture) -> PathBuf {
    f.script(
        &format!("install/releases/{}/bin/vessel", "a".repeat(64)),
        "exit 0",
    )
}
fn process(f: &Fixture, pid: u32, exe: &std::path::Path, argv: &[&str]) {
    std::os::unix::fs::symlink(exe, f.root.join(format!("pid-{pid}"))).unwrap();
    fs::write(
        f.root.join(format!("cmdline-{pid}")),
        argv.join("\0") + "\0",
    )
    .unwrap();
}
fn definition(f: &Fixture, unit: &str, text: &str) {
    f.call(
        &[
            "show",
            unit,
            "--property=ExecStart,FragmentPath,DropInPaths",
        ],
        text,
    );
}
#[test]
fn bounded_commands_capture_both_streams_and_remove_logs_on_failure() {
    let f = installation();
    let script = f.script("command", "printf out; printf err >&2");
    assert_eq!(command(Command::new(script)).unwrap(), "outerr");
    for body in ["exit 7", "printf '\\377'", "while :; do :; done"] {
        let script = f.script("command", body);
        assert!(command_for(Command::new(script), Duration::from_millis(30)).is_err());
        assert!(
            !fs::read_dir(root().unwrap()).unwrap().any(|e| e
                .unwrap()
                .path()
                .extension()
                .is_some_and(|x| x == "log"))
        );
    }
    let script = f.script("command", "head -c 1048577 /dev/zero; while :; do :; done");
    assert!(
        command(Command::new(script))
            .unwrap_err()
            .to_string()
            .contains("did not confirm")
    );
    f.done();
}
#[test]
fn gateway_discovery_filters_processes_and_sorts_managed_units() {
    let f = installation();
    let exe = managed_binary(&f);
    let state = f.root.join(".local/state/voyage/vessel");
    let persistent = f.root.join(".local/bin/vessel");
    process(
        &f,
        11,
        &exe,
        &[
            persistent.to_str().unwrap(),
            "--process-directory",
            state.to_str().unwrap(),
        ],
    );
    process(&f, 12, &exe, &["vessel", "--process-directory", "/wrong"]);
    f.call(LIST, r#"[{}, {"unit":"voyage-vessel.service"}, {"unit":"bad-pid"}, {"unit":"missing"}, {"unit":"wrong-state"}, {"unit":"z.service"}, {"unit":"a.service"}]"#);
    for (name, pid) in [
        ("bad-pid", "no"),
        ("missing", "10"),
        ("wrong-state", "12"),
        ("z.service", "11"),
        ("a.service", "11"),
    ] {
        f.call(&["show", name, "--property=MainPID", "--value"], pid);
        if name.ends_with(".service") {
            definition(&f, name, "ExecStart=stable\n");
        }
    }
    let found = gateways().unwrap();
    assert_eq!(
        found.iter().map(|g| g.unit.as_str()).collect::<Vec<_>>(),
        ["a.service", "z.service"]
    );
    definition(&f, "a.service", "ExecStart=stable\n");
    assert!(unchanged_gateway(&found[0]).unwrap());
    definition(&f, "a.service", "ExecStart=changed\n");
    assert!(!unchanged_gateway(&found[0]).unwrap());
    f.done();
}
#[test]
fn gateway_discovery_refuses_unmanaged_commands_and_unsafe_unit_names() {
    for (name, managed, expected) in [
        ("gateway.service", false, "managed command"),
        ("bad/name.service", true, "Unsupported"),
        ("gateway.socket", true, "Unsupported"),
    ] {
        let f = installation();
        let exe = managed_binary(&f);
        let state = f.root.join(".local/state/voyage/vessel");
        let command = if managed {
            f.root.join(".local/bin/vessel")
        } else {
            exe.clone()
        };
        process(
            &f,
            21,
            &exe,
            &[
                command.to_str().unwrap(),
                "--process-directory",
                state.to_str().unwrap(),
            ],
        );
        f.call(LIST, &serde_json::json!([{"unit":name}]).to_string());
        f.call(&["show", name, "--property=MainPID", "--value"], "21");
        assert!(gateways().err().unwrap().to_string().contains(expected));
        f.done();
    }
}
#[test]
fn staging_cleanup_is_confined_and_idempotent() {
    let f = installation();
    let mut r = record("ready");
    for relative in [
        "outside",
        ".cache/voyage/upgrades/not-prepared",
        ".cache/voyage/upgrades/prepare-parent/prepare-child",
    ] {
        let p = f.root.join(relative);
        fs::create_dir_all(&p).unwrap();
        fs::write(p.join("preserve"), "data").unwrap();
        r.staging_root = Some(p.clone());
        assert!(cleanup_staging(&mut r).is_err());
        assert!(p.join("preserve").exists());
    }
    let p = f.root.join(".cache/voyage/upgrades/prepare-valid");
    fs::create_dir_all(&p).unwrap();
    r.bin_dir = Some(p.join("bin"));
    r.staging_root = Some(p.clone());
    cleanup_staging(&mut r).unwrap();
    assert!(!p.exists());
    assert!(r.bin_dir.is_none() && r.staging_root.is_none());
    r.staging_root = Some(p);
    cleanup_staging(&mut r).unwrap();
}
#[test]
fn prepare_admission_is_durable_idempotent_and_exclusive() {
    let f = installation();
    f.fail(&["launch-update", OP, "prepare"], "uncertain admission");
    run(&args(&["prepare", OP, "stable"])).unwrap();
    assert_eq!(load(OP).unwrap().phase, "preparing");
    run(&args(&["prepare", OP, "stable"])).unwrap();
    assert!(
        run(&args(&[
            "prepare",
            "20000000-0000-4000-8000-000000000002",
            "stable"
        ]))
        .is_err()
    );
    f.done();
}
#[test]
fn invalid_requests_and_locked_coordinator_never_launch() {
    let f = installation();
    for values in [
        vec![],
        vec!["worker", OP],
        vec!["status", "../escape"],
        vec!["unknown", OP],
        vec!["prepare", OP, "main"],
        vec!["status", OP, "extra"],
        vec!["discard", OP, "extra"],
        vec!["apply", OP],
    ] {
        assert!(run(&args(&values)).is_err());
    }
    record("ready");
    assert!(run(&args(&["worker", "prepare", OP])).is_err());
    record("applying");
    assert!(
        run(&args(&["worker", "invalid", OP]))
            .unwrap_err()
            .to_string()
            .contains("Invalid worker phase")
    );
    let _lock = files::lock(&root().unwrap().join("coordinator.lock")).unwrap();
    assert!(run(&args(&["status", OP])).is_err());
    f.done();
}
#[test]
fn stale_preparation_and_empty_latest_status_never_replay() {
    let f = installation();
    run(&args(&["status", "00000000-0000-0000-0000-000000000000"])).unwrap();
    let mut r = record("preparing");
    r.updated_at = now() - 1231;
    files::atomic_json(&path(OP).unwrap(), &r).unwrap();
    run(&args(&["status", OP])).unwrap();
    assert_eq!(load(OP).unwrap().phase, "failed");
    f.done();
}
fn installed_release(f: &Fixture, r: &mut Record) -> PathBuf {
    let bin = crate::fixture_tests::release(f, "candidate", "1.0.0");
    let manifest = Manifest::inspect(&bin).unwrap();
    let identity = manifest.id().unwrap();
    let install = installation_root().unwrap();
    let release = install.join("releases").join(&identity);
    fs::create_dir_all(release.parent().unwrap()).unwrap();
    fs::rename(bin.parent().unwrap(), &release).unwrap();
    fs::remove_file(install.join("current")).unwrap();
    std::os::unix::fs::symlink(&release, install.join("current")).unwrap();
    files::atomic_json(
        &install.join("transaction.json"),
        &serde_json::json!({"schema_version":1,"current":identity,"previous":null,"pending":null}),
    )
    .unwrap();
    r.current_release = identity;
    release
}
fn stopped(f: &Fixture) {
    let unit = format!("voyage-update-{OP}-apply.service");
    f.call(
        &["show", &unit, "--property=ActiveState", "--value"],
        "inactive",
    );
    f.call(&["show", &unit, "--property=Job", "--value"], "");
}
#[test]
fn reconciliation_confirms_both_approved_and_restored_releases_without_replay() {
    for approved in [true, false] {
        let f = installation();
        let mut r = record("unconfirmed");
        let release = installed_release(&f, &mut r);
        if approved {
            r.release_id = Some(r.current_release.clone());
        }
        r.gateways.push(Gateway {
            unit: "gateway.service".into(),
            definition: "ExecStart=stable\n".into(),
        });
        process(&f, 31, &release.join("bin/vessel"), &[]);
        stopped(&f);
        for unit in ["voyage-vessel.service", "gateway.service"] {
            f.call(
                &["show", unit, "--property=ActiveState", "--value"],
                "active",
            );
            f.call(&["show", unit, "--property=MainPID", "--value"], "31");
        }
        definition(&f, "gateway.service", "ExecStart=stable\n");
        reconcile(&mut r).unwrap();
        assert_eq!(
            load(OP).unwrap().phase,
            if approved { "complete" } else { "failed" }
        );
        assert!(load(OP).unwrap().message.contains("replayed"));
        f.done();
    }
}
#[test]
fn reconciliation_refuses_pending_jobs_inactive_services_and_wrong_processes() {
    for failure in ["job", "state", "pid", "exe", "definition"] {
        let f = installation();
        let mut r = record("unconfirmed");
        let release = installed_release(&f, &mut r);
        if failure == "job" {
            let unit = format!("voyage-update-{OP}-apply.service");
            f.call(
                &["show", &unit, "--property=ActiveState", "--value"],
                "failed",
            );
            f.call(&["show", &unit, "--property=Job", "--value"], "12");
        } else {
            stopped(&f);
            f.call(
                &[
                    "show",
                    "voyage-vessel.service",
                    "--property=ActiveState",
                    "--value",
                ],
                if failure == "state" {
                    "inactive"
                } else {
                    "active"
                },
            );
            if failure != "state" {
                f.call(
                    &[
                        "show",
                        "voyage-vessel.service",
                        "--property=MainPID",
                        "--value",
                    ],
                    if failure == "pid" { "invalid" } else { "41" },
                );
                process(
                    &f,
                    41,
                    &if failure == "exe" {
                        f.root.join("wrong")
                    } else {
                        release.join("bin/vessel")
                    },
                    &[],
                );
                if failure == "definition" {
                    r.gateways.push(Gateway {
                        unit: "gateway.service".into(),
                        definition: "old\n".into(),
                    });
                    f.call(
                        &[
                            "show",
                            "gateway.service",
                            "--property=ActiveState",
                            "--value",
                        ],
                        "active",
                    );
                    f.call(
                        &["show", "gateway.service", "--property=MainPID", "--value"],
                        "41",
                    );
                    definition(&f, "gateway.service", "changed\n");
                }
            }
        }
        assert!(reconcile(&mut r).is_err());
        assert_eq!(r.phase, "unconfirmed");
        f.done();
    }
}

// The source seam executes this offline copier, never the downloader. Candidate
// binaries remain real executable scripts, so protocol and loader checks run.
fn acquisition(bin: &std::path::Path) {
    crate::fixture_tests::set_acquire(&format!(
        r#"
import json,pathlib,shutil,sys
source=pathlib.Path({})
root=pathlib.Path(sys.argv[2])
shutil.copytree(source.parent, root, dirs_exist_ok=True)
(root/'prepared.json').write_text(json.dumps({{'bin_dir':str(root/'bin'),'description':'offline pinned candidate'}}))
"#,
        serde_json::to_string(&bin.to_string_lossy()).unwrap()
    ));
}
fn candidate(f: &Fixture, version: &str, protocol: &str, vessel: &str) -> PathBuf {
    let bin = crate::fixture_tests::release(f, "download", version);
    f.script("download/bin/voyage-installer", protocol);
    f.script("download/bin/vessel", vessel);
    let path = bin.parent().unwrap().join("release.json");
    let mut manifest: Manifest = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    for name in ["voyage-installer", "vessel"] {
        manifest.binaries.get_mut(name).unwrap().sha256 = files::hash(&bin.join(name)).unwrap();
    }
    fs::write(path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    bin
}
fn inactive_plan(f: &Fixture) {
    f.effective(false);
    f.query("ActiveState", "inactive");
    f.query("UnitFileState", "not-found");
    f.query("InvocationID", "");
}
#[test]
fn preparation_validates_downgrade_loader_protocol_and_retains_only_ready_staging() {
    for scenario in ["downgrade", "loader", "protocol", "ready", "same"] {
        let f = installation();
        let mut r = record("preparing");
        let installed = installed_release(&f, &mut r);
        let bin = if scenario == "same" {
            installed.join("bin")
        } else {
            candidate(
                &f,
                if scenario == "downgrade" {
                    "0.1.0"
                } else {
                    "2.0.0"
                },
                if scenario == "protocol" {
                    "printf 2"
                } else {
                    "printf 1"
                },
                if scenario == "loader" {
                    "exit 1"
                } else {
                    "printf 'vessel 2.0.0'"
                },
            )
        };
        acquisition(&bin);
        f.call(LIST, "[]");
        if scenario == "ready" {
            inactive_plan(&f);
        }
        let result = prepare_worker(&mut r);
        match scenario {
            "same" => {
                result.unwrap();
                assert_eq!(r.phase, "complete");
                assert!(r.staging_root.is_none());
            }
            "ready" => {
                result.unwrap();
                assert_eq!(r.phase, "ready");
                assert_eq!(r.version.as_deref(), Some("2.0.0"));
                assert!(r.staging_root.as_ref().unwrap().exists());
                assert_eq!(load(OP).unwrap().release_id, r.release_id);
                cleanup_staging(&mut r).unwrap();
            }
            _ => {
                let error = format!("{:#}", result.unwrap_err());
                assert!(
                    error.contains(match scenario {
                        "downgrade" => "older",
                        "loader" => "cannot run",
                        _ => "unsupported updater protocol",
                    }),
                    "{error}"
                );
                assert_eq!(r.phase, "preparing");
                assert!(r.staging_root.is_none());
            }
        }
        assert_eq!(current().unwrap(), r.current_release);
        f.done();
    }
}
#[test]
fn apply_publishes_reviewed_candidate_and_cleans_retained_staging() {
    let f = installation();
    let mut r = record("preparing");
    installed_release(&f, &mut r);
    let bin = candidate(&f, "2.0.0", "printf 1", "printf 'vessel 2.0.0'");
    acquisition(&bin);
    f.call(LIST, "[]");
    inactive_plan(&f);
    prepare_worker(&mut r).unwrap();
    let staging = r.staging_root.clone().unwrap();
    f.call(LIST, "[]");
    inactive_plan(&f);
    f.call(&["daemon-reload"], "");
    f.effective(true);
    apply_worker(&mut r).unwrap();
    assert_eq!(r.phase, "complete");
    assert_eq!(Some(current().unwrap()), r.release_id);
    assert!(!staging.exists());
    assert!(r.bin_dir.is_none());
    f.done();
}
#[test]
fn apply_rejects_missing_candidate_and_changed_gateway_before_publication() {
    let f = installation();
    let mut r = record("applying");
    managed_binary(&f);
    assert!(
        apply_worker(&mut r)
            .unwrap_err()
            .to_string()
            .contains("Prepared source missing")
    );
    let bin = crate::fixture_tests::release(&f, "candidate-refusal", "1.0.1");
    r.release_id = Some(Manifest::inspect(&bin).unwrap().id().unwrap());
    r.bin_dir = Some(bin);
    r.gateways.push(Gateway {
        unit: "old.service".into(),
        definition: "old\n".into(),
    });
    f.call(LIST, "[]");
    assert!(
        apply_worker(&mut r)
            .unwrap_err()
            .to_string()
            .contains("Gateway services changed")
    );
    assert_eq!(current().unwrap(), r.current_release);
    f.done();
}

#[test]
fn failed_service_configuration_rolls_back_published_binaries() {
    let f = installation();
    let mut r = record("preparing");
    installed_release(&f, &mut r);
    let bin = candidate(&f, "2.0.0", "printf 1", "exit 0");
    acquisition(&bin);
    f.call(LIST, "[]");
    inactive_plan(&f);
    prepare_worker(&mut r).unwrap();
    f.call(LIST, "[]");
    f.effective(false);
    f.query("ActiveState", "activating");
    assert!(
        apply_worker(&mut r)
            .unwrap_err()
            .to_string()
            .contains("transitioning")
    );
    assert_eq!(current().unwrap(), r.current_release);
    assert!(r.staging_root.as_ref().unwrap().exists());
    cleanup_staging(&mut r).unwrap();
    f.done();
}
#[test]
fn current_identity_requires_valid_journal_and_exact_pointer() {
    let f = installation();
    let journal = installation_root().unwrap().join("transaction.json");
    for value in [
        serde_json::json!({"pending":"other","current":"a".repeat(64)}),
        serde_json::json!({"current":null}),
        serde_json::json!({"current":"invalid"}),
        serde_json::json!({"current":"g".repeat(64)}),
        serde_json::json!({"current":"b".repeat(64)}),
    ] {
        files::atomic_json(&journal, &value).unwrap();
        assert!(current().is_err());
    }
    fs::write(&journal, b"invalid JSON").unwrap();
    assert!(current().is_err());
    f.done();
}
