//! The link as a QR code, as pixels for the clipboard (never in a window). Low
//! error correction keeps it small (a screen is not scratched), with the
//! 4-module quiet zone the standard asks for.

use anyhow::Result;
use qrcode::{Color, EcLevel, QrCode};

const QUIET_ZONE: usize = 4;

/// The code as a square grid of modules, `true` for dark, quiet zone included.
fn modules(text: &str) -> Result<Vec<Vec<bool>>> {
    let code = QrCode::with_error_correction_level(text, EcLevel::L)?;
    let width = code.width();
    let colors = code.to_colors();
    let size = width + 2 * QUIET_ZONE;
    let mut grid = vec![vec![false; size]; size];
    for (index, color) in colors.iter().enumerate() {
        grid[QUIET_ZONE + index / width][QUIET_ZONE + index % width] = *color == Color::Dark;
    }
    Ok(grid)
}

/// The code as RGBA pixels, `scale` pixels per module: (pixels, width, height).
pub fn rgba(text: &str, scale: usize) -> Result<(Vec<u8>, u32, u32)> {
    let grid = modules(text)?;
    let side = grid.len() * scale;
    let mut pixels = Vec::with_capacity(side * side * 4);
    for y in 0..side {
        for x in 0..side {
            let value = if grid[y / scale][x / scale] { 0 } else { 255 };
            pixels.extend_from_slice(&[value, value, value, 255]);
        }
    }
    let side = u32::try_from(side)?;
    Ok((pixels, side, side))
}

#[cfg(test)]
mod tests {
    use super::*;

    const LINK: &str = "https://ourstick.com/s/#EXAMPLEPUBLICPART.examplesecretpart";

    #[test]
    fn the_pixels_match_the_modules_at_any_scale() {
        let grid = modules(LINK).unwrap();
        let (pixels, width, height) = rgba(LINK, 3).unwrap();
        assert_eq!(width as usize, grid.len() * 3);
        assert_eq!(pixels.len(), (width * height * 4) as usize);
        for (row, cells) in grid.iter().enumerate() {
            for (column, dark) in cells.iter().enumerate() {
                let at = ((row * 3 + 1) * width as usize + column * 3 + 1) * 4;
                assert_eq!(pixels[at] == 0, *dark, "module {row},{column}");
            }
        }
        // The quiet zone is white.
        assert_eq!(&pixels[..4], &[255, 255, 255, 255]);
    }
}
