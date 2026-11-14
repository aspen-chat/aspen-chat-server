#!/usr/bin/env python3
"""Builds Google's WebRTC library for the mobile apps' call engine, from Google's source at the
milestone pinned here, with Google's own tools: the iOS xcframework on a Mac, the Android archive
on Linux. Each app links what this builds rather than a package someone else built, so what parses
media from the network on a phone is source we chose, at a milestone we move on our own schedule.

    scripts/webrtc_build.py checkout        fetch the source, or move a checkout to the pinned branch
    scripts/webrtc_build.py build           build this machine's platform (checks out first if needed)
    scripts/webrtc_build.py status          what is checked out, and the last build's result

The checkout is large (30 GB and more) and lives outside the repository, in `~/webrtc-build` unless
`--root` says otherwise, beside Google's `depot_tools`, which the script fetches too. Keep it: a
milestone bump is a `checkout` and a `build`, not another fetch.

`build` takes `--jobs N` to cap the compile jobs; the machine's cores otherwise. An 8 GB Mac
builds both iOS slices in about 17 minutes at 3 jobs without growing its swap. The artifact
(`WebRTC.xcframework` or `libwebrtc.aar`) is copied to `--out`, `target/webrtc/` by default.

Needs Python 3.9 or later (what macOS ships), git, network access, and on a Mac Xcode; on Linux
a JDK (`JAVA_HOME`, or `java` on the `PATH`) for the Android archive's Java side.
"""

from __future__ import annotations

import argparse
import os
import platform
import shutil
import subprocess
import sys
import time
from pathlib import Path

# The WebRTC milestone the apps build against: a `branch-heads/N` branch of Google's repository,
# N being the Chromium branch of a milestone (`webrtc_branch` in chromiumdash's
# `fetch_milestones?mstone=M`). M157, stable from 2026-11. Move it with a stable release, read
# the release notes between, and rebuild on both platforms.
WEBRTC_BRANCH = "8103"
WEBRTC_MILESTONE = "M157"

# The lowest iOS the app runs on, as its Xcode project says.
IOS_DEPLOYMENT_TARGET = "15.0"
# The device, and the simulator on the Macs that build and test the app. Intel simulators are
# not built for.
IOS_ARCHS = ["device:arm64", "simulator:arm64"]
# Phones, older phones, and the emulator.
ANDROID_ARCHS = ["arm64-v8a", "armeabi-v7a", "x86_64"]

DEPOT_TOOLS = "https://chromium.googlesource.com/chromium/tools/depot_tools.git"


def platform_name() -> str:
    system = platform.system()
    if system == "Darwin":
        return "ios"
    if system == "Linux":
        return "android"
    sys.exit(f"WebRTC is built for iOS on a Mac and for Android on Linux, not on {system}")


class Build:
    def __init__(self, root: Path, out: Path) -> None:
        self.root = root
        self.out = out
        self.platform = platform_name()
        self.depot_tools = root / "depot_tools"
        self.checkout = root / self.platform
        self.src = self.checkout / "src"

    def env(self) -> dict[str, str]:
        """The build's environment: depot_tools first on the path, and before it depot_tools'
        own Python. macOS's `/usr/bin/python3` is a shim that resolves through Xcode's developer
        directory, and the Chromium toolchain's linker driver runs `python3` with that directory
        pointed at a linked SDK, where the shim gives up."""
        env = dict(os.environ)
        env["PATH"] = os.pathsep.join(
            [str(self.depot_tools / "python-bin"), str(self.depot_tools), env.get("PATH", "")]
        )
        if self.platform == "android" and "JAVA_HOME" not in env:
            java = shutil.which("java")
            if java is not None:
                env["JAVA_HOME"] = str(Path(java).resolve().parent.parent)
        # Google's tools would otherwise offer to report usage.
        env["DEPOT_TOOLS_METRICS"] = "0"
        return env

    def run(self, command: list[str], cwd: Path) -> None:
        print("+", " ".join(command), flush=True)
        subprocess.run(command, cwd=cwd, env=self.env(), check=True)

    def commit_of(self, ref: str) -> str | None:
        """The commit `ref` names in the checkout, `None` without a checkout or such a ref."""
        if not (self.src / ".git").exists():
            return None
        found = subprocess.run(
            ["git", "rev-parse", "--verify", "--quiet", ref],
            cwd=self.src,
            capture_output=True,
            text=True,
        )
        return found.stdout.strip() or None

    def on_pinned_branch(self) -> bool:
        """Whether the checkout is at the pinned branch's head, which it is checked out as a
        detached commit, not a local branch."""
        head = self.commit_of("HEAD")
        return head is not None and head == self.commit_of(f"refs/branch-heads/{WEBRTC_BRANCH}")

    def fetch_depot_tools(self) -> None:
        if not self.depot_tools.exists():
            self.root.mkdir(parents=True, exist_ok=True)
            self.run(["git", "clone", "--quiet", DEPOT_TOOLS, str(self.depot_tools)], self.root)
        # Its own first run bootstraps the Python it brings.
        self.run([str(self.depot_tools / "ensure_bootstrap")], self.root)

    def do_checkout(self) -> None:
        self.fetch_depot_tools()
        if not self.src.exists():
            self.checkout.mkdir(parents=True, exist_ok=True)
            # Without history: a checkout is for building one pinned branch, not for reading
            # the repository's past, and the dependency tree is tens of gigabytes as it is.
            self.run(
                ["fetch", "--nohooks", "--no-history", f"webrtc_{self.platform}"], self.checkout
            )
        ref = f"refs/branch-heads/{WEBRTC_BRANCH}"
        self.run(["git", "fetch", "--quiet", "--depth", "1", "origin", f"{ref}:{ref}"], self.src)
        self.run(["git", "checkout", "--quiet", ref], self.src)
        self.run(["gclient", "sync", "--with_branch_heads", "-D"], self.src)

    def do_build(self, jobs: int | None) -> None:
        if not self.on_pinned_branch():
            self.do_checkout()
        started = time.monotonic()
        if self.platform == "ios":
            artifact = self.build_ios(jobs)
        else:
            artifact = self.build_android(jobs)
        self.out.mkdir(parents=True, exist_ok=True)
        target = self.out / artifact.name
        if target.is_dir():
            shutil.rmtree(target)
        elif target.exists():
            target.unlink()
        if artifact.is_dir():
            shutil.copytree(artifact, target, symlinks=True)
        else:
            shutil.copy2(artifact, target)
        minutes = (time.monotonic() - started) / 60
        print(f"built {WEBRTC_MILESTONE} ({WEBRTC_BRANCH}) in {minutes:.1f} min: {target}")

    def build_ios(self, jobs: int | None) -> Path:
        script = self.src / "tools_webrtc" / "ios" / "build_ios_libs.py"
        # The script drives the build through siso with as many local jobs as there are cores
        # and offers no way to say fewer, so the cap is written into its command line, into
        # Google's script as it stands, and taken out again after.
        if jobs is not None:
            self.run(["git", "checkout", "--quiet", "--", str(script)], self.src)
            ninja_line = "        'ninja',\n"
            text = script.read_text()
            if text.count(ninja_line) != 1:
                sys.exit(f"{script} no longer runs ninja where the job cap is written; see build_ios")
            script.write_text(
                text.replace(ninja_line, f"        'ninja', '-local_jobs', '{jobs}',\n")
            )
        try:
            # Every build starts from nothing (the script's `--clean` only deletes, so it is a
            # run of its own). Build state kept between builds names the Xcode SDK it was made
            # with, and an Xcode update between builds removes that SDK, after which the linker
            # runs with no system root and fails on a path it then looks for.
            self.run(["python3", str(script), "--clean"], self.src)
            self.run(
                [
                    "python3",
                    str(script),
                    "--build_config",
                    "release",
                    "--arch",
                    *IOS_ARCHS,
                    "--deployment-target",
                    IOS_DEPLOYMENT_TARGET,
                    # The build's host tools are linked against the macOS SDK with clang's
                    # default library path `/usr/local/lib` among the search paths, a
                    # directory the SDK does not have. lld warns of it, the build makes linker
                    # warnings fatal, and nothing needs the directory.
                    "--extra-gn-args",
                    "fatal_linker_warnings=false",
                ],
                self.src,
            )
        finally:
            if jobs is not None:
                self.run(["git", "checkout", "--quiet", "--", str(script)], self.src)
        return self.src / "out_ios_libs" / "WebRTC.xcframework"

    def build_android(self, jobs: int | None) -> Path:
        # Google's script insists on building inside the source tree.
        build_dir = self.src / "out_aar"
        archive = build_dir / "libwebrtc.aar"
        # From nothing, as on iOS.
        shutil.rmtree(build_dir, ignore_errors=True)
        command = [
            "python3",
            str(self.src / "tools_webrtc" / "android" / "build_aar.py"),
            "--build-dir",
            str(build_dir),
            "--output",
            str(archive),
            "--arch",
            *ANDROID_ARCHS,
        ]
        if jobs is not None:
            # One argument, since the switch itself begins with a dash.
            command.append(f"--extra-ninja-switches=-local_jobs={jobs}")
        self.run(command, self.src)
        return archive

    def status(self) -> None:
        print(f"platform: {self.platform}")
        print(f"pinned:   {WEBRTC_MILESTONE}, refs/branch-heads/{WEBRTC_BRANCH}")
        head = self.commit_of("HEAD")
        if head is None:
            print(f"checkout: none at {self.src}")
        elif self.on_pinned_branch():
            print(f"checkout: {self.src}, at the pinned branch")
        else:
            print(f"checkout: {self.src}, at {head[:12]}, not the pinned branch")
        artifact = (
            self.out / "WebRTC.xcframework" if self.platform == "ios" else self.out / "libwebrtc.aar"
        )
        if artifact.exists():
            built = time.strftime("%Y-%m-%d %H:%M", time.localtime(artifact.stat().st_mtime))
            print(f"artifact: {artifact} (built {built})")
        else:
            print(f"artifact: none at {artifact}")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument(
        "--root", type=Path, default=Path.home() / "webrtc-build", help="where the checkout lives"
    )
    parser.add_argument(
        "--out",
        type=Path,
        default=Path(__file__).resolve().parent.parent / "target" / "webrtc",
        help="where the built artifact is copied",
    )
    commands = parser.add_subparsers(dest="command", required=True)
    commands.add_parser("checkout")
    build = commands.add_parser("build")
    build.add_argument("--jobs", type=int, help="compile jobs at once; the machine's cores otherwise")
    commands.add_parser("status")
    args = parser.parse_args()
    build = Build(args.root, args.out)
    try:
        if args.command == "checkout":
            build.do_checkout()
        elif args.command == "build":
            build.do_build(args.jobs)
        else:
            build.status()
    except subprocess.CalledProcessError as error:
        sys.exit(f"failed: {' '.join(error.cmd)} exited {error.returncode}")


if __name__ == "__main__":
    main()
