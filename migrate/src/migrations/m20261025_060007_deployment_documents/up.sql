-- What the deployment publishes for anyone to read, signed in or not: its Privacy Policy and its
-- Terms of Use, written by its administrators in Markdown (`app::deployment_document`). A
-- document it has not published has no row; there is no default.
CREATE TABLE deployment_document (
    kind TEXT PRIMARY KEY CHECK (kind IN ('privacyPolicy', 'termsOfUse')),
    body TEXT NOT NULL CHECK (char_length(body) BETWEEN 1 AND 65536),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
