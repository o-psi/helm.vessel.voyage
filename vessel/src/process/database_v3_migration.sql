CREATE TABLE administrator_authority(
 id INTEGER PRIMARY KEY CHECK(id=1),
 vessel_id TEXT NOT NULL,
 revision INTEGER NOT NULL CHECK(revision>0)
) STRICT;
CREATE TABLE administrative_owners(
 principal_id TEXT PRIMARY KEY NOT NULL,
 enabled INTEGER NOT NULL CHECK(enabled IN (0,1))
) STRICT;
CREATE TABLE administrator_owner_commands(
 command_id TEXT PRIMARY KEY NOT NULL,
 request TEXT NOT NULL CHECK(json_valid(request)),
 receipt TEXT NOT NULL CHECK(json_valid(receipt))
) STRICT;
CREATE TABLE execution_reviews(
 review_id TEXT PRIMARY KEY NOT NULL,
 command_id TEXT UNIQUE NOT NULL,
 owner_id TEXT NOT NULL,
 review TEXT NOT NULL CHECK(json_valid(review)),
 receipt TEXT NOT NULL CHECK(json_valid(receipt)),
 grant_id TEXT REFERENCES administrator_grants(grant_id)
) STRICT;
CREATE TRIGGER immutable_execution_review_facts BEFORE UPDATE OF review,command_id,owner_id ON execution_reviews BEGIN SELECT RAISE(ABORT,'immutable execution review facts'); END;
CREATE TABLE execution_review_controls(
 command_id TEXT PRIMARY KEY NOT NULL,
 review_id TEXT NOT NULL REFERENCES execution_reviews(review_id),
 request TEXT NOT NULL CHECK(json_valid(request)),
 receipt TEXT NOT NULL CHECK(json_valid(receipt))
) STRICT;
UPDATE schema_version SET version=3 WHERE id=1;
