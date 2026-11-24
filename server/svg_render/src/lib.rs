//! Drawing an SVG attachment as pixels for its preview, in `aspen-svg-render`, a program of its
//! own that the API server runs once for each SVG (`aspen_previews::svg`).
//!
//! An SVG is a program of sorts: its `<use>` elements, filters, patterns, and embedded pictures
//! can ask for any amount of memory and time, which no library drawing it bounds from inside the
//! process that calls it. So it is drawn in a process of its own, under the limits its caller
//! sets as it starts (address space, CPU time, no file written, few open) and a wall-clock
//! deadline, and behind a seccomp filter it installs on itself ([`sandbox`]) once its fonts are
//! open and before it reads a byte of the SVG: from then on it may read its standard input,
//! write its standard output and error, manage its memory, and exit, and nothing else, so no
//! SVG can have it open a file or a connection.
//!
//! [`protocol`] is what the program and its caller agree on; it alone is built without the
//! `render` feature, which is all the API server needs of this crate.

pub mod protocol;

#[cfg(feature = "render")]
pub mod render;

#[cfg(feature = "render")]
pub mod sandbox;

pub use protocol::fit;
