CREATE TABLE execution_identities(
 identity_id TEXT NOT NULL,
 revision INTEGER NOT NULL CHECK(revision>0),
 record TEXT NOT NULL CHECK(json_valid(record)),
 PRIMARY KEY(identity_id,revision)
) STRICT;
CREATE TABLE execution_bindings(
 session_id TEXT PRIMARY KEY NOT NULL REFERENCES voyages(session_id),
 incarnation TEXT NOT NULL REFERENCES incarnations(incarnation),
 identity_id TEXT NOT NULL,
 identity_revision INTEGER NOT NULL,
 record TEXT NOT NULL CHECK(json_valid(record)),
 FOREIGN KEY(identity_id,identity_revision) REFERENCES execution_identities(identity_id,revision)
) STRICT;
CREATE TABLE administrator_grants(
 grant_id TEXT PRIMARY KEY NOT NULL,
 session_id TEXT NOT NULL,
 record TEXT NOT NULL CHECK(json_valid(record))
) STRICT;
CREATE TRIGGER immutable_administrator_grants BEFORE UPDATE ON administrator_grants BEGIN SELECT RAISE(ABORT,'immutable administrator grant'); END;
CREATE TABLE administrator_revocations(
 grant_id TEXT PRIMARY KEY NOT NULL REFERENCES administrator_grants(grant_id),
 command_id TEXT UNIQUE NOT NULL,
 revoked_at_ms INTEGER NOT NULL CHECK(revoked_at_ms>0)
) STRICT;
CREATE TRIGGER immutable_administrator_revocations BEFORE UPDATE ON administrator_revocations BEGIN SELECT RAISE(ABORT,'immutable administrator revocation'); END;
UPDATE schema_version SET version=2 WHERE id=1 AND version=1;
