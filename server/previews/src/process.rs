//! Running another program over an attachment's bytes: `ffmpeg` and `ffprobe` for videos'
//! posters ([`crate::video`]), `aspen-svg-render` for SVGs ([`crate::svg`]). Each runs with
//! nothing of this server's environment ([`confined`]) and under resource limits set as it
//! starts ([`limit`]).

use std::time::Duration;
use tokio::process::Command;

/// A command for `program` with nothing of this server's environment (its configuration may be
/// there) but where to find programs, which a `program` named without a path needs.
pub fn confined(program: &str) -> Command {
    let mut command = Command::new(program);
    command.env_clear();
    if let Some(path) = std::env::var_os("PATH") {
        command.env("PATH", path);
    }
    command
}

/// What a program run over an attachment may use.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// Bytes of address space.
    pub memory: u64,
    /// CPU time, in whole seconds: past it the program is sent `SIGXCPU`, and a second later
    /// killed.
    pub cpu: Duration,
    /// Files open at once, its pipes among them.
    pub open_files: u64,
}

/// Sets `limits` on `command`'s process as it starts, and lets it write no file (its output is
/// a pipe) and dump no core.
#[cfg(unix)]
pub fn limit(command: &mut Command, limits: Limits) {
    let cpu = limits.cpu.as_secs().max(1);
    let limits = [
        (libc::RLIMIT_AS, limits.memory, limits.memory),
        (libc::RLIMIT_CPU, cpu, cpu + 1),
        (libc::RLIMIT_FSIZE, 0, 0),
        (libc::RLIMIT_CORE, 0, 0),
        (libc::RLIMIT_NOFILE, limits.open_files, limits.open_files),
    ];
    // SAFETY: the closure runs in the child between fork and exec, where only async-signal-safe
    // calls are allowed; it calls only setrlimit, which is, on values made before the fork.
    unsafe {
        command.pre_exec(move || {
            for (resource, soft, hard) in limits {
                let limit = libc::rlimit {
                    rlim_cur: soft as libc::rlim_t,
                    rlim_max: hard as libc::rlim_t,
                };
                if libc::setrlimit(resource, &limit) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
            }
            Ok(())
        });
    }
}

#[cfg(not(unix))]
pub fn limit(_command: &mut Command, _limits: Limits) {}
