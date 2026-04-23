CREATE TABLE pin (
    message_id UUID PRIMARY KEY REFERENCES message(id),
    channel UUID NOT NULL REFERENCES channel(id),
    timestamp TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    sort_index INT NOT NULL DEFAULT 0
);
