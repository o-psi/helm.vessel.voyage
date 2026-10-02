//! Post-restoration service boundary only. Real private files/receipt checks,
//! existing scripted manager/PID seam: no native systemd or SQLite rollback claim.
use super::*;

const NAME: &str = "voyage-vessel.service";
struct Restored {
    f: Fixture,
    record: Record,
    previous: PathBuf,
    candidate: PathBuf,
    activation: service::Activation,
    unit: PathBuf,
    quarantine: PathBuf,
    retained: Vec<(PathBuf, Vec<u8>)>,
}
impl Restored {
    fn new() -> Self {
        let f = installation();
        let mut record = record("unconfirmed");
        let previous = installation_root()
            .unwrap()
            .join("releases")
            .join(&record.current_release);
        let candidate = installation_root()
            .unwrap()
            .join("releases")
            .join(record.release_id.as_ref().unwrap());
        for release in [&previous, &candidate] {
            for binary in ["helm", "vessel", "voyage"] {
                let relative = release
                    .strip_prefix(&f.root)
                    .unwrap()
                    .join("bin")
                    .join(binary);
                f.script(
                    relative.to_str().unwrap(),
                    if binary == "helm" {
                        "printf '[]'"
                    } else {
                        "exit 0"
                    },
                );
            }
        }
        // Obtain both exact unit definitions through the maintained public
        // preview; do not copy its renderer into this fixture.
        inactive_plan(&f);
        let original = service::preview(&previous.join("bin"), false).unwrap();
        inactive_plan(&f);
        let failed = service::preview(&candidate.join("bin"), false).unwrap();
        let original = original.split_once("\nUnit: ").unwrap().0.to_owned();
        let failed = failed.split_once("\nUnit: ").unwrap().0.to_owned();
        files::private_directory(&f.root.join("units")).unwrap();
        files::private_directory(&f.root.join("state")).unwrap();
        let unit = f.root.join("units").join(NAME);
        fs::write(&unit, failed).unwrap();
        let activation = service::Activation {
            active: true,
            enabled: true,
            unit_file_state: "enabled".into(),
            definition: Some(original),
            state: f.root.join("state"),
        };
        record.legacy_mode = true;
        record.supervisor_activation = Some(activation.clone());
        record.gateways = vec![
            Gateway {
                unit: "first.service".into(),
                definition: "ExecStart=approved-first\n".into(),
            },
            Gateway {
                unit: "second.service".into(),
                definition: "ExecStart=approved-second\n".into(),
            },
        ];
        save(
            &mut record,
            "unconfirmed",
            "original failure retained; no replay",
        )
        .unwrap();
        let quarantine = activation.state.join("update-quarantine.json");
        files::atomic_json(&quarantine,&serde_json::json!({"operation_id":record.operation_id,"previous":record.current_release,"target":record.release_id})).unwrap();
        let retained = vec![
            path(OP).unwrap(),
            quarantine.clone(),
            installation_root().unwrap().join("transaction.json"),
        ]
        .into_iter()
        .map(|path| {
            let bytes = fs::read(&path).unwrap();
            (path, bytes)
        })
        .collect();
        Self {
            f,
            record,
            previous,
            candidate,
            activation,
            unit,
            quarantine,
            retained,
        }
    }
    fn supervisor(&self, fail_start: bool) {
        self.f.effective(true);
        self.f.query("UnitFileState", "enabled");
        self.f.query("ActiveState", "failed");
        self.f.call(&["daemon-reload"], "");
        self.f.effective(true);
        self.f.call(&["reset-failed", NAME], "");
        if fail_start {
            self.f.fail(
                &["--no-block", "start", NAME],
                "FIXTURE-SUPERVISOR-START-REFUSED",
            );
        } else {
            self.f.call(&["--no-block", "start", NAME], "");
            self.f.query("ActiveState", "active");
            self.f.query("MainPID", "72");
        }
        process(&self.f, 72, &self.previous.join("bin/vessel"), &["vessel"]);
    }
    fn gateway(&self, index: usize, variant: u8) {
        let gateway = &self.record.gateways[index];
        fs::write(
            self.f.root.join("units").join(&gateway.unit),
            if variant == 1 {
                "ExecStart=operator-change\n"
            } else {
                &gateway.definition
            },
        )
        .unwrap();
        definition(
            &self.f,
            &gateway.unit,
            if variant == 1 {
                "ExecStart=operator-change\n"
            } else {
                &gateway.definition
            },
        );
        if variant == 1 {
            return;
        }
        self.f.call(&["reset-failed", &gateway.unit], "");
        if variant == 2 {
            self.f
                .fail(&["restart", &gateway.unit], "FIXTURE-GATEWAY-START-REFUSED");
            return;
        }
        self.f.call(&["restart", &gateway.unit], "");
        self.f.call(
            &["show", &gateway.unit, "--property=ActiveState", "--value"],
            if variant == 3 { "failed" } else { "active" },
        );
        if variant == 3 {
            return;
        }
        let pid = 81 + index as u32;
        self.f.call(
            &["show", &gateway.unit, "--property=MainPID", "--value"],
            &pid.to_string(),
        );
        process(
            &self.f,
            pid,
            &if variant == 4 {
                self.candidate.join("bin/vessel")
            } else {
                self.previous.join("bin/vessel")
            },
            &["vessel"],
        );
    }
    fn observed(&self, candidate: bool) {
        self.f.call(
            &["show", NAME, "--property=ActiveState", "--value"],
            "active",
        );
        self.f
            .call(&["show", NAME, "--property=MainPID", "--value"], "91");
        process(
            &self.f,
            91,
            &if candidate {
                self.candidate.join("bin/vessel")
            } else {
                self.previous.join("bin/vessel")
            },
            &["vessel"],
        );
    }
    fn restore(&self) -> Result<()> {
        restore_legacy_services(
            &self.record,
            &self.previous,
            &self.candidate,
            &self.activation,
        )
    }
    fn retained(&self) {
        assert_eq!(current().unwrap(), self.record.current_release);
        assert_eq!(load(OP).unwrap().phase, "unconfirmed");
        for (path, before) in &self.retained {
            assert_eq!(&fs::read(path).unwrap(), before);
        }
        assert!(self.quarantine.is_file());
        assert_eq!(
            fs::read_to_string(&self.unit).unwrap(),
            self.activation.definition.as_deref().unwrap()
        );
        self.f.done();
    }
}

#[test]
fn original_supervisor_start_failure_still_attempts_every_unchanged_gateway_and_old_image_observation()
 {
    let owned = Restored::new();
    owned.supervisor(true);
    owned.gateway(0, 0);
    owned.gateway(1, 0);
    owned.observed(false);
    let error = owned.restore().unwrap_err().to_string();
    assert!(error.contains("FIXTURE-SUPERVISOR-START-REFUSED"));
    assert!(error.contains("unconfirmed"));
    owned.retained();
}

#[test]
fn changed_or_failing_first_gateway_is_preserved_or_refused_and_second_is_independently_attempted()
{
    for variant in 1..5 {
        let owned = Restored::new();
        owned.supervisor(true);
        owned.gateway(0, variant);
        owned.gateway(1, 0);
        owned.observed(false);
        let error = owned.restore().unwrap_err().to_string();
        assert!(error.contains("FIXTURE-SUPERVISOR-START-REFUSED"));
        assert!(error.contains("first.service"));
        if variant == 1 {
            assert!(error.contains("changed gateway"));
            assert_eq!(
                fs::read_to_string(owned.f.root.join("units/first.service")).unwrap(),
                "ExecStart=operator-change\n"
            );
        }
        if variant == 2 {
            assert!(error.contains("FIXTURE-GATEWAY-START-REFUSED"));
        }
        owned.retained();
    }
}

#[test]
fn pointer_and_gateway_readiness_cannot_mask_a_candidate_supervisor_image() {
    let owned = Restored::new();
    owned.supervisor(true);
    owned.gateway(0, 0);
    owned.gateway(1, 0);
    owned.observed(true);
    let error = owned.restore().unwrap_err().to_string();
    assert!(error.contains("FIXTURE-SUPERVISOR-START-REFUSED"));
    assert!(error.contains("Service executable not verified"));
    owned.retained();
}

#[test]
fn all_service_observations_can_succeed_without_this_boundary_promoting_receipt_or_clearing_quarantine()
 {
    let owned = Restored::new();
    owned.supervisor(false);
    owned.gateway(0, 0);
    owned.gateway(1, 0);
    owned.observed(false);
    owned.restore().unwrap();
    owned.retained();
    // apply_legacy must still verify original namespace/context before clearing
    // quarantine. This service-only result does not declare a full restoration.
}
