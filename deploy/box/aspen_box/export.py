"""Taking everything out of a box, for the deployment that replaces it: the database, people's
files, and what that deployment must know of this one (deploy/aws/migrate.py reads it)."""

from __future__ import annotations

import argparse
import json
import os
import tempfile

from .common import BUCKET, SLOTS, Box, Failed
from .data import dump
from .lifecycle import converge
from .nats_tls import rootless


def export(store: Box, opt: argparse.Namespace) -> None:
    box = store.load()
    if box.get("role") == "voice":
        raise Failed("a voice server's box keeps nothing to export: export the deployment's own box")
    target = opt.directory.expanduser().resolve()
    if target.exists() and any(target.iterdir()):
        raise Failed(f"{target} is not empty: give an empty directory, or one that does not exist")
    if not opt.yes:
        answer = input(
            f"This stops https://{box['domain']} and leaves it stopped, so that nothing is written after the copy; "
            "`aspen-box up` starts it again. Type the domain to go on: "
        )
        if answer.strip() != box["domain"]:
            raise Failed("not confirmed")
    target.mkdir(mode=0o700, parents=True, exist_ok=True)
    # The database and the bundled storage run; the servers, which write to them, do not.
    converge(store, box)
    store.compose("stop", *SLOTS, "voice")
    dump(store, box, target / "database.dump")
    if not opt.database_only:
        copy_files(store, box, target / "files")
    elsewhere = [server["name"] for server in box["voice_servers"] if not server["here"]]
    (target / "export.json").write_text(json.dumps({
        "domain": box["domain"],
        # Each is registered in the database, which the deployment taking it over removes.
        "voice_servers": [server["name"] for server in box["voice_servers"]],
        "files": not opt.database_only,
    }, indent=2) + "\n")  # fmt: skip
    print(
        f"\nExported https://{box['domain']} into {target}, which holds every account's data: keep it where only you can read it.\n"
        "The deployment is stopped. `aspen-box up` starts it again, with whatever it held when it stopped."
    )
    if elsewhere:
        print(f"The voice servers on other machines ({', '.join(elsewhere)}) go on running: stop them there with `aspen-box down`.")


def copy_files(store: Box, box: dict, target) -> None:
    """Copies every object of the deployment's bucket into `target`, with the box's own keys, or
    the machine's role when it was given none."""
    storage = box["storage"]
    if storage["kind"] == "bundled":
        endpoint, bucket, region = "http://127.0.0.1:3900", BUCKET, "garage"
        keys = (box["secrets"]["s3_access_key"], box["secrets"]["s3_secret_key"])
    else:
        endpoint, bucket, region = storage["endpoint"], storage["bucket"], storage["region"]
        keys = (storage.get("access_key"), storage.get("secret_key"))
    target.mkdir(mode=0o700)
    # The keys reach the container through its environment, never a command line, which any
    # user of the machine may read.
    env = {"AWS_REGION": region, "AWS_CONFIG_FILE": "/aws/config", "HOME": "/tmp"}
    if keys[0]:
        env.update(AWS_ACCESS_KEY_ID=keys[0], AWS_SECRET_ACCESS_KEY=keys[1])
    with tempfile.TemporaryDirectory(dir=store.dir) as settings:
        # The servers name objects path-style, which every S3 the box may use answers.
        with open(os.path.join(settings, "config"), "w") as config:
            config.write("[default]\ns3 =\n  addressing_style = path\n")
        os.chmod(settings, 0o755)
        print(f"Copying people's files from {bucket}…", flush=True)
        done = store.compose(
            "run", "--rm", "--no-deps", "-T", *[flag for name in env for flag in ("-e", name)],
            "-v", f"{settings}:/aws:ro", "-v", f"{target}:/files",
            # Written as this user, who may then move them; under rootless Docker the container's
            # root is this user already.
            *([] if rootless() else ["--user", f"{os.getuid()}:{os.getgid()}"]),
            "files", "s3", "sync", f"s3://{bucket}", "/files", "--endpoint-url", endpoint, "--only-show-errors",
            env=env, check=False,
        )  # fmt: skip
    if done.returncode != 0:
        raise Failed(f"people's files could not be copied from {endpoint}; what the AWS command line said is above")
