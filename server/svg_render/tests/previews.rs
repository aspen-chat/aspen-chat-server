//! The renderer as the API server runs it (`aspen_previews::svg`): under its limits, through
//! its protocol, against SVGs that draw and SVGs made to exhaust it.

use aspen_previews::{Outcome, PreviewConfig, svg};
use std::path::PathBuf;
use std::time::Instant;

fn config() -> PreviewConfig {
    let fonts = std::env::var_os("ASPEN_SVG_FONTS")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/svg-fonts")
        });
    assert!(
        fonts.is_dir(),
        "build the fonts with scripts/svg_fonts.py, or name them in ASPEN_SVG_FONTS"
    );
    PreviewConfig {
        svg_renderer: env!("CARGO_BIN_EXE_aspen-svg-render").into(),
        svg_fonts: fonts.display().to_string(),
        ..PreviewConfig::default()
    }
}

#[tokio::test]
async fn the_renderer_checks_itself() {
    svg::available(&config()).await.unwrap();
    let missing = PreviewConfig {
        svg_fonts: "/nonexistent".into(),
        ..config()
    };
    assert!(svg::available(&missing).await.is_err());
    let none = PreviewConfig {
        svg_renderer: String::new(),
        ..config()
    };
    assert!(svg::available(&none).await.is_err());
}

#[tokio::test]
async fn an_svg_is_drawn_as_a_webp_preview_at_its_own_size() {
    let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="64" height="32">
        <rect width="64" height="32" fill="teal"/><text y="24" font-size="20" fill="white">Aspen</text></svg>"#;
    let Outcome::Made(made) = svg::preview(&config(), svg.to_vec()).await else {
        panic!("no preview");
    };
    assert_eq!(
        (made.width, made.height, made.mime_type),
        (64, 32, "image/webp")
    );
    let decoded =
        image::load_from_memory_with_format(&made.bytes, image::ImageFormat::WebP).unwrap();
    assert_eq!((decoded.width(), decoded.height()), (64, 32));
    // Every pixel is covered, so the preview has no alpha.
    assert!(!decoded.color().has_alpha());
}

#[tokio::test]
async fn a_large_svg_is_fitted_within_the_box_keeping_its_shape() {
    let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="10000" height="2500"><circle cx="5000" cy="1250" r="1000"/></svg>"#;
    let Outcome::Made(made) = svg::preview(&config(), svg.to_vec()).await else {
        panic!("no preview");
    };
    assert_eq!((made.width, made.height), (1920, 480));
    let decoded =
        image::load_from_memory_with_format(&made.bytes, image::ImageFormat::WebP).unwrap();
    assert!(decoded.color().has_alpha());
}

#[tokio::test]
async fn what_is_not_an_svg_gets_no_preview() {
    for bytes in [&b"<html></html>"[..], b"\x89PNG\r\n\x1a\n", b""] {
        assert!(
            matches!(
                svg::preview(&config(), bytes.to_vec()).await,
                Outcome::NoPreview(_)
            ),
            "{bytes:?}"
        );
    }
}

#[tokio::test]
async fn an_svg_past_max_svg_bytes_is_not_drawn() {
    let config = PreviewConfig {
        max_svg_bytes: 10,
        ..config()
    };
    let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"/>"#;
    assert!(matches!(
        svg::preview(&config, svg.to_vec()).await,
        Outcome::NoPreview("larger than max_svg_bytes")
    ));
}

/// Groups of ten `<use>`s of the group before, `depth` deep: ten to the `depth` rectangles from
/// a few hundred bytes.
fn uses(depth: usize) -> Vec<u8> {
    let mut svg = String::from(
        r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="100" height="100"><defs><g id="g0"><rect width="1" height="1"/></g>"#,
    );
    for level in 1..=depth {
        svg.push_str(&format!(r#"<g id="g{level}">"#));
        for _ in 0..10 {
            svg.push_str(&format!(r##"<use xlink:href="#g{}"/>"##, level - 1));
        }
        svg.push_str("</g>");
    }
    svg.push_str(&format!(r##"</defs><use xlink:href="#g{depth}"/></svg>"##));
    svg.into_bytes()
}

#[tokio::test]
async fn an_svg_of_a_billion_elements_is_not_drawn() {
    let started = Instant::now();
    let outcome = svg::preview(&config(), uses(9)).await;
    assert!(
        matches!(outcome, Outcome::NoPreview(_)),
        "a billion rectangles were drawn"
    );
    assert!(started.elapsed() < svg::DEADLINE);
}

#[tokio::test]
async fn an_svg_made_to_exhaust_memory_is_stopped_at_its_limit() {
    // A picture of a hundred million pixels, within max_picture_pixels, which takes 400 MB
    // to decode and a few hundred kilobytes to send.
    let side = 10_000u32;
    // The stream writer owns what it writes to, so the bytes are shared with it.
    #[derive(Clone, Default)]
    struct Shared(std::rc::Rc<std::cell::RefCell<Vec<u8>>>);
    impl std::io::Write for Shared {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.borrow_mut().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let png = Shared::default();
    {
        let mut encoder = png::Encoder::new(png.clone(), side, side);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_compression(png::Compression::Fast);
        let mut writer = encoder
            .write_header()
            .unwrap()
            .into_stream_writer()
            .unwrap();
        let row = vec![0u8; side as usize * 4];
        for _ in 0..side {
            std::io::Write::write_all(&mut writer, &row).unwrap();
        }
        writer.finish().unwrap();
    }
    let svg = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100"><image width="100" height="100" href="data:image/png;base64,{}"/></svg>"#,
        base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &*png.0.borrow())
    );
    let config = PreviewConfig {
        svg_memory_mib: 256,
        ..config()
    };
    let started = Instant::now();
    let outcome = svg::preview(&config, svg.clone().into_bytes()).await;
    let Outcome::NoPreview(why) = outcome else {
        panic!("a picture larger than svg_memory_mib was drawn or failed");
    };
    assert_eq!(why, "the SVG renderer ran out of memory or stopped");
    assert!(started.elapsed() < svg::DEADLINE);
    // With room for the picture, the same SVG is drawn: the limit is what stopped it.
    let roomy = PreviewConfig {
        svg_memory_mib: 2048,
        ..config
    };
    assert!(matches!(
        svg::preview(&roomy, svg.into_bytes()).await,
        Outcome::Made(_)
    ));
}

#[tokio::test]
async fn an_svg_made_to_take_long_is_stopped_at_its_cpu_time() {
    // A blur as wide as the picture, of a hundred thousand rectangles' worth of drawing each
    // time it is used, used ten thousand times over.
    let mut svg = String::from(
        r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="1920" height="960"><defs><filter id="f"><feGaussianBlur stdDeviation="200"/><feTurbulence baseFrequency="0.9" numOctaves="10"/></filter><g id="g0"><rect width="1920" height="960" filter="url(#f)"/></g>"#,
    );
    for level in 1..=4 {
        svg.push_str(&format!(r#"<g id="g{level}">"#));
        for _ in 0..10 {
            svg.push_str(&format!(r##"<use xlink:href="#g{}"/>"##, level - 1));
        }
        svg.push_str("</g>");
    }
    svg.push_str(r##"</defs><use xlink:href="#g4"/></svg>"##);
    let started = Instant::now();
    let outcome = svg::preview(&config(), svg.into_bytes()).await;
    let Outcome::NoPreview(why) = outcome else {
        panic!("ten thousand full-size blurs were drawn or failed");
    };
    eprintln!("{why} after {:?}", started.elapsed());
    assert!(started.elapsed() < svg::DEADLINE + std::time::Duration::from_secs(1));
}
