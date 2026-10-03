-- How the deployment presents itself to people: the name and picture its sign-in screen
-- welcomes them with. One row, always present; `singleton` is its key and can hold nothing but
-- true, so a second row cannot be made. Either field is NULL when the deployment has not set
-- it. The picture is an icon row, and the deployment shows none when that icon is deleted.
CREATE TABLE deployment_profile (
    singleton BOOLEAN PRIMARY KEY DEFAULT TRUE CHECK (singleton),
    display_name TEXT,
    icon UUID REFERENCES icon (id) ON DELETE SET NULL
);
INSERT INTO deployment_profile DEFAULT VALUES;
