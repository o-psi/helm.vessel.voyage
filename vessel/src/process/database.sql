CREATE TABLE schema_version(id INTEGER PRIMARY KEY CHECK(id=1), version INTEGER NOT NULL);
INSERT INTO schema_version VALUES(1,1);
CREATE TABLE voyages(
 session_id TEXT PRIMARY KEY NOT NULL,
 incarnation TEXT NOT NULL,
 workspace BLOB NOT NULL,
 state TEXT NOT NULL CHECK(state IN ('starting','live','suspended','unavailable','stopped','cleanup_unconfirmed','relinquished')),
 name TEXT,
 registration TEXT NOT NULL CHECK(json_valid(registration)),
 updated_at_ms INTEGER NOT NULL
) STRICT;
CREATE TABLE incarnations(
 incarnation TEXT PRIMARY KEY NOT NULL,
 session_id TEXT NOT NULL REFERENCES voyages(session_id),
 command_id TEXT NOT NULL,
 registration TEXT NOT NULL CHECK(json_valid(registration)),
 created_at_ms INTEGER NOT NULL
) STRICT;
CREATE INDEX incarnation_session ON incarnations(session_id,created_at_ms);
CREATE TABLE lifecycle_commands(
 namespace TEXT NOT NULL,
 command_id TEXT NOT NULL,
 request BLOB NOT NULL CHECK(length(request)<=16384),
 PRIMARY KEY(namespace,command_id)
) STRICT;
CREATE TRIGGER immutable_lifecycle BEFORE UPDATE ON lifecycle_commands BEGIN SELECT RAISE(ABORT,'immutable lifecycle command'); END;
CREATE TABLE creation_receipts(
 command_id TEXT PRIMARY KEY NOT NULL,
 session_id TEXT NOT NULL REFERENCES voyages(session_id),
 result TEXT NOT NULL CHECK(json_valid(result))
) STRICT;
CREATE TRIGGER immutable_creation BEFORE UPDATE ON creation_receipts BEGIN SELECT RAISE(ABORT,'immutable creation receipt'); END;
CREATE TABLE catalogue(
 session_id TEXT PRIMARY KEY NOT NULL REFERENCES voyages(session_id),
 incarnation TEXT NOT NULL,
 summary TEXT CHECK(summary IS NULL OR json_valid(summary)),
 observed_at_ms INTEGER,
 fingerprint TEXT,
 process_info TEXT CHECK(process_info IS NULL OR json_valid(process_info)),
 error_code TEXT,
 failures INTEGER NOT NULL DEFAULT 0 CHECK(failures>=0),
 next_attempt_ms INTEGER NOT NULL DEFAULT 0
) STRICT;
CREATE TABLE catalogue_events(
 sequence INTEGER PRIMARY KEY AUTOINCREMENT,
 session_id TEXT NOT NULL REFERENCES voyages(session_id),
 kind TEXT NOT NULL,
 recorded_at_ms INTEGER NOT NULL
) STRICT;
CREATE TRIGGER bounded_catalogue_events AFTER INSERT ON catalogue_events BEGIN
 DELETE FROM catalogue_events WHERE sequence<=NEW.sequence-4096;
END;
CREATE TABLE legacy_imports(
 source TEXT PRIMARY KEY NOT NULL,
 digest BLOB NOT NULL CHECK(length(digest)=32)
) STRICT;
