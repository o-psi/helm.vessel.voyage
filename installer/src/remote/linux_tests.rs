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
        contracts_sha256: None,
        supervisor_activation: None,
        legacy_mode: false,
        legacy_proof: None,
        legacy_accounts: None,
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
fn declare_contract(bin: &std::path::Path) {
    let path = bin.parent().unwrap().join("release.json");
    let mut manifest: Manifest = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    manifest.update_compatibility = Some(crate::install::release::UpdateCompatibility {
        schema_version: 1,
        formats: [
            "catalogue_read",
            "catalogue_write",
            "journal_read",
            "journal_write",
            "process_protocol",
            "vessel_protocol",
            "execution_identity",
        ]
        .into_iter()
        .map(|name| {
            (
                name.into(),
                if name.ends_with("_read") {
                    vec![1, 2]
                } else {
                    vec![1]
                },
            )
        })
        .collect(),
        implementation_sha256: "a".repeat(64),
        build_inputs_sha256: "b".repeat(64),
    });
    fs::write(path, serde_json::to_vec(&manifest).unwrap()).unwrap();
}

fn installed_release(f: &Fixture, r: &mut Record) -> PathBuf {
    let bin = crate::fixture_tests::release(f, "candidate", "1.0.0");
    declare_contract(&bin);
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
    declare_contract(&bin);
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
    inactive_plan(&f); // Fresh activation/definition pin before publication.
    inactive_plan(&f); // Configuration observes the same unchanged unit.
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
    installed_release(&f, &mut r);
    let bin = crate::fixture_tests::release(&f, "candidate-refusal", "1.0.1");
    declare_contract(&bin);
    r.release_id = Some(Manifest::inspect(&bin).unwrap().id().unwrap());
    r.contracts_sha256 = Some([
        contract_id(&installed_manifest(&r.current_release).unwrap()).unwrap(),
        contract_id(&Manifest::inspect(&bin).unwrap()).unwrap(),
    ]);
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
fn transitioning_supervisor_is_refused_before_binary_publication() {
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
    assert!(format!("{:#}", apply_worker(&mut r).unwrap_err()).contains("transitioning"));
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

#[test]
fn declared_rollback_reader_must_cover_every_candidate_catalogue_and_journal_writer() {
    let f = installation();
    let mut r = record("preparing");
    let installed = installed_release(&f, &mut r);
    let bin = candidate(&f, "2.0.0", "printf 1", "exit 0");
    let old = Manifest::inspect(&installed.join("bin")).unwrap();
    let new = Manifest::inspect(&bin).unwrap();
    rollback_formats(&old, &new).unwrap();
    for variant in 0..4 {
        let mut prior = old.clone();
        let mut next = new.clone();
        match variant {
            0 => prior.update_compatibility = None,
            1 => next.update_compatibility = None,
            2 => {
                let formats = &mut next.update_compatibility.as_mut().unwrap().formats;
                formats.insert("catalogue_read".into(), vec![1, 3]);
                formats.insert("catalogue_write".into(), vec![3]);
            }
            _ => {
                let formats = &mut next.update_compatibility.as_mut().unwrap().formats;
                formats.insert("journal_read".into(), vec![1, 3]);
                formats.insert("journal_write".into(), vec![3]);
            }
        }
        assert!(rollback_formats(&prior, &next).is_err());
        assert_eq!(current().unwrap(), r.current_release);
    }
    f.done();
}

#[test]
fn legacy_unknown_rollback_contract_is_refused_before_review_publication_or_units() {
    let f = installation();
    let mut r = record("preparing");
    let release = installed_release(&f, &mut r);
    let path = release.join("release.json");
    let mut legacy: Manifest = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    legacy.update_compatibility = None;
    fs::write(path, serde_json::to_vec(&legacy).unwrap()).unwrap();
    let bin = candidate(&f, "2.0.0", "printf 1", "exit 0");
    acquisition(&bin);
    f.call(LIST, "[]");
    let error = prepare_worker(&mut r).unwrap_err().to_string();
    assert!(error.contains("legacy migration/backup plan"));
    assert_eq!(current().unwrap(), r.current_release);
    assert!(
        r.release_id.as_deref() != Some(Manifest::inspect(&bin).unwrap().id().unwrap().as_str())
    );
    assert!(r.staging_root.is_none());
    assert!(
        fs::read_dir(f.root.join(".cache/voyage/upgrades"))
            .unwrap()
            .next()
            .is_none()
    );
    f.done();
}

#[test]
fn rollback_gateway_matrix_requires_unchanged_definition_active_previous_executable_and_continues_peers()
 {
    for variant in 0..4 {
        let f = installation();
        let mut r = record("applying");
        let previous = installed_release(&f, &mut r);
        let gateway = Gateway {
            unit: "first.service".into(),
            definition: "ExecStart=stable\n".into(),
        };
        r.gateways = vec![
            gateway.clone(),
            Gateway {
                unit: "second.service".into(),
                definition: "ExecStart=stable\n".into(),
            },
        ];
        for (index, gateway) in r.gateways.iter().enumerate() {
            definition(
                &f,
                &gateway.unit,
                if variant == 1 && index == 0 {
                    "changed\n"
                } else {
                    "ExecStart=stable\n"
                },
            );
            if variant == 1 && index == 0 {
                continue;
            }
            f.call(&["reset-failed", &gateway.unit], "");
            f.call(&["restart", &gateway.unit], "");
            f.call(
                &["show", &gateway.unit, "--property=ActiveState", "--value"],
                if variant == 2 && index == 0 {
                    "failed"
                } else {
                    "active"
                },
            );
            if variant == 2 && index == 0 {
                continue;
            }
            let pid = 21 + index as u32;
            f.call(
                &["show", &gateway.unit, "--property=MainPID", "--value"],
                &pid.to_string(),
            );
            let executable = if variant == 3 && index == 0 {
                f.root.join("candidate/vessel")
            } else {
                previous.join("bin/vessel")
            };
            process(&f, pid, &executable, &["vessel"]);
        }
        assert_eq!(rollback_gateways(&r, &previous).is_ok(), variant == 0);
        f.done();
    }
}

#[test]
fn supervisor_readiness_requires_actual_previous_pid_not_only_previous_pointer() {
    for candidate in [false, true] {
        let f = installation();
        let mut r = record("applying");
        let previous = installed_release(&f, &mut r);
        f.call(
            &[
                "show",
                "voyage-vessel.service",
                "--property=ActiveState",
                "--value",
            ],
            "active",
        );
        f.call(
            &[
                "show",
                "voyage-vessel.service",
                "--property=MainPID",
                "--value",
            ],
            "31",
        );
        let executable = if candidate {
            f.root.join("new-release/bin/vessel")
        } else {
            previous.join("bin/vessel")
        };
        process(&f, 31, &executable, &["vessel"]);
        assert_eq!(
            verified_service("voyage-vessel.service", &previous).is_ok(),
            !candidate
        );
        assert_eq!(current().unwrap(), r.current_release);
        f.done();
    }
}

#[test]
fn approved_contract_change_is_refused_even_when_legacy_release_identity_is_unchanged() {
    let f = installation();
    let mut r = record("preparing");
    installed_release(&f, &mut r);
    let bin = candidate(&f, "2.0.0", "printf 1", "exit 0");
    acquisition(&bin);
    f.call(LIST, "[]");
    inactive_plan(&f);
    prepare_worker(&mut r).unwrap();
    let prepared = r.bin_dir.as_ref().unwrap();
    let path = prepared.parent().unwrap().join("release.json");
    let mut changed: Manifest = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    let unchanged_id = changed.id().unwrap();
    changed
        .update_compatibility
        .as_mut()
        .unwrap()
        .build_inputs_sha256 = "c".repeat(64);
    assert_eq!(changed.id().unwrap(), unchanged_id);
    fs::write(path, serde_json::to_vec(&changed).unwrap()).unwrap();
    assert!(
        apply_worker(&mut r)
            .unwrap_err()
            .to_string()
            .contains("not pinned before approval")
    );
    assert_eq!(current().unwrap(), r.current_release);
    assert!(r.staging_root.as_ref().unwrap().exists());
    cleanup_staging(&mut r).unwrap();
    f.done();
}

#[test]
fn current_installer_normal_legacy_entry_refuses_inactive_original_before_publication_or_old_updater_call()
 {
    let f = installation();
    let mut r = record("preparing");
    let installed = installed_release(&f, &mut r);
    let path = installed.join("release.json");
    let mut old: Manifest = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    old.version = "1.0.2".into();
    old.update_compatibility = None;
    fs::write(&path, serde_json::to_vec(&old).unwrap()).unwrap();
    let identity = old.id().unwrap();
    let renamed = installed.parent().unwrap().join(&identity);
    fs::rename(&installed, &renamed).unwrap();
    fs::remove_file(installation_root().unwrap().join("current")).unwrap();
    std::os::unix::fs::symlink(&renamed, installation_root().unwrap().join("current")).unwrap();
    files::atomic_json(
        &installation_root().unwrap().join("transaction.json"),
        &serde_json::json!({"schema_version":1,"current":identity,"previous":null,"pending":null}),
    )
    .unwrap();
    let bin = candidate(&f, "1.0.3", "printf 1", "exit 0");
    let options = cli::Options::parse(&[
        "install".into(),
        "--bin-dir".into(),
        bin.to_string_lossy().into_owned(),
    ])
    .unwrap();
    let report = flow::plan(&options).unwrap();
    inactive_plan(&f);
    assert!(
        local_legacy_review(&options, &report)
            .unwrap_err()
            .to_string()
            .contains("original supervisor active")
    );
    assert_eq!(current().unwrap(), identity);
    assert!(!root().unwrap().join("latest.json").exists());
    f.done();
}
