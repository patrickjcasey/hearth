//! Recipe photos. Imported recipes only carry a link to their photo, which
//! disappears offline and slows the recipe grid, so the app keeps a shrunk
//! copy beside each recipe.

use uuid::Uuid;

/// Longest side of a stored photo, in pixels. Enough for a full-width hero
/// on a Retina display without keeping multi-megabyte originals.
pub const MAX_SIDE: u32 = 1200;

const JPEG_QUALITY: u8 = 82;

#[derive(Debug, thiserror::Error)]
pub enum PhotoError {
    #[cfg(feature = "fetch")]
    #[error("could not download photo: {0}")]
    Fetch(#[from] reqwest::Error),
    #[error("not an image we can read: {0}")]
    Decode(#[from] image::ImageError),
}

/// File stem for a recipe's stored photo, such as `<id>-9f2a…`. The hash of
/// the source link is part of the name, so changing the link naturally
/// makes the old file stale, and a synced folder never confuses the two.
pub fn photo_stem(id: Uuid, image_url: &str) -> String {
    format!("{id}-{:016x}", fnv1a(image_url.as_bytes()))
}

/// Does `stem` name a photo of recipe `id`?
pub fn stem_belongs_to(stem: &str, id: Uuid) -> bool {
    stem.strip_prefix(&id.to_string()).is_some_and(|rest| rest.starts_with('-'))
}

/// A small, stable hash: the file name must come out the same on every
/// device and Rust version, which the standard library's hasher does not
/// promise.
fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, b| (hash ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3))
}

/// Downloads the photo at `url`.
#[cfg(feature = "fetch")]
pub async fn fetch_photo(url: &str) -> Result<Vec<u8>, PhotoError> {
    let bytes = reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (compatible; Hestia/0.1; +https://github.com/)")
        .timeout(std::time::Duration::from_secs(20))
        .build()?
        .get(url)
        .send()
        .await?
        .error_for_status()?
        .bytes()
        .await?;
    Ok(bytes.to_vec())
}

/// Decodes a photo in any common format, fixes its orientation, fits it
/// within [`MAX_SIDE`] and re-encodes it as a JPEG. CPU-bound: call it off
/// the UI thread.
pub fn shrink_photo(bytes: &[u8]) -> Result<Vec<u8>, PhotoError> {
    use image::codecs::jpeg::JpegEncoder;
    use image::imageops::FilterType;
    use image::{DynamicImage, ImageDecoder, ImageReader};

    let mut decoder = ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(image::ImageError::IoError)?
        .into_decoder()?;
    let orientation = decoder.orientation()?;
    let mut photo = DynamicImage::from_decoder(decoder)?;
    photo.apply_orientation(orientation);
    if photo.width() > MAX_SIDE || photo.height() > MAX_SIDE {
        photo = photo.resize(MAX_SIDE, MAX_SIDE, FilterType::Triangle);
    }

    let mut out = Vec::new();
    // JPEG has no alpha channel; flatten anything that does.
    photo.to_rgb8().write_with_encoder(JpegEncoder::new_with_quality(&mut out, JPEG_QUALITY))?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use image::{ImageReader, RgbaImage};

    use super::*;

    fn png(width: u32, height: u32) -> Vec<u8> {
        let mut out = std::io::Cursor::new(Vec::new());
        RgbaImage::from_pixel(width, height, image::Rgba([200, 120, 40, 255]))
            .write_to(&mut out, image::ImageFormat::Png)
            .unwrap();
        out.into_inner()
    }

    fn size(jpeg: &[u8]) -> (u32, u32) {
        ImageReader::new(std::io::Cursor::new(jpeg))
            .with_guessed_format()
            .unwrap()
            .into_dimensions()
            .unwrap()
    }

    #[test]
    fn shrinks_large_photos_to_fit() {
        let jpeg = shrink_photo(&png(3000, 2000)).unwrap();
        assert_eq!(size(&jpeg), (MAX_SIDE, 800));
    }

    #[test]
    fn leaves_small_photos_alone() {
        let jpeg = shrink_photo(&png(640, 480)).unwrap();
        assert_eq!(size(&jpeg), (640, 480));
    }

    #[test]
    fn rejects_things_that_are_not_images() {
        assert!(shrink_photo(b"<html>not a photo</html>").is_err());
    }

    #[test]
    fn stems_are_stable_and_change_with_the_link() {
        let id = Uuid::nil();
        let a = photo_stem(id, "https://example.com/a.jpg");
        assert_eq!(a, photo_stem(id, "https://example.com/a.jpg"));
        assert_ne!(a, photo_stem(id, "https://example.com/b.jpg"));
        assert!(stem_belongs_to(&a, id));
        assert!(!stem_belongs_to(&a, Uuid::new_v4()));
        assert!(!stem_belongs_to("00000000-0000-0000-0000-000000000000", id));
    }
}
