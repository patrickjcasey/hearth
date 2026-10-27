//! Recipe photos. Imported recipes only carry a link to their photo, which
//! disappears offline and slows the recipe grid, so the app keeps a shrunk
//! copy beside each recipe.

use uuid::Uuid;

/// Longest side of a stored photo, in pixels. Enough for a full-width hero
/// on a Retina display without keeping multi-megabyte originals.
pub const MAX_SIDE: u32 = 1200;

const JPEG_QUALITY: u8 = 82;

/// Largest download we accept; phone photos are a few MB.
#[cfg(feature = "fetch")]
const MAX_PHOTO_BYTES: u64 = 20_000_000;
/// Decoding limits, so a small file can't claim a huge canvas.
const MAX_DECODE_SIDE: u32 = 8000;
const MAX_DECODE_BYTES: u64 = 128 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum PhotoError {
    #[cfg(feature = "fetch")]
    #[error("could not download photo: {0}")]
    Fetch(#[from] crate::fetch::FetchError),
    #[error("not an image we can read: {0}")]
    Decode(#[from] image::ImageError),
    #[cfg(feature = "fetch")]
    #[error("not a public web link")]
    NotPublic,
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

/// Downloads the photo at `url`. Refuses links that aren't to a public web
/// host, so a recipe file can't make the device probe its own network.
#[cfg(feature = "fetch")]
pub async fn fetch_photo(url: &str) -> Result<Vec<u8>, PhotoError> {
    if !crate::url::is_public_web_url(url) {
        return Err(PhotoError::NotPublic);
    }
    Ok(crate::fetch::get(url, MAX_PHOTO_BYTES).await?)
}

/// Decodes a photo in any common format, fits it within [`MAX_SIDE`], fixes
/// its orientation and re-encodes it as a JPEG. CPU-bound: call it off the
/// UI thread.
pub fn shrink_photo(bytes: &[u8]) -> Result<Vec<u8>, PhotoError> {
    use image::codecs::jpeg::JpegEncoder;
    use image::imageops::FilterType;
    use image::{DynamicImage, ImageDecoder, ImageReader, Limits};

    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_DECODE_SIDE);
    limits.max_image_height = Some(MAX_DECODE_SIDE);
    limits.max_alloc = Some(MAX_DECODE_BYTES);
    let mut reader = ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(image::ImageError::IoError)?;
    reader.limits(limits.clone());
    let mut decoder = reader.into_decoder()?;
    // The decoded pixels count too, as in `ImageReader::decode`.
    limits.reserve(decoder.total_bytes())?;
    let orientation = decoder.orientation()?;
    let mut photo = DynamicImage::from_decoder(decoder)?;
    // Shrink first, so rotating copies the small image, not the original.
    if photo.width() > MAX_SIDE || photo.height() > MAX_SIDE {
        photo = photo.resize(MAX_SIDE, MAX_SIDE, FilterType::Triangle);
    }
    photo.apply_orientation(orientation);

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

    #[cfg(feature = "fetch")]
    #[tokio::test]
    async fn never_fetches_private_hosts() {
        // A listener that must not be contacted: the check happens first.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}/photo.jpg", listener.local_addr().unwrap());
        for bad in [url.as_str(), "http://localhost/x.jpg", "http://nas.local/x.jpg", "file:///etc/hosts"] {
            assert!(matches!(fetch_photo(bad).await, Err(PhotoError::NotPublic)), "{bad}");
        }
        assert!(matches!(listener.accept(), Err(e) if e.kind() == std::io::ErrorKind::WouldBlock));
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
    fn rejects_huge_canvases() {
        for (w, h) in [(MAX_DECODE_SIDE + 1, 1), (1, MAX_DECODE_SIDE + 1)] {
            let err = shrink_photo(&png(w, h)).unwrap_err();
            assert!(matches!(err, PhotoError::Decode(image::ImageError::Limits(_))), "{w}x{h}: {err}");
        }
        assert!(shrink_photo(&png(MAX_DECODE_SIDE, 1)).is_ok());
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
