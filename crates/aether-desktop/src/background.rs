//! Background image validation.
//!
//! AETHER accepts any valid image as a background. There is NO required
//! resolution: the renderer adapts to the actual viewport via CSS
//! `object-fit` (fill/fit/stretch/center). Validation only rejects files that
//! are genuinely unusable: undecodable data, zero dimensions, or images that
//! exceed practical safety limits.

/// Maximum accepted file size (25 MB). Guards against absurd payloads over IPC.
pub const MAX_FILE_SIZE: usize = 25 * 1024 * 1024;
/// Maximum accepted pixel dimension on either axis (8192). Well above any
/// real wallpaper (4K = 3840) while keeping decode cost bounded.
pub const MAX_DIMENSION: u32 = 8192;

/// Rejection returned when a chosen file is not a usable image.
#[derive(Debug, Clone)]
pub struct ImageError {
    pub message: String,
}

impl std::fmt::Display for ImageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ImageError {}

fn invalid() -> ImageError {
    ImageError {
        message: "Unable to use this image.\nPlease choose a supported image file (PNG or JPEG).".into(),
    }
}

/// Validate image bytes: must be a decodable PNG/JPEG with non-zero
/// dimensions within practical limits. Any resolution is accepted.
pub fn validate_bytes(bytes: &[u8]) -> Result<(u32, u32), ImageError> {
    if bytes.is_empty() || bytes.len() > MAX_FILE_SIZE {
        return Err(invalid());
    }
    let reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|_| invalid())?;
    let (w, h) = reader.into_dimensions().map_err(|_| invalid())?;
    if w == 0 || h == 0 || w > MAX_DIMENSION || h > MAX_DIMENSION {
        return Err(invalid());
    }
    Ok((w, h))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png_bytes(w: u32, h: u32) -> Vec<u8> {
        let img = image::RgbImage::new(w, h);
        let mut buf = std::io::Cursor::new(Vec::new());
        img.write_to(&mut buf, image::ImageFormat::Png).unwrap();
        buf.into_inner()
    }

    #[test]
    fn accepts_1920x1081() {
        let (w, h) = validate_bytes(&png_bytes(1920, 1081)).unwrap();
        assert_eq!((w, h), (1920, 1081));
    }

    #[test]
    fn accepts_various_resolutions_and_aspect_ratios() {
        for (w, h) in [(1920, 1080), (2560, 1440), (3840, 2160), (1366, 768), (1440, 900), (2560, 1600), (800, 600), (2560, 1080)] {
            assert!(validate_bytes(&png_bytes(w, h)).is_ok(), "{w}x{h} must be accepted");
        }
    }

    #[test]
    fn rejects_garbage_and_empty() {
        assert!(validate_bytes(&[]).is_err());
        assert!(validate_bytes(b"not an image at all").is_err());
    }
}
