//! Helpers shared by the host `neetan copy` CLI and the HLE `COPY`
//! command: 8.3 name validation, DOS path manipulation, and DOS
//! timestamp encoding.

use std::time::SystemTime;

/// Validates a single DOS file or directory name against the 8.3 ASCII
/// constraints and returns the canonical uppercase display form.
pub fn validate_dos_basename(name: &[u8]) -> Result<Vec<u8>, String> {
    if name.is_empty() {
        return Err("name is empty".to_string());
    }
    if name == b"." || name == b".." {
        return Err("name is reserved".to_string());
    }
    if name.iter().any(|b| !b.is_ascii() || (*b) < 0x20) {
        return Err("non-ASCII or control character".to_string());
    }
    const FORBIDDEN: &[u8] = b"<>:\"|?*/\\";
    if name.iter().any(|b| FORBIDDEN.contains(b)) {
        return Err("character is reserved in DOS names".to_string());
    }
    let dot = name.iter().rposition(|b| *b == b'.');
    let (base, ext) = match dot {
        Some(idx) if idx == name.len() - 1 => (&name[..idx], &[][..]),
        Some(idx) => (&name[..idx], &name[idx + 1..]),
        None => (name, &[][..]),
    };
    if base.is_empty() {
        return Err("name has no base".to_string());
    }
    if base.len() > 8 {
        return Err(format!("base '{}' exceeds 8 characters", show(base)));
    }
    if ext.len() > 3 {
        return Err(format!("extension '{}' exceeds 3 characters", show(ext)));
    }
    if ext.contains(&b'.') {
        return Err("extension may not contain '.'".to_string());
    }
    let mut display = Vec::with_capacity(base.len() + 1 + ext.len());
    display.extend(base.iter().map(|b| b.to_ascii_uppercase()));
    if !ext.is_empty() {
        display.push(b'.');
        display.extend(ext.iter().map(|b| b.to_ascii_uppercase()));
    }
    Ok(display)
}

fn show(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// Walks a DOS path and validates each component as a valid 8.3 ASCII name.
/// Accepts an optional drive letter prefix.
pub fn validate_dos_components(path: &[u8]) -> Result<(), String> {
    let mut pos = 0;
    if path.len() >= 2 && path[1] == b':' {
        let letter = path[0].to_ascii_uppercase();
        if !letter.is_ascii_uppercase() {
            return Err("invalid drive letter".to_string());
        }
        pos = 2;
    }
    let mut current = Vec::new();
    while pos < path.len() {
        let byte = path[pos];
        if byte == 0 {
            break;
        }
        if byte == b'\\' || byte == b'/' {
            if !current.is_empty() {
                validate_dos_basename(&current)?;
                current.clear();
            }
            pos += 1;
            continue;
        }
        current.push(byte);
        pos += 1;
    }
    if !current.is_empty() {
        validate_dos_basename(&current)?;
    }
    Ok(())
}

/// Joins a DOS directory path with a single 8.3 leaf name using `\` as the
/// separator. The directory path may or may not end with `\`; the leaf must
/// not contain separators.
pub fn join_dos(dir: &[u8], leaf: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(dir.len() + 1 + leaf.len());
    out.extend_from_slice(dir);
    if !dir.is_empty() && !dir.ends_with(b"\\") && !dir.ends_with(b"/") {
        out.push(b'\\');
    }
    out.extend_from_slice(leaf);
    out
}

/// Splits a DOS path into (parent, leaf). Returns `None` if there is no
/// parent (root-level leaf).
pub fn split_dos_parent(path: &[u8]) -> Option<Vec<u8>> {
    let stripped = trim_trailing_separator(path);
    let mut pos = 0;
    if stripped.len() >= 2 && stripped[1] == b':' {
        pos = 2;
    }
    let body = &stripped[pos..];
    let body_pos = body.iter().rposition(|b| *b == b'\\' || *b == b'/')?;
    let mut out = Vec::with_capacity(pos + body_pos);
    out.extend_from_slice(&stripped[..pos]);
    out.extend_from_slice(&body[..body_pos]);
    if out.is_empty() {
        return None;
    }
    Some(out)
}

pub fn trim_trailing_separator(path: &[u8]) -> &[u8] {
    let mut end = path.len();
    while end > 0 && (path[end - 1] == b'\\' || path[end - 1] == b'/') {
        end -= 1;
    }
    &path[..end]
}

pub fn dos_display(path: &[u8]) -> String {
    String::from_utf8_lossy(path).into_owned()
}

pub fn dos_leaf_basename(path: &[u8]) -> Option<Vec<u8>> {
    let stripped = trim_trailing_separator(path);
    let pos = stripped
        .iter()
        .rposition(|b| *b == b'\\' || *b == b'/' || *b == b':')
        .map(|i| i + 1)
        .unwrap_or(0);
    if pos >= stripped.len() {
        return None;
    }
    Some(stripped[pos..].to_vec())
}

/// Returns the current host time encoded as `(dos_time, dos_date)` using
/// the standard FAT packed representation.
pub fn dos_now() -> (u16, u16) {
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let (year, month, day, hour, minute, second) = unix_to_local(now);
    let dos_date = if year >= 1980 {
        ((year - 1980) << 9) | (month << 5) | day
    } else {
        0
    };
    let dos_time = (hour << 11) | (minute << 5) | (second / 2);
    (dos_time as u16, dos_date as u16)
}

/// Cheap conversion from a Unix timestamp to (Y, M, D, h, m, s) in UTC.
/// DOS timestamps have no timezone; using UTC matches the convention DOS
/// uses when the host clock is unsynced.
pub fn unix_to_local(secs: u64) -> (u32, u32, u32, u32, u32, u32) {
    let day_seconds = 86_400u64;
    let total_days = (secs / day_seconds) as i64;
    let time_of_day = (secs % day_seconds) as u32;
    let hour = time_of_day / 3600;
    let minute = (time_of_day % 3600) / 60;
    let second = time_of_day % 60;

    let mut days = total_days;
    let mut year: i64 = 1970;
    loop {
        let len = if is_leap(year) { 366 } else { 365 };
        if days < len {
            break;
        }
        days -= len;
        year += 1;
    }
    let month_lengths: [i64; 12] = [
        31,
        if is_leap(year) { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    let mut month = 1u32;
    for (idx, &len) in month_lengths.iter().enumerate() {
        if days < len {
            month = idx as u32 + 1;
            break;
        }
        days -= len;
    }
    let day = days as u32 + 1;
    (year as u32, month, day, hour, minute, second)
}

pub fn is_leap(year: i64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_dos_basename_accepts_8_3() {
        assert_eq!(validate_dos_basename(b"FILE.TXT").unwrap(), b"FILE.TXT");
        assert_eq!(validate_dos_basename(b"file.txt").unwrap(), b"FILE.TXT");
        assert_eq!(validate_dos_basename(b"ABCDEFGH").unwrap(), b"ABCDEFGH");
        assert_eq!(validate_dos_basename(b"A.B").unwrap(), b"A.B");
        assert_eq!(validate_dos_basename(b"NOEXT").unwrap(), b"NOEXT");
    }

    #[test]
    fn validate_dos_basename_rejects_too_long_base() {
        assert!(validate_dos_basename(b"too_long_name.txt").is_err());
        assert!(validate_dos_basename(b"ABCDEFGHI.TXT").is_err());
    }

    #[test]
    fn validate_dos_basename_rejects_too_long_ext() {
        assert!(validate_dos_basename(b"file.text").is_err());
    }

    #[test]
    fn validate_dos_basename_rejects_reserved_chars() {
        for &c in b"<>:\"|?*/\\" {
            let mut name = b"FILE".to_vec();
            name.push(c);
            assert!(
                validate_dos_basename(&name).is_err(),
                "char {c:#04x} should be rejected"
            );
        }
    }

    #[test]
    fn validate_dos_basename_rejects_empty_and_dot() {
        assert!(validate_dos_basename(b"").is_err());
        assert!(validate_dos_basename(b".").is_err());
        assert!(validate_dos_basename(b"..").is_err());
        assert!(validate_dos_basename(b".only_ext").is_err());
    }

    #[test]
    fn validate_dos_basename_rejects_non_ascii() {
        assert!(validate_dos_basename(b"f\xE3\x81\x82le.txt").is_err());
        assert!(validate_dos_basename(b"FILE.\x01XT").is_err());
    }

    #[test]
    fn validate_dos_components_walks_components() {
        assert!(validate_dos_components(b"A:\\FOO\\BAR.TXT").is_ok());
        assert!(validate_dos_components(b"\\FOO\\BAR.TXT").is_ok());
        assert!(validate_dos_components(b"BAR.TXT").is_ok());
        assert!(validate_dos_components(b"").is_ok());
        assert!(validate_dos_components(b"A:\\toolong_name").is_err());
        assert!(validate_dos_components(b"\\FOO\\toolong_name\\X.TXT").is_err());
    }

    #[test]
    fn join_dos_concatenates_with_backslash() {
        assert_eq!(join_dos(b"A:", b"FOO"), b"A:\\FOO");
        assert_eq!(join_dos(b"A:\\FOO", b"BAR"), b"A:\\FOO\\BAR");
        assert_eq!(join_dos(b"A:\\FOO\\", b"BAR"), b"A:\\FOO\\BAR");
        assert_eq!(join_dos(b"", b"FOO"), b"FOO");
    }

    #[test]
    fn trim_trailing_separator_strips_slashes() {
        assert_eq!(trim_trailing_separator(b"A:\\FOO\\"), b"A:\\FOO");
        assert_eq!(trim_trailing_separator(b"A:\\FOO"), b"A:\\FOO");
        assert_eq!(trim_trailing_separator(b"/foo//"), b"/foo");
        assert_eq!(trim_trailing_separator(b""), b"");
    }

    #[test]
    fn split_dos_parent_returns_dirname() {
        assert_eq!(
            split_dos_parent(b"A:\\FOO\\BAR.TXT"),
            Some(b"A:\\FOO".to_vec())
        );
        assert_eq!(split_dos_parent(b"A:\\BAR.TXT"), Some(b"A:".to_vec()));
        assert_eq!(split_dos_parent(b"BAR.TXT"), None);
        assert_eq!(
            split_dos_parent(b"\\BAR.TXT"),
            Some(b"".to_vec()).filter(|p| !p.is_empty())
        );
    }

    #[test]
    fn dos_leaf_basename_returns_last_component() {
        assert_eq!(
            dos_leaf_basename(b"A:\\FOO\\BAR.TXT"),
            Some(b"BAR.TXT".to_vec())
        );
        assert_eq!(dos_leaf_basename(b"A:\\FOO\\"), Some(b"FOO".to_vec()));
        assert_eq!(dos_leaf_basename(b"BAR.TXT"), Some(b"BAR.TXT".to_vec()));
        assert_eq!(dos_leaf_basename(b"\\"), None);
    }

    #[test]
    fn dos_now_encodes_current_year() {
        let (_time, date) = dos_now();
        let year = (date >> 9) + 1980;
        assert!((2025..2200).contains(&year), "got year {year}");
    }
}
