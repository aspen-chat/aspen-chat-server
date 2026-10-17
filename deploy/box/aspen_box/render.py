"""Writing a box's configuration from its `box.json`: the shared templates (`../templates`)
filled for this machine, Caddy's configuration, and the variables `compose.yaml` reads."""

from __future__ import annotations

import json
import pathlib

from . import nats_tls
from .common import (
    BUCKET,
    GIB,
    HERE,
    HERE_VOICE,
    TEMPLATES,
    Box,
    Failed,
    addresses,
    encoded,
    fill,
    memory_bytes,
    voice_url,
    write_env,
    write_files,
)
from .database import database_url, external_database

NATS_CA = "/etc/aspen/nats-ca.pem"
# Caddy, the only thing on the machine that reaches either server.
TRUSTED_PROXIES = ["127.0.0.1"]


def remote_voice_servers(box: dict) -> list:
    return [server for server in box["voice_servers"] if not server["here"]]


def nats_open(box: dict) -> bool:
    """Whether NATS is opened to the network, over TLS: when a voice server elsewhere reaches it."""
    return bool(remote_voice_servers(box))


def nats_url(box: dict) -> str:
    return "tls://127.0.0.1:4222" if nats_open(box) else "nats://127.0.0.1:4222"


def nats_tls_toml(open_: bool) -> str:
    return f'\n[nats.tls]\nca_file = "{NATS_CA}"\n' if open_ else ""


def rtc_more(box: dict) -> str:
    upload = box["rtc"].get("upload_mbps")
    return f"upload_mbps = {json.dumps(upload)}\n" if upload else ""


def voice_server_toml(*, id: str, name: str, nats: str, password: str, announced: list, box: dict, tls: bool) -> str:
    return fill(TEMPLATES / "voice_server.toml.tftpl", {
        **encoded(dict(
            id=id,
            name=name,
            nats_url=nats,
            listen_addr="127.0.0.1:9000",
            nats_password=password,
            trusted_proxies=TRUSTED_PROXIES,
            announced_address=announced,
            min_port=box["rtc"]["min_port"],
            max_port=box["rtc"]["max_port"],
        )),
        "rtc_more": rtc_more(box),
        "more": nats_tls_toml(tls),
    })  # fmt: skip


def caddyfile(box: dict, sites: list) -> str:
    return fill(HERE / "caddy" / "Caddyfile.tmpl", dict(
        acme_email=f"\temail {box['acme_email']}\n" if box.get("acme_email") else "",
        sites="\n".join(sites),
    ))  # fmt: skip


def render(store: Box, box: dict) -> set:
    """Writes `config/` and `.env` from `box`, and answers which of its files changed."""
    (store.dir / "config").mkdir(mode=0o700, parents=True, exist_ok=True)
    (store.dir / "backups").mkdir(mode=0o700, exist_ok=True)
    (store.dir / "data" / "caddy").mkdir(parents=True, exist_ok=True)
    (store.dir / "config" / "nats-tls").mkdir(mode=0o700, exist_ok=True)
    if box.get("role") == "voice":
        return render_voice_only(store, box)
    return render_main(store, box)


def render_voice_only(store: Box, box: dict) -> set:
    join = box["join"]
    voice = box["images"]["voice"]
    # The services this box does not run are named too, so Compose has nothing to warn of.
    write_env(store, {
        "VALKEY_MEMORY_MB": 64, "ASPEN_API_IMAGE_A": voice, "ASPEN_API_IMAGE_B": voice, "ASPEN_TOOL_IMAGE": voice,
        "ASPEN_VOICE_IMAGE": voice, "COMPOSE_PROFILES": "",
    })  # fmt: skip
    return write_files(store.dir / "config", {
        "voice_server.toml": voice_server_toml(
            id=join["id"], name=box["name"], nats=join["nats_url"], password=join["password"],
            announced=addresses(box), box=box, tls=True,
        ),
        "nats-ca.pem": join["ca"],
        "Caddyfile": caddyfile(box, [fill(HERE / "caddy" / "voice.tmpl", dict(voice_domain=f"{box['name']}.{box['domain']}"))]),
    })  # fmt: skip


def render_main(store: Box, box: dict) -> set:
    external = external_database(box)
    for name in ("garage/meta", "garage/data", *(() if external else ("postgres",))):
        (store.dir / "data" / name).mkdir(parents=True, exist_ok=True)
    authorities = ""
    if external and external.get("ca_file"):
        try:
            authorities = pathlib.Path(external["ca_file"]).expanduser().read_text()
        except OSError as error:
            raise Failed(f"the database's authorities, {external['ca_file']}, could not be read: {error}") from None
    secret, storage, domain = box["secrets"], box["storage"], box["domain"]
    small = memory_bytes() < 3.5 * GIB
    bundled = storage["kind"] == "bundled"
    images = box["images"]
    active = box.get("active_slot", "api-a")
    write_env(store, {
        "VALKEY_MEMORY_MB": 64 if small else 128,
        # The slot that is not running keeps the image it last ran, until an upgrade gives it
        # the next.
        "ASPEN_API_IMAGE_A": images["api"] if active == "api-a" else images.get("api_idle", images["api"]),
        "ASPEN_API_IMAGE_B": images["api"] if active == "api-b" else images.get("api_idle", images["api"]),
        "ASPEN_VOICE_IMAGE": images["voice"],
        "ASPEN_TOOL_IMAGE": images.get("tool", images["api"]),
        "COMPOSE_PROFILES": ",".join(["main", *(["bundled-storage"] if bundled else []), *([] if external else ["bundled-database"])]),
    })  # fmt: skip
    open_ = nats_open(box)
    # Made before NATS is given the configuration that names it.
    nats_authority = nats_tls.ensure(store, domain) if open_ else ""
    media_url = f"https://media.{domain}" if bundled else storage["public_base_url"]
    if bundled:
        s3 = dict(endpoint="http://127.0.0.1:3900", bucket=BUCKET, region="garage", public_endpoint=media_url,
                  access_key=secret["s3_access_key"], secret_key=secret["s3_secret_key"])  # fmt: skip
    else:
        s3 = storage
    s3_more = "".join(
        f"{key} = {json.dumps(s3[key])}\n" for key in ("public_endpoint", "access_key", "secret_key") if s3.get(key)
    )
    more = nats_tls_toml(open_)
    if box.get("email"):
        more += f"\n[email]\nsmtp_url = {json.dumps(box['email']['smtp_url'])}\nfrom = {json.dumps(box['email']['from'])}\n"
    here = next((server for server in box["voice_servers"] if server["here"]), None)
    voice_users = "".join(
        fill(TEMPLATES / "nats_voice_user.tftpl", dict(user=json.dumps(server["name"]), password=json.dumps(server["password"]), id=server["id"]))
        for server in box["voice_servers"]
    )  # fmt: skip
    sites = [fill(HERE / "caddy" / "api.tmpl", dict(domain=domain))]
    if here:
        sites.append(fill(HERE / "caddy" / "voice.tmpl", dict(voice_domain=f"{HERE_VOICE}.{domain}")))
    if bundled:
        sites.append(fill(HERE / "caddy" / "media.tmpl", dict(media_domain=f"media.{domain}", bucket=BUCKET)))
    files = {
        "aspen.toml": fill(TEMPLATES / "aspen.toml.tftpl", {
            **encoded(dict(
                public_url=f"https://{domain}",
                database_url=database_url(box),
                valkey_url="redis://127.0.0.1:6379",
                nats_url=nats_url(box),
                nats_password=secret["nats_server"],
                nats_monitor_url="http://127.0.0.1:8222",
                trusted_proxies=TRUSTED_PROXIES,
                s3_endpoint=s3["endpoint"],
                media_url=media_url,
                bucket=s3["bucket"],
                region=s3["region"],
            )),
            "s3_more": s3_more,
            "more": more,
        }),
        "nats.conf": fill(TEMPLATES / "nats.conf.tftpl", dict(
            listen=json.dumps("0.0.0.0:4222" if open_ else "127.0.0.1:4222"),
            monitor=json.dumps("127.0.0.1:8222"),
            tls=nats_tls.NATS_CONF_TLS if open_ else "",
            # What the two in-memory streams may reserve, which the servers size them to. Each
            # needs 16 MiB, and voice reports take an eighth.
            max_memory_store_bytes=(144 if small else 256) << 20,
            server_password=json.dumps(secret["nats_server"]),
            voice_users=voice_users,
        )),
        "nats-ca.pem": nats_authority,
        "Caddyfile": caddyfile(box, sites),
        "postgres.env": f"POSTGRES_USER=aspen\nPOSTGRES_DB=aspen\nPOSTGRES_PASSWORD={secret['database']}\n",
        # Always there, empty when unused: the servers' containers mount it either way.
        "database-ca.pem": authorities,
        "garage.toml": (HERE / "garage.toml.tmpl").read_text(),
        "garage.env": (
            f"GARAGE_RPC_SECRET={secret['garage_rpc']}\nGARAGE_DEFAULT_BUCKET={BUCKET}\n"
            f"GARAGE_DEFAULT_ACCESS_KEY={secret['s3_access_key']}\nGARAGE_DEFAULT_SECRET_KEY={secret['s3_secret_key']}\n"
        ),
    }  # fmt: skip
    if here:
        files["voice_server.toml"] = voice_server_toml(
            id=here["id"], name=here["name"], nats=nats_url(box), password=here["password"],
            announced=addresses(box), box=box, tls=open_,
        )  # fmt: skip
    return write_files(store.dir / "config", files)


def voice_urls(box: dict) -> dict:
    """Each voice server's address, by name."""
    return {server["name"]: voice_url(server["name"], box["domain"]) for server in box["voice_servers"]}
