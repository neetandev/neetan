//! Strict isolated common configuration for the automation frontend.
//!
//! The `--config` layer supplies only common host settings, chiefly the
//! per-family ROM directories. It does not select a machine. Layering is
//! `defaults -> --global-config -> --config -> command line`. The normal
//! Neetan settings file is never loaded implicitly. The parser rejects
//! malformed lines, unknown keys, invalid values, and duplicate keys within a
//! single file. Relative paths resolve against that file's directory.
//! A `roms-folder` value derives every ROM directory that no file sets.

use std::{
    collections::{BTreeMap, HashSet},
    path::{Path, PathBuf},
    sync::Arc,
};

use common::{FixedHostDateTime, HostDateTime, SharedHostDateTimeSource};
use machine_factory::config::{
    settings::{ROMS_FOLDER_KEY, Scope, find_setting},
    systems::{derived_rom_folder, rom_folders},
};

/// The default per-script wall-clock timeout in seconds.
pub const DEFAULT_TIMEOUT_SECONDS: u64 = 600;
/// The fixed audio output rate for every automated machine.
pub const AUDIO_SAMPLE_RATE: u32 = 48_000;

/// A fixed guest real-time-clock value.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GuestDateTime {
    /// Four-digit year.
    pub year: u16,
    /// Month, 1 through 12.
    pub month: u8,
    /// Day of month, 1 through 31.
    pub day: u8,
    /// Hour, 0 through 23.
    pub hour: u8,
    /// Minute, 0 through 59.
    pub minute: u8,
    /// Second, 0 through 59.
    pub second: u8,
}

impl Default for GuestDateTime {
    fn default() -> Self {
        Self {
            year: 2000,
            month: 1,
            day: 1,
            hour: 0,
            minute: 0,
            second: 0,
        }
    }
}

impl GuestDateTime {
    /// Parses an `YYYY-MM-DDThh:mm:ss` value.
    pub fn parse(value: &str) -> Result<Self, String> {
        let (date, time) = value
            .split_once('T')
            .ok_or_else(|| format!("invalid datetime (expected YYYY-MM-DDThh:mm:ss): {value}"))?;
        let date_parts: Vec<&str> = date.split('-').collect();
        let time_parts: Vec<&str> = time.split(':').collect();
        if date_parts.len() != 3 || time_parts.len() != 3 {
            return Err(format!(
                "invalid datetime (expected YYYY-MM-DDThh:mm:ss): {value}"
            ));
        }
        let year = parse_field(date_parts[0], "year", value)?;
        let month = parse_field(date_parts[1], "month", value)?;
        let day = parse_field(date_parts[2], "day", value)?;
        let hour = parse_field(time_parts[0], "hour", value)?;
        let minute = parse_field(time_parts[1], "minute", value)?;
        let second = parse_field(time_parts[2], "second", value)?;
        if !(1..=12).contains(&month)
            || !(1..=31).contains(&day)
            || hour > 23
            || minute > 59
            || second > 59
        {
            return Err(format!("datetime field out of range: {value}"));
        }
        Ok(Self {
            year,
            month: month as u8,
            day: day as u8,
            hour: hour as u8,
            minute: minute as u8,
            second: second as u8,
        })
    }
}

fn parse_field(text: &str, field: &str, value: &str) -> Result<u16, String> {
    text.parse::<u16>()
        .map_err(|_| format!("invalid {field} in datetime: {value}"))
}

impl GuestDateTime {
    /// Converts to a host date-time, deriving the day of week (0 = Sunday).
    #[must_use]
    pub fn to_host_date_time(self) -> HostDateTime {
        HostDateTime {
            year: self.year,
            month: self.month,
            day: self.day,
            day_of_week: day_of_week(self.year, self.month, self.day),
            hour: self.hour,
            minute: self.minute,
            second: self.second,
        }
    }
}

/// Computes the day of week (0 = Sunday) via Sakamoto's method.
fn day_of_week(year: u16, month: u8, day: u8) -> u8 {
    const MONTH_OFFSETS: [i32; 12] = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
    let mut adjusted_year = year as i32;
    if month < 3 {
        adjusted_year -= 1;
    }
    let index = (month - 1) as usize;
    let weekday = (adjusted_year + adjusted_year / 4 - adjusted_year / 100
        + adjusted_year / 400
        + MONTH_OFFSETS[index]
        + day as i32)
        .rem_euclid(7);
    weekday as u8
}

/// Common host settings shared by every script.
#[derive(Clone, Debug, Default)]
pub struct CommonConfig {
    /// The ROM directories by their settings key, for example `pc98-roms`.
    pub rom_folders: BTreeMap<&'static str, PathBuf>,
    /// The folder that derives every ROM directory that no file sets.
    pub roms_folder: Option<PathBuf>,
    /// Explicit artifact root override. When absent, the root is derived
    /// per-script as `<script-dir>/artifacts/<stem>`.
    pub artifact_root: Option<PathBuf>,
    /// Per-script wall-clock timeout in seconds.
    pub timeout_seconds: u64,
    /// Fixed guest real-time-clock value.
    pub guest_time: GuestDateTime,
}

impl CommonConfig {
    /// Creates a configuration populated with the documented defaults.
    #[must_use]
    pub fn with_defaults() -> Self {
        Self {
            timeout_seconds: DEFAULT_TIMEOUT_SECONDS,
            guest_time: GuestDateTime::default(),
            ..Self::default()
        }
    }

    /// Loads defaults, then the optional global-config, then the optional
    /// `--config` file, in that order.
    pub fn load(global: Option<&Path>, config: Option<&Path>) -> Result<Self, String> {
        let mut result = Self::with_defaults();
        if let Some(path) = global {
            result.apply_file(path)?;
        }
        if let Some(path) = config {
            result.apply_file(path)?;
        }
        result.derive_rom_folders();
        Ok(result)
    }

    /// Fills every unset ROM directory with its folder below `roms-folder`.
    fn derive_rom_folders(&mut self) {
        let Some(roms_folder) = &self.roms_folder else {
            return;
        };
        for folder in rom_folders() {
            if self.rom_folders.contains_key(folder.key) {
                continue;
            }
            if let Some(path) = derived_rom_folder(&folder, roms_folder) {
                self.rom_folders.insert(folder.key, path);
            }
        }
    }

    /// The fixed audio output rate.
    #[must_use]
    pub const fn audio_sample_rate(&self) -> u32 {
        AUDIO_SAMPLE_RATE
    }

    /// Returns the fixed guest real-time-clock source.
    #[must_use]
    pub fn host_date_time_source(&self) -> SharedHostDateTimeSource {
        Arc::new(FixedHostDateTime(self.guest_time.to_host_date_time()))
    }

    /// Returns the artifact root for one script.
    #[must_use]
    pub fn artifact_root_for(&self, script: &Path) -> PathBuf {
        if let Some(root) = &self.artifact_root {
            return root.clone();
        }
        let script_dir = script
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .map_or_else(|| PathBuf::from("."), Path::to_path_buf);
        let stem = script.file_stem().map_or_else(
            || "script".to_owned(),
            |stem| stem.to_string_lossy().into_owned(),
        );
        script_dir.join("artifacts").join(stem)
    }

    fn apply_file(&mut self, path: &Path) -> Result<(), String> {
        let text = std::fs::read_to_string(path)
            .map_err(|error| format!("cannot read config {}: {error}", path.display()))?;
        let base_dir = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .map_or_else(|| PathBuf::from("."), Path::to_path_buf);
        let mut seen: HashSet<String> = HashSet::new();
        for (index, raw_line) in text.lines().enumerate() {
            let line_number = index + 1;
            let line = raw_line.trim();
            if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
                continue;
            }
            let (key, value) = line.split_once('=').ok_or_else(|| {
                format!(
                    "{}:{line_number}: malformed line (expected key = value): {line}",
                    path.display()
                )
            })?;
            let key = key.trim().to_owned();
            let value = value.trim();
            if !seen.insert(key.clone()) {
                return Err(format!(
                    "{}:{line_number}: duplicate key '{key}'",
                    path.display()
                ));
            }
            self.apply_setting(&key, value, &base_dir, path, line_number)?;
        }
        Ok(())
    }

    fn apply_setting(
        &mut self,
        key: &str,
        value: &str,
        base_dir: &Path,
        path: &Path,
        line_number: usize,
    ) -> Result<(), String> {
        if key == ROMS_FOLDER_KEY {
            if value.is_empty() {
                return Err(format!(
                    "{}:{line_number}: '{key}' names no folder",
                    path.display()
                ));
            }
            self.roms_folder = Some(resolve_relative(base_dir, value));
            return Ok(());
        }
        if let Some(folder) = rom_folders().find(|folder| folder.key == key) {
            if value.is_empty() {
                self.rom_folders.remove(folder.key);
            } else {
                self.rom_folders
                    .insert(folder.key, resolve_relative(base_dir, value));
            }
            return Ok(());
        }
        if find_setting(key).is_some_and(|setting| setting.scope == Scope::Host) {
            return Err(format!(
                "{}:{line_number}: host key '{key}' has no effect in a headless session",
                path.display()
            ));
        }
        match key {
            "artifacts" => {
                self.artifact_root = Some(resolve_relative(base_dir, value));
                Ok(())
            }
            "timeout" => {
                self.timeout_seconds = value.parse::<u64>().map_err(|_| {
                    format!(
                        "{}:{line_number}: invalid timeout (expected seconds): {value}",
                        path.display()
                    )
                })?;
                Ok(())
            }
            "guest-time" => {
                self.guest_time = GuestDateTime::parse(value)
                    .map_err(|error| format!("{}:{line_number}: {error}", path.display()))?;
                Ok(())
            }
            _ => Err(format!(
                "{}:{line_number}: unknown key '{key}'",
                path.display()
            )),
        }
    }
}

fn resolve_relative(base_dir: &Path, value: &str) -> PathBuf {
    let candidate = Path::new(value);
    if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        base_dir.join(candidate)
    }
}
