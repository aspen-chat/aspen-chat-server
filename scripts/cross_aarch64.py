#!/usr/bin/env python3
"""Cross-compiles Aspen's servers for 64-bit ARM Linux (a Raspberry Pi 5, say) on an x86-64 Linux
machine, with the clang and lld already installed there.

The binaries are linked against a Debian sysroot, by default Debian 12 (bookworm, glibc 2.36),
so they run on Raspberry Pi OS Bookworm and anything newer; a toolchain that links against the
build machine's own, newer glibc makes binaries an older Pi refuses to start. The script fetches
the Debian arm64 packages Aspen builds against (the C library, libstdc++, OpenSSL, libpq, and
their dependencies) and unpacks them into a sysroot, which needs no root and installs nothing
outside the build directory. Everything is compiled with clang for the target and linked with
lld: the Rust code, the C in the crates that build C (ring, aws-lc, jemalloc), and mediasoup's
C++ worker, through a Meson cross file.

jemalloc is built for 64 KiB pages, which also runs on kernels with 4 KiB and 16 KiB pages:
Raspberry Pi OS on a Pi 5 boots a 16 KiB-page kernel by default, where a jemalloc built for
4 KiB pages aborts at startup.

    scripts/cross_aarch64.py            set up if needed, build, and check the binaries
    scripts/cross_aarch64.py setup      fetch the sysroot and write the toolchain files
    scripts/cross_aarch64.py build      build (sets up first if needed)
    scripts/cross_aarch64.py check      check the built binaries
    scripts/cross_aarch64.py env        print the environment, to run cargo by hand:
                                        eval "$(scripts/cross_aarch64.py env)"

On the Pi, the binaries need `apt install libssl3 libpq5` (present on most installations).
Needs Python 3.12 or later (3.14 for a Debian release whose packages are zstd-compressed),
clang, lld, llvm-ar, llvm-readelf, pkg-config, rustup,
and network access, both here and during the build (mediasoup fetches its C++ dependencies).
mediasoup builds a code generator for the target and runs it while it builds, so the script also
unpacks Debian's user-mode arm64 emulator, which Meson runs target programs through; the checks
start each built binary under it too.

`check --dir DIR` checks binaries built some other way, such as natively on an ARM machine in
CI; that needs only Python 3.10 and `readelf`.
"""

from __future__ import annotations

import argparse
import io
import lzma
import os
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
RUST_TARGET = "aarch64-unknown-linux-gnu"
TRIPLE = "aarch64-linux-gnu"
MIRROR = "https://deb.debian.org/debian"
# Debian releases, and the glibc each ships: the newest glibc symbol a binary may need.
RELEASES = {"bookworm": (2, 36), "trixie": (2, 41)}
# What Aspen compiles and links against; their dependencies come along.
ROOT_PACKAGES = [
    "libc6-dev",
    "linux-libc-dev",
    "libgcc-12-dev",
    "libstdc++-12-dev",
    "libssl-dev",
    "libpq-dev",
]
# Trixie's compiler runtime is GCC 14's.
ROOT_PACKAGES_TRIXIE = {"libgcc-12-dev": "libgcc-14-dev", "libstdc++-12-dev": "libstdc++-14-dev"}
# The binaries a Pi needs: the chat server, the voice server, and the migration runner.
PACKAGES = ["aspen-chat-server", "voice_server", "aspen-migrate"]
BINARIES = ["aspen-chat-server", "voice_server", "aspen-migrate"]
# jemalloc's page size, as a power of two: 64 KiB, which serves every smaller kernel page.
JEMALLOC_LG_PAGE = "16"
# Shared libraries the binaries may need beyond the C and C++ runtimes, and the Debian
# package that provides each on the Pi.
RUNTIME_PACKAGES = {"libssl.so.3": "libssl3", "libcrypto.so.3": "libssl3", "libpq.so.5": "libpq5"}


@dataclass
class Paths:
    release: str

    @property
    def root(self) -> Path:
        return REPO / "target" / "cross" / f"aarch64-{self.release}"

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
    def meson_cross_file(self) -> Path:
        return self.root / "meson-cross.ini"

    @property
    def tools(self) -> Path:
        return self.root / "tools"

    @property
    def ready(self) -> Path:
        return self.root / ".ready"


def say(message: str) -> None:
    print(f"cross_aarch64: {message}", file=sys.stderr, flush=True)


def fail(message: str) -> None:
    say(message)
    sys.exit(1)


def require_tools() -> None:
    missing = [
        tool
        for tool in ["clang", "clang++", "ld.lld", "llvm-ar", "llvm-readelf", "pkg-config", "rustup", "cargo"]
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


def package_index(paths: Paths) -> dict[str, dict[str, str]]:
    """The release's arm64 package index, by package name."""
    index_file = fetch(
        f"{MIRROR}/dists/{paths.release}/main/binary-arm64/Packages.xz",
        paths.debs / "Packages.xz",
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


def resolve(index: dict[str, dict[str, str]], roots: list[str], release: str) -> list[dict[str, str]]:
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
                fail(f"{name} is not in Debian {release}'s arm64 index")
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
    index = package_index(paths)
    roots = [ROOT_PACKAGES_TRIXIE.get(p, p) if paths.release == "trixie" else p for p in ROOT_PACKAGES]
    chosen = resolve(index, roots, paths.release)
    say(f"fetching {len(chosen)} Debian {paths.release} arm64 packages")
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
    return [f"--target={TRIPLE}", f"--sysroot={paths.sysroot}"]


def wrapper_flags(paths: Paths) -> list[str]:
    """The target flags, and how the wrappers cargo calls link: `--ld-path` names this
    machine's lld outright, because rustc puts its own bundled lld first on the search path
    (`-B .../gcc-ld`), and that one links for the build machine. Compiling alone does not use
    it, which clang would otherwise warn about on every C file a crate builds."""
    return [*target_flags(paths), f"--ld-path={shutil.which('ld.lld')}", "-Wno-unused-command-line-argument"]


# --- An emulator for the target -------------------------------------------------------------


def qemu(paths: Paths) -> Path:
    """Debian's statically linked user-mode emulator for arm64, unpacked into the build directory.

    Meson runs what it builds for the target through it: its compiler checks, and mediasoup's
    FlatBuffers code generator, which mediasoup builds from source for the target and runs during
    the build. It is called by path, not registered with the kernel, so it needs no root.
    """
    emulator = paths.tools / "qemu-aarch64-static"
    if emulator.exists():
        return emulator
    index_file = fetch(
        f"{MIRROR}/dists/bookworm/main/binary-amd64/Packages.xz", paths.debs / "Packages-amd64.xz"
    )
    text = lzma.decompress(index_file.read_bytes()).decode()
    stanza = next(s for s in text.split("\n\n") if s.startswith("Package: qemu-user-static\n"))
    filename = re.search(r"^Filename: (.+)$", stanza, re.M).group(1)
    deb = fetch(f"{MIRROR}/{filename}", paths.debs / Path(filename).name)
    staging = paths.tools / "qemu-staging"
    unpack(deb, staging)
    paths.tools.mkdir(parents=True, exist_ok=True)
    shutil.copy2(staging / "usr" / "bin" / "qemu-aarch64-static", emulator)
    shutil.rmtree(staging)
    return emulator


def write_toolchain(paths: Paths) -> None:
    paths.bin.mkdir(parents=True, exist_ok=True)
    emulator = qemu(paths)
    flags = " ".join(shlex.quote(f) for f in wrapper_flags(paths))
    for name, compiler in [("cc", "clang"), ("c++", "clang++")]:
        wrapper = paths.bin / f"{TRIPLE}-{name}"
        wrapper.write_text(f'#!/bin/sh\nexec {compiler} {flags} "$@"\n')
        wrapper.chmod(0o755)
    quoted = lambda items: "[" + ", ".join(repr(i) for i in items) + "]"
    paths.meson_cross_file.write_text(
        "[binaries]\n"
        # Meson picks lld by `c_ld`, when it links; flags given here are also used for its
        # compile-only checks, which it runs with unused arguments as errors.
        f"c = {quoted(['clang', *target_flags(paths)])}\n"
        f"cpp = {quoted(['clang++', *target_flags(paths)])}\n"
        "c_ld = 'lld'\n"
        "cpp_ld = 'lld'\n"
        "ar = 'llvm-ar'\n"
        "strip = 'llvm-strip'\n"
        "pkg-config = 'pkg-config'\n"
        f"exe_wrapper = {quoted([str(emulator), '-L', str(paths.sysroot)])}\n"
        "\n[properties]\n"
        f"sys_root = '{paths.sysroot}'\n"
        f"pkg_config_libdir = '{paths.sysroot}/usr/lib/{TRIPLE}/pkgconfig:{paths.sysroot}/usr/share/pkgconfig'\n"
        "\n[host_machine]\n"
        "system = 'linux'\n"
        "cpu_family = 'aarch64'\n"
        "cpu = 'aarch64'\n"
        "endian = 'little'\n"
    )


def build_env(paths: Paths) -> dict[str, str]:
    """What cargo, the crates' build scripts, and mediasoup's Meson build read."""
    target = RUST_TARGET.replace("-", "_")
    pkg_config_libdir = f"{paths.sysroot}/usr/lib/{TRIPLE}/pkgconfig:{paths.sysroot}/usr/share/pkgconfig"
    cc = str(paths.bin / f"{TRIPLE}-cc")
    cxx = str(paths.bin / f"{TRIPLE}-c++")
    return {
        f"CARGO_TARGET_{target.upper()}_LINKER": cc,
        f"CC_{target}": cc,
        f"CXX_{target}": cxx,
        f"AR_{target}": "llvm-ar",
        # mediasoup-sys links libstdc++ statically, from wherever plain `CXX` says it is, so
        # that must be the target's; build scripts compiling for this machine read HOST_*.
        "CXX": cxx,
        "HOST_CC": "clang",
        "HOST_CXX": "clang++",
        # Anything that asks pkg-config is answered from the sysroot.
        f"PKG_CONFIG_SYSROOT_DIR_{target}": str(paths.sysroot),
        f"PKG_CONFIG_LIBDIR_{target}": pkg_config_libdir,
        f"PKG_CONFIG_ALLOW_CROSS_{target}": "1",
        f"{target.upper()}_OPENSSL_LIB_DIR": f"{paths.sysroot}/usr/lib/{TRIPLE}",
        f"{target.upper()}_OPENSSL_INCLUDE_DIR": f"{paths.sysroot}/usr/include",
        # pq-sys, built without its pkg-config feature, otherwise asks this machine's pg_config
        # and links against this machine's libraries.
        f"PQ_LIB_DIR_{target.upper()}": f"{paths.sysroot}/usr/lib/{TRIPLE}",
        "JEMALLOC_SYS_WITH_LG_PAGE": JEMALLOC_LG_PAGE,
        # mediasoup's worker: Meson compiles it for the target with the cross file, and runs
        # the code generator it builds on the way through the emulator the cross file names.
        "MESON_ARGS": f"--cross-file={paths.meson_cross_file}",
    }


def setup(paths: Paths) -> None:
    require_tools()
    # Run from the checkout so the target goes to the toolchain rust-toolchain.toml pins.
    subprocess.run(["rustup", "target", "add", RUST_TARGET], cwd=REPO, check=True)
    make_sysroot(paths)
    write_toolchain(paths)
    paths.ready.write_text("")
    say(f"ready: sysroot and toolchain in {paths.root.relative_to(REPO)}")


def ensure_setup(paths: Paths) -> None:
    if not paths.ready.exists():
        setup(paths)
    else:
        # The wrappers and cross file name absolute paths; rewrite them in case the checkout moved.
        write_toolchain(paths)


# --- Building and checking ------------------------------------------------------------------


def build(paths: Paths, profile: str, jobs: int | None) -> None:
    ensure_setup(paths)
    command = ["cargo", "build", "--target", RUST_TARGET]
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


def check(paths: Paths, profile: str, directory: Path | None = None) -> None:
    out = directory or REPO / "target" / RUST_TARGET / profile
    newest_allowed = RELEASES[paths.release]
    problems = []
    needed_packages: set[str] = set()
    for name in BINARIES:
        binary = out / name
        if not binary.exists():
            problems.append(f"{name}: not built ({binary})")
            continue
        header = readelf(binary, "--file-header")
        if "AArch64" not in header:
            problems.append(f"{name}: not an AArch64 binary")
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
        needed_packages |= {RUNTIME_PACKAGES[n] for n in needed if n in RUNTIME_PACKAGES}
        size = binary.stat().st_size / (1024 * 1024)
        say(
            f"{name}: AArch64, needs glibc {'.'.join(map(str, newest))}, "
            f"{size:.0f} MiB, links {', '.join(needed)}"
        )
    problems += run_checks(paths, out)
    if problems:
        for problem in problems:
            say(problem)
        sys.exit(1)
    say(f"binaries in {out}; they run on Debian {paths.release} or newer")
    if needed_packages:
        say(f"on the Pi: sudo apt install {' '.join(sorted(needed_packages))}")


def runner(paths: Paths) -> list[str] | None:
    """How to run a target binary here: directly on an ARM machine, through the emulator on
    this one when setup has unpacked it, or not at all."""
    import platform

    if platform.machine() in ("aarch64", "arm64"):
        return []
    emulator = paths.tools / "qemu-aarch64-static"
    if emulator.exists():
        return [str(emulator), "-L", str(paths.sysroot)]
    return None


def run_checks(paths: Paths, out: Path) -> list[str]:
    """Starts each binary, and checks jemalloc runs with 64 KiB pages, which also serve a
    kernel with 4 KiB or 16 KiB pages."""
    prefix = runner(paths)
    if prefix is None:
        say("not starting the binaries: no emulator here (run setup) and this is not an ARM machine")
        return []
    problems = []
    for name in BINARIES:
        started = subprocess.run([*prefix, str(out / name), "--help"], capture_output=True, text=True)
        if started.returncode != 0:
            problems.append(f"{name}: does not start: {started.stderr.strip()[-300:]}")
    stats = subprocess.run(
        [*prefix, str(out / "voice_server"), "--help"],
        capture_output=True,
        text=True,
        env={**os.environ, "MALLOC_CONF": "stats_print:true"},
    )
    page = re.search(r"Page size: (\d+)", stats.stderr + stats.stdout)
    expected = 1 << int(JEMALLOC_LG_PAGE)
    if page is None or int(page.group(1)) != expected:
        problems.append(
            f"jemalloc runs with {page.group(1) if page else 'unknown'}-byte pages, not {expected}; "
            "build with JEMALLOC_SYS_WITH_LG_PAGE=16"
        )
    if not problems:
        say(f"all start{' under emulation' if prefix else ''}; jemalloc uses {expected // 1024} KiB pages")
    return problems


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Cross-compile Aspen's servers for 64-bit ARM Linux.",
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    parser.add_argument("command", nargs="?", default="all", choices=["all", "setup", "build", "check", "env"])
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
    paths = Paths(args.release)
    profile = "debug" if args.debug else "release"
    match args.command:
        case "setup":
            setup(paths)
        case "build":
            build(paths, profile, args.jobs)
        case "check":
            check(paths, profile, args.dir)
        case "env":
            ensure_setup(paths)
            for key, value in build_env(paths).items():
                print(f"export {key}={shlex.quote(value)}")
        case "all":
            build(paths, profile, args.jobs)
            check(paths, profile)


if __name__ == "__main__":
    main()
