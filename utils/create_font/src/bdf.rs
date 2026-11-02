use std::collections::HashMap;

pub struct BitmapFont {
    ascent: i32,
    glyphs: HashMap<u32, Glyph>,
}

struct Glyph {
    rows: Vec<u8>,
    bbx_width: u32,
    bbx_height: u32,
    bbx_x_offset: i32,
    bbx_y_offset: i32,
}

impl BitmapFont {
    pub fn parse(source: &str) -> Self {
        let mut ascent = 14;
        let mut glyphs = HashMap::new();

        let mut encoding = None;
        let mut bbx = (0u32, 0u32, 0i32, 0i32);
        let mut in_bitmap = false;
        let mut bitmap_rows: Vec<u8> = Vec::new();
        let mut bbx_width = 0u32;

        for raw_line in source.lines() {
            let line = match raw_line.find('#') {
                Some(pos) => raw_line[..pos].trim(),
                None => raw_line.trim(),
            };

            if let Some(value) = line.strip_prefix("FONT_ASCENT ") {
                ascent = value.trim().parse().unwrap_or(14);
            } else if let Some(value) = line.strip_prefix("ENCODING ") {
                encoding = value.trim().parse::<u32>().ok();
            } else if let Some(value) = line.strip_prefix("BBX ") {
                let parts: Vec<&str> = value.split_whitespace().collect();
                if parts.len() >= 4 {
                    bbx = (
                        parts[0].parse().unwrap_or(0),
                        parts[1].parse().unwrap_or(0),
                        parts[2].parse().unwrap_or(0),
                        parts[3].parse().unwrap_or(0),
                    );
                }
            } else if line == "BITMAP" {
                in_bitmap = true;
                bitmap_rows.clear();
                bbx_width = bbx.0;
            } else if line == "ENDCHAR" {
                if in_bitmap && let Some(enc) = encoding {
                    glyphs.insert(
                        enc,
                        Glyph {
                            rows: bitmap_rows.clone(),
                            bbx_width,
                            bbx_height: bbx.1,
                            bbx_x_offset: bbx.2,
                            bbx_y_offset: bbx.3,
                        },
                    );
                }
                in_bitmap = false;
                encoding = None;
            } else if in_bitmap {
                let bytes_per_row = (bbx_width as usize).div_ceil(8);
                let mut row_bytes = vec![0u8; bytes_per_row];
                for (i, ch) in line.chars().enumerate() {
                    if ch == '@' {
                        row_bytes[i / 8] |= 0x80 >> (i % 8);
                    }
                }
                bitmap_rows.extend_from_slice(&row_bytes);
            }
        }

        BitmapFont { ascent, glyphs }
    }

    pub fn merge(&mut self, overlay: &BitmapFont) {
        for (&encoding, glyph) in &overlay.glyphs {
            self.glyphs.insert(
                encoding,
                Glyph {
                    rows: glyph.rows.clone(),
                    bbx_width: glyph.bbx_width,
                    bbx_height: glyph.bbx_height,
                    bbx_x_offset: glyph.bbx_x_offset,
                    bbx_y_offset: glyph.bbx_y_offset,
                },
            );
        }
    }

    pub fn get_8x8(&self, encoding: u32) -> Option<[u8; 8]> {
        let glyph = self.glyphs.get(&encoding)?;
        let mut cell = [0u8; 8];

        let glyph_top_from_baseline = glyph.bbx_y_offset + glyph.bbx_height as i32;
        let cell_row_start = self.ascent - glyph_top_from_baseline;
        let x_offset = glyph.bbx_x_offset;

        let bytes_per_row = (glyph.bbx_width as usize).div_ceil(8);

        for r in 0..glyph.bbx_height as usize {
            let cell_row = cell_row_start + r as i32;
            if !(0..8).contains(&cell_row) {
                continue;
            }

            let src_byte = glyph.rows.get(r * bytes_per_row)?;
            if x_offset == 0 {
                cell[cell_row as usize] = *src_byte;
            } else if x_offset > 0 {
                cell[cell_row as usize] = src_byte >> x_offset;
            } else {
                cell[cell_row as usize] = src_byte << (-x_offset);
            }
        }

        Some(cell)
    }

    pub fn get_8x16(&self, encoding: u32) -> Option<[u8; 16]> {
        let glyph = self.glyphs.get(&encoding)?;
        let mut cell = [0u8; 16];

        let glyph_top_from_baseline = glyph.bbx_y_offset + glyph.bbx_height as i32;
        let cell_row_start = self.ascent - glyph_top_from_baseline;
        let x_offset = glyph.bbx_x_offset;

        let bytes_per_row = (glyph.bbx_width as usize).div_ceil(8);

        for r in 0..glyph.bbx_height as usize {
            let cell_row = cell_row_start + r as i32;
            if !(0..16).contains(&cell_row) {
                continue;
            }

            let src_byte = glyph.rows.get(r * bytes_per_row)?;
            if x_offset == 0 {
                cell[cell_row as usize] = *src_byte;
            } else if x_offset > 0 {
                cell[cell_row as usize] = src_byte >> x_offset;
            } else {
                cell[cell_row as usize] = src_byte << (-x_offset);
            }
        }

        Some(cell)
    }

    /// Extracts a baseline-aligned 8x14 cell for the given encoding.
    pub fn get_8x14(&self, encoding: u32) -> Option<[u8; 14]> {
        let glyph = self.glyphs.get(&encoding)?;
        let mut cell = [0u8; 14];

        let glyph_top_from_baseline = glyph.bbx_y_offset + glyph.bbx_height as i32;
        let cell_row_start = self.ascent - glyph_top_from_baseline;
        let x_offset = glyph.bbx_x_offset;

        let bytes_per_row = (glyph.bbx_width as usize).div_ceil(8);

        for r in 0..glyph.bbx_height as usize {
            let cell_row = cell_row_start + r as i32;
            if !(0..14).contains(&cell_row) {
                continue;
            }

            let src_byte = glyph.rows.get(r * bytes_per_row)?;
            if x_offset == 0 {
                cell[cell_row as usize] = *src_byte;
            } else if x_offset > 0 {
                cell[cell_row as usize] = src_byte >> x_offset;
            } else {
                cell[cell_row as usize] = src_byte << (-x_offset);
            }
        }

        Some(cell)
    }

    /// Returns the encodings of all glyphs in ascending order.
    pub fn encodings(&self) -> Vec<u32> {
        let mut encodings: Vec<u32> = self.glyphs.keys().copied().collect();
        encodings.sort_unstable();
        encodings
    }

    /// Extracts a baseline-aligned 8x12 cell for the given encoding.
    pub fn get_8x12(&self, encoding: u32) -> Option<[u8; 12]> {
        let mut cell = [0u8; 12];
        cell.copy_from_slice(&self.cell_in_row_order(encoding, 8, 12)?);
        Some(cell)
    }

    /// Extracts a baseline-aligned 12x12 cell for the given encoding: two bytes
    /// per row, bits 3-0 of the right byte zero.
    pub fn get_12x12(&self, encoding: u32) -> Option<[u8; 24]> {
        let mut cell = [0u8; 24];
        cell.copy_from_slice(&self.cell_in_row_order(encoding, 12, 12)?);
        Some(cell)
    }

    /// Extracts a baseline-aligned 16x16 cell in row order: byte `2r` holds the
    /// left and byte `2r + 1` the right 8 pixels of row `r`.
    pub fn get_16x16_rows(&self, encoding: u32) -> Option<[u8; 32]> {
        let mut cell = [0u8; 32];
        cell.copy_from_slice(&self.cell_in_row_order(encoding, 16, 16)?);
        Some(cell)
    }

    /// Places a glyph into a cell of `width` x `height` pixels, stored row by
    /// row with `width.div_ceil(8)` bytes per row and bit 7 as the leftmost
    /// pixel. Pixels outside the cell are dropped.
    fn cell_in_row_order(&self, encoding: u32, width: usize, height: usize) -> Option<Vec<u8>> {
        let glyph = self.glyphs.get(&encoding)?;
        let cell_bytes_per_row = width.div_ceil(8);
        let mut cell = vec![0u8; cell_bytes_per_row * height];

        let glyph_top_from_baseline = glyph.bbx_y_offset + glyph.bbx_height as i32;
        let cell_row_start = self.ascent - glyph_top_from_baseline;
        let glyph_bytes_per_row = (glyph.bbx_width as usize).div_ceil(8);

        for row in 0..glyph.bbx_height as usize {
            let cell_row = cell_row_start + row as i32;
            if !(0..height as i32).contains(&cell_row) {
                continue;
            }
            for column in 0..glyph.bbx_width as usize {
                let byte = glyph.rows.get(row * glyph_bytes_per_row + column / 8)?;
                if byte & (0x80 >> (column % 8)) == 0 {
                    continue;
                }
                let cell_column = column as i32 + glyph.bbx_x_offset;
                if !(0..width as i32).contains(&cell_column) {
                    continue;
                }
                let cell_column = cell_column as usize;
                cell[cell_row as usize * cell_bytes_per_row + cell_column / 8] |=
                    0x80 >> (cell_column % 8);
            }
        }

        Some(cell)
    }

    pub fn get_16x16(&self, encoding: u32) -> Option<[u8; 32]> {
        let glyph = self.glyphs.get(&encoding)?;
        let mut cell = [0u8; 32];

        let glyph_top_from_baseline = glyph.bbx_y_offset + glyph.bbx_height as i32;
        let cell_row_start = self.ascent - glyph_top_from_baseline;

        let bytes_per_row = (glyph.bbx_width as usize).div_ceil(8);

        for r in 0..glyph.bbx_height as usize {
            let cell_row = cell_row_start + r as i32;
            if !(0..16).contains(&cell_row) {
                continue;
            }

            let src_offset = r * bytes_per_row;

            // V98 format: first 16 bytes = left column, next 16 bytes = right column
            if let Some(&left) = glyph.rows.get(src_offset) {
                cell[cell_row as usize] = left;
            }
            if bytes_per_row >= 2
                && let Some(&right) = glyph.rows.get(src_offset + 1)
            {
                cell[16 + cell_row as usize] = right;
            }
        }

        Some(cell)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a one-glyph font with a full-cell bounding box.
    fn font(width: u32, height: u32, rows: &[&str]) -> BitmapFont {
        let mut source = format!(
            "STARTFONT 2.1\nFONT_ASCENT {height}\nSTARTCHAR test\nENCODING 65\n\
             BBX {width} {height} 0 0\nBITMAP\n"
        );
        for row in rows {
            source.push_str(row);
            source.push('\n');
        }
        source.push_str("ENDCHAR\nENDFONT\n");
        BitmapFont::parse(&source)
    }

    #[test]
    fn get_16x16_rows_interleaves_left_and_right_bytes() {
        let mut rows = vec!["................"; 16];
        rows[0] = "@..............@";
        rows[15] = ".@.....@@.....@.";
        let cell = font(16, 16, &rows).get_16x16_rows(65).unwrap();
        assert_eq!(&cell[0..2], &[0x80, 0x01]);
        assert_eq!(&cell[30..32], &[0x41, 0x82]);
        assert!(cell[2..30].iter().all(|&byte| byte == 0));
    }

    #[test]
    fn get_12x12_packs_two_bytes_per_row() {
        let mut rows = vec!["............"; 12];
        rows[0] = "@..........@";
        rows[11] = "@@@@@@@@@@@@";
        let cell = font(12, 12, &rows).get_12x12(65).unwrap();
        assert_eq!(&cell[0..2], &[0x80, 0x10]);
        assert_eq!(&cell[22..24], &[0xFF, 0xF0]);
    }

    #[test]
    fn get_8x12_returns_one_byte_per_row() {
        let mut rows = vec!["........"; 12];
        rows[3] = "@......@";
        let cell = font(8, 12, &rows).get_8x12(65).unwrap();
        assert_eq!(cell[3], 0x81);
        assert_eq!(cell.iter().filter(|&&byte| byte != 0).count(), 1);
    }

    #[test]
    fn encodings_are_sorted() {
        let source = "FONT_ASCENT 8\nENCODING 9\nBBX 8 8 0 0\nBITMAP\nENDCHAR\n\
                      ENCODING 3\nBBX 8 8 0 0\nBITMAP\nENDCHAR\n";
        assert_eq!(BitmapFont::parse(source).encodings(), vec![3, 9]);
    }
}
