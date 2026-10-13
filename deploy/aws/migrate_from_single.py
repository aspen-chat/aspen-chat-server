#!/usr/bin/env python3
"""Moves a deployment from the one-machine configuration (../aws-single) to this one, in three
steps with a `tofu apply` of this configuration after each of the first two:

    migrate_from_single.py prepare   while the old deployment serves: point this configuration
                                     at its address and its storage
    migrate_from_single.py copy      stop the old deployment and copy its database here
    migrate_from_single.py finish    once this deployment serves: let go of the old one

README.md#moving-from-one-machine is the whole procedure. Each step says what to do next, and
may be run again if it fails part of the way. It needs `tofu`, the AWS command line, and the
credentials both configurations are applied with.
"""

import argparse
import json
import pathlib
import subprocess
import sys
import urllib.error
import urllib.request

HERE = pathlib.Path(__file__).resolve().parent
# Written by `prepare` into this configuration's directory: the variables the old deployment
# decides, and the two things of its that this one imports rather than makes.
VARIABLES_FILE = "migration.auto.tfvars.json"
IMPORTS_FILE = "adopt_media.tf"


class Failed(Exception):
    pass


def run(*command: str, capture: bool = False) -> str:
    done = subprocess.run(command, capture_output=capture, text=True)
    if done.returncode != 0:
        detail = f": {done.stderr.strip()}" if capture else ""
        raise Failed(f"`{' '.join(command[:4])} …` failed{detail}")
    return done.stdout if capture else ""


def output(directory: pathlib.Path, name: str, missing: str) -> dict:
    """One output of the configuration applied in `directory`."""
    outputs = json.loads(run("tofu", f"-chdir={directory}", "output", "-json", capture=True))
    if name not in outputs:
        raise Failed(missing)
    return outputs[name]["value"]


def old(opt: argparse.Namespace) -> dict:
    return output(
        opt.single,
        "handover",
        f"{opt.single} has no `handover` output: apply it once with this release of the "
        "configuration, which adds the task that exports its database",
    )


def new(opt: argparse.Namespace) -> dict:
    return output(
        opt.fleet,
        "migration",
        f"{opt.fleet} has not been applied: run `prepare`, then `tofu apply` there",
    )


def variable(directory: pathlib.Path, expression: str):
    """A value of the configuration in `directory`, as its variables stand."""
    done = subprocess.run(
        ["tofu", f"-chdir={directory}", "console"],
        input=f"jsonencode({expression})\n",
        capture_output=True,
        text=True,
    )
    if done.returncode != 0:
        raise Failed(f"could not read {expression} in {directory}: {done.stderr.strip()}")
    return json.loads(json.loads(done.stdout))


def run_task(deployment: dict, task: str, prefix: str, *command: str) -> None:
    """Runs one of a deployment's tasks to its end, on Fargate where it has subnets for it."""
    arguments = [
        sys.executable, str(HERE / "run_task.py"),
        "--region", deployment["region"],
        "--cluster", deployment["cluster"],
        "--task-definition", task,
        "--log-group", deployment["log_group"],
        "--log-prefix", prefix,
    ]  # fmt: skip
    if "subnets" in deployment:
        arguments += ["--subnets", deployment["subnets"], "--security-group", deployment["security_group"]]
    run(*arguments, *(["--", *command] if command else []))


def prepare(opt: argparse.Namespace) -> None:
    source = old(opt)
    (opt.fleet / VARIABLES_FILE).write_text(
        json.dumps(
            {
                "region": source["region"],
                # The address is the deployment: its federation domain and its passkeys.
                "domain": source["domain"],
                "route53_zone_id": source["route53_zone_id"],
                "media_bucket": source["bucket"],
                # The old deployment keeps the name until `copy` has run.
                "public_dns": False,
            },
            indent=2,
        )
        + "\n"
    )
    (opt.fleet / IMPORTS_FILE).write_text(
        "# Written by migrate_from_single.py: the storage of the deployment this one replaces,\n"
        "# kept rather than copied. `finish` removes this file.\n"
        f'import {{\n  to = module.media.aws_s3_bucket.media\n  id = "{source["bucket"]}"\n}}\n\n'
        f'import {{\n  to = module.media.aws_cloudfront_distribution.media\n  id = "{source["distribution_id"]}"\n}}\n'
    )
    # Checked with the file above in place, which gives the variables that have no default.
    name = variable(opt.fleet, "var.name")
    if name == source["name"]:
        raise Failed(
            f"both deployments are named {name!r}, and they exist side by side until `finish`: "
            f"set `name` in {opt.fleet}/terraform.tfvars to another"
        )
    if source["voice_server"] in variable(opt.fleet, "keys(var.voice_servers)"):
        raise Failed(
            f"the old deployment's voice server is registered as {source['voice_server']!r}, and "
            "its registration comes with the database: call none of `voice_servers` that"
        )
    print(
        f"Wrote {VARIABLES_FILE} and {IMPORTS_FILE} in {opt.fleet}.\n\n"
        f"Next, in {opt.fleet}, with no `image_tag` set:\n\n"
        "    tofu init && tofu apply\n\n"
        "The plan imports the bucket and the distribution and changes the distribution in\n"
        "place. It must not replace or destroy either. The old deployment goes on serving.\n"
        "Then push this deployment's images, and run `copy`."
    )


def copy(opt: argparse.Namespace) -> None:
    source, target = old(opt), new(opt)
    if not opt.yes:
        answer = input(
            f"This stops the deployment at https://{source['domain']} until the new one is "
            "released. Type the domain to go on: "
        )
        if answer.strip() != source["domain"]:
            raise Failed("not confirmed")
    # Nothing may be written after the dump is taken, or it is lost.
    for server in source["servers"]:
        run(
            "aws", "--region", source["region"], "ecs", "update-service",
            "--cluster", source["cluster"], "--service", server, "--desired-count", "0",
            "--query", "service.serviceName", "--output", "text",
        )  # fmt: skip
    run(
        "aws", "--region", source["region"], "ecs", "wait", "services-stable",
        "--cluster", source["cluster"], "--services", *source["servers"],
    )  # fmt: skip
    print("The old deployment's servers are stopped. Exporting its database…", flush=True)
    run_task(source, source["export_task"], "export")
    print("Restoring it into the new deployment…", flush=True)
    run_task(target, target["restore_task"], "restore")
    variables = json.loads((opt.fleet / VARIABLES_FILE).read_text())
    variables.pop("public_dns", None)
    (opt.fleet / VARIABLES_FILE).write_text(json.dumps(variables, indent=2) + "\n")
    print(
        "\nThe database is copied, and the next apply gives this deployment the name.\n"
        f"Set `image_tag` in {opt.fleet}/terraform.tfvars, and there:\n\n"
        "    tofu apply\n\n"
        "When the deployment answers at its address, run `finish`. To go back instead, see\n"
        "README.md#going-back."
    )


def finish(opt: argparse.Namespace) -> None:
    source, target = old(opt), new(opt)
    url = f"https://{target['domain']}/api/v1/deployment"
    try:
        with urllib.request.urlopen(url, timeout=15) as response:
            serving = response.status == 200
    except (urllib.error.URLError, TimeoutError):
        serving = False
    if not serving and not opt.force:
        raise Failed(
            f"{url} does not answer. The old deployment is the way back until this one serves, "
            "so it is kept; `--force` lets go of it anyway"
        )
    try:
        run_task(
            target, target["release_task"], "release",
            "aspen-chat-server", "voice-servers", "remove", source["voice_server"],
        )  # fmt: skip
    except Failed:
        # It is refused while calls that were on it when it stopped are still recorded.
        print(
            f"The old voice server, {source['voice_server']!r}, is still registered. It takes no "
            "calls, since it reports nothing. Remove it from the dashboard's Server fleet tab, or "
            "run this step again in a few minutes.",
            file=sys.stderr,
        )
    held = set(run("tofu", f"-chdir={opt.single}", "state", "list", capture=True).split())
    kept = [address for address in source["kept"] if address in held]
    if kept:
        run("tofu", f"-chdir={opt.single}", "state", "rm", *kept)
    (opt.fleet / IMPORTS_FILE).unlink(missing_ok=True)
    print(
        f"\nThe old deployment no longer holds the storage, the name, or its data volume.\n"
        f"Take the rest of it down, in {opt.single}:\n\n"
        "    tofu destroy\n\n"
        f"Its data volume, {source['data_volume']}, and the snapshots of it are kept: they are the\n"
        "old database. Delete them in the EC2 console once the new deployment has run a while.\n"
        f"Keep {VARIABLES_FILE} in {opt.fleet}: `media_bucket` in it is this deployment's bucket."
    )


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--single", type=pathlib.Path, default=HERE.parent / "aws-single",
                        help="the old deployment's configuration directory")  # fmt: skip
    parser.add_argument("--fleet", type=pathlib.Path, default=HERE,
                        help="this configuration's directory")  # fmt: skip
    steps = parser.add_subparsers(dest="step", required=True)
    steps.add_parser("prepare")
    steps.add_parser("copy").add_argument("--yes", action="store_true", help="do not ask before stopping the old deployment")
    steps.add_parser("finish").add_argument("--force", action="store_true", help="let go of the old deployment though the new one does not answer")
    opt = parser.parse_args()
    {"prepare": prepare, "copy": copy, "finish": finish}[opt.step](opt)


if __name__ == "__main__":
    try:
        main()
    except Failed as failure:
        print(f"migrate_from_single: {failure}", file=sys.stderr)
        sys.exit(1)
