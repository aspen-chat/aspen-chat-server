-- What each other deployment said of itself when last contacted: the range of protocol
-- versions it speaks, its capabilities, and the software it runs. `NULL` until contacted.
ALTER TABLE federated_deployment
    ADD COLUMN protocol_version INTEGER,
    ADD COLUMN protocol_minimum INTEGER,
    ADD COLUMN capabilities TEXT[] NOT NULL DEFAULT '{}',
    ADD COLUMN software_name TEXT,
    ADD COLUMN software_version TEXT,
    ADD CONSTRAINT federated_deployment_protocol_whole
        CHECK ((protocol_version IS NULL) = (protocol_minimum IS NULL));
