-- Shipped v1.0.2 journal bootstrap, schema 12. Derived from that tag's
-- attachment/journal.rs plus imports, steering, catalogue, reconciliation and
-- notifications initialization. No current Goal or handoff schema is introduced.
PRAGMA foreign_keys=ON;
CREATE TABLE attachment_schema(id INTEGER PRIMARY KEY CHECK(id=1), version INTEGER NOT NULL);
CREATE TABLE sessions(id TEXT PRIMARY KEY, revision INTEGER NOT NULL CHECK(revision>=0), state TEXT NOT NULL, next_sequence INTEGER NOT NULL DEFAULT 1 CHECK(next_sequence>0));
CREATE TABLE runs(id TEXT PRIMARY KEY, session_id TEXT NOT NULL REFERENCES sessions(id), record TEXT NOT NULL, active INTEGER NOT NULL CHECK(active IN (0,1)));
CREATE UNIQUE INDEX one_active_run ON runs(session_id) WHERE active=1;
CREATE TABLE commands(id TEXT PRIMARY KEY, digest BLOB NOT NULL, run_id TEXT NOT NULL UNIQUE REFERENCES runs(id));
CREATE TABLE events(session_id TEXT NOT NULL REFERENCES sessions(id), sequence INTEGER NOT NULL, event TEXT NOT NULL, PRIMARY KEY(session_id,sequence));
CREATE TABLE imports(session_id TEXT PRIMARY KEY REFERENCES sessions(id), transfer_id TEXT NOT NULL UNIQUE, provenance TEXT NOT NULL);
CREATE TABLE steering(id TEXT PRIMARY KEY,run_id TEXT NOT NULL REFERENCES runs(id),session_id TEXT NOT NULL REFERENCES sessions(id),ordinal INTEGER NOT NULL CHECK(ordinal>0),machine_id TEXT NOT NULL,principal_id TEXT NOT NULL,digest BLOB NOT NULL,record TEXT NOT NULL,status TEXT NOT NULL CHECK(status IN ('queued','applied','not_applied')),UNIQUE(run_id,ordinal));
CREATE TABLE local_cancel_intents(run_id TEXT PRIMARY KEY REFERENCES runs(id), session_id TEXT NOT NULL REFERENCES sessions(id), installation_id TEXT NOT NULL, principal_id TEXT NOT NULL, requested_at_ms INTEGER NOT NULL CHECK(requested_at_ms>=0), expires_at_ms INTEGER NOT NULL CHECK(expires_at_ms>requested_at_ms));
CREATE TABLE local_cleanup_obligations(run_id TEXT PRIMARY KEY REFERENCES runs(id), session_id TEXT NOT NULL REFERENCES sessions(id), installation_id TEXT NOT NULL, principal_id TEXT NOT NULL, confirmation TEXT CHECK(confirmation IN ('observed','operator_attested')));
CREATE UNIQUE INDEX one_pending_cleanup ON local_cleanup_obligations(session_id) WHERE confirmation IS NULL;
CREATE TABLE local_tool_reconciliations(run_id TEXT PRIMARY KEY REFERENCES runs(id), session_id TEXT NOT NULL REFERENCES sessions(id), record TEXT NOT NULL);
CREATE TABLE notification_cursor(id INTEGER PRIMARY KEY CHECK(id=1), sequence TEXT NOT NULL);
CREATE TABLE notification_outbox(slot INTEGER PRIMARY KEY, sequence TEXT NOT NULL, metadata TEXT NOT NULL);
INSERT INTO notification_cursor VALUES(1,'00000000000000000000');
WITH RECURSIVE slots(slot) AS (SELECT 0 UNION ALL SELECT slot+1 FROM slots WHERE slot<255)
INSERT INTO notification_outbox SELECT slot,'00000000000000000000',printf('%1024s','') FROM slots;
INSERT INTO attachment_schema VALUES(1,12);
-- v1.0.2 retain_initial_configuration creates this session-private table.
CREATE TABLE process_configuration(session_id TEXT PRIMARY KEY, settings TEXT NOT NULL);
