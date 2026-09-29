ALTER TABLE federated_deployment
    DROP CONSTRAINT federated_deployment_protocol_whole,
    DROP COLUMN software_version,
    DROP COLUMN software_name,
    DROP COLUMN capabilities,
    DROP COLUMN protocol_minimum,
    DROP COLUMN protocol_version;
