//! The ticket as a QR code in the terminal: two rows of modules per line of
//! text, with half-block characters, black on white whatever the terminal's
//! colours, and the quiet zone the standard asks for.

use anyhow::Result;
use qrcode::{Color, EcLevel, QrCode};

/// Light modules around the code, as the QR standard requires.
const QUIET_ZONE: usize = 4;
/// Black text on a bright white background, then back to normal.
const COLOURS_ON: &str = "\u{1b}[30;107m";
const COLOURS_OFF: &str = "\u{1b}[0m";

/// The QR code as a grid of modules, `true` for dark, quiet zone included.
fn modules(text: &str) -> Result<Vec<Vec<bool>>> {
    // Low error correction keeps the code small; a screen is not scratched.
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

/// One character for a dark/light module above and one below.
fn half_block(top: bool, bottom: bool) -> char {
    match (top, bottom) {
        (true, true) => '█',
        (true, false) => '▀',
        (false, true) => '▄',
        (false, false) => ' ',
    }
}

/// Lines to print for a QR code of `text`.
pub fn qr_lines(text: &str) -> Result<Vec<String>> {
    let grid = modules(text)?;
    let light_row = vec![false; grid.len()];
    Ok(grid
        .chunks(2)
        .map(|rows| {
            let bottom = rows.get(1).unwrap_or(&light_row);
            let line: String = rows[0]
                .iter()
                .zip(bottom)
                .map(|(top, bottom)| half_block(*top, *bottom))
                .collect();
            format!("{COLOURS_ON}{line}{COLOURS_OFF}")
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reads the printed lines back into a module grid.
    fn read_back(lines: &[String]) -> Vec<Vec<bool>> {
        let mut grid = Vec::new();
        for line in lines {
            let inner = line
                .strip_prefix(COLOURS_ON)
                .and_then(|line| line.strip_suffix(COLOURS_OFF))
                .expect("every line is coloured");
            let (top, bottom): (Vec<bool>, Vec<bool>) = inner
                .chars()
                .map(|character| match character {
                    '█' => (true, true),
                    '▀' => (true, false),
                    '▄' => (false, true),
                    ' ' => (false, false),
                    other => panic!("unexpected {other:?}"),
                })
                .unzip();
            grid.push(top);
            grid.push(bottom);
        }
        grid
    }

    const LINK: &str = "https://ourstick.com/s/#EXAMPLEPUBLICPART.examplesecretpart";

    #[test]
    fn lines_hold_exactly_the_code_and_its_quiet_zone() {
        let grid = modules(LINK).unwrap();
        let mut printed = read_back(&qr_lines(LINK).unwrap());
        // An odd number of rows gets a light row below.
        if grid.len() % 2 == 1 {
            assert!(printed.pop().unwrap().iter().all(|dark| !dark));
        }
        assert_eq!(printed, grid);
    }

    #[test]
    fn quiet_zone_is_light_and_the_finder_pattern_is_where_it_belongs() {
        let grid = modules(LINK).unwrap();
        let size = grid.len();
        for edge in 0..QUIET_ZONE {
            assert!(grid[edge].iter().all(|dark| !dark));
            assert!(grid[size - 1 - edge].iter().all(|dark| !dark));
            assert!(grid.iter().all(|row| !row[edge] && !row[size - 1 - edge]));
        }
        // The top-left finder pattern: a dark 7×7 ring.
        let corner = |row: usize, column: usize| grid[QUIET_ZONE + row][QUIET_ZONE + column];
        assert!((0..7).all(|index| corner(0, index) && corner(6, index)));
        assert!((0..7).all(|index| corner(index, 0) && corner(index, 6)));
        assert!(!corner(1, 1));
    }

    #[test]
    fn a_real_ticket_fits_in_a_terminal() {
        // As long as a real ticket with IPv4 and IPv6 address hints (417).
        let link = format!(
            "https://ourstick.com/s/#{}.{}",
            "A".repeat(360),
            "b".repeat(52)
        );
        let lines = qr_lines(&link).unwrap();
        let width = lines[0].chars().count() - COLOURS_ON.len() - COLOURS_OFF.len();
        assert!(width <= 80, "{width} columns");
        assert!(lines.len() <= 45, "{} lines", lines.len());
    }
}
