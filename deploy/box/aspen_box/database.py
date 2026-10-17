"""The deployment's PostgreSQL: the box's own, or one kept elsewhere."""

from __future__ import annotations

import urllib.parse

from .common import Box, Failed

# Where the servers find the authorities of a database kept elsewhere (compose.yaml).
DATABASE_CA = "/etc/aspen/database-ca.pem"
# The migrations use `uuidv7()`, which earlier versions lack.
MINIMUM_POSTGRES = 18
DATABASE_URL_VARIABLE = "ASPEN_BOX_DATABASE_URL"


def external_database(box: dict) -> dict | None:
    """The database's `url` and `ca_file` when it is kept elsewhere, None when the box runs it."""
    database = box.get("database") or {}
    return database if database.get("kind") == "external" else None


def check_database_url(url: str) -> None:
    parts = urllib.parse.urlsplit(url)
    if parts.scheme not in ("postgres", "postgresql") or not parts.hostname or not parts.username:
        raise Failed("the database's address is a URL: postgres://user:password@host:5432/database")
    for unsupported in {"sslcert", "sslkey"} & set(dict(urllib.parse.parse_qsl(parts.query))):
        raise Failed(f"the box cannot give the servers a client certificate ({unsupported}): sign in to the database with a password")


def database_url(box: dict) -> str:
    """`database_url` as the servers are given it. A database kept elsewhere is reached over a
    network, so unless its URL says otherwise its certificate is checked: against `ca_file`
    when the box was given one, and the system's authorities when not."""
    external = external_database(box)
    if external is None:
        return f"postgres://aspen:{box['secrets']['database']}@127.0.0.1:5432/aspen"
    parts = urllib.parse.urlsplit(external["url"])
    query = dict(urllib.parse.parse_qsl(parts.query))
    if external.get("ca_file"):
        query.setdefault("sslmode", "verify-full")
        query["sslrootcert"] = DATABASE_CA
    elif "sslmode" not in query:
        query.update(sslmode="verify-full", sslrootcert=query.get("sslrootcert", "system"))
    return urllib.parse.urlunsplit(parts._replace(query=urllib.parse.urlencode(query, safe="/")))


def database_name(box: dict) -> str:
    parts = urllib.parse.urlsplit(database_url(box))
    return urllib.parse.unquote(parts.path.lstrip("/")) or urllib.parse.unquote(parts.username)


def database_host(box: dict) -> str:
    return urllib.parse.urlsplit(database_url(box)).hostname


def pg(store: Box, box: dict, tool: str, *arguments: str, **how):
    """Runs one of PostgreSQL's own tools against the deployment's database: in its container
    when the box runs it, and otherwise in a container of the same image, which is told where
    the database is through its environment, so that no password is on a command line."""
    if external_database(box) is None:
        return store.compose("exec", "-T", "-e", "PGHOST=127.0.0.1", "-e", "PGUSER=aspen", "postgres", tool, *arguments, **how)
    parts = urllib.parse.urlsplit(database_url(box))
    query = dict(urllib.parse.parse_qsl(parts.query))
    env = {
        "PGHOST": parts.hostname,
        "PGPORT": str(parts.port or 5432),
        "PGUSER": urllib.parse.unquote(parts.username),
        "PGPASSWORD": urllib.parse.unquote(parts.password or ""),
    }
    for setting, variable in (("sslmode", "PGSSLMODE"), ("sslrootcert", "PGSSLROOTCERT"), ("channel_binding", "PGCHANNELBINDING"), ("options", "PGOPTIONS")):
        if setting in query:
            env[variable] = query[setting]
    passed = [flag for name in env for flag in ("-e", name)]
    return store.compose(
        "run", "--rm", "--no-deps", "-T", *passed,
        "-v", f"{store.dir / 'config' / 'database-ca.pem'}:{DATABASE_CA}:ro",
        "postgres", tool, *arguments, env=env, **how,
    )  # fmt: skip


def database_version(store: Box, box: dict) -> int:
    """The database's major version, having reached it and signed in."""
    done = pg(store, box, "psql", "--no-psqlrc", "--tuples-only", "--no-align", f"--dbname={database_name(box)}",
              "--command=SHOW server_version_num", capture=True, check=False)  # fmt: skip
    if done.returncode != 0:
        said = (done.stderr.strip().splitlines() or ["no reason given"])[-1]
        raise Failed(f"the database could not be reached: {said}")
    return int(done.stdout.strip()) // 10000


def require_postgres(store: Box, box: dict) -> None:
    version = database_version(store, box)
    if version < MINIMUM_POSTGRES:
        raise Failed(f"the database is PostgreSQL {version}, and Aspen needs {MINIMUM_POSTGRES} or later")
