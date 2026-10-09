# Step 3: Configuring and migrating

## Write `aspen.toml`

Write `aspen.toml` beside where the API server will run. The required settings are:

- your deployment's address (`public_url`),
- where the web client is (`[web_client] dir`),
- the four services.

```toml
public_url = "https://chat.example.org"
database_url = "postgres://aspen:…@db.internal/aspen"
nats_url = "nats.internal:4222"
nats_auth_token = "…"
valkey_url = "redis://valkey.internal:6379"

[media.s3]
endpoint = "http://storage.internal:8333"
public_endpoint = "https://storage.chat.example.org"
public_base_url = "https://media.chat.example.org/aspen-media"
bucket = "aspen-media"
region = "us-east-1"
access_key = "…"
secret_key = "…"

[web_client]
dir = "/srv/aspen/dist"
```

Every other setting has a default. [Configuration](../configuration/index.md) lists them all.

### Choosing `public_url`

`public_url` is the one address people use for everything: the API, the web client, and every
link the deployment hands out. Its host is also:

- your deployment's [federation](../federation/index.md) domain,
- the domain passkeys belong to.

**Choose it for good.** Once a server has started with an `https` address, a server started with
another host refuses to start. A new host would make every passkey useless.

### Email

To send mail (email verification, password reset by email, the daily digest, and a newsletter),
add an SMTP server:

```toml
[email]
smtp_url = "smtps://aspen:…@smtp.example.org"
from = "Example Chat <noreply@chat.example.org>"
```

Without it, the deployment works without email.

### Environment variables

Each setting can also be given in the environment, which overrides the file. Write `ASPEN_` and
the key, with `__` between nested keys: `ASPEN_DATABASE_URL`, `ASPEN_VOICE__TOKEN_SECRET`.

## Create the tables

```
aspen-migrate up
```

It reads the database from `--database-url`, then `DATABASE_URL`, then `database_url` in
`aspen.toml`.

Run it again after every upgrade, before starting the new servers. It applies only what has not
been applied.

---

Previous: [Step 2: Building](2-building.md) · Next:
[Step 4: Starting the API server](4-api-server.md)
