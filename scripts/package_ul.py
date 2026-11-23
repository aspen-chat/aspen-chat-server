#!/usr/bin/env python3
"""Builds aspen-ul as it is released: one file, the binary with its runtime appended.

    scripts/package_ul.py --arch x86_64        # or --arch aarch64
    scripts/package_ul.py --arch x86_64 --binary target/debug/aspen-ul --out aspen-ul
    scripts/package_ul.py --arch x86_64 --postgres-only   # prints where it is

The runtime is the bundled PostgreSQL (`pgsql/`) and the built web client (`web-client/`), a
zstd-compressed tar the binary unpacks into its data directory on first start (`server/ul/src/
bundle.rs`, whose trailer this writes: the archive's SHA-256, its length as a little-endian u64,
and the magic). PostgreSQL is built from source against the Debian 12 sysroot of
`scripts/baseline_build.py`, so that it runs wherever the binary does, with only the extensions
the migrations create, and pruned to what running, dumping, restoring, and `pg_upgrade` need; it
is built once per version and architecture into `target/ul/<arch>/`, which `--postgres-only`
builds alone, for running a build of aspen-ul with `--runtime` (`scripts/stack.py`'s `UlStack`
does). The binary is built by
`baseline_build.py` unless `--binary` names one. The web client must already be built
(`pnpm build` in `client/`).

It needs what `baseline_build.py` needs, and `make`, `patchelf`, and `zstd`.
"""

import argparse
import hashlib
import io
import json
import re
import shutil
import struct
import subprocess
import sys
import tarfile
import urllib.request
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import baseline_build as baseline  # noqa: E402

# The PostgreSQL a release bundles. A runtime of a newer major upgrades the cluster an older one
# made (server/ul/src/postgres.rs).
POSTGRES_VERSION = "18.7"
# The extensions the migrations create.
EXTENSIONS = ["pg_trgm", "btree_gin"]
# What running, dumping, restoring, and pg_upgrade need (pg_upgrade checks for each of these).
KEEP_BIN = {
    "postgres", "initdb", "pg_ctl", "pg_dump", "pg_dumpall", "pg_restore", "pg_upgrade",
    "pg_resetwal", "pg_controldata", "psql", "vacuumdb", "pg_isready",
}  # fmt: skip
# `server/ul/src/bundle.rs`'s `MAGIC`.
MAGIC = b"ASPENUL\x01"
WEB_CLIENT = baseline.REPO / "client" / "packages" / "app" / "dist"
UL = baseline.REPO / "target" / "ul"


def say(message: str) -> None:
    print(f"package_ul: {message}", flush=True)


def run(command, log, **kwargs) -> None:
    print("+", " ".join(str(c) for c in command), file=log, flush=True)
    subprocess.run([str(c) for c in command], check=True, stdout=log, stderr=subprocess.STDOUT, **kwargs)


def postgres_source(version: str) -> Path:
    url = f"https://ftp.postgresql.org/pub/source/v{version}/postgresql-{version}.tar.bz2"
    archive = UL / "src" / f"postgresql-{version}.tar.bz2"
    archive.parent.mkdir(parents=True, exist_ok=True)
    if not archive.exists():
        say(f"fetching {url}")
        urllib.request.urlretrieve(url, archive)
    with urllib.request.urlopen(f"{url}.sha256") as answer:
        expected = answer.read().decode().split()[0]
    actual = hashlib.sha256(archive.read_bytes()).hexdigest()
    if actual != expected:
        archive.unlink()
        baseline.fail(f"{archive.name}: checksum {actual}, expected {expected}")
    return archive


def static_zlib(paths: baseline.Paths, deps: Path) -> None:
    """Debian's static zlib alone, so no machine needs libz and pg_dump still compresses."""
    index = baseline.package_index(paths, paths.release, paths.arch.debian)
    filename = index["zlib1g-dev"]["Filename"]
    deb = baseline.fetch(f"{baseline.MIRROR}/{filename}", paths.debs / Path(filename).name)
    baseline.unpack(deb, deps)
    (deps / "usr/lib" / paths.arch.triple / "libz.so").unlink(missing_ok=True)


def prune(prefix: Path) -> None:
    for binary in (prefix / "bin").iterdir():
        if binary.name not in KEEP_BIN:
            binary.unlink()
    shutil.rmtree(prefix / "include")
    for directory in ("share/doc", "share/man", "lib/pkgconfig", "lib/pgxs"):
        shutil.rmtree(prefix / directory, ignore_errors=True)
    for path in list((prefix / "lib").iterdir()):
        if path.suffix == ".a" or path.name.startswith(("libecpg", "libpgtypes")):
            path.unlink()


def check_postgres(prefix: Path, paths: baseline.Paths) -> None:
    """Refuses a build needing a library the machine may lack, or a glibc newer than the sysroot's."""
    allowed = baseline.C_RUNTIME | {p.name for p in (prefix / "lib").glob("*.so*")}
    newest_allowed = baseline.RELEASES[paths.release]
    for path in [*(prefix / "bin").iterdir(), *(prefix / "lib").rglob("*.so*")]:
        if path.is_symlink() or not path.is_file():
            continue
        needed = set(re.findall(r"Shared library: \[([^\]]+)\]", baseline.readelf(path, "-d")))
        if needed - allowed:
            baseline.fail(f"{path} needs {sorted(needed - allowed)}")
        for major, minor in re.findall(r"GLIBC_(\d+)\.(\d+)", baseline.readelf(path, "--dyn-symbols")):
            if (int(major), int(minor)) > newest_allowed:
                baseline.fail(f"{path} needs glibc {major}.{minor}")


def postgres_prefix(arch: str, version: str) -> Path:
    """Where the pruned installation of `version` for `arch` is built."""
    return UL / arch / f"postgresql-{version}" / "pgsql"


def build_postgres(arch: str, version: str, jobs: int) -> Path:
    """The pruned installation of `version` for `arch`, built if it is not yet."""
    paths = baseline.Paths(arch, "bookworm")
    prefix = postgres_prefix(arch, version)
    root = prefix.parent
    done = root / ".built"
    if done.exists():
        return prefix
    baseline.ensure_setup(paths)
    # A cross build installs time zone data with the build machine's zic.
    native_zic = None
    if arch != "x86_64":
        build_postgres("x86_64", version, jobs)
        native_zic = UL / "x86_64" / f"postgresql-{version}" / "build" / "src/timezone/zic"
    say(f"building PostgreSQL {version} for {arch}")
    shutil.rmtree(root, ignore_errors=True)
    root.mkdir(parents=True)
    deps = root / "deps"
    static_zlib(paths, deps)
    with tarfile.open(postgres_source(version)) as tar:
        tar.extractall(root, filter="tar")
    tree = root / "build"
    (root / f"postgresql-{version}").rename(tree)
    triple = paths.arch.triple
    configure = [
        "./configure", f"--prefix={prefix}", f"--host={triple}", "--build=x86_64-linux-gnu",
        "--without-icu", "--without-readline", "--without-llvm", "--with-zlib", "--disable-rpath",
        "--disable-nls", f"--with-includes={deps}/usr/include", f"--with-libraries={deps}/usr/lib/{triple}",
        f"CC={paths.bin}/{triple}-cc", "AR=llvm-ar", "RANLIB=llvm-ranlib", "STRIP=llvm-strip",
        "CFLAGS=-O2 -g0", "PKG_CONFIG=false",
    ]  # fmt: skip
    zic = [f"ZIC={native_zic}"] if native_zic else []
    # configure cannot try this flag when cross compiling, and without it the server exports
    # nothing to the extensions it loads.
    exported = ["LDFLAGS_EX_BE=-Wl,--export-dynamic"]
    log_path = root / "build.log"
    with log_path.open("w") as log:
        try:
            run(configure, log, cwd=tree)
            run(["make", f"-j{jobs}", "-C", "src", "all", *exported], log, cwd=tree)
            for extension in EXTENSIONS:
                run(["make", f"-j{jobs}", "-C", f"contrib/{extension}", "all"], log, cwd=tree)
            run(["make", "-C", "src", "install", *zic, *exported], log, cwd=tree)
            for extension in EXTENSIONS:
                run(["make", "-C", f"contrib/{extension}", "install"], log, cwd=tree)
            prune(prefix)
            for binary in (prefix / "bin").iterdir():
                run(["llvm-strip", "--strip-unneeded", binary], log)
                run(["patchelf", "--set-rpath", "$ORIGIN/../lib", binary], log)
            for library in (prefix / "lib").rglob("*.so*"):
                if library.is_file() and not library.is_symlink():
                    run(["llvm-strip", "--strip-unneeded", library], log)
        except subprocess.CalledProcessError:
            baseline.fail(f"building PostgreSQL failed; {log_path} says why")
    check_postgres(prefix, paths)
    done.touch()
    return prefix


def add_tree(tar: tarfile.TarFile, source: Path, name: str) -> None:
    """Adds `source` as `name`, in a fixed order with fixed owners and times, so the same inputs
    make the same archive and so the same runtime directory."""
    paths = [source, *sorted(source.rglob("*"))]
    for path in paths:
        info = tar.gettarinfo(path, arcname=str(Path(name) / path.relative_to(source)))
        info.uid = info.gid = 0
        info.uname = info.gname = ""
        info.mtime = 0
        info.mode &= 0o755
        if info.isreg():
            with path.open("rb") as file:
                tar.addfile(info, file)
        else:
            tar.addfile(info)


def pack(binary: Path, postgres: Path, web_client: Path, out: Path) -> None:
    if not (web_client / "index.html").is_file():
        baseline.fail(f"{web_client} holds no built web client: run pnpm build in client/")
    plain = io.BytesIO()
    with tarfile.open(fileobj=plain, mode="w", format=tarfile.PAX_FORMAT) as tar:
        add_tree(tar, postgres, "pgsql")
        add_tree(tar, web_client, "web-client")
    say(f"compressing the runtime ({plain.tell() // 2**20} MiB)")
    archive = subprocess.run(
        ["zstd", "-q", "-19", "--long=27", "-T0", "-c"], input=plain.getvalue(), capture_output=True, check=True
    ).stdout
    out.parent.mkdir(parents=True, exist_ok=True)
    partial = out.with_name(out.name + ".partial")
    with partial.open("wb") as file:
        file.write(binary.read_bytes())
        file.write(archive)
        file.write(hashlib.sha256(archive).digest())
        file.write(struct.pack("<Q", len(archive)))
        file.write(MAGIC)
    partial.chmod(0o755)
    partial.replace(out)
    report = {
        "binary_bytes": binary.stat().st_size,
        "runtime_bytes": len(plain.getvalue()),
        "runtime_compressed_bytes": len(archive),
        "total_bytes": out.stat().st_size,
    }
    say(f"wrote {out}: {json.dumps(report)}")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--arch", required=True, choices=sorted(baseline.ARCHES))
    parser.add_argument("--binary", type=Path, help="an aspen-ul to pack instead of building one")
    parser.add_argument("--web-client", type=Path, default=WEB_CLIENT, help=f"the built web client ({WEB_CLIENT})")
    parser.add_argument("--out", type=Path, help="where to write it (target/ul/<arch>/aspen-ul)")
    parser.add_argument("--postgres-only", action="store_true",
                        help="build PostgreSQL alone, printing where its installation is")
    parser.add_argument("-j", "--jobs", type=int, default=8, help="parallel jobs for make and cargo")
    parser.add_argument(
        "--postgres-version", default=POSTGRES_VERSION, help=f"the PostgreSQL to bundle ({POSTGRES_VERSION})"
    )
    args = parser.parse_args()
    for tool in ("make", "patchelf", *([] if args.postgres_only else ["zstd"])):
        if shutil.which(tool) is None:
            baseline.fail(f"missing tool: {tool}")
    postgres = build_postgres(args.arch, args.postgres_version, args.jobs)
    if args.postgres_only:
        print(postgres)
        return
    binary = args.binary
    if binary is None:
        paths = baseline.Paths(args.arch, "bookworm")
        baseline.build(paths, "release", args.jobs)
        baseline.check(paths, "release")
        binary = paths.out("release") / "aspen-ul"
    pack(binary, postgres, args.web_client, args.out or UL / args.arch / "aspen-ul")


if __name__ == "__main__":
    main()
