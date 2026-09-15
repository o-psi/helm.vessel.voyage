# Startup resource contention (#283)

An accepted turn can construct its runtime before registering the session-owned
terminal manager. That registration previously used the journal's default
zero-wait SQLite connection. With DELETE journaling, a concurrent reader can
block its commit; another writer can block transaction admission. Both can fail
before inference even though subsequent failure/cleanup persistence succeeds.

Resource adoption and observed closure now opt into the existing two-second
SQLite statement wait budget inside their blocking workers. They restore the
nonblocking handler on success and error. The operation callback, provider calls
and tool effects are not replayed. Persistent contention still fails; an adoption
transaction that cannot commit rolls back. Cancellation/revocation does not erase
resource obligations or prevent recording observed cleanup; existing journal
identity and execution-authority boundaries remain in place.

Safe public startup labels distinguish preparation record reads, participant
configuration, terminal registration, and busy terminal registration. Raw errors,
paths and credentials are not exposed. This fixes an identified susceptible path;
the original incident discarded its exception, so its historical cause remains
unproven. Failed user submissions are not automatically replayed.

## Verification

Five focused tests cover writer and reader/commit contention, persistent lock
rollback, resource/observation uniqueness, handler restoration, bookkeeping after
cancellation/revocation, and safe classification/projection. Six existing
checkpoint regressions also pass. The final Linux workspace coverage run passed
529 tests, failed zero, and ignored one; `coverage/latest.json` records the dirty
source fingerprint and full current executable-set measurement. Line coverage is
41.55% (previously 41.43%); functions 39.51%; regions 40.58%.

These are offline Linux results, not native macOS/Windows or live-provider
verification. Existing unrelated staged Helm changes and deleted workflow scripts
were preserved, not included in the fix commit.
