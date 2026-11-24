//! What `aspen-svg-render` and the server running it agree on: its arguments, the exit codes
//! that say what became of the SVG, and the pixels it writes.
//!
//! The SVG is written to the program's standard input, all of it, and the input closed. On
//! [`Exit::Drawn`], its standard output holds [`HEADER_BYTES`] (the width and then the height in
//! pixels, each a little-endian `u32`) and then `width * height` pixels of 8-bit RGBA, rows top
//! to bottom, alpha not premultiplied. On any other exit it writes nothing there, and says why
//! on its standard error, for the log.

/// Names the directory of fonts the program draws text with: every `.ttf` and `.otf` in it, in
/// the order of their file names, which is the order a character a face lacks is looked for in
/// the others.
pub const FONTS: &str = "--fonts";

/// Names the box, `<width>x<height>` in pixels, an SVG is drawn fitted within ([`fit`]).
pub const BOX: &str = "--box";

/// Names the most pixels a picture an SVG embeds may have; an SVG embedding a larger one is
/// refused ([`Exit::Refused`]).
pub const MAX_PIXELS: &str = "--max-pixels";

/// Names the most bytes an SVG may have once a compressed one (SVGZ) is unpacked.
pub const MAX_BYTES: &str = "--max-bytes";

/// Asks the program to check itself instead of drawing what it reads: that its fonts open, that
/// it can enter its sandbox, and that it then draws an SVG of its own with text in it. It exits
/// [`Exit::Drawn`] when all three hold, and reads nothing.
pub const CHECK: &str = "--check";

/// The width and height before the pixels.
pub const HEADER_BYTES: usize = 8;

/// The most XML nodes an SVG may have: elements, text, and attributes' expansions together.
/// Far beyond any drawing someone exports (a detailed map has tens of thousands), and a bound on
/// what parsing it holds before the memory limit would be reached.
pub const MAX_NODES: u32 = 1_000_000;

/// What became of an SVG, as the program's exit code. A program killed by a signal (the CPU or
/// memory limit, the deadline, its sandbox) exits with none.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exit {
    /// It was drawn, and the pixels written.
    Drawn,
    /// It is not an SVG this program reads, or draws nothing.
    Unreadable,
    /// It asks for more than the limits it was given allow.
    Refused,
    /// The program cannot draw anything here: its arguments are wrong, its fonts do not open,
    /// or its sandbox cannot be entered.
    Unusable,
}

impl Exit {
    pub const fn code(self) -> i32 {
        match self {
            Exit::Drawn => 0,
            Exit::Unreadable => 2,
            Exit::Refused => 3,
            Exit::Unusable => 4,
        }
    }

    /// The exit `code` stands for, if it is one of these.
    pub const fn of(code: i32) -> Option<Self> {
        match code {
            0 => Some(Exit::Drawn),
            2 => Some(Exit::Unreadable),
            3 => Some(Exit::Refused),
            4 => Some(Exit::Unusable),
            _ => None,
        }
    }
}

/// Fits `size` within `bounds`, keeping its aspect ratio and never enlarging it; neither side
/// is less than a pixel. Previews of every kind are fitted by it, an SVG's by this program.
pub fn fit(size: (u32, u32), bounds: (u32, u32)) -> (u32, u32) {
    let (width, height) = size;
    let (max_width, max_height) = bounds;
    if width <= max_width && height <= max_height {
        return size;
    }
    // The tighter of the two ratios, compared without division: max_width / width against
    // max_height / height.
    let (scaled, other, limit) =
        if u64::from(max_width) * u64::from(height) <= u64::from(max_height) * u64::from(width) {
            (max_width, height, width)
        } else {
            (max_height, width, height)
        };
    let other = ((u64::from(other) * u64::from(scaled) + u64::from(limit) / 2) / u64::from(limit))
        .max(1) as u32;
    if scaled == max_width {
        (scaled, other)
    } else {
        (other, scaled)
    }
}

/// `<width>x<height>`, as [`BOX`] takes it.
pub fn box_arg((width, height): (u32, u32)) -> String {
    format!("{width}x{height}")
}

/// The box [`box_arg`] wrote, or `None` when `text` is not one.
pub fn parse_box(text: &str) -> Option<(u32, u32)> {
    let (width, height) = text.split_once('x')?;
    let (width, height) = (width.parse().ok()?, height.parse().ok()?);
    (width > 0 && height > 0).then_some((width, height))
}

#[cfg(test)]
mod tests {
    use super::*;

    const BOX: (u32, u32) = (1920, 960);

    #[test]
    fn a_preview_fits_its_box_and_keeps_its_shape() {
        assert_eq!(fit((800, 600), BOX), (800, 600));
        assert_eq!(fit((1920, 960), BOX), (1920, 960));
        assert_eq!(fit((4032, 3024), BOX), (1280, 960));
        assert_eq!(fit((3024, 4032), BOX), (720, 960));
        assert_eq!(fit((8000, 1000), BOX), (1920, 240));
        assert_eq!(fit((100_000, 1), BOX), (1920, 1));
        assert_eq!(fit((1, 100_000), BOX), (1, 960));
        assert_eq!(fit((u32::MAX, u32::MAX), BOX), (960, 960));
    }

    #[test]
    fn every_exit_reads_back_as_itself() {
        for exit in [Exit::Drawn, Exit::Unreadable, Exit::Refused, Exit::Unusable] {
            assert_eq!(Exit::of(exit.code()), Some(exit));
        }
        assert_eq!(Exit::of(1), None);
        assert_eq!(Exit::of(101), None);
    }

    #[test]
    fn a_box_reads_back_as_itself() {
        assert_eq!(parse_box(&box_arg(BOX)), Some(BOX));
        for wrong in [
            "",
            "x",
            "1920",
            "0x960",
            "1920x0",
            "-1x2",
            "1920x960x1",
            "a x b",
        ] {
            assert_eq!(parse_box(wrong), None, "{wrong}");
        }
    }
}
