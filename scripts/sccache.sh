#!/bin/sh
# Cargo's rustc wrapper (`.cargo/config.toml`), run as `sccache.sh <compiler> <args>…`: compiles go
# through sccache when it is installed, so worktrees share what they compile, and straight to the
# compiler when it is not. The `cc` crate hands build scripts' C compiles to a rustc wrapper named
# `sccache` as well, which is why the script is named for it.
if command -v sccache >/dev/null 2>&1; then
    exec sccache "$@"
fi
exec "$@"
