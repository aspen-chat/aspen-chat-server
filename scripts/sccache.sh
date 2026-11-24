#!/usr/bin/env bash
# Cargo's rustc wrapper (`.cargo/config.toml`), run as `sccache.sh <compiler> <args>…`: compiles go
# through sccache when it is installed, so worktrees share what they compile, and straight to the
# compiler when it is not. The `cc` crate hands build scripts' C compiles to a rustc wrapper named
# `sccache` as well, which is why the script is named for it.
#
# Bash, not `sh`: Cargo hands integration tests `CARGO_BIN_EXE_<bin>`, whose name holds the binary's
# hyphens (`CARGO_BIN_EXE_aspen-svg-render`), and dash, Debian's and Ubuntu's `sh`, drops variables
# whose names are not shell identifiers when it execs, so `env!` would find none. Bash passes them on.
if command -v sccache >/dev/null 2>&1; then
    exec sccache "$@"
fi
exec "$@"
