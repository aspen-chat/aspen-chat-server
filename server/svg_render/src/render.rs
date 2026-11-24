//! Drawing one SVG: parsing it with `usvg` under [`MAX_NODES`],
//! with nothing outside it reachable, and drawing it with `resvg` at its own size, fitted within
//! the box it is given.

use crate::protocol::{MAX_NODES, fit};
use resvg::tiny_skia::{Pixmap, Transform};
use resvg::usvg::{self, ImageHrefResolver, ImageKind, Options, fontdb, roxmltree};
use std::io::Read;
use std::path::Path;
use std::sync::Arc;

/// The family text is drawn in where an SVG names none, or a generic family other than
/// monospace: the app's own text face, so an SVG reads as the messages around it do.
pub const TEXT_FAMILY: &str = "Inclusive Sans";

/// The family `monospace` is drawn in: the app's own code face.
pub const MONOSPACE_FAMILY: &str = "Intel One Mono";

/// The fonts text is drawn with, mapped from their files before the sandbox closes, in the
/// order of their file names.
pub struct Fonts(Vec<memmap2::Mmap>);

impl Fonts {
    /// Maps every `.ttf` and `.otf` in `dir`. A directory holding none is refused, since every
    /// SVG's text would then be left out without a word.
    pub fn open(dir: &Path) -> std::io::Result<Self> {
        let mut paths = Vec::new();
        for entry in std::fs::read_dir(dir)? {
            let path = entry?.path();
            let font = path
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| {
                    extension.eq_ignore_ascii_case("ttf") || extension.eq_ignore_ascii_case("otf")
                });
            if font {
                paths.push(path);
            }
        }
        if paths.is_empty() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("{} holds no .ttf or .otf fonts", dir.display()),
            ));
        }
        paths.sort();
        let mut maps = Vec::with_capacity(paths.len());
        for path in paths {
            let file = std::fs::File::open(&path)?;
            // SAFETY: the fonts are read-only files of the deployment's own, which nothing
            // writes while a renderer runs; a font changed under the map could at worst draw
            // wrong glyphs in this one short-lived process.
            maps.push(unsafe { memmap2::Mmap::map(&file)? });
        }
        Ok(Self(maps))
    }

    /// The fonts as `usvg` finds them, each face of each file in order.
    fn database(self) -> fontdb::Database {
        let mut database = fontdb::Database::new();
        for map in self.0 {
            database.load_font_source(fontdb::Source::Binary(Arc::new(map)));
        }
        database.set_sans_serif_family(TEXT_FAMILY);
        database.set_serif_family(TEXT_FAMILY);
        database.set_cursive_family(TEXT_FAMILY);
        database.set_fantasy_family(TEXT_FAMILY);
        database.set_monospace_family(MONOSPACE_FAMILY);
        database
    }
}

/// What an SVG may ask for.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// The box it is fitted within, width then height, in pixels.
    pub bounds: (u32, u32),
    /// The most pixels a picture it embeds may have.
    pub max_pixels: u64,
    /// The most bytes it may have once unpacked, when it is compressed.
    pub max_bytes: u64,
}

/// An SVG drawn: `width * height` pixels of 8-bit RGBA, alpha not premultiplied.
#[derive(Debug)]
pub struct Drawn {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// Why an SVG was not drawn.
#[derive(Debug, PartialEq, Eq)]
pub enum Failure {
    /// It is not an SVG this reads, or it has no size.
    Unreadable(&'static str),
    /// It asks for more than [`Limits`] allow.
    Refused(&'static str),
}

/// Draws the SVG in `bytes`, plain or compressed (SVGZ), with `fonts`.
///
/// Its size is its own (`width` and `height`, or its `viewBox`), rounded to whole pixels and
/// fitted within `limits.bounds` with its aspect ratio kept, never enlarged. Nothing outside the
/// SVG is reached: an `<image>` naming a file or an address draws nothing, and of those it
/// embeds as `data:` URLs, a picture of more than `limits.max_pixels` refuses the whole SVG.
pub fn draw(bytes: &[u8], fonts: Fonts, limits: Limits) -> Result<Drawn, Failure> {
    let unpacked;
    let bytes = if bytes.starts_with(&[0x1f, 0x8b]) {
        unpacked = unpack(bytes, limits.max_bytes)?;
        &unpacked[..]
    } else {
        bytes
    };
    let text = std::str::from_utf8(bytes).map_err(|_| Failure::Unreadable("not UTF-8"))?;
    let document = roxmltree::Document::parse_with_options(
        text,
        roxmltree::ParsingOptions {
            // Illustrator writes its namespaces as entities in a DTD. roxmltree bounds how far
            // entities expand, and resolves none outside the document.
            allow_dtd: true,
            nodes_limit: MAX_NODES,
            ..Default::default()
        },
    )
    .map_err(|e| match e {
        roxmltree::Error::NodesLimitReached => Failure::Refused("more XML nodes than MAX_NODES"),
        roxmltree::Error::EntityReferenceLoop(_) => Failure::Refused("an entity loop"),
        _ => Failure::Unreadable("not XML"),
    })?;

    let refused = std::sync::atomic::AtomicBool::new(false);
    let default_data = ImageHrefResolver::default_data_resolver();
    let options = Options {
        resources_dir: None,
        font_family: TEXT_FAMILY.to_owned(),
        fontdb: Arc::new(fonts.database()),
        image_href_resolver: ImageHrefResolver {
            resolve_data: Box::new(|mime, data, options| {
                let kind = default_data(mime, data, options)?;
                if embeds_too_many_pixels(&kind, limits.max_pixels) {
                    refused.store(true, std::sync::atomic::Ordering::Relaxed);
                    return None;
                }
                Some(kind)
            }),
            resolve_string: Box::new(|_, _| None),
        },
        ..Options::default()
    };
    let tree = usvg::Tree::from_xmltree(&document, &options)
        .map_err(|_| Failure::Unreadable("not an SVG this reads"))?;
    drop(document);
    if refused.load(std::sync::atomic::Ordering::Relaxed) {
        return Err(Failure::Refused(
            "it embeds a picture of more than max_picture_pixels",
        ));
    }

    let size = tree.size();
    let own = (whole_pixels(size.width()), whole_pixels(size.height()));
    let (width, height) = fit(own, limits.bounds);
    let mut pixmap =
        Pixmap::new(width, height).ok_or(Failure::Unreadable("a size that cannot be drawn"))?;
    let transform =
        Transform::from_scale(width as f32 / size.width(), height as f32 / size.height());
    resvg::render(&tree, transform, &mut pixmap.as_mut());
    let rgba = pixmap
        .pixels()
        .iter()
        .flat_map(|pixel| {
            let pixel = pixel.demultiply();
            [pixel.red(), pixel.green(), pixel.blue(), pixel.alpha()]
        })
        .collect();
    Ok(Drawn {
        width,
        height,
        rgba,
    })
}

/// A compressed SVG unpacked, refused past `max_bytes`.
fn unpack(bytes: &[u8], max_bytes: u64) -> Result<Vec<u8>, Failure> {
    let mut unpacked = Vec::new();
    flate2::read::GzDecoder::new(bytes)
        .take(max_bytes.saturating_add(1))
        .read_to_end(&mut unpacked)
        .map_err(|_| Failure::Unreadable("not gzip"))?;
    if unpacked.len() as u64 > max_bytes {
        return Err(Failure::Refused("more than max_svg_bytes once unpacked"));
    }
    Ok(unpacked)
}

/// Whether a picture an SVG embeds has more than `max_pixels`, judged by its header before
/// anything decodes it. One whose size cannot be read is drawn as nothing by `resvg`. An SVG
/// embedded in an SVG was parsed under the same resolver, its own pictures judged as it was.
fn embeds_too_many_pixels(kind: &ImageKind, max_pixels: u64) -> bool {
    match kind {
        ImageKind::JPEG(data)
        | ImageKind::PNG(data)
        | ImageKind::GIF(data)
        | ImageKind::WEBP(data) => imagesize::blob_size(data)
            .is_ok_and(|size| (size.width as u64).saturating_mul(size.height as u64) > max_pixels),
        ImageKind::SVG(_) => false,
    }
}

/// A side of an SVG's size in whole pixels: rounded, at least one, and at most `u32::MAX`.
fn whole_pixels(side: f32) -> u32 {
    // `as` saturates a float beyond the integer's range, and makes NaN 0.
    (side.round() as u32).max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIMITS: Limits = Limits {
        bounds: (1920, 960),
        max_pixels: 100_000_000,
        max_bytes: 8 << 20,
    };

    /// The fonts `scripts/svg_fonts.py` builds, where it has built them; tests that draw text
    /// need them, and say so when they are missing.
    fn fonts() -> Fonts {
        let dir = std::env::var_os("ASPEN_SVG_FONTS")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/svg-fonts")
            });
        Fonts::open(&dir).unwrap_or_else(|e| {
            panic!(
                "{e}: build the fonts with scripts/svg_fonts.py, or name them in ASPEN_SVG_FONTS"
            )
        })
    }

    fn pixel(drawn: &Drawn, x: u32, y: u32) -> [u8; 4] {
        let at = ((y * drawn.width + x) * 4) as usize;
        drawn.rgba[at..at + 4].try_into().unwrap()
    }

    #[test]
    fn an_svg_is_drawn_at_its_own_size() {
        let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="30">
            <rect width="40" height="30" fill="red"/></svg>"#;
        let drawn = draw(svg, fonts(), LIMITS).unwrap();
        assert_eq!((drawn.width, drawn.height), (40, 30));
        assert_eq!(drawn.rgba.len(), 40 * 30 * 4);
        assert_eq!(pixel(&drawn, 20, 15), [255, 0, 0, 255]);
    }

    #[test]
    fn a_large_svg_is_fitted_in_the_box_keeping_its_shape() {
        let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 4000 1000">
            <rect width="4000" height="1000" fill="lime"/></svg>"#;
        let drawn = draw(svg, fonts(), LIMITS).unwrap();
        assert_eq!((drawn.width, drawn.height), (1920, 480));
        assert_eq!(pixel(&drawn, 1919, 479), [0, 255, 0, 255]);
    }

    #[test]
    fn transparency_is_kept_unpremultiplied() {
        let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10">
            <rect width="10" height="10" fill="red" fill-opacity="0.5"/></svg>"#;
        let [red, green, blue, alpha] = pixel(&draw(svg, fonts(), LIMITS).unwrap(), 5, 5);
        assert_eq!((red, green, blue), (255, 0, 0));
        assert!((127..=128).contains(&alpha), "{alpha}");
    }

    #[test]
    fn text_is_drawn_in_the_bundled_fonts() {
        let draw_text = |text: &str| {
            let svg = format!(
                r#"<svg xmlns="http://www.w3.org/2000/svg" width="300" height="60">
                <text x="10" y="45" font-size="40" fill="black">{text}</text></svg>"#
            );
            draw(svg.as_bytes(), fonts(), LIMITS).unwrap().rgba
        };
        // Characters no font draws come out as boxes, which ink pixels too, so what each text
        // draws is compared with as many boxes.
        for text in ["Aspen", "白杨", "Осина", "مرحبا", "😀", "👍🏽"] {
            let drawn = draw_text(text);
            assert!(
                drawn.chunks(4).any(|pixel| pixel[3] > 0),
                "{text} drew nothing"
            );
            let boxes: String = text.chars().map(|_| '\u{E000}').collect();
            assert_ne!(drawn, draw_text(&boxes), "{text} drew as boxes");
        }
    }

    #[test]
    fn nothing_outside_the_svg_is_reached() {
        let file =
            std::env::temp_dir().join(format!("aspen-svg-render-{}.png", std::process::id()));
        let mut png = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut png, 1, 1);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            encoder
                .write_header()
                .unwrap()
                .write_image_data(&[0, 0, 255, 255])
                .unwrap();
        }
        std::fs::write(&file, &png).unwrap();
        let svg = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink"
                width="10" height="10">
            <image width="10" height="10" xlink:href="{}"/>
            <image width="10" height="10" href="https://example.com/a.png"/></svg>"#,
            file.display()
        );
        let drawn = draw(svg.as_bytes(), fonts(), LIMITS);
        std::fs::remove_file(&file).unwrap();
        let drawn = drawn.unwrap();
        assert!(drawn.rgba.chunks(4).all(|pixel| pixel[3] == 0));
    }

    #[test]
    fn an_embedded_picture_is_drawn_and_one_too_large_refused() {
        let mut png = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut png, 2, 2);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            encoder
                .write_header()
                .unwrap()
                .write_image_data(&[0, 0, 255, 255].repeat(4))
                .unwrap();
        }
        let svg = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10">
            <image width="10" height="10" href="data:image/png;base64,{}"/></svg>"#,
            base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &png)
        );
        let drawn = draw(svg.as_bytes(), fonts(), LIMITS).unwrap();
        assert_eq!(pixel(&drawn, 5, 5), [0, 0, 255, 255]);
        let small = Limits {
            max_pixels: 3,
            ..LIMITS
        };
        assert!(matches!(
            draw(svg.as_bytes(), fonts(), small),
            Err(Failure::Refused(_))
        ));
    }

    #[test]
    fn a_compressed_svg_is_unpacked_within_its_limit() {
        use std::io::Write;
        let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="4">
            <rect width="4" height="4"/></svg>"#;
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        gz.write_all(svg).unwrap();
        let gz = gz.finish().unwrap();
        assert_eq!(draw(&gz, fonts(), LIMITS).unwrap().width, 4);
        let tight = Limits {
            max_bytes: svg.len() as u64 - 1,
            ..LIMITS
        };
        assert!(matches!(
            draw(&gz, fonts(), tight),
            Err(Failure::Refused(_))
        ));
    }

    #[test]
    fn what_is_not_an_svg_is_unreadable() {
        for bytes in [
            &b""[..],
            b"\xff\xfe",
            b"<html></html>",
            b"<svg",
            b"\x1f\x8bnot gzip",
        ] {
            assert!(
                matches!(draw(bytes, fonts(), LIMITS), Err(Failure::Unreadable(_))),
                "{bytes:?}"
            );
        }
    }

    #[test]
    fn too_many_nodes_are_refused() {
        let mut svg =
            String::from(r#"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1">"#);
        for _ in 0..MAX_NODES {
            svg.push_str("<g/>");
        }
        svg.push_str("</svg>");
        assert_eq!(
            draw(svg.as_bytes(), fonts(), LIMITS).unwrap_err(),
            Failure::Refused("more XML nodes than MAX_NODES")
        );
    }

    #[test]
    fn a_side_is_whole_and_at_least_a_pixel() {
        assert_eq!(whole_pixels(23.5), 24);
        assert_eq!(whole_pixels(0.2), 1);
        assert_eq!(whole_pixels(f32::MAX), u32::MAX);
        assert_eq!(whole_pixels(f32::NAN), 1);
    }
}
