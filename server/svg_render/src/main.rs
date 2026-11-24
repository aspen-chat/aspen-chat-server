//! `aspen-svg-render`: draws the SVG on its standard input as pixels on its standard output, as
//! `aspen_svg_render::protocol` says, for the API server's previews (`aspen_previews::svg`).
//!
//! Its fonts are mapped first, then it enters its sandbox (`aspen_svg_render::sandbox`), and
//! only then is the SVG read. Its caller sets its resource limits and deadline.

use aspen_svg_render::protocol::{self, Exit};
use aspen_svg_render::render::{self, Failure, Fonts, Limits};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::ExitCode;

/// The SVG `--check` draws: text, so a renderer whose fonts draw nothing fails its check.
const CHECK_SVG: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" width="120" height="40">
<text x="4" y="30" font-size="24">Aspen</text></svg>"#;

struct Args {
    fonts: PathBuf,
    limits: Limits,
    check: bool,
}

fn args() -> Result<Args, String> {
    let mut fonts = None;
    let mut bounds = None;
    let mut max_pixels = None;
    let mut max_bytes = None;
    let mut check = false;
    let mut args = std::env::args_os().skip(1);
    while let Some(arg) = args.next() {
        let arg = arg
            .into_string()
            .map_err(|arg| format!("{arg:?} is not an argument"))?;
        if arg == protocol::CHECK {
            check = true;
            continue;
        }
        let value = args.next().ok_or_else(|| format!("{arg} needs a value"))?;
        match arg.as_str() {
            protocol::FONTS => fonts = Some(PathBuf::from(value)),
            protocol::BOX => {
                bounds = value
                    .to_str()
                    .and_then(protocol::parse_box)
                    .map(Some)
                    .ok_or_else(|| format!("{value:?} is not a box"))?;
            }
            protocol::MAX_PIXELS => max_pixels = Some(number(&value)?),
            protocol::MAX_BYTES => max_bytes = Some(number(&value)?),
            other => return Err(format!("{other} is not an argument")),
        }
    }
    let missing = |name: &str| format!("{name} is needed");
    Ok(Args {
        fonts: fonts.ok_or_else(|| missing(protocol::FONTS))?,
        limits: Limits {
            bounds: bounds.ok_or_else(|| missing(protocol::BOX))?,
            max_pixels: max_pixels.ok_or_else(|| missing(protocol::MAX_PIXELS))?,
            max_bytes: max_bytes.ok_or_else(|| missing(protocol::MAX_BYTES))?,
        },
        check,
    })
}

fn number(value: &std::ffi::OsStr) -> Result<u64, String> {
    value
        .to_str()
        .and_then(|value| value.parse().ok())
        .ok_or_else(|| format!("{value:?} is not a number"))
}

fn exit(exit: Exit) -> ExitCode {
    ExitCode::from(u8::try_from(exit.code()).expect("exit codes are small"))
}

/// What `--help` prints.
const USAGE: &str =
    "aspen-svg-render --fonts DIR --box WIDTHxHEIGHT --max-pixels N --max-bytes N [--check]

Draws the SVG on standard input as pixels on standard output, for the API server's previews,
which runs it under its limits (aspen_previews::svg). --check draws an SVG of its own instead.";

fn main() -> ExitCode {
    if std::env::args_os()
        .nth(1)
        .is_some_and(|arg| arg == "--help")
    {
        println!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    let args = match args() {
        Ok(args) => args,
        Err(e) => {
            eprintln!("aspen-svg-render: {e}");
            return exit(Exit::Unusable);
        }
    };
    let fonts = match Fonts::open(&args.fonts) {
        Ok(fonts) => fonts,
        Err(e) => {
            eprintln!(
                "aspen-svg-render: the fonts at {} do not open: {e}",
                args.fonts.display()
            );
            return exit(Exit::Unusable);
        }
    };
    if let Err(e) = aspen_svg_render::sandbox::enter() {
        eprintln!("aspen-svg-render: the sandbox cannot be entered: {e}");
        return exit(Exit::Unusable);
    }

    if args.check {
        return match render::draw(CHECK_SVG, fonts, args.limits) {
            Ok(drawn) if drawn.rgba.chunks(4).any(|pixel| pixel[3] > 0) => exit(Exit::Drawn),
            Ok(_) => {
                eprintln!("aspen-svg-render: its fonts draw no text");
                exit(Exit::Unusable)
            }
            Err(e) => {
                eprintln!("aspen-svg-render: its own SVG was not drawn: {e:?}");
                exit(Exit::Unusable)
            }
        };
    }

    let mut svg = Vec::new();
    if let Err(e) = std::io::stdin().lock().read_to_end(&mut svg) {
        eprintln!("aspen-svg-render: the SVG could not be read: {e}");
        return exit(Exit::Unusable);
    }
    let drawn = match render::draw(&svg, fonts, args.limits) {
        Ok(drawn) => drawn,
        Err(Failure::Unreadable(why)) => {
            eprintln!("aspen-svg-render: {why}");
            return exit(Exit::Unreadable);
        }
        Err(Failure::Refused(why)) => {
            eprintln!("aspen-svg-render: {why}");
            return exit(Exit::Refused);
        }
    };
    drop(svg);
    let mut out = std::io::stdout().lock();
    let written = out
        .write_all(&drawn.width.to_le_bytes())
        .and_then(|()| out.write_all(&drawn.height.to_le_bytes()))
        .and_then(|()| out.write_all(&drawn.rgba))
        .and_then(|()| out.flush());
    match written {
        Ok(()) => exit(Exit::Drawn),
        Err(e) => {
            eprintln!("aspen-svg-render: the pixels could not be written: {e}");
            exit(Exit::Unusable)
        }
    }
}
