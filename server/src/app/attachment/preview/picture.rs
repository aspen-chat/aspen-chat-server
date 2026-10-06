//! Previews of pictures, made in the server's own process (see `super`).

use super::{Made, Outcome};
use crate::app::context::GlobalServerContext;
use crate::aspen_config::PreviewConfig;
use fast_image_resize::{FilterType, ResizeAlg, ResizeOptions, Resizer};
use image::codecs::gif::GifDecoder;
use image::codecs::png::PngDecoder;
use image::codecs::webp::WebPDecoder;
use image::metadata::Orientation;
use image::{AnimationDecoder, DynamicImage, ImageDecoder, ImageFormat, ImageReader, Limits};
use moxcms::{ColorProfile, DataColorSpace, Layout, TransformOptions};
use std::io::Cursor;

/// The WebP quality previews are encoded at. Shown at no more than a third of their size on the
/// densest screens, and usually much less, they show no loss at it; above it, files grow
/// quickly for nothing anyone sees.
pub const QUALITY: f32 = 85.0;

/// Reads a picture from storage and makes its preview.
pub async fn make(
    state: &GlobalServerContext,
    config: &PreviewConfig,
    key: &str,
    original_bytes: u64,
) -> Outcome {
    if original_bytes > config.max_picture_bytes {
        return Outcome::NoPreview("larger than max_picture_bytes");
    }
    let mut bytes = Vec::with_capacity(usize::try_from(original_bytes).unwrap_or_default());
    match state
        .media_store
        .copy_object_to(key, config.max_picture_bytes, &mut bytes)
        .await
    {
        Ok(Some(_)) => {}
        Ok(None) => return Outcome::NoPreview("larger than max_picture_bytes"),
        Err(e) => return Outcome::Failed(e.to_string()),
    }
    let max_pixels = config.max_picture_pixels;
    match tokio::task::spawn_blocking(move || preview(&bytes, max_pixels)).await {
        Ok(Ok(picture)) => Outcome::Made(Made {
            bytes: picture.webp,
            mime_type: "image/webp",
            width: picture.width,
            height: picture.height,
        }),
        Ok(Err(reason)) => Outcome::NoPreview(reason),
        Err(e) => Outcome::Failed(format!("the preview's maker stopped: {e}")),
    }
}

/// A picture's preview, encoded.
#[derive(Debug)]
pub struct Picture {
    pub webp: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// Makes the preview of the picture in `bytes`, or says why none is made. The picture is
/// anyone's, so it is refused before it is decoded when it has more than `max_pixels`, and
/// decoding allocates no more than such a picture needs.
pub fn preview(bytes: &[u8], max_pixels: u64) -> Result<Picture, &'static str> {
    let mut reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|_| "unreadable")?;
    let format = reader.format().ok_or("not a picture this server reads")?;
    if animated(bytes, format) {
        return Err("animated");
    }
    let mut limits = Limits::default();
    // Eight bytes a pixel holds the deepest colour a decoder makes (16-bit RGBA), and the rest
    // is room for its working buffers.
    limits.max_alloc = Some(max_pixels.saturating_mul(8).saturating_add(64 << 20));
    reader.limits(limits);
    let mut decoder = reader
        .into_decoder()
        .map_err(|_| "not a picture this server reads")?;
    let (width, height) = decoder.dimensions();
    if u64::from(width) * u64::from(height) > max_pixels {
        return Err("more pixels than max_picture_pixels");
    }
    let orientation = decoder.orientation().unwrap_or(Orientation::NoTransforms);
    let icc = decoder.icc_profile().ok().flatten();
    let mut image = DynamicImage::from_decoder(decoder).map_err(|_| "could not be decoded")?;
    image.apply_orientation(orientation);

    let alpha = image.color().has_alpha() && !opaque(&image);
    let mut image = if alpha {
        DynamicImage::ImageRgba8(image.into_rgba8())
    } else {
        DynamicImage::ImageRgb8(image.into_rgb8())
    };
    if let Some(icc) = icc {
        let layout = if alpha { Layout::Rgba } else { Layout::Rgb };
        let pixels = match &mut image {
            DynamicImage::ImageRgba8(buffer) => buffer.as_mut(),
            DynamicImage::ImageRgb8(buffer) => buffer.as_mut(),
            _ => unreachable!("converted to 8-bit RGB or RGBA above"),
        };
        to_srgb(pixels, layout, &icc)?;
    }

    let (width, height) = (image.width(), image.height());
    let (target_width, target_height) = super::fit((width, height), super::BOX);
    if (target_width, target_height) != (width, height) {
        let mut resized = if alpha {
            DynamicImage::new_rgba8(target_width, target_height)
        } else {
            DynamicImage::new_rgb8(target_width, target_height)
        };
        Resizer::new()
            .resize(
                &image,
                &mut resized,
                &ResizeOptions::new().resize_alg(ResizeAlg::Convolution(FilterType::Lanczos3)),
            )
            .map_err(|_| "could not be resized")?;
        image = resized;
    }

    let mut config = webp::WebPConfig::new().map_err(|_| "could not be encoded")?;
    config.quality = QUALITY;
    config.method = 4;
    // Keeps fine detail in saturated colours (red text on white) that WebP's half-resolution
    // colour would otherwise smear.
    config.use_sharp_yuv = 1;
    config.alpha_quality = 100;
    let (layout, raw) = match &image {
        DynamicImage::ImageRgba8(buffer) => (webp::PixelLayout::Rgba, buffer.as_raw()),
        DynamicImage::ImageRgb8(buffer) => (webp::PixelLayout::Rgb, buffer.as_raw()),
        _ => unreachable!("converted to 8-bit RGB or RGBA above"),
    };
    let encoded = webp::Encoder::new(raw, layout, target_width, target_height)
        .encode_advanced(&config)
        .map_err(|_| "could not be encoded")?;
    Ok(Picture {
        webp: encoded.to_vec(),
        width: target_width,
        height: target_height,
    })
}

/// Whether the picture moves. A preview would hold its first frame only, so a picture that
/// moves keeps its original.
fn animated(bytes: &[u8], format: ImageFormat) -> bool {
    match format {
        ImageFormat::Png => PngDecoder::new(Cursor::new(bytes))
            .and_then(|decoder| decoder.is_apng())
            .unwrap_or(false),
        ImageFormat::WebP => WebPDecoder::new(Cursor::new(bytes))
            .map(|decoder| decoder.has_animation())
            .unwrap_or(false),
        ImageFormat::Gif => GifDecoder::new(Cursor::new(bytes))
            .map(|decoder| decoder.into_frames().take(2).count() > 1)
            .unwrap_or(false),
        _ => false,
    }
}

/// Whether a picture with an alpha channel covers all of itself, as many screenshots do; its
/// preview then needs none.
fn opaque(image: &DynamicImage) -> bool {
    match image {
        DynamicImage::ImageLumaA8(buffer) => buffer.pixels().all(|pixel| pixel.0[1] == u8::MAX),
        DynamicImage::ImageRgba8(buffer) => buffer.pixels().all(|pixel| pixel.0[3] == u8::MAX),
        DynamicImage::ImageLumaA16(buffer) => buffer.pixels().all(|pixel| pixel.0[1] == u16::MAX),
        DynamicImage::ImageRgba16(buffer) => buffer.pixels().all(|pixel| pixel.0[3] == u16::MAX),
        DynamicImage::ImageRgba32F(buffer) => buffer.pixels().all(|pixel| pixel.0[3] >= 1.0),
        _ => true,
    }
}

/// Brings `pixels` from the colours the profile `icc` describes into sRGB, which is what a
/// picture without a profile is shown in. A grey profile is left alone, since grey is grey in
/// either; a profile that cannot be read, or of colours other than RGB (a CMYK print file),
/// makes no preview, as its colours would be wrong.
fn to_srgb(pixels: &mut [u8], layout: Layout, icc: &[u8]) -> Result<(), &'static str> {
    let profile =
        ColorProfile::new_from_slice(icc).map_err(|_| "a colour profile that cannot be read")?;
    match profile.color_space {
        DataColorSpace::Rgb => {}
        DataColorSpace::Gray => return Ok(()),
        _ => return Err("colours other than RGB"),
    }
    let transform = profile
        .create_transform_8bit(
            layout,
            &ColorProfile::new_srgb(),
            layout,
            TransformOptions::default(),
        )
        .map_err(|_| "a colour profile that cannot be applied")?;
    let source = pixels.to_vec();
    transform
        .transform(&source, pixels)
        .map_err(|_| "a colour profile that cannot be applied")
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageEncoder, Rgb, RgbImage, Rgba, RgbaImage};

    const MAX_PIXELS: u64 = 100_000_000;

    /// A picture with detail everywhere, as a photo has.
    fn photo(width: u32, height: u32) -> RgbImage {
        RgbImage::from_fn(width, height, |x, y| {
            Rgb([
                (x * 255 / width) as u8,
                (y * 255 / height) as u8,
                ((x ^ y) & 0xff) as u8,
            ])
        })
    }

    fn png(image: &DynamicImage) -> Vec<u8> {
        let mut bytes = Vec::new();
        image
            .write_to(&mut Cursor::new(&mut bytes), ImageFormat::Png)
            .unwrap();
        bytes
    }

    fn jpeg(image: &RgbImage) -> Vec<u8> {
        let mut bytes = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, 95)
            .write_image(
                image.as_raw(),
                image.width(),
                image.height(),
                image::ExtendedColorType::Rgb8,
            )
            .unwrap();
        bytes
    }

    fn decode(webp: &[u8]) -> DynamicImage {
        image::load_from_memory_with_format(webp, ImageFormat::WebP).unwrap()
    }

    #[test]
    fn a_large_photo_is_fitted_in_the_box() {
        let picture = preview(&jpeg(&photo(4032, 3024)), MAX_PIXELS).unwrap();
        assert_eq!((picture.width, picture.height), (1280, 960));
        let decoded = decode(&picture.webp);
        assert_eq!((decoded.width(), decoded.height()), (1280, 960));
        assert!(!decoded.color().has_alpha());
    }

    #[test]
    fn a_small_picture_keeps_its_size() {
        let picture = preview(&png(&DynamicImage::ImageRgb8(photo(300, 200))), MAX_PIXELS).unwrap();
        assert_eq!((picture.width, picture.height), (300, 200));
    }

    #[test]
    fn transparency_is_kept_and_opaque_alpha_dropped() {
        let clear = RgbaImage::from_fn(64, 64, |x, _| Rgba([200, 30, 30, (x * 4) as u8]));
        let picture = preview(&png(&DynamicImage::ImageRgba8(clear)), MAX_PIXELS).unwrap();
        assert!(decode(&picture.webp).color().has_alpha());

        let solid = RgbaImage::from_pixel(64, 64, Rgba([200, 30, 30, 255]));
        let picture = preview(&png(&DynamicImage::ImageRgba8(solid)), MAX_PIXELS).unwrap();
        assert!(!decode(&picture.webp).color().has_alpha());
    }

    #[test]
    fn a_photo_is_turned_upright() {
        // Stored landscape, to be shown turned a quarter clockwise (EXIF orientation 6).
        let bytes = with_orientation(&jpeg(&photo(400, 300)), 6);
        let picture = preview(&bytes, MAX_PIXELS).unwrap();
        assert_eq!((picture.width, picture.height), (300, 400));
    }

    #[test]
    fn display_p3_colours_are_brought_into_srgb() {
        // Pure P3 green lies outside sRGB, so in sRGB it is clipped to the purest green there
        // is, with its red and blue at nothing; read as sRGB it would be duller.
        let green = RgbImage::from_pixel(32, 32, Rgb([0, 255, 0]));
        let mut bytes = Vec::new();
        let mut encoder = image::codecs::png::PngEncoder::new(&mut bytes);
        encoder
            .set_icc_profile(ColorProfile::new_display_p3().encode().unwrap())
            .unwrap();
        encoder
            .write_image(green.as_raw(), 32, 32, image::ExtendedColorType::Rgb8)
            .unwrap();
        let picture = preview(&bytes, MAX_PIXELS).unwrap();
        let pixel = decode(&picture.webp).to_rgb8().get_pixel(16, 16).0;
        assert!(pixel[1] > 240, "{pixel:?}");

        // A mid P3 red is a stronger red than the same numbers in sRGB.
        let red = RgbImage::from_pixel(32, 32, Rgb([180, 60, 60]));
        let mut bytes = Vec::new();
        let mut encoder = image::codecs::png::PngEncoder::new(&mut bytes);
        encoder
            .set_icc_profile(ColorProfile::new_display_p3().encode().unwrap())
            .unwrap();
        encoder
            .write_image(red.as_raw(), 32, 32, image::ExtendedColorType::Rgb8)
            .unwrap();
        let picture = preview(&bytes, MAX_PIXELS).unwrap();
        let pixel = decode(&picture.webp).to_rgb8().get_pixel(16, 16).0;
        assert!(pixel[0] > 190 && pixel[1] < 60, "{pixel:?}");
    }

    #[test]
    fn a_moving_picture_keeps_its_original() {
        let mut bytes = Vec::new();
        {
            let mut encoder = image::codecs::gif::GifEncoder::new(&mut bytes);
            for shade in [0u8, 255] {
                let frame = RgbaImage::from_pixel(16, 16, Rgba([shade, shade, shade, 255]));
                encoder.encode_frame(image::Frame::new(frame)).unwrap();
            }
        }
        assert_eq!(preview(&bytes, MAX_PIXELS).unwrap_err(), "animated");

        let mut still = Vec::new();
        image::codecs::gif::GifEncoder::new(&mut still)
            .encode_frame(image::Frame::new(RgbaImage::from_pixel(
                16,
                16,
                Rgba([9, 9, 9, 255]),
            )))
            .unwrap();
        assert!(preview(&still, MAX_PIXELS).is_ok());
    }

    #[test]
    fn too_many_pixels_and_strangers_are_refused() {
        let bytes = png(&DynamicImage::ImageRgb8(photo(200, 200)));
        assert_eq!(
            preview(&bytes, 199 * 200).unwrap_err(),
            "more pixels than max_picture_pixels"
        );
        assert!(preview(b"<svg xmlns='http://www.w3.org/2000/svg'/>", MAX_PIXELS).is_err());
        assert!(preview(&bytes[..bytes.len() / 2], MAX_PIXELS).is_err());
    }

    /// `jpeg` with an EXIF segment giving `orientation`, put after its start marker.
    fn with_orientation(jpeg: &[u8], orientation: u8) -> Vec<u8> {
        let mut tiff = b"MM\0\x2a\0\0\0\x08".to_vec();
        tiff.extend_from_slice(&1u16.to_be_bytes());
        tiff.extend_from_slice(&0x0112u16.to_be_bytes());
        tiff.extend_from_slice(&3u16.to_be_bytes());
        tiff.extend_from_slice(&1u32.to_be_bytes());
        tiff.extend_from_slice(&[0, orientation, 0, 0]);
        tiff.extend_from_slice(&0u32.to_be_bytes());
        let mut segment = b"Exif\0\0".to_vec();
        segment.extend_from_slice(&tiff);
        let mut out = jpeg[..2].to_vec();
        out.extend_from_slice(&[0xff, 0xe1]);
        out.extend_from_slice(&((segment.len() + 2) as u16).to_be_bytes());
        out.extend_from_slice(&segment);
        out.extend_from_slice(&jpeg[2..]);
        out
    }
}
