//! The pictures benchmark users post: JPEGs that look enough like photographs (smooth shading
//! under fine noise) to compress as one does, so uploads and previews cost what a real
//! picture's would.

use image::codecs::jpeg::JpegEncoder;
use image::{ImageBuffer, Rgb};

/// The quality phone cameras and browsers save at.
const QUALITY: u8 = 85;

/// A `width` by `height` JPEG.
pub fn jpeg(width: u32, height: u32) -> bytes::Bytes {
    // xorshift, so every run posts the same picture.
    let mut state = 0x2545_f491_4f6c_dd1d_u64;
    let mut noise = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        (state % 48) as u8
    };
    let picture = ImageBuffer::from_fn(width, height, |x, y| {
        let across = (x * 160 / width.max(1)) as u8;
        let down = (y * 160 / height.max(1)) as u8;
        Rgb([
            across + noise(),
            down + noise(),
            (160 - across / 2 - down / 2) + noise(),
        ])
    });
    let mut out = Vec::new();
    JpegEncoder::new_with_quality(&mut out, QUALITY)
        .encode_image(&picture)
        .expect("a picture in memory encodes");
    out.into()
}

#[cfg(test)]
mod tests {
    #[test]
    fn pictures_are_jpegs_of_photo_size() {
        let picture = super::jpeg(1600, 1200);
        assert_eq!(&picture[..3], &[0xff, 0xd8, 0xff]);
        assert!(
            (100_000..3_000_000).contains(&picture.len()),
            "{} bytes",
            picture.len()
        );
    }
}
