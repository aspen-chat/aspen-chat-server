"""Voice servers beyond the box's own: another machine, nearer some of the people who call, or
more of them. The box registers each and gives it a NATS user of its own; the other machine runs
a box of its own that holds the voice server alone (`init-voice`), told everything it needs in
one file the box writes (`voice add`).

A voice server elsewhere reaches the box's NATS over the network. While there is one, NATS
listens beyond loopback, on TCP 4222, over TLS from the box's own authority (`nats_tls`)."""

from __future__ import annotations

import argparse
import json
import pathlib
import re
import secrets
import uuid

from .common import HERE_VOICE, Box, Failed, addresses, ask, voice_url
from .lifecycle import published, up
from .render import nats_open, render

NAME = re.compile(r"[a-z][a-z0-9-]{0,30}")
# Names under the domain the box itself answers at.
TAKEN = {HERE_VOICE, "media"}


def add(store: Box, opt: argparse.Namespace) -> None:
    box = store.load()
    if box.get("role") == "voice":
        raise Failed("add voice servers on the deployment's own box, not on a voice server's")
    name = opt.name
    if not NAME.fullmatch(name) or name in TAKEN:
        raise Failed(f"a voice server's name is a DNS label under {box['domain']} of lowercase letters, digits, and hyphens, and not {' or '.join(sorted(TAKEN))}")
    if any(server["name"] == name for server in box["voice_servers"]):
        raise Failed(f"there is a voice server called {name} already: `aspen-box voice list`")
    was_open = nats_open(box)
    box["voice_servers"].append({
        "name": name,
        "id": str(uuid.uuid4()),
        # Letters and digits alone, so it needs no escaping where it is written.
        "password": secrets.token_hex(20),
        "capacity": opt.capacity,
        "here": False,
    })  # fmt: skip
    before = store.load()
    store.save(box)
    # Opens NATS, the first time, registers the server, and gives NATS its user.
    try:
        up(store, opt)
    except Failed as failure:
        store.save(before)
        raise Failed(f"{failure}. {name} was not added: `aspen-box up` puts the rest back as it was") from None
    join = store.dir / f"voice-{name}.join"
    write_join(store, box, name, join)
    print(
        f"\nWrote {join}. It holds {name}'s NATS password: keep it as you would box.json, and delete it once used.\n\n"
        f"On the machine that will run {name}:\n\n"
        f"    1. Copy {join.name} there, and run:  aspen-box init-voice --join {join.name} --tag <the same release>\n"
        f"    2. In your domain's DNS:  {name}.{box['domain']}.  A (and AAAA)  <that machine's address>\n"
        f"    3. aspen-box up\n"
    )
    if not was_open:
        print(
            "NATS now listens beyond this machine, over TLS, for voice servers elsewhere: forward TCP 4222 to this "
            "machine if it is behind a router, and where you can, let only the voice servers' machines reach it."
        )


def write_join(store: Box, box: dict, name: str, path: pathlib.Path) -> None:
    server = next(server for server in box["voice_servers"] if server["name"] == name)
    join = {
        "domain": box["domain"],
        "name": name,
        "id": server["id"],
        "password": server["password"],
        "nats_url": f"tls://{box['domain']}:4222",
        "ca": (store.dir / "config" / "nats-ca.pem").read_text(),
    }
    path.touch(mode=0o600, exist_ok=True)
    path.chmod(0o600)
    path.write_text(json.dumps(join, indent=2) + "\n")


def remove(store: Box, opt: argparse.Namespace) -> None:
    box = store.load()
    if opt.name == HERE_VOICE:
        raise Failed(f"{HERE_VOICE} is this machine's own voice server, which the box keeps")
    if not any(server["name"] == opt.name for server in box["voice_servers"]):
        raise Failed(f"there is no voice server called {opt.name}: `aspen-box voice list`")
    removed = store.compose("run", "--rm", "tool", "aspen-chat-server", "voice-servers", "remove", opt.name, capture=True, check=False)
    if removed.returncode != 0:
        raise Failed(
            f"{opt.name} could not be taken out of the registry, so it is kept: {removed.stderr.strip().splitlines()[-1] if removed.stderr.strip() else 'no reason given'}. "
            "A voice server that holds calls is removed once they end, or from the dashboard's Server fleet tab, which ends them"
        )
    box["voice_servers"] = [server for server in box["voice_servers"] if server["name"] != opt.name]
    store.save(box)
    # Takes its NATS user away, and closes NATS again when it was the last elsewhere.
    up(store, opt)
    print(f"\nRemoved {opt.name}. Stop its machine's box: `aspen-box down` there.")


def listing(store: Box, opt: argparse.Namespace) -> None:
    box = store.load()
    for server in box["voice_servers"]:
        where = "this machine" if server["here"] else "elsewhere"
        print(f"{server['name']:<16} {voice_url(server['name'], box['domain']):<40} capacity {server['capacity']:<5} {where}")


def init_voice(store: Box, opt: argparse.Namespace) -> None:
    """A box that holds one voice server of a deployment whose own box is elsewhere."""
    if store.file.exists() and not opt.force:
        raise Failed(f"{store.dir} already holds a box. Start over with --force")
    try:
        join = json.loads(pathlib.Path(opt.join).read_text())
    except (OSError, ValueError) as error:
        raise Failed(f"{opt.join} is not a file `aspen-box voice add` wrote: {error}") from None
    images = published(opt.tag) if opt.tag else {"voice": opt.voice_image}
    if not images.get("voice"):
        raise Failed("give the release to run: --tag, or --voice-image")
    acme = opt.acme_email or ask("Email for Let's Encrypt to write to about certificates (- for none)", "-")
    box = {
        "role": "voice",
        "name": join["name"],
        "domain": join["domain"],
        "acme_email": None if acme == "-" else acme,
        "address4": opt.address4,
        "address6": opt.address6,
        "images": {"voice": images["voice"]},
        "rtc": {"min_port": 40000, "max_port": 40199, "upload_mbps": opt.upload_mbps},
        "join": {key: join[key] for key in ("id", "password", "nats_url", "ca")},
    }
    store.save(box)
    render(store, box)
    print(f"\nWrote {store.dir}. Delete {opt.join}: its password is in box.json now.\n")
    print("Point this name at this machine, in your domain's DNS:\n")
    for address in addresses(box):
        print(f"    {join['name']}.{join['domain']}.  {'AAAA' if ':' in address else 'A'}  {address}")
    print(
        "\nIf the machine is behind a router, forward to it: TCP 80 and 443, UDP 3478, and UDP and\n"
        "TCP 40000-40199.\n\nThen: aspen-box up, and aspen-box doctor."
    )
