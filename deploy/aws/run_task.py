#!/usr/bin/env python3
"""Runs one task of a deployment to its end: the release task as it is defined, or an operator
command in its place.

    run_task.py [options]                      the task definition's own command
    run_task.py [options] -- COMMAND ARG...    COMMAND instead, with the deployment's aspen.toml

`tofu apply` runs the first form for each release (release.tf), and `tofu output -raw run_task`
prints the second with this deployment's options filled in. It prints what the task logged and
exits with the task's exit code. It runs the task on Fargate, in the subnets and security group
given, and needs the AWS command line and the credentials `tofu` uses.
"""

import argparse
import json
import os
import subprocess
import sys
import time

# The container a command given here replaces the command of, and the last to run in a task of
# several (servers.tf, migration.tf).
CONTAINER = "aspen"
# How long a release may take: a migration that rewrites a large table is slow.
DEFAULT_TIMEOUT_SECONDS = 3600
POLL_SECONDS = 5


class Failed(Exception):
    pass


def aws(region: str, *arguments: str) -> dict:
    """One AWS command's JSON answer."""
    done = subprocess.run(
        ["aws", "--region", region, "--output", "json", *arguments],
        capture_output=True,
        text=True,
    )
    if done.returncode != 0:
        raise Failed(f"aws {' '.join(arguments[:2])} failed: {done.stderr.strip()}")
    return json.loads(done.stdout) if done.stdout.strip() else {}


def options() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])

    def option(name: str, **kwargs) -> None:
        variable = "ASPEN_RUN_" + name.upper().replace("-", "_")
        default = os.environ.get(variable, kwargs.pop("default", None))
        parser.add_argument(
            f"--{name}",
            default=default,
            required=default is None,
            help=f"(or {variable})",
            **kwargs,
        )

    option("region")
    option("cluster")
    option("task-definition")
    option("subnets")
    option("security-group")
    option("log-group")
    option("log-prefix")
    option("timeout", default=str(DEFAULT_TIMEOUT_SECONDS), type=int)
    parser.add_argument("command", nargs="*", help="run this in place of the task's own command")
    return parser.parse_args()


def start(opt: argparse.Namespace) -> str:
    network = {
        "awsvpcConfiguration": {
            "subnets": opt.subnets.split(","),
            "securityGroups": [opt.security_group],
            # The subnets are public ones; without an address the task cannot pull its image.
            "assignPublicIp": "ENABLED",
        }
    }
    arguments = [
        "ecs", "run-task", "--cluster", opt.cluster, "--task-definition", opt.task_definition,
        "--launch-type", "FARGATE", "--network-configuration", json.dumps(network),
    ]  # fmt: skip
    if opt.command:
        overrides = {"containerOverrides": [{"name": CONTAINER, "command": opt.command}]}
        arguments += ["--overrides", json.dumps(overrides)]
    started = aws(opt.region, *arguments)
    if started.get("failures"):
        raise Failed(f"ECS did not start the task: {started['failures']}")
    return started["tasks"][0]["taskArn"]


def wait(opt: argparse.Namespace, task: str) -> dict:
    deadline = time.monotonic() + int(opt.timeout)
    while time.monotonic() < deadline:
        described = aws(opt.region, "ecs", "describe-tasks", "--cluster", opt.cluster, "--tasks", task)
        state = described["tasks"][0]
        if state["lastStatus"] == "STOPPED":
            return state
        time.sleep(POLL_SECONDS)
    aws(opt.region, "ecs", "stop-task", "--cluster", opt.cluster, "--task", task)
    raise Failed(f"the task was still running after {opt.timeout} seconds, and has been stopped")


def print_logs(opt: argparse.Namespace, task: str, container: str) -> None:
    stream = f"{opt.log_prefix}/{container}/{task.rsplit('/', 1)[-1]}"
    arguments = [
        "logs", "get-log-events",
        "--log-group-name", opt.log_group,
        "--log-stream-name", stream,
        "--start-from-head",
    ]  # fmt: skip
    token = None
    while True:
        try:
            page = aws(opt.region, *arguments, *(["--next-token", token] if token else []))
        except Failed:
            # A container that never started wrote no stream.
            return
        for event in page.get("events", []):
            print(event["message"])
        # The last page answers with the token it was asked for.
        if page.get("nextForwardToken") == token:
            return
        token = page.get("nextForwardToken")


def main() -> int:
    opt = options()
    task = start(opt)
    print(f"started {task}", file=sys.stderr)
    state = wait(opt, task)
    # A task of several containers runs them in turn, each waiting for the one before to
    # succeed, so the first that failed is why the task did.
    containers = sorted(state["containers"], key=lambda c: c["name"] == CONTAINER)
    for container in containers:
        if len(containers) > 1:
            print(f"--- {container['name']}", file=sys.stderr)
        print_logs(opt, task, container["name"])
    for container in containers:
        code = container.get("exitCode")
        if code is None:
            # It never ran: its image could not be pulled, its configuration could not be
            # read, or a container before it failed.
            reason = container.get("reason") or state.get("stoppedReason") or "no reason given"
            raise Failed(f"{container['name']} stopped without running: {reason}")
        if code != 0:
            print(f"{container['name']} exited with {code}", file=sys.stderr)
            return code
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except Failed as failure:
        print(f"run_task: {failure}", file=sys.stderr)
        sys.exit(1)
