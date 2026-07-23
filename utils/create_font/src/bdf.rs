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
