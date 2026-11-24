#!/usr/bin/env python3
"""Moves an Ultra Lite deployment (`aspen-ul`) to this configuration, in three steps with a
`tofu apply` of this configuration after each of the first two:

    migrate.py prepare --domain D       while the old deployment serves: point this configuration
                                        at its address
    migrate.py copy --export DIR        once it is stopped and exported, copy it here
    migrate.py finish                   once this deployment serves: let go of the old one

Ultra Lite is exported on its machine (`aspen-ul export DIR`), and `copy --export DIR` copies its
database here from that directory and uploads its files, each kept with the type and disposition
it is served with, by `aspen-chat-server storage import` from the API image (`--image`), run in
Docker on this machine with the AWS command line's credentials.

README.md#moving-ultra-lite-here is the whole procedure. Each step says what to do next, and may
be run again if it fails part of the way. It needs `tofu`, the AWS command line, Docker, and the
credentials the configuration is applied with.
"""

import argparse
import json
import os
import pathlib
import subprocess
import sys
import tempfile
import urllib.error
import urllib.parse
import urllib.request

HERE = pathlib.Path(__file__).resolve().parent
# Written by `prepare` into this configuration's directory: the variables the old deployment
# decides.
VARIABLES_FILE = "migration.auto.tfvars.json"
# Where the move stands: once copied, what the old deployment held.
STATE_FILE = "migration.json"
# The name Ultra Lite registers its own voice server under, whose registration comes with its
# database (server/ul/src/main.rs's `VOICE_NAME`).
UL_VOICE = "aspen-ul"
# The variables of the AWS command line's credentials that reach the import's container, by name.
AWS_VARIABLES = ["AWS_PROFILE", "AWS_ACCESS_KEY_ID", "AWS_SECRET_ACCESS_KEY", "AWS_SESSION_TOKEN"]


class Failed(Exception):
    pass


def run(*command: str, capture: bool = False) -> str:
    done = subprocess.run(command, capture_output=capture, text=True)
    if done.returncode != 0:
        detail = f": {done.stderr.strip()}" if capture else ""
        raise Failed(f"`{' '.join(command[:4])} …` failed{detail}")
    return done.stdout if capture else ""


def fleet(opt: argparse.Namespace) -> dict:
    outputs = json.loads(run("tofu", f"-chdir={opt.fleet}", "output", "-json", capture=True))
    if "migration" not in outputs:
        raise Failed(f"{opt.fleet} has not been applied: run `prepare`, then `tofu apply` there")
    return outputs["migration"]["value"]


def variable(directory: pathlib.Path, expression: str):
    """A value of the configuration in `directory`, as its variables stand."""
    done = subprocess.run(
        ["tofu", f"-chdir={directory}", "console"], input=f"jsonencode({expression})\n", capture_output=True, text=True
    )
    if done.returncode != 0:
        raise Failed(f"could not read {expression} in {directory}: {done.stderr.strip()}")
    # The value is the last line; warnings about the configuration may come before it. A
    # variable that is not set has no value yet: `(known after apply)`.
    last = (done.stdout.strip().splitlines() or [""])[-1]
    if not last.startswith('"'):
        raise Failed(f"{expression} is not set in {directory}")
    return json.loads(json.loads(last))


def state(opt: argparse.Namespace) -> dict:
    path = opt.fleet / STATE_FILE
    if not path.exists():
        raise Failed(f"{path} does not exist: run `prepare` first")
    return json.loads(path.read_text())


def save_state(opt: argparse.Namespace, value: dict) -> None:
    (opt.fleet / STATE_FILE).write_text(json.dumps(value, indent=2) + "\n")


def write_variables(opt: argparse.Namespace, variables: dict) -> None:
    (opt.fleet / VARIABLES_FILE).write_text(json.dumps(variables, indent=2) + "\n")


def run_task(target: dict, task: str, prefix: str, *command: str) -> None:
    """Runs one of this deployment's tasks to its end, on Fargate."""
    run(
        sys.executable, str(HERE / "run_task.py"),
        "--region", target["region"], "--cluster", target["cluster"], "--task-definition", task,
        "--log-group", target["log_group"], "--log-prefix", prefix,
        "--subnets", target["subnets"], "--security-group", target["security_group"],
        *(["--", *command] if command else []),
    )  # fmt: skip


def prepare(opt: argparse.Namespace) -> None:
    # The address is the deployment: its federation domain and its passkeys. The old deployment
    # keeps the name until `copy` has run.
    write_variables(opt, {"domain": opt.domain, "public_dns": False})
    run("tofu", f"-chdir={opt.fleet}", "init", "-input=false", capture=True)
    for needed, why in (("region", ""), ("route53_zone_id", ", the hosted zone that holds the domain")):
        try:
            variable(opt.fleet, f"var.{needed}")
        except Failed:
            raise Failed(f"set `{needed}`{why} in {opt.fleet}/terraform.tfvars") from None
    if UL_VOICE in variable(opt.fleet, "keys(var.voice_servers)"):
        raise Failed(f"Ultra Lite's voice server is registered as {UL_VOICE!r}, which comes with its database: "
                     "call none of `voice_servers` that")  # fmt: skip
    save_state(opt, {})
    print(
        f"Wrote {VARIABLES_FILE} in {opt.fleet}.\n\n"
        f"Next, in {opt.fleet}, with no `image_tag` set:\n\n"
        "    tofu apply\n\n"
        "The old deployment goes on serving. Then, on its machine, `sudo systemctl stop aspen-ul` and\n"
        "`sudo aspen-ul export DIR`. Copy DIR to this machine, keeping it where only you can read it,\n"
        "and run `copy --export DIR`."
    )


def copy(opt: argparse.Namespace) -> None:
    moving, target = state(opt), fleet(opt)
    exported = ul_export(json.loads((opt.export / "export.json").read_text()))
    if exported["domain"] != target["domain"]:
        raise Failed(f"{opt.export} is an export of {exported['domain']}, and this deployment is {target['domain']}")
    clash = set(exported["voice_servers"]) & set(target["voice_servers"])
    if clash:
        raise Failed(
            f"the old deployment's voice servers {sorted(clash)} have the names of this one's, and their registrations "
            "come with the database: rename this one's in `voice_servers`, `tofu apply`, and run `copy` again"
        )
    print("Copying the database into the bucket…", flush=True)
    run("aws", "--region", target["region"], "s3", "cp", str(opt.export / "database.dump"), target["dump"], "--only-show-errors")
    import_files(opt, target, exported)
    print("Restoring the database into the new deployment…", flush=True)
    run_task(target, target["restore_task"], "restore")
    save_state(opt, {**moving, "voice_servers": exported["voice_servers"]})
    variables = json.loads((opt.fleet / VARIABLES_FILE).read_text())
    variables.pop("public_dns", None)
    write_variables(opt, variables)
    print(
        "\nThe database is copied, and the next apply gives this deployment the name.\n"
        f"Set `image_tag` in {opt.fleet}/terraform.tfvars, and there:\n\n"
        "    tofu apply\n\n"
        "When the deployment answers at its address, run `finish`. To go back instead, see\n"
        "README.md#going-back."
    )


def ul_export(exported: dict) -> dict:
    """What an Ultra Lite export's `export.json` (`server/ul/src/export.rs`) says of it."""
    if exported.get("format") != 1:
        raise Failed(f"this export is of format {exported.get('format')}, which this migrate.py does not know")
    voice = exported.get("voiceServer")
    return {
        "domain": urllib.parse.urlsplit(exported["publicUrl"]).hostname,
        "voice_servers": [voice["name"]] if voice else [],
        "files_in_s3": exported["files"] == "s3",
    }


def import_files(opt: argparse.Namespace, target: dict, exported: dict) -> None:
    """Uploads an Ultra Lite export's files into this deployment's bucket, each with the type and
    disposition it is served with, by `aspen-chat-server storage import` from the API image, as
    this machine's user and with the AWS command line's credentials. Run again, it uploads only
    what is not there yet."""
    if exported["files_in_s3"]:
        if not opt.files_copied:
            raise Failed(
                "the old deployment keeps its files in object storage, which the export does not hold. Copy them into "
                f"s3://{target['bucket']} with `aws s3 sync s3://<its bucket> s3://{target['bucket']} --exclude 'migration/*'` "
                "(a copy between buckets keeps each file's type), then run `copy --export DIR --files-copied`"
            )
        return
    print("Uploading people's files into the bucket…", flush=True)
    aws = pathlib.Path.home() / ".aws"
    with tempfile.TemporaryDirectory() as settings:
        # What `storage import` reads is the storage alone; the rest names nothing it reaches.
        pathlib.Path(settings, "aspen.toml").write_text(
            f'public_url = "https://{target["domain"]}"\n'
            'database_url = "postgres://unused.invalid/aspen"\n'
            'nats_url = "nats://unused.invalid"\nnats_auth_token = "unused"\n'
            'valkey_url = "redis://unused.invalid"\n'
            "[media.s3]\n"
            f'endpoint = "https://s3.{target["region"]}.amazonaws.com"\n'
            f'region = "{target["region"]}"\nbucket = "{target["bucket"]}"\n'
            f'public_base_url = "https://{target["domain"]}"\n'
        )
        os.chmod(settings, 0o755)
        credentials = ["-v", f"{aws}:/aws:ro", "-e", "AWS_CONFIG_FILE=/aws/config",
                       "-e", "AWS_SHARED_CREDENTIALS_FILE=/aws/credentials"] if aws.is_dir() else []  # fmt: skip
        run(
            "docker", "run", "--rm", "--user", f"{os.getuid()}:{os.getgid()}", "-e", "HOME=/tmp",
            "-e", f"AWS_REGION={target['region']}",
            # The keys reach the container through its environment, never a command line, which any
            # user of the machine may read.
            *[flag for name in AWS_VARIABLES if name in os.environ for flag in ("-e", name)],
            *credentials,
            "-v", f"{settings}:/etc/aspen:ro", "-v", f"{opt.export.resolve()}:/export:ro",
            opt.image, "aspen-chat-server", "storage", "import", "/export",
        )  # fmt: skip


def finish(opt: argparse.Namespace) -> None:
    moving, target = state(opt), fleet(opt)
    if "voice_servers" not in moving:
        raise Failed("nothing has been copied yet: run `copy` first")
    url = f"https://{target['domain']}/api/v1/health/ready"
    try:
        with urllib.request.urlopen(url, timeout=15) as response:
            serving = response.status == 204
    except (urllib.error.URLError, TimeoutError):
        serving = False
    if not serving and not opt.force:
        raise Failed(
            f"{url} does not answer. The old deployment is the way back until this one serves, "
            "so it is kept; `--force` lets go of it anyway"
        )
    for name in moving["voice_servers"]:
        try:
            run_task(target, target["release_task"], "release", "aspen-chat-server", "voice-servers", "remove", name)
        except Failed:
            # It is refused while calls that were on it when it stopped are still recorded.
            print(
                f"The old voice server {name!r} is still registered. It takes no calls, since it reports nothing. "
                "Remove it from the dashboard's Server fleet tab, or run this step again in a few minutes.",
                file=sys.stderr,
            )
    print(
        "\nThe old voice server's registration is gone. On the Ultra Lite machine, keep it from\n"
        "starting again with `sudo systemctl disable aspen-ul`. Keep its data directory, and the\n"
        "export, until the new deployment has run a while: they are the old deployment. Keep\n"
        f"{VARIABLES_FILE} in {opt.fleet}: `domain` is in it."
    )
    (opt.fleet / STATE_FILE).unlink(missing_ok=True)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--fleet", type=pathlib.Path, default=HERE, help="this configuration's directory")
    steps = parser.add_subparsers(dest="step", required=True)
    steps.add_parser("prepare").add_argument("--domain", required=True, help="the Ultra Lite deployment's domain")
    copied = steps.add_parser("copy")
    copied.add_argument("--export", type=pathlib.Path, required=True,
                        help="the directory `aspen-ul export` wrote, copied to this machine")  # fmt: skip
    copied.add_argument("--image", default="ghcr.io/aspen-chat/aspen-api:main",
                        help="the API image whose `storage import` uploads the export's files")  # fmt: skip
    copied.add_argument("--files-copied", action="store_true",
                        help="the files of an Ultra Lite deployment on object storage are copied into this one's bucket")  # fmt: skip
    steps.add_parser("finish").add_argument("--force", action="store_true", help="let go of the old deployment though the new one does not answer")
    opt = parser.parse_args()
    {"prepare": prepare, "copy": copy, "finish": finish}[opt.step](opt)


if __name__ == "__main__":
    try:
        main()
    except Failed as failure:
        print(f"migrate: {failure}", file=sys.stderr)
        sys.exit(1)
