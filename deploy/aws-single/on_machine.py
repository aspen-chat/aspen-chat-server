#!/usr/bin/env python3
"""Runs one command on the deployment's machine, through Session Manager, and prints what it
wrote.

    on_machine.py --region REGION --instance ID -- COMMAND ARG...

`tofu apply` runs `aspen-release` this way for each release (release.tf), and `tofu output -raw
on_machine` prints the command with this deployment's options filled in, for running
`aspen-box` there: `$(tofu output -raw on_machine) aspen-box run -- aspen-chat-server settings
show`. It exits with the command's exit code. It needs the AWS command line and the
credentials `tofu` uses.
"""

import argparse
import json
import os
import shlex
import subprocess
import sys
import time

# A first release waits for the machine's setup and pulls both images, and a migration that
# rewrites a large table is slow.
TIMEOUT_SECONDS = 3600
POLL_SECONDS = 5
# How long a machine just started may take to be reachable through Session Manager.
REGISTER_SECONDS = 600


class Failed(Exception):
    pass


def aws(region: str, *arguments: str, check: bool = True) -> dict:
    done = subprocess.run(["aws", "--region", region, "--output", "json", *arguments], capture_output=True, text=True)
    if done.returncode != 0:
        if not check:
            return {"error": done.stderr.strip()}
        raise Failed(f"aws {' '.join(arguments[:2])} failed: {done.stderr.strip()}")
    return json.loads(done.stdout) if done.stdout.strip() else {}


def send(region: str, instance: str, command: str) -> str:
    """Sends the command, waiting first for the machine to be reachable. Answers its id."""
    parameters = json.dumps({"commands": [command], "executionTimeout": [str(TIMEOUT_SECONDS)]})
    deadline = time.monotonic() + REGISTER_SECONDS
    while True:
        sent = aws(
            region, "ssm", "send-command", "--instance-ids", instance, "--document-name", "AWS-RunShellScript",
            "--parameters", parameters, "--timeout-seconds", "60", check=False,
        )  # fmt: skip
        if "Command" in sent:
            return sent["Command"]["CommandId"]
        if "InvalidInstanceId" not in sent["error"] or time.monotonic() > deadline:
            raise Failed(f"could not send the command: {sent['error']}")
        time.sleep(POLL_SECONDS)


def wait(region: str, instance: str, command_id: str) -> dict:
    while True:
        time.sleep(POLL_SECONDS)
        found = aws(region, "ssm", "get-command-invocation", "--command-id", command_id, "--instance-id", instance, check=False)
        if "error" in found:
            if "InvocationDoesNotExist" in found["error"]:
                continue
            raise Failed(found["error"])
        if found["Status"] not in ("Pending", "InProgress", "Delayed"):
            return found


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--region", default=os.environ.get("ASPEN_MACHINE_REGION"), help="(or ASPEN_MACHINE_REGION)")
    parser.add_argument("--instance", default=os.environ.get("ASPEN_MACHINE_INSTANCE"), help="(or ASPEN_MACHINE_INSTANCE)")
    parser.add_argument("command", nargs="+")
    opt = parser.parse_args()
    if not opt.region or not opt.instance:
        parser.error("give --region and --instance")
    command_id = send(opt.region, opt.instance, shlex.join(opt.command))
    print(f"Running on {opt.instance} ({command_id})", file=sys.stderr)
    done = wait(opt.region, opt.instance, command_id)
    # Session Manager keeps only the first 24,000 characters of each.
    sys.stdout.write(done.get("StandardOutputContent", ""))
    sys.stderr.write(done.get("StandardErrorContent", ""))
    if done["Status"] != "Success":
        print(f"{shlex.join(opt.command)}: {done['Status']} ({done.get('StatusDetails', '')})", file=sys.stderr)
    code = done.get("ResponseCode", -1)
    return code if code >= 0 else 1


if __name__ == "__main__":
    try:
        sys.exit(main())
    except Failed as failure:
        print(f"on_machine.py: {failure}", file=sys.stderr)
        sys.exit(1)
