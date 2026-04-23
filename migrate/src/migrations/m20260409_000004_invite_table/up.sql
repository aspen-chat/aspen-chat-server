CREATE TABLE invite (
    code TEXT PRIMARY KEY,
    community UUID NOT NULL REFERENCES community(id),
    created_by UUID NOT NULL REFERENCES "user"(id),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    expires_at TIMESTAMPTZ,
    deleted_at TIMESTAMPTZ
);
