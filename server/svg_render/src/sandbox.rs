//! The seccomp filter the renderer installs on itself once its fonts are mapped and before it
//! reads the SVG ([`enter`]).
//!
//! It allows what reading standard input, drawing in memory, and writing standard output and
//! error need, and kills the process on any other system call: no file can be opened, no socket
//! made, no program run, and no memory made executable. A system call the allowed code turns
//! out to need shows as the renderer killed by `SIGSYS`, which its caller logs as a warning,
//! never as an SVG drawn wrong.

/// Why the sandbox could not be entered.
#[derive(Debug)]
pub struct Unavailable(pub String);

impl std::fmt::Display for Unavailable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Installs the filter on this process, which has only its one thread, for good.
#[cfg(target_os = "linux")]
pub fn enter() -> Result<(), Unavailable> {
    use seccompiler::{
        BpfProgram, SeccompAction, SeccompCmpArgLen, SeccompCmpOp, SeccompCondition, SeccompFilter,
        SeccompRule, TargetArch,
    };
    use std::collections::BTreeMap;

    let failed = |e: &dyn std::fmt::Display| Unavailable(e.to_string());
    let arch = TargetArch::try_from(std::env::consts::ARCH).map_err(|e| failed(&e))?;
    let condition = |arg: u8, op: SeccompCmpOp, value: u64| {
        SeccompCondition::new(arg, SeccompCmpArgLen::Dword, op, value).map_err(|e| failed(&e))
    };
    let rule =
        |conditions: Vec<SeccompCondition>| SeccompRule::new(conditions).map_err(|e| failed(&e));
    // A descriptor among `fds`, as the first argument.
    let on = |fds: &[u64]| -> Result<Vec<SeccompRule>, Unavailable> {
        fds.iter()
            .map(|&fd| rule(vec![condition(0, SeccompCmpOp::Eq, fd)?]))
            .collect()
    };
    // Protections without `PROT_EXEC`, as the third argument.
    let not_executable = || -> Result<Vec<SeccompRule>, Unavailable> {
        let exec = libc::PROT_EXEC as u64;
        Ok(vec![rule(vec![condition(
            2,
            SeccompCmpOp::MaskedEq(exec),
            0,
        )?])?])
    };

    let mut rules: BTreeMap<i64, Vec<SeccompRule>> = BTreeMap::new();
    rules.insert(libc::SYS_read, on(&[0])?);
    rules.insert(libc::SYS_write, on(&[1, 2])?);
    rules.insert(libc::SYS_writev, on(&[1, 2])?);
    rules.insert(libc::SYS_mmap, not_executable()?);
    rules.insert(libc::SYS_mprotect, not_executable()?);
    for unconditional in [
        // Memory.
        libc::SYS_brk,
        libc::SYS_munmap,
        libc::SYS_mremap,
        libc::SYS_madvise,
        // What the standard library and the C runtime call on their own: a lock waited on, the
        // hash maps' random keys, the clock where the vDSO does not answer.
        libc::SYS_futex,
        libc::SYS_getrandom,
        libc::SYS_clock_gettime,
        libc::SYS_sched_yield,
        // Ending: exiting, and a panic's abort raising `SIGABRT` at itself.
        libc::SYS_exit,
        libc::SYS_exit_group,
        libc::SYS_rt_sigreturn,
        libc::SYS_rt_sigprocmask,
        libc::SYS_rt_sigaction,
        libc::SYS_sigaltstack,
        libc::SYS_getpid,
        libc::SYS_gettid,
        libc::SYS_tgkill,
    ] {
        rules.insert(unconditional, Vec::new());
    }
    let program: BpfProgram = SeccompFilter::new(
        rules,
        SeccompAction::KillProcess,
        SeccompAction::Allow,
        arch,
    )
    .and_then(TryInto::try_into)
    .map_err(|e| failed(&e))?;
    seccompiler::apply_filter(&program).map_err(|e| failed(&e))
}

/// Refused: only Linux has the filter, and an SVG is never drawn without it.
#[cfg(not(target_os = "linux"))]
pub fn enter() -> Result<(), Unavailable> {
    Err(Unavailable(
        "SVGs are drawn only on Linux, whose seccomp filter the renderer runs behind".into(),
    ))
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use std::os::unix::process::ExitStatusExt;
    use std::process::Command;

    /// Names what the test run again as a child of itself does once in the sandbox.
    const CHILD: &str = "ASPEN_SVG_SANDBOX_CHILD";

    /// What the child does, run in a process of its own: entering the sandbox cannot be undone.
    #[test]
    fn child() {
        let Some(what) = std::env::var_os(CHILD) else {
            return;
        };
        let fonts = crate::render::Fonts::open(&fonts_dir()).unwrap();
        super::enter().unwrap();
        match what.to_str().unwrap() {
            "draw" => {
                let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="80" height="30">
                    <text y="25" font-size="20">Aspen 白杨</text></svg>"#;
                let limits = crate::render::Limits {
                    bounds: (1920, 960),
                    max_pixels: 1000,
                    max_bytes: 1000,
                };
                let drawn = crate::render::draw(svg.as_bytes(), fonts, limits).unwrap();
                assert!(drawn.rgba.chunks(4).any(|pixel| pixel[3] > 0));
                std::process::exit(0);
            }
            "open" => {
                let opened = std::fs::File::open("/etc/hostname");
                eprintln!("opened: {opened:?}");
            }
            "socket" => {
                let connected = std::net::TcpStream::connect("127.0.0.1:9");
                eprintln!("connected: {connected:?}");
            }
            "exec" => {
                let ran = Command::new("/bin/true").status();
                eprintln!("ran: {ran:?}");
            }
            other => panic!("no child named {other}"),
        }
        // Reaching here means the call was let through.
        std::process::exit(7);
    }

    fn fonts_dir() -> std::path::PathBuf {
        std::env::var_os("ASPEN_SVG_FONTS")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/svg-fonts")
            })
    }

    fn run(what: &str) -> std::process::ExitStatus {
        Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "sandbox::tests::child",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(CHILD, what)
            .status()
            .unwrap()
    }

    #[test]
    fn drawing_needs_nothing_the_sandbox_refuses() {
        let status = run("draw");
        assert_eq!(status.code(), Some(0), "{status:?}");
    }

    #[test]
    fn files_sockets_and_programs_are_refused() {
        for what in ["open", "socket", "exec"] {
            let status = run(what);
            assert_eq!(status.signal(), Some(libc::SIGSYS), "{what}: {status:?}");
        }
    }
}
