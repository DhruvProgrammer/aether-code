//! Background image dimension validation (spec §13-§14).
//!
//! AETHER enforces a single canonical background resolution. The desktop binary,
//! the renderer and any future tooling must all compare against the same numbers.
//! We do NOT auto-resize, crop, stretch, or upscale invalid images.

use aether_config::{BACKGROUND_HEIGHT, BACKGROUND_WIDTH};

/// Rejection returned to the user when a chosen image does not match the
/// canonical AETHER background resolution.
#[derive(Debug, Clone)]
pub struct DimensionError {
    pub required_w: u32,
    pub required_h: u32,
    pub got_w: u32,
    pub got_h: u32,
}

impl std::fmt::Display for DimensionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Invalid background image.\n\nAETHER requires:\n{} x {} px\n\nYour image:\n{} x {} px\n\nPlease select an image with the required dimensions.",
            self.required_w, self.required_h, self.got_w, self.got_h
        )
    }
}

impl std::error::Error for DimensionError {}

/// Inspect an image at `path` and reject anything that is not exactly
/// `BACKGROUND_WIDTH x BACKGROUND_HEIGHT`.
#[allow(dead_code)]
pub fn validate_dimensions(path: &std::path::Path) -> Result<(u32, u32), DimensionError> {
    let reader = image::image_dimensions(path).map_err(|_| DimensionError {
        required_w: BACKGROUND_WIDTH,
        required_h: BACKGROUND_HEIGHT,
        got_w: 0,
        got_h: 0,
    })?;
    let (w, h) = reader;
    if w != BACKGROUND_WIDTH || h != BACKGROUND_HEIGHT {
        return Err(DimensionError {
            required_w: BACKGROUND_WIDTH,
            required_h: BACKGROUND_HEIGHT,
            got_w: w,
            got_h: h,
        });
    }
    Ok((w, h))
}

/// As above, but reads directly from a byte slice (used by the frontend upload
/// path, which sends the file over IPC as bytes).
pub fn validate_dimensions_bytes(bytes: &[u8]) -> Result<(u32, u32), DimensionError> {
    let reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|_| DimensionError {
            required_w: BACKGROUND_WIDTH,
            required_h: BACKGROUND_HEIGHT,
            got_w: 0,
            got_h: 0,
        })?;
    let dims = reader.into_dimensions().map_err(|_| DimensionError {
        required_w: BACKGROUND_WIDTH,
        required_h: BACKGROUND_HEIGHT,
        got_w: 0,
        got_h: 0,
    })?;
    if dims.0 != BACKGROUND_WIDTH || dims.1 != BACKGROUND_HEIGHT {
        return Err(DimensionError {
            required_w: BACKGROUND_WIDTH,
            required_h: BACKGROUND_HEIGHT,
            got_w: dims.0,
            got_h: dims.1,
        });
    }
    Ok(dims)
}

/// The canonical resolution string the UI shows to the user.
pub fn required_resolution_label() -> String {
    format!("{BACKGROUND_WIDTH} x {BACKGROUND_HEIGHT} px")
}
