UPDATE deployment_role SET permissions = permissions & ~64;
DROP TABLE federation_list_entry;
DROP TABLE federated_deployment;
DROP TABLE federation_key;
CREATE TABLE other_server_auth_token (
    token TEXT NOT NULL PRIMARY KEY,
    expires TIMESTAMP NOT NULL,
    "user" UUID NOT NULL REFERENCES "user" (id),
    domain TEXT NOT NULL
);
