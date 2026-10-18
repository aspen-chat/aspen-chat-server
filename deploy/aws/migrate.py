#!/usr/bin/env python3
"""Moves a deployment that runs on one machine, a box, to this configuration, in three steps with
a `tofu apply` of this configuration after each of the first two:

    migrate.py prepare --from-single    while the old deployment serves: point this configuration
    migrate.py prepare --from-box       at its address, and, from ../aws-single, its storage
    migrate.py copy [--export DIR]      stop the old deployment, and copy it here
    migrate.py finish                   once this deployment serves: let go of the old one

A box made with ../aws-single keeps its files in a bucket, which this deployment takes over, so
only its database is copied, and `copy` runs the box's export on its machine itself. Any other
box is exported on its machine (`aspen-box export DIR`), and `copy --export DIR` copies its
database and its files here from that directory.

README.md#moving-a-box-here is the whole procedure. Each step says what to do next, and may be
run again if it fails part of the way. It needs `tofu`, the AWS command line, and the
credentials the configurations are applied with.
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
# decides, and, from ../aws-single, the two things of its that this one imports rather than
# makes.
VARIABLES_FILE = "migration.auto.tfvars.json"
IMPORTS_FILE = "adopt_media.tf"
# Where the move stands: where it is from, and, once copied, what the old deployment held.
STATE_FILE = "migration.json"
# Where ../aws-single's box is exported on its machine, on its data volume.
SINGLE_EXPORT = "/data/aspen/export"
AWS_CLI = "public.ecr.aws/aws-cli/aws-cli:latest"


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


def single(opt: argparse.Namespace) -> dict:
    return output(
        opt.single, "handover",
        f"{opt.single} has no `handover` output: apply it once with this release of the configuration",
    )  # fmt: skip


def fleet(opt: argparse.Namespace) -> dict:
    return output(opt.fleet, "migration", f"{opt.fleet} has not been applied: run `prepare`, then `tofu apply` there")


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


def on_machine(opt: argparse.Namespace, source: dict, *command: str, capture: bool = False) -> str:
    """Runs a command on the machine of a deployment made with ../aws-single."""
    return run(
        sys.executable, str(opt.single / "on_machine.py"), "--region", source["region"], "--instance", source["instance"],
        "--", *command, capture=capture,
    )  # fmt: skip


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
    if opt.from_single:
        source = single(opt)
        write_variables(opt, {
            "region": source["region"],
            # The address is the deployment: its federation domain and its passkeys.
            "domain": source["domain"],
            "route53_zone_id": source["route53_zone_id"],
            "media_bucket": source["bucket"],
            # The old deployment keeps the name until `copy` has run.
            "public_dns": False,
        })  # fmt: skip
        (opt.fleet / IMPORTS_FILE).write_text(
            "# Written by migrate.py: the storage of the deployment this one replaces, kept rather\n"
            "# than copied. `finish` removes this file.\n"
            f'import {{\n  to = module.media.aws_s3_bucket.media\n  id = "{source["bucket"]}"\n}}\n\n'
            f'import {{\n  to = module.media.aws_cloudfront_distribution.media\n  id = "{source["distribution_id"]}"\n}}\n'
        )
        # Checked with the files above in place, which give the variables that have no default.
        run("tofu", f"-chdir={opt.fleet}", "init", "-input=false", capture=True)
        name = variable(opt.fleet, "var.name")
        if name == source["name"]:
            raise Failed(
                f"both deployments are named {name!r}, and they exist side by side until `finish`: "
                f"set `name` in {opt.fleet}/terraform.tfvars to another"
            )
    else:
        if not opt.domain:
            raise Failed("give the box's domain: --domain chat.example.org")
        write_variables(opt, {"domain": opt.domain, "public_dns": False})
        run("tofu", f"-chdir={opt.fleet}", "init", "-input=false", capture=True)
        for needed, why in (("region", ""), ("route53_zone_id", ", the hosted zone that holds the box's domain")):
            try:
                variable(opt.fleet, f"var.{needed}")
            except Failed:
                raise Failed(f"set `{needed}`{why} in {opt.fleet}/terraform.tfvars") from None
    # The box's own voice server is registered as `voice`, and its registration comes with the
    # database.
    if "voice" in variable(opt.fleet, "keys(var.voice_servers)"):
        raise Failed("the box's voice server is registered as 'voice', which comes with its database: call none of `voice_servers` that")
    save_state(opt, {"from": "single" if opt.from_single else "box"})
    copying = (
        "Then run `copy`, which stops the old deployment."
        if opt.from_single
        else "Then, on the box's machine, `aspen-box export DIR`, which stops it. Copy DIR to this\n"
        "machine, keeping it where only you can read it, and run `copy --export DIR`."
    )
    print(
        f"Wrote {VARIABLES_FILE}{f' and {IMPORTS_FILE}' if opt.from_single else ''} in {opt.fleet}.\n\n"
        f"Next, in {opt.fleet}, with no `image_tag` set:\n\n"
        "    tofu apply\n\n"
        + (
            "The plan imports the bucket and the distribution and changes the distribution in\n"
            "place. It must not replace or destroy either. "
            if opt.from_single
            else ""
        )
        + f"The old deployment goes on serving.\n{copying}"
    )


def copy(opt: argparse.Namespace) -> None:
    moving, target = state(opt), fleet(opt)
    if moving["from"] == "single":
        source = single(opt)
        if not opt.yes:
            answer = input(f"This stops the deployment at https://{source['domain']} until the new one is released. Type the domain to go on: ")
            if answer.strip() != source["domain"]:
                raise Failed("not confirmed")
        print("Stopping the old deployment and exporting its database…", flush=True)
        # On the machine, as its role, which may write the bucket. The dump is not left there:
        # the data volume is kept, as the way back, after the deployment is gone.
        on_machine(opt, source, "sh", "-ec", "\n".join([
            f"rm -rf {SINGLE_EXPORT}",
            f"aspen-box export {SINGLE_EXPORT} --database-only --yes",
            f"docker run --rm --network host -e AWS_REGION={source['region']} -v {SINGLE_EXPORT}:/export:ro "
            f"{AWS_CLI} s3 cp /export/database.dump {target['dump']} --only-show-errors",
            f"rm {SINGLE_EXPORT}/database.dump",
        ]))  # fmt: skip
        exported = json.loads(on_machine(opt, source, "cat", f"{SINGLE_EXPORT}/export.json", capture=True))
    else:
        if not opt.export:
            raise Failed("give the directory `aspen-box export` wrote, copied to this machine: --export DIR")
        exported = json.loads((opt.export / "export.json").read_text())
        if exported["domain"] != target["domain"]:
            raise Failed(f"{opt.export} is an export of {exported['domain']}, and this deployment is {target['domain']}")
    clash = set(exported["voice_servers"]) & set(target["voice_servers"])
    if clash:
        raise Failed(
            f"the old deployment's voice servers {sorted(clash)} have the names of this one's, and their registrations "
            "come with the database: rename this one's in `voice_servers`, `tofu apply`, and run `copy` again"
        )
    if moving["from"] == "box":
        print("Copying the database into the bucket…", flush=True)
        run("aws", "--region", target["region"], "s3", "cp", str(opt.export / "database.dump"), target["dump"], "--only-show-errors")
        if exported["files"]:
            print("Copying people's files into the bucket…", flush=True)
            run(
                "aws", "--region", target["region"], "s3", "sync", str(opt.export / "files"), f"s3://{target['bucket']}",
                "--exclude", "migration/*", "--only-show-errors",
            )  # fmt: skip
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
    if moving["from"] == "single":
        source = single(opt)
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
            "old deployment. Delete them in the EC2 console once the new deployment has run a while.\n"
            f"Keep {VARIABLES_FILE} in {opt.fleet}: `media_bucket` in it is this deployment's bucket."
        )
    else:
        print(
            "\nThe old voice servers' registrations are gone. Take the box down on its machine with\n"
            "`aspen-box down`, and any voice servers it had elsewhere on theirs. Keep the box's\n"
            "directory, and the export, until the new deployment has run a while: they are the old\n"
            f"deployment. Keep {VARIABLES_FILE} in {opt.fleet}: `domain` is in it."
        )
    (opt.fleet / STATE_FILE).unlink(missing_ok=True)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--single", type=pathlib.Path, default=HERE.parent / "aws-single",
                        help="the configuration directory of a deployment made with ../aws-single")  # fmt: skip
    parser.add_argument("--fleet", type=pathlib.Path, default=HERE, help="this configuration's directory")
    steps = parser.add_subparsers(dest="step", required=True)
    ready = steps.add_parser("prepare")
    origin = ready.add_mutually_exclusive_group(required=True)
    origin.add_argument("--from-single", action="store_true", help="a deployment made with ../aws-single")
    origin.add_argument("--from-box", action="store_true", help="any other box")
    ready.add_argument("--domain", help="the box's domain (--from-box)")
    copied = steps.add_parser("copy")
    copied.add_argument("--export", type=pathlib.Path, help="the directory `aspen-box export` wrote (--from-box)")
    copied.add_argument("--yes", action="store_true", help="do not ask before stopping the old deployment")
    steps.add_parser("finish").add_argument("--force", action="store_true", help="let go of the old deployment though the new one does not answer")
    opt = parser.parse_args()
    {"prepare": prepare, "copy": copy, "finish": finish}[opt.step](opt)


if __name__ == "__main__":
    try:
        main()
    except Failed as failure:
        print(f"migrate: {failure}", file=sys.stderr)
        sys.exit(1)
