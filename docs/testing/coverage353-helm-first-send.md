# Helm first-send and creation recovery assertions

Scope: [#353](https://github.com/o-psi/helm.vessel.voyage/issues/353).
The additional source cohort is prepared; execution and integrated workspace
coverage remain pending until the coordinated verification run.

`helm/src/process_client/ui/new_draft/plain_launch_tests.rs` uses the maintained
per-test private storage fixture and a bounded loopback public duplex peer. It
calls production `start_plain`, transport, launch configuration capture and
private journal code. It neither constructs a provider nor starts a Vessel or
Voyage process, reads human credentials, changes HOME, or claims native runtime
acceptance.

Assertions cover:

- Original creation and trusted configuration are durable before the first wire
  command; account/provider secrets are not needed for this metadata operation.
- The observed snapshot revision and original text are frozen in the durable
  first-turn envelope before transmission.
- A matching accepted receipt completes the retained record. Unknown, rejected
  and foreign-identity receipts preserve the original recovery envelope without
  another creation or submission command.
- Plain, configured and atomic account creation recover through exact resolution
  commands retaining the original identity, workspace, configuration and inference
  selection. Changing the retained account generation, model, thinking, service
  or account host cannot validate against the frozen account creation envelope.

After integration, the focused entry is:

```sh
cargo test -p helm --locked plain_launch_tests -j 1
```

Run it in the repository's existing bounded verification unit. Follow
[quality](../quality.md) and the full current-artifact Rust coverage requirements
before publishing this Rust test delivery. Synthetic duplex replies establish
client state and persistence behavior only; actual native processes, deployed
TLS, installation, providers and cleanup need their separate evidence.
