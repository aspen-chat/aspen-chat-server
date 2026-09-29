-- The deployment's record of files offered in calls and sent between people
-- (`app::file_transfer`), for its moderators: who offered what, by name and size, and who
-- received it, how, and how it ended. The files themselves never reach this server. Rows outlive
-- the people, channels, and calls they name, which are cleared when those go.
CREATE TABLE file_offer (
    id UUID PRIMARY KEY,
    channel UUID REFERENCES channel (id) ON DELETE SET NULL,
    sender UUID REFERENCES "user" (id) ON DELETE SET NULL,
    file_name TEXT NOT NULL,
    file_size BIGINT NOT NULL,
    allow_direct BOOLEAN NOT NULL,
    valid_for_seconds INTEGER NOT NULL,
    offered_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX file_offer_sender ON file_offer (sender, id);
CREATE INDEX file_offer_channel ON file_offer (channel);

-- One row per acceptance: a receiver may accept an offer again after a transfer of it stopped.
CREATE TABLE file_transfer (
    id UUID PRIMARY KEY,
    offer UUID NOT NULL REFERENCES file_offer (id) ON DELETE CASCADE,
    receiver UUID REFERENCES "user" (id) ON DELETE SET NULL,
    mode TEXT NOT NULL CHECK (mode IN ('directPreferred', 'relayOnly')),
    started_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    ended_at TIMESTAMPTZ,
    outcome TEXT CHECK (outcome IN ('completed', 'cancelled', 'failed', 'left')),
    ended_by UUID REFERENCES "user" (id) ON DELETE SET NULL
);
CREATE INDEX file_transfer_offer ON file_transfer (offer, receiver);
CREATE INDEX file_transfer_receiver ON file_transfer (receiver, offer);
CREATE INDEX file_transfer_ended_by ON file_transfer (ended_by);
