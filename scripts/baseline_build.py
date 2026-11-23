#!/usr/bin/env python3
"""Builds Aspen's servers for 64-bit Linux, x86-64 or ARM, against a Debian sysroot, so that the
binaries run on that Debian release and anything newer, whatever the build machine runs.

A binary links against the C library and OpenSSL of the system it is built on, and needs at least
those versions wherever it runs: built on a rolling-release machine, it needs that machine's glibc
and OpenSSL, which a server on a long-term release lacks. This script builds against the oldest
release the binaries must run on instead, by default Debian 12 (bookworm: glibc 2.36, OpenSSL 3),
which every current Debian, Ubuntu LTS, and Raspberry Pi OS meets. Every server binary that leaves
the machine it was built on is built this way.

The script fetches the Debian packages Aspen builds against (the C library, OpenSSL, libpq, and
their dependencies) for the target architecture and unpacks them into a sysroot, which needs no
root and installs nothing outside the build directory. Everything is compiled with clang for the
target and linked with lld: the Rust code and the C in the crates that build C (ring, aws-lc,
jemalloc, libwebp). Each architecture and release builds into a target directory of its own under
`target/cross/`, so a baseline build never rebuilds the usual `target/` or the other way round.

On ARM, jemalloc is built for 64 KiB pages, which also runs on kernels with 4 KiB and 16 KiB pages:
Raspberry Pi OS on a Pi 5 boots a 16 KiB-page kernel by default, where a jemalloc built for 4 KiB
pages aborts at startup.

    scripts/baseline_build.py --arch ARCH            set up if needed, build, and check
    scripts/baseline_build.py --arch ARCH setup      fetch the sysroot and write the toolchain files
    scripts/baseline_build.py --arch ARCH build      build (sets up first if needed)
    scripts/baseline_build.py --arch ARCH check      check the built binaries
    scripts/baseline_build.py --arch ARCH out        print the directory the binaries land in
    scripts/baseline_build.py --arch ARCH env        print the environment, to run cargo by hand:
                                                     eval "$(scripts/baseline_build.py --arch ARCH env)"

ARCH is `x86_64` or `aarch64`. Where the binaries run, they need `apt install libssl3 libpq5`
(present on most installations), and the voice server neither. Needs Python 3.12 or later (3.14 for
a Debian release whose packages are zstd-compressed), clang, lld, llvm-ar, llvm-readelf,
pkg-config, rustup, and network access.

The checks start each built binary against the sysroot's own C library and OpenSSL: through the
sysroot's dynamic loader when the target is this machine's architecture, and otherwise under
Debian's user-mode emulator for it, which setup unpacks. They also refuse a binary that needs a
glibc symbol newer than the release's or a shared library the release does not provide.

`check --dir DIR` checks binaries built some other way, such as natively on an ARM machine in CI;
that needs only Python 3.10 and `readelf`, and starts the binaries directly when no sysroot has
been set up.
"""

from __future__ import annotations

import argparse
import io
import lzma
import os
import platform
import re
import shlex
import shutil
import subprocess
import sys
import tarfile
import urllib.request
from dataclasses import dataclass
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
MIRROR = "https://deb.debian.org/debian"


@dataclass(frozen=True)
class Arch:
    rust_target: str
    triple: str
    # Debian's name for the architecture, in its package indexes.
    debian: str
    # What readelf's file header names the machine.
    elf_machine: str
    # The dynamic loader, in `lib/<triple>/` of the sysroot.
    loader: str
    # jemalloc's page size as a power of two, when the architecture's kernels differ in it.
    jemalloc_lg_page: str | None


ARCHES = {
    "x86_64": Arch(
        rust_target="x86_64-unknown-linux-gnu",
        triple="x86_64-linux-gnu",
        debian="amd64",
        elf_machine="X86-64",
        loader="ld-linux-x86-64.so.2",
        jemalloc_lg_page=None,
    ),
    # 64 KiB, which serves every smaller kernel page.
    "aarch64": Arch(
        rust_target="aarch64-unknown-linux-gnu",
        triple="aarch64-linux-gnu",
        debian="arm64",
        elf_machine="AArch64",
        loader="ld-linux-aarch64.so.1",
        jemalloc_lg_page="16",
    ),
}
# Debian releases, and the glibc each ships: the newest glibc symbol a binary may need.
RELEASES = {"bookworm": (2, 36), "trixie": (2, 41)}
# What Aspen compiles and links against; their dependencies come along.
ROOT_PACKAGES = [
    "libc6-dev",
    "linux-libc-dev",
    "libgcc-12-dev",
    "libssl-dev",
    "libpq-dev",
]
# Trixie's compiler runtime is GCC 14's.
ROOT_PACKAGES_TRIXIE = {"libgcc-12-dev": "libgcc-14-dev"}
# The servers: the chat server, the voice server, the migration runner, and Ultra Lite, whose
# runtime `scripts/package_ul.py` appends.
PACKAGES = ["aspen-chat-server", "voice_server", "aspen-migrate", "aspen-ul"]
BINARIES = ["aspen-chat-server", "voice_server", "aspen-migrate", "aspen-ul"]
# Shared libraries the binaries may need beyond the C runtime, and the Debian package that
# provides each where they run. A binary needing any other library is refused: it was linked
# against something the deployment's machines are not promised to have.
RUNTIME_PACKAGES = {"libssl.so.3": "libssl3", "libcrypto.so.3": "libssl3", "libpq.so.5": "libpq5"}
# The C runtime, which every Linux the release floor admits has.
C_RUNTIME = {
    "libc.so.6",
    "libm.so.6",
    "libgcc_s.so.1",
    "libdl.so.2",
    "libpthread.so.0",
    "librt.so.1",
    *(arch.loader for arch in ARCHES.values()),
}


def host_arch() -> str | None:
    machine = platform.machine()
    return {"x86_64": "x86_64", "amd64": "x86_64", "aarch64": "aarch64", "arm64": "aarch64"}.get(machine)


@dataclass
class Paths:
    arch_name: str
    release: str

    @property
    def arch(self) -> Arch:
        return ARCHES[self.arch_name]

    @property
    def root(self) -> Path:
        return REPO / "target" / "cross" / f"{self.arch_name}-{self.release}"

    @property
    def sysroot(self) -> Path:
        return self.root / "sysroot"

    @property
    def debs(self) -> Path:
        return self.root / "debs"

    @property
    def bin(self) -> Path:
        return self.root / "bin"

    @property
    def tools(self) -> Path:
        return self.root / "tools"

    @property
    def target_dir(self) -> Path:
        return self.root / "target"

    @property
    def ready(self) -> Path:
        return self.root / ".ready"

    def out(self, profile: str) -> Path:
        return self.target_dir / self.arch.rust_target / profile

    @property
    def library_dirs(self) -> list[Path]:
        triple = self.arch.triple
        return [self.sysroot / d for d in (f"lib/{triple}", f"usr/lib/{triple}", "lib", "usr/lib")]

    @property
    def emulator(self) -> Path:
        return self.tools / f"qemu-{self.arch_name}-static"


def say(message: str) -> None:
    print(f"baseline_build: {message}", file=sys.stderr, flush=True)


def fail(message: str) -> None:
    say(message)
    sys.exit(1)


def require_tools() -> None:
    missing = [
        tool
        for tool in ["clang", "ld.lld", "llvm-ar", "llvm-readelf", "pkg-config", "rustup", "cargo"]
        if shutil.which(tool) is None
    ]
    if missing:
        fail(f"missing tools: {', '.join(missing)} (on Arch: pacman -S clang lld llvm pkgconf rustup)")


# --- The sysroot ----------------------------------------------------------------------------


def fetch(url: str, destination: Path) -> Path:
    if destination.exists():
        return destination
    destination.parent.mkdir(parents=True, exist_ok=True)
    partial = destination.with_suffix(destination.suffix + ".part")
    with urllib.request.urlopen(url, timeout=60) as response, open(partial, "wb") as out:
        shutil.copyfileobj(response, out)
    partial.rename(destination)
    return destination


def package_index(paths: Paths, release: str, debian_arch: str) -> dict[str, dict[str, str]]:
    """A release's package index for one architecture, by package name."""
    index_file = fetch(
        f"{MIRROR}/dists/{release}/main/binary-{debian_arch}/Packages.xz",
        paths.debs / f"Packages-{release}-{debian_arch}.xz",
    )
    packages: dict[str, dict[str, str]] = {}
    text = lzma.decompress(index_file.read_bytes()).decode()
    for stanza in text.split("\n\n"):
        fields: dict[str, str] = {}
        key = None
        for line in stanza.splitlines():
            if line.startswith((" ", "\t")) and key:
                fields[key] += "\n" + line
            elif ":" in line:
                key, _, value = line.partition(":")
                fields[key] = value.strip()
        if "Package" in fields:
            packages[fields["Package"]] = fields
    return packages


def dependencies(fields: dict[str, str]) -> list[list[str]]:
    """A package's run and install dependencies, each a list of alternatives."""
    found = []
    for key in ("Pre-Depends", "Depends"):
        for clause in fields.get(key, "").split(","):
            names = [re.sub(r"\s*\(.*?\)|:any|\s*\[.*?\]", "", alt).strip() for alt in clause.split("|")]
            names = [n for n in names if n]
            if names:
                found.append(names)
    return found


def wanted(name: str) -> bool:
    """Whether a dependency belongs in a sysroot: libraries and headers, not tools."""
    return name.startswith("lib") or name in {"linux-libc-dev"} or name.startswith("gcc-") and name.endswith("-base")


def resolve(index: dict[str, dict[str, str]], roots: list[str], paths: Paths) -> list[dict[str, str]]:
    provides: dict[str, str] = {}
    for name, fields in index.items():
        for provided in fields.get("Provides", "").split(","):
            provided = re.sub(r"\s*\(.*?\)", "", provided).strip()
            if provided:
                provides.setdefault(provided, name)
    chosen: dict[str, dict[str, str]] = {}
    queue = list(roots)
    while queue:
        name = queue.pop()
        real = name if name in index else provides.get(name)
        if real is None:
            if name in roots:
                fail(f"{name} is not in Debian {paths.release}'s {paths.arch.debian} index")
            continue
        if real in chosen:
            continue
        chosen[real] = index[real]
        for alternatives in dependencies(index[real]):
            pick = next((a for a in alternatives if a in index or a in provides), None)
            if pick and wanted(pick):
                queue.append(pick)
    return sorted(chosen.values(), key=lambda f: f["Package"])


def deb_members(deb: Path) -> dict[str, bytes]:
    """The members of a .deb, an `ar` archive."""
    data = deb.read_bytes()
    if not data.startswith(b"!<arch>\n"):
        fail(f"{deb.name} is not a Debian package")
    members = {}
    offset = 8
    while offset + 60 <= len(data):
        header = data[offset : offset + 60]
        name = header[:16].decode().strip().rstrip("/")
        size = int(header[48:58].decode().strip())
        offset += 60
        members[name] = data[offset : offset + size]
        offset += size + (size % 2)
    return members


def unpack(deb: Path, sysroot: Path) -> None:
    members = deb_members(deb)
    name, payload = next((n, p) for n, p in members.items() if n.startswith("data.tar"))
    if name.endswith(".xz"):
        payload = lzma.decompress(payload)
    elif name.endswith(".zst"):
        try:
            from compression import zstd  # Python 3.14
        except ImportError:
            fail(f"{deb.name} is zstd-compressed, which needs Python 3.14 to unpack")

        payload = zstd.decompress(payload)
    elif not name.endswith(".tar"):
        fail(f"{deb.name}: unsupported {name}")
    with tarfile.open(fileobj=io.BytesIO(payload)) as archive:
        archive.extractall(sysroot, filter="tar")


def relativise_links(sysroot: Path) -> None:
    """Points absolute symlinks inside the sysroot at the sysroot, not at this machine."""
    for link in sysroot.rglob("*"):
        if link.is_symlink():
            target = os.readlink(link)
            if target.startswith("/"):
                inside = sysroot / target.lstrip("/")
                link.unlink()
                link.symlink_to(os.path.relpath(inside, link.parent))


def make_sysroot(paths: Paths) -> None:
    index = package_index(paths, paths.release, paths.arch.debian)
    roots = [ROOT_PACKAGES_TRIXIE.get(p, p) if paths.release == "trixie" else p for p in ROOT_PACKAGES]
    chosen = resolve(index, roots, paths)
    say(f"fetching {len(chosen)} Debian {paths.release} {paths.arch.debian} packages")
    if paths.sysroot.exists():
        shutil.rmtree(paths.sysroot)
    paths.sysroot.mkdir(parents=True)
    for fields in chosen:
        filename = fields["Filename"]
        deb = fetch(f"{MIRROR}/{filename}", paths.debs / Path(filename).name)
        unpack(deb, paths.sysroot)
    relativise_links(paths.sysroot)


# --- The toolchain --------------------------------------------------------------------------


def target_flags(paths: Paths) -> list[str]:
    """What makes clang compile for the target, against the sysroot."""
    return [f"--target={paths.arch.triple}", f"--sysroot={paths.sysroot}"]


def wrapper_flags(paths: Paths) -> list[str]:
    """The target flags, and how the wrappers cargo calls link: `--ld-path` names this
    machine's lld outright, because rustc puts its own bundled lld first on the search path
    (`-B .../gcc-ld`), and that one links for the build machine. Compiling alone does not use
    it, which clang would otherwise warn about on every C file a crate builds."""
    return [*target_flags(paths), f"--ld-path={shutil.which('ld.lld')}", "-Wno-unused-command-line-argument"]


# --- An emulator for the target -------------------------------------------------------------


def unpack_emulator(paths: Paths) -> None:
    """Debian's statically linked user-mode emulator for the target, for this machine's
    architecture, unpacked into the build directory when the two differ. The checks start the
    built binaries under it. It is called by path, not registered with the kernel, so it needs
    no root.
    """
    host = host_arch()
    if host == paths.arch_name or paths.emulator.exists():
        return
    if host is None:
        fail(f"no emulator for {paths.arch_name} on a {platform.machine()} machine")
    # The emulator is static, so bookworm's runs on any newer system.
    index = package_index(paths, "bookworm", ARCHES[host].debian)
    filename = index["qemu-user-static"]["Filename"]
    deb = fetch(f"{MIRROR}/{filename}", paths.debs / Path(filename).name)
    staging = paths.tools / "qemu-staging"
    unpack(deb, staging)
    paths.tools.mkdir(parents=True, exist_ok=True)
    shutil.copy2(staging / "usr" / "bin" / paths.emulator.name, paths.emulator)
    shutil.rmtree(staging)


def write_toolchain(paths: Paths) -> None:
    paths.bin.mkdir(parents=True, exist_ok=True)
    unpack_emulator(paths)
    flags = " ".join(shlex.quote(f) for f in wrapper_flags(paths))
    wrapper = paths.bin / f"{paths.arch.triple}-cc"
    wrapper.write_text(f'#!/bin/sh\nexec clang {flags} "$@"\n')
    wrapper.chmod(0o755)


def build_env(paths: Paths) -> dict[str, str]:
    """What cargo and the crates' build scripts read.

    When the target is this machine's architecture, cargo links the build scripts and proc macros
    with the target's linker too, and their C is compiled with the target's compiler, both against
    the sysroot; they then run on this machine, whose glibc is newer than the sysroot's.
    """
    triple = paths.arch.triple
    target = paths.arch.rust_target.replace("-", "_")
    pkg_config_libdir = f"{paths.sysroot}/usr/lib/{triple}/pkgconfig:{paths.sysroot}/usr/share/pkgconfig"
    cc = str(paths.bin / f"{triple}-cc")
    env = {
        "CARGO_TARGET_DIR": str(paths.target_dir),
        f"CARGO_TARGET_{target.upper()}_LINKER": cc,
        f"CC_{target}": cc,
        f"AR_{target}": "llvm-ar",
        # Build scripts compiling for this machine read HOST_*. When the target is this machine's
        # architecture, some (aws-lc-sys's) read it for the target's C as well, so it compiles
        # against the sysroot too.
        "HOST_CC": cc if host_arch() == paths.arch_name else "clang",
        "HOST_AR": "llvm-ar",
        # Anything that asks pkg-config is answered from the sysroot.
        f"PKG_CONFIG_SYSROOT_DIR_{target}": str(paths.sysroot),
        f"PKG_CONFIG_LIBDIR_{target}": pkg_config_libdir,
        f"PKG_CONFIG_ALLOW_CROSS_{target}": "1",
        f"{target.upper()}_OPENSSL_LIB_DIR": f"{paths.sysroot}/usr/lib/{triple}",
        f"{target.upper()}_OPENSSL_INCLUDE_DIR": f"{paths.sysroot}/usr/include",
        # pq-sys, built without its pkg-config feature, otherwise asks this machine's pg_config
        # and links against this machine's libraries.
        f"PQ_LIB_DIR_{target.upper()}": f"{paths.sysroot}/usr/lib/{triple}",
    }
    if paths.arch.jemalloc_lg_page:
        env["JEMALLOC_SYS_WITH_LG_PAGE"] = paths.arch.jemalloc_lg_page
    return env


def setup(paths: Paths) -> None:
    require_tools()
    # Run from the checkout so the target goes to the toolchain rust-toolchain.toml pins.
    subprocess.run(["rustup", "target", "add", paths.arch.rust_target], cwd=REPO, check=True)
    make_sysroot(paths)
    write_toolchain(paths)
    paths.ready.write_text("")
    say(f"ready: sysroot and toolchain in {paths.root.relative_to(REPO)}")


def ensure_setup(paths: Paths) -> None:
    if not paths.ready.exists():
        setup(paths)
    else:
        # The wrappers name absolute paths; rewrite them in case the checkout moved.
        write_toolchain(paths)


# --- Building and checking ------------------------------------------------------------------


def build(paths: Paths, profile: str, jobs: int | None) -> None:
    ensure_setup(paths)
    command = ["cargo", "build", "--locked", "--target", paths.arch.rust_target]
    if profile == "release":
        command.append("--release")
    for package in PACKAGES:
        command += ["-p", package]
    if jobs:
        command += ["-j", str(jobs)]
    env = {**os.environ, **build_env(paths)}
    say("building: " + " ".join(command))
    subprocess.run(command, cwd=REPO, env=env, check=True)


def readelf(binary: Path, *flags: str) -> str:
    tool = shutil.which("llvm-readelf") or shutil.which("readelf")
    if tool is None:
        fail("neither llvm-readelf nor readelf is installed")
    return subprocess.run([tool, "--wide", *flags, str(binary)], capture_output=True, text=True, check=True).stdout


def in_sysroot(paths: Paths, library: str) -> bool:
    return any((directory / library).exists() for directory in paths.library_dirs)


def check(paths: Paths, profile: str, directory: Path | None = None) -> None:
    out = directory or paths.out(profile)
    newest_allowed = RELEASES[paths.release]
    has_sysroot = paths.ready.exists()
    problems = []
    needed_packages: set[str] = set()
    for name in BINARIES:
        binary = out / name
        if not binary.exists():
            problems.append(f"{name}: not built ({binary})")
            continue
        header = readelf(binary, "--file-header")
        machine = re.search(r"^\s*Machine:\s*(.+)$", header, re.M)
        if machine is None or not machine.group(1).strip().endswith(paths.arch.elf_machine):
            problems.append(f"{name}: not an {paths.arch.elf_machine} binary")
        versions = {
            tuple(int(p) for p in m.split("."))
            for m in re.findall(r"GLIBC_(\d+\.\d+(?:\.\d+)?)", readelf(binary, "--version-info"))
        }
        newest = max(versions) if versions else (0, 0)
        if newest[:2] > newest_allowed:
            problems.append(
                f"{name}: needs glibc {'.'.join(map(str, newest))}, newer than {paths.release}'s "
                f"{'.'.join(map(str, newest_allowed))}"
            )
        needed = re.findall(r"\(NEEDED\).*\[(.+?)\]", readelf(binary, "--dynamic"))
        for library in needed:
            if library in RUNTIME_PACKAGES:
                needed_packages.add(RUNTIME_PACKAGES[library])
            elif library not in C_RUNTIME:
                problems.append(f"{name}: needs {library}, which the servers are not built to need")
                continue
            if has_sysroot and not in_sysroot(paths, library):
                problems.append(f"{name}: needs {library}, which Debian {paths.release} does not provide")
        size = binary.stat().st_size / (1024 * 1024)
        say(
            f"{name}: {paths.arch.elf_machine}, needs glibc {'.'.join(map(str, newest))}, "
            f"{size:.0f} MiB, links {', '.join(needed)}"
        )
    if not problems:
        problems += run_checks(paths, out)
    if problems:
        for problem in problems:
            say(problem)
        sys.exit(1)
    say(f"binaries in {out}; they run on Debian {paths.release} or newer")
    if needed_packages:
        say(f"where they run: sudo apt install {' '.join(sorted(needed_packages))}")


def runner(paths: Paths) -> tuple[list[str], str] | None:
    """How to run a target binary here, and what that proves: against the sysroot's own C
    library and OpenSSL, through its loader or under the emulator; directly, when this machine
    is the target's architecture and has no sysroot; or not at all."""
    native = host_arch() == paths.arch_name
    if paths.ready.exists():
        if native:
            loader = paths.sysroot / "lib" / paths.arch.triple / paths.arch.loader
            library_path = ":".join(str(d) for d in paths.library_dirs)
            # The loader reads this machine's /etc/ld.so.cache unless told not to, which would
            # find this machine's libraries before the sysroot's.
            return [str(loader), "--inhibit-cache", "--library-path", library_path], "against the sysroot"
        if paths.emulator.exists():
            return [str(paths.emulator), "-L", str(paths.sysroot)], "under emulation, against the sysroot"
    if native:
        return [], "on this machine"
    return None


def run_checks(paths: Paths, out: Path) -> list[str]:
    """Starts each binary, and checks jemalloc runs with the page size the architecture needs."""
    found = runner(paths)
    if found is None:
        say(f"not starting the binaries: no emulator here (run setup) and this is not an {paths.arch_name} machine")
        return []
    prefix, how = found
    problems = []
    for name in BINARIES:
        started = subprocess.run([*prefix, str(out / name), "--help"], capture_output=True, text=True)
        if started.returncode != 0:
            problems.append(f"{name}: does not start: {started.stderr.strip()[-300:]}")
    lg_page = paths.arch.jemalloc_lg_page
    if lg_page:
        stats = subprocess.run(
            [*prefix, str(out / "voice_server"), "--help"],
            capture_output=True,
            text=True,
            env={**os.environ, "MALLOC_CONF": "stats_print:true"},
        )
        page = re.search(r"Page size: (\d+)", stats.stderr + stats.stdout)
        expected = 1 << int(lg_page)
        if page is None or int(page.group(1)) != expected:
            problems.append(
                f"jemalloc runs with {page.group(1) if page else 'unknown'}-byte pages, not {expected}; "
                f"build with JEMALLOC_SYS_WITH_LG_PAGE={lg_page}"
            )
        how += f"; jemalloc uses {expected // 1024} KiB pages"
    if not problems:
        say(f"all start {how}")
    return problems


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Build Aspen's servers for 64-bit Linux against a Debian sysroot.",
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    parser.add_argument(
        "command", nargs="?", default="all", choices=["all", "setup", "build", "check", "out", "env"]
    )
    parser.add_argument("--arch", required=True, choices=sorted(ARCHES), help="the architecture to build for")
    parser.add_argument(
        "--release-of",
        dest="release",
        default="bookworm",
        choices=sorted(RELEASES),
        help="the oldest Debian release the binaries must run on (default: bookworm)",
    )
    parser.add_argument("--debug", action="store_true", help="build without optimisation")
    parser.add_argument("-j", "--jobs", type=int, help="parallel jobs for cargo")
    parser.add_argument("--dir", type=Path, help="check: the directory holding the binaries")
    args = parser.parse_args()
    paths = Paths(args.arch, args.release)
    profile = "debug" if args.debug else "release"
    match args.command:
        case "setup":
            setup(paths)
        case "build":
            build(paths, profile, args.jobs)
        case "check":
            check(paths, profile, args.dir)
        case "out":
            print(paths.out(profile))
        case "env":
            ensure_setup(paths)
            for key, value in build_env(paths).items():
                print(f"export {key}={shlex.quote(value)}")
        case "all":
            build(paths, profile, args.jobs)
            check(paths, profile)


if __name__ == "__main__":
    main()
