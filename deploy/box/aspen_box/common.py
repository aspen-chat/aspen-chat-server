"""What every command shares: the box's directory and its `box.json`, Docker Compose run against
it, and filling the configuration templates."""

from __future__ import annotations

import ipaddress
import json
import os
import pathlib
import re
import subprocess
import urllib.request

HERE = pathlib.Path(__file__).resolve().parent.parent
TEMPLATES = HERE.parent / "templates"
# The published images, which `--tag` names a release of.
IMAGES = {"api": "ghcr.io/aspen-chat/aspen-api", "voice": "ghcr.io/aspen-chat/aspen-voice"}
BUCKET = "aspen-media"
# The voice server on the deployment's own machine, at voice.<domain>.
HERE_VOICE = "voice"
# The two places an API server may run, by compose service: an upgrade starts the one that is
# not running before it stops the one that is.
SLOTS = {"api-a": 8080, "api-b": 8081}
# Answers with the caller's address as text, over whichever family the name was reached by.
ADDRESS_SERVICES = {4: "https://1.1.1.1/cdn-cgi/trace", 6: "https://[2606:4700:4700::1111]/cdn-cgi/trace"}
GIB = 1 << 30


class Failed(Exception):
    pass


class Box:
    """A box's directory: `box.json`, `config/` written from it, `data/`, and `backups/`."""

    def __init__(self, directory: pathlib.Path):
        self.dir = directory
        self.file = directory / "box.json"

    def load(self) -> dict:
        if not self.file.exists():
            raise Failed(f"{self.dir} holds no deployment yet: run `aspen-box init`")
        return json.loads(self.file.read_text())

    def save(self, box: dict) -> None:
        self.dir.mkdir(parents=True, exist_ok=True)
        # It holds every password.
        self.file.touch(mode=0o600, exist_ok=True)
        self.file.chmod(0o600)
        self.file.write_text(json.dumps(box, indent=2) + "\n")

    def compose(self, *arguments: str, capture: bool = False, check: bool = True, stdin=None, stdout=None, env: dict | None = None):
        """Docker Compose on this box's services. `ASPEN_BOX_COMPOSE_FILES` names more files laid
        over `compose.yaml` (`:` between them), and `ASPEN_BOX_PROJECT` the project, so that
        boxes can stand side by side on one machine for testing."""
        files = [str(HERE / "compose.yaml"), *filter(None, os.environ.get("ASPEN_BOX_COMPOSE_FILES", "").split(":"))]
        command = [
            "docker", "compose", "--project-name", os.environ.get("ASPEN_BOX_PROJECT", "aspen"),
            "--project-directory", str(self.dir), "--env-file", str(self.dir / ".env"),
        ]  # fmt: skip
        for file in files:
            command += ["-f", file]
        done = subprocess.run([*command, *arguments], text=stdout is None, capture_output=capture, stdin=stdin, stdout=stdout,
                              env={**os.environ, **env} if env else None)  # fmt: skip
        if check and done.returncode != 0:
            detail = f": {done.stderr.strip()[-600:]}" if capture else ""
            raise Failed(f"`docker compose {' '.join(arguments[:4])}` failed{detail}")
        return done

    def running(self) -> set:
        try:
            done = self.compose("ps", "--services", "--status", "running", capture=True)
        except Failed:
            return set()
        return set(done.stdout.split())

    def exec_ok(self, service: str, *command: str) -> bool:
        return self.compose("exec", "-T", service, *command, capture=True, check=False).returncode == 0


def fill(template: pathlib.Path, values: dict) -> str:
    """A template with each `${name}` replaced. A name the values lack is an error, as it is to
    OpenTofu's `templatefile`, which fills the shared templates the same way."""
    return re.sub(r"\$\{(\w+)\}", lambda match: str(values[match.group(1)]), template.read_text())


def encoded(values: dict) -> dict:
    """Each value as TOML and NATS write it, which is as JSON does (`../templates/README.md`)."""
    return {name: json.dumps(value) for name, value in values.items()}


def detect_address(family: int) -> str | None:
    """This machine's public address as the internet sees it, or None when it has none of that
    family or the service that says is unreachable."""
    try:
        with urllib.request.urlopen(ADDRESS_SERVICES[family], timeout=6) as response:
            found = re.search(r"^ip=(\S+)$", response.read().decode(), re.M)
            return str(ipaddress.ip_address(found.group(1))) if found else None
    except (OSError, ValueError):
        return None


def addresses(box: dict) -> list:
    """The addresses clients send media to: those `init` was given, `auto` looked up now."""
    found = []
    for family in (4, 6):
        wanted = box[f"address{family}"]
        address = detect_address(family) if wanted == "auto" else (None if wanted == "none" else wanted)
        if address:
            found.append(address)
    if not found:
        raise Failed(
            "this machine's public address could not be found, and the voice server must announce "
            "one: give --address4 (and --address6), or check the machine reaches the internet"
        )
    return found


def memory_bytes() -> int:
    return os.sysconf("SC_PAGE_SIZE") * os.sysconf("SC_PHYS_PAGES")


def write_files(directory: pathlib.Path, files: dict) -> set:
    """Writes each of `files` that changed, and answers which did."""
    changed = set()
    for name, text in files.items():
        path = directory / name
        path.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
        if not path.exists() or path.read_text() != text:
            changed.add(name)
            # Written in place: a container that has the file mounted sees what is written.
            path.write_text(text)
    return changed


def write_env(store: Box, values: dict) -> None:
    (store.dir / ".env").write_text("".join(f"{name}={value}\n" for name, value in values.items()))


def voice_url(name: str, domain: str) -> str:
    return f"https://{name}.{domain}"


def ask(question: str, default: str | None = None) -> str:
    suffix = f" [{default}]" if default else ""
    while True:
        answer = input(f"{question}{suffix}: ").strip() or (default or "")
        if answer:
            return answer
