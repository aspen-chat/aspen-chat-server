-- How many MiB of NATS's memory each of the deployment's two in-memory streams may hold
-- (`app::nats_memory`): the events of the last minute, and the voice servers' reports not yet
-- applied. NULL sizes the stream to what NATS has.
ALTER TABLE deployment_settings
    ADD COLUMN event_stream_mib INTEGER CHECK (event_stream_mib >= 16),
    ADD COLUMN voice_report_stream_mib INTEGER CHECK (voice_report_stream_mib >= 16);
