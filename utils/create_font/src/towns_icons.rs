//! Parser of the FM Towns system icon art (`utils/bios_towns/icons.txt`).
//!
//! The art holds 128 icons of 32 x 32 pixels in slot order. Each icon starts
//! with a line `icon <slot> <name>` followed by 32 rows of 32 pixels: `#` is a
//! black pixel, `.` a white pixel and `-` a transparent pixel. Lines that start
//! with `#` outside an icon and empty lines are comments.
//!
//! The binary layout matches the icon area of the system ROM: 256 bytes per
//! icon, a 128-byte pattern followed by a 128-byte mask, one bit per pixel with
//! the leftmost pixel in the most significant bit. A black pixel sets the
//! pattern bit. A transparent pixel sets the mask bit.

/// Number of icons in the icon area.
pub const ICON_COUNT: usize = 128;
/// Icon width and height in pixels.
pub const ICON_SIZE: usize = 32;
/// Bytes of one icon: pattern and mask.
pub const ICON_BYTES: usize = 256;
/// Bytes of one bit plane of an icon.
const PLANE_BYTES: usize = ICON_BYTES / 2;

/// Parses the icon art and returns the binary icon area (`ICON_COUNT *
/// ICON_BYTES` bytes).
pub fn parse_icons(source: &str) -> Result<Vec<u8>, String> {
    let mut data = Vec::with_capacity(ICON_COUNT * ICON_BYTES);
    let mut lines = source.lines().enumerate();
    while let Some((number, line)) = lines.next() {
        let Some(header) = line.strip_prefix("icon ") else {
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            return Err(format!(
                "line {}: unexpected text outside an icon",
                number + 1
            ));
        };
        let slot: usize = header
            .split_whitespace()
            .next()
            .and_then(|value| value.parse().ok())
            .ok_or_else(|| format!("line {}: icon header without a slot", number + 1))?;
        let expected = data.len() / ICON_BYTES;
        if slot != expected {
            return Err(format!(
                "line {}: icon {slot} found, icon {expected} expected",
                number + 1
            ));
        }
        let mut icon = [0u8; ICON_BYTES];
        for row in 0..ICON_SIZE {
            let (number, line) = lines
                .next()
                .ok_or_else(|| format!("icon {slot}: row {row} is missing"))?;
            let pixels = line.as_bytes();
            if pixels.len() != ICON_SIZE {
                return Err(format!(
                    "line {}: a row has {} pixels, {ICON_SIZE} expected",
                    number + 1,
                    pixels.len()
                ));
            }
            for (column, pixel) in pixels.iter().enumerate() {
                let byte = row * ICON_SIZE / 8 + column / 8;
                let bit = 0x80 >> (column % 8);
                match pixel {
                    b'#' => icon[byte] |= bit,
                    b'.' => {}
                    b'-' => icon[PLANE_BYTES + byte] |= bit,
                    other => {
                        return Err(format!(
                            "line {}: unknown pixel '{}'",
                            number + 1,
                            char::from(*other)
                        ));
                    }
                }
            }
        }
        data.extend_from_slice(&icon);
    }
    if data.len() != ICON_COUNT * ICON_BYTES {
        return Err(format!(
            "{} icons found, {ICON_COUNT} expected",
            data.len() / ICON_BYTES
        ));
    }
    Ok(data)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds an icon art source with `count` icons whose rows all equal `row`.
    fn source(count: usize, row: &str) -> String {
        let mut text = String::from("# comment\n");
        for slot in 0..count {
            text.push_str(&format!("\nicon {slot} test\n"));
            for _ in 0..ICON_SIZE {
                text.push_str(row);
                text.push('\n');
            }
        }
        text
    }

    #[test]
    fn pixels_map_to_pattern_and_mask_bits() {
        let row = format!("#.-{}", ".".repeat(ICON_SIZE - 3));
        let data = parse_icons(&source(ICON_COUNT, &row)).unwrap();
        assert_eq!(data.len(), ICON_COUNT * ICON_BYTES);
        assert_eq!(&data[0..4], &[0x80, 0, 0, 0]);
        assert_eq!(&data[PLANE_BYTES..PLANE_BYTES + 4], &[0x20, 0, 0, 0]);
        assert_eq!(data[ICON_BYTES - 4], 0x20);
    }

    #[test]
    fn missing_icons_are_rejected() {
        let row = ".".repeat(ICON_SIZE);
        assert!(parse_icons(&source(ICON_COUNT - 1, &row)).is_err());
    }

    #[test]
    fn short_rows_and_unknown_pixels_are_rejected() {
        assert!(parse_icons(&source(1, &".".repeat(ICON_SIZE - 1))).is_err());
        assert!(parse_icons(&source(1, &"x".repeat(ICON_SIZE))).is_err());
    }
}
