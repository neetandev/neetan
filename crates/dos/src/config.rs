//! CONFIG.SYS and AUTOEXEC.BAT parsing.

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ShellConfig {
    pub path: Vec<u8>,
    pub raw_arguments: Vec<u8>,
    pub initial_drive: Option<u8>,
    pub environment_size_bytes: Option<u16>,
    /// `/P` is preserved from CONFIG.SYS but does not change root HLE shell behavior.
    pub permanent: bool,
}

impl ShellConfig {
    fn parse(value: &[u8]) -> Option<Self> {
        let (path, arguments) = split_first_token(value);
        if path.is_empty() {
            return None;
        }

        let raw_arguments = arguments.to_vec();
        let mut initial_drive = None;
        let mut environment_size_bytes = None;
        let mut permanent = false;
        let mut remaining = arguments;

        while !remaining.is_empty() {
            let (token, rest) = split_first_token(remaining);
            remaining = rest;

            if token.is_empty() {
                continue;
            }

            if token.len() == 2 && token[1] == b':' && token[0].is_ascii_alphabetic() {
                initial_drive = Some(token[0].to_ascii_uppercase() - b'A');
                continue;
            }

            let upper_token: Vec<u8> = token.iter().map(|b| b.to_ascii_uppercase()).collect();

            if upper_token == b"/P" {
                permanent = true;
                continue;
            }

            if upper_token.starts_with(b"/E:")
                && upper_token.len() > 3
                && upper_token[3..].iter().all(|byte| byte.is_ascii_digit())
            {
                environment_size_bytes = parse_u16(&upper_token[3..]);
                continue;
            }

            if upper_token == b"/MSG" {
                // TODO: Honor /MSG for root COMMAND.COM compatibility.
                continue;
            }

            if upper_token == b"/Y" {
                // TODO: Honor /Y for root COMMAND.COM compatibility.
                continue;
            }
        }

        Some(Self {
            path: path.to_vec(),
            raw_arguments,
            initial_drive,
            environment_size_bytes,
            permanent,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DeviceDirective {
    Device,
    DeviceHigh,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ConfigDeviceLine {
    pub directive: DeviceDirective,
    pub raw_value: Vec<u8>,
    pub path: Vec<u8>,
    pub arguments: Vec<u8>,
}

/// Parsed CONFIG.SYS directives with defaults matching MS-DOS 6.20.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ConfigSys {
    /// Maximum number of open file handles (FILES=).
    pub files: u16,
    /// Number of DOS disk buffers (BUFFERS=).
    pub buffers: u16,
    /// Last valid drive letter as 1-based index: 1=A .. 26=Z (LASTDRIVE=).
    pub lastdrive: u8,
    /// Country code (COUNTRY=). 081 = Japan.
    pub country: u16,
    /// Extended Ctrl-Break checking (BREAK=).
    pub ctrl_break: bool,
    /// Parsed COMMAND.COM-compatible SHELL= configuration.
    pub shell: Option<ShellConfig>,
    /// MSCDEX device name extracted from DEVICE=NECCD.SYS /D:name.
    pub cdrom_device_name: Option<Vec<u8>>,
    /// Ordered DEVICE= and DEVICEHIGH= lines from CONFIG.SYS.
    pub devices: Vec<ConfigDeviceLine>,
}

impl Default for ConfigSys {
    fn default() -> Self {
        Self {
            files: 20,
            buffers: 15,
            lastdrive: 26,
            country: 81,
            ctrl_break: false,
            shell: None,
            cdrom_device_name: None,
            devices: Vec::new(),
        }
    }
}

/// Parses CONFIG.SYS file content into a `ConfigSys` struct.
pub(crate) fn parse_config_sys(data: &[u8]) -> ConfigSys {
    let lines = split_lines(data);
    let mut config = ConfigSys::default();

    for line in &lines {
        let trimmed = line.trim_ascii();
        if trimmed.is_empty() {
            continue;
        }
        // Comments: lines starting with ';' or 'REM '
        if trimmed[0] == b';' {
            continue;
        }
        let upper: Vec<u8> = trimmed.iter().map(|b| b.to_ascii_uppercase()).collect();
        if upper.starts_with(b"REM ") || upper == b"REM" {
            continue;
        }

        // Find '=' separator
        let eq_pos = match trimmed.iter().position(|&b| b == b'=') {
            Some(p) => p,
            None => continue,
        };
        let directive = upper[..eq_pos].trim_ascii();
        let value = &trimmed[eq_pos + 1..];
        let value_trimmed = value.trim_ascii();

        match directive {
            b"FILES" => {
                if let Some(n) = parse_u16(value_trimmed)
                    && n >= 8
                {
                    config.files = n;
                }
            }
            b"BUFFERS" => {
                if let Some(n) = parse_u16(value_trimmed)
                    && (1..=99).contains(&n)
                {
                    config.buffers = n;
                }
            }
            b"LASTDRIVE" if value_trimmed.len() == 1 => {
                let ch = value_trimmed[0].to_ascii_uppercase();
                if ch.is_ascii_uppercase() {
                    config.lastdrive = ch - b'A' + 1;
                }
            }
            b"COUNTRY" => {
                if let Some(n) = parse_u16(value_trimmed) {
                    config.country = n;
                }
            }
            b"BREAK" => {
                let val_upper: Vec<u8> = value_trimmed
                    .iter()
                    .map(|b| b.to_ascii_uppercase())
                    .collect();
                if val_upper == b"ON" {
                    config.ctrl_break = true;
                } else if val_upper == b"OFF" {
                    config.ctrl_break = false;
                }
            }
            b"SHELL" if !value_trimmed.is_empty() => {
                config.shell = ShellConfig::parse(value_trimmed);
            }
            b"DEVICE" | b"DEVICEHIGH" => {
                let directive = if directive == b"DEVICEHIGH" {
                    DeviceDirective::DeviceHigh
                } else {
                    DeviceDirective::Device
                };
                parse_device_line(directive, value_trimmed, &mut config);
            }
            _ => {
                // Unrecognized directive: silently ignored
            }
        }
    }

    config
}

/// Parses a DEVICE= value for NECCD.SYS / NECCDD.SYS driver recognition.
fn parse_device_line(directive: DeviceDirective, value: &[u8], config: &mut ConfigSys) {
    // Extract the filename (first token, may include path)
    let (path_token, rest) = split_first_token(value);
    if path_token.is_empty() {
        return;
    }

    config.devices.push(ConfigDeviceLine {
        directive,
        raw_value: value.to_vec(),
        path: path_token.to_vec(),
        arguments: rest.to_vec(),
    });

    let upper_path: Vec<u8> = path_token.iter().map(|b| b.to_ascii_uppercase()).collect();

    // Check if filename ends with NECCD.SYS or NECCDD.SYS
    let is_neccd = upper_path.ends_with(b"NECCD.SYS") || upper_path.ends_with(b"NECCDD.SYS");
    if !is_neccd {
        return;
    }

    // Look for /D:name parameter
    let mut i = 0;
    let rest_bytes = rest;
    while i < rest_bytes.len() {
        if rest_bytes[i] == b'/' && i + 2 < rest_bytes.len() {
            let flag = rest_bytes[i + 1].to_ascii_uppercase();
            if flag == b'D' && rest_bytes[i + 2] == b':' {
                // Extract device name (until next space or end)
                let name_start = i + 3;
                let name_end = rest_bytes[name_start..]
                    .iter()
                    .position(|&b| b == b' ' || b == b'\t')
                    .map(|p| name_start + p)
                    .unwrap_or(rest_bytes.len());
                if name_start < name_end {
                    config.cdrom_device_name = Some(rest_bytes[name_start..name_end].to_vec());
                }
                return;
            }
        }
        i += 1;
    }

    config.cdrom_device_name = Some(b"MSCD001".to_vec());
}

/// Splits the first whitespace-delimited token from the rest.
fn split_first_token(data: &[u8]) -> (&[u8], &[u8]) {
    let trimmed = data.trim_ascii_start();
    if let Some(pos) = trimmed.iter().position(|&b| b == b' ' || b == b'\t') {
        (&trimmed[..pos], trimmed[pos + 1..].trim_ascii_start())
    } else {
        (trimmed, &[])
    }
}

/// Splits raw file data into lines on \r\n or \n.
fn split_lines(data: &[u8]) -> Vec<Vec<u8>> {
    let mut lines = Vec::new();
    let mut current = Vec::new();
    for &byte in data {
        if byte == b'\n' {
            lines.push(current);
            current = Vec::new();
        } else if byte == b'\r' {
            // skip, we split on \n
        } else {
            current.push(byte);
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines
}

/// Parses an ASCII decimal number.
fn parse_u16(data: &[u8]) -> Option<u16> {
    if data.is_empty() {
        return None;
    }
    let mut result: u16 = 0;
    for &byte in data {
        if !byte.is_ascii_digit() {
            break;
        }
        result = result.checked_mul(10)?.checked_add((byte - b'0') as u16)?;
    }
    if result == 0 && data[0] != b'0' {
        return None;
    }
    Some(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_input_returns_defaults() {
        let config = parse_config_sys(b"");
        assert_eq!(config.files, 20);
        assert_eq!(config.buffers, 15);
        assert_eq!(config.lastdrive, 26);
        assert_eq!(config.country, 81);
        assert!(!config.ctrl_break);
        assert!(config.shell.is_none());
        assert!(config.cdrom_device_name.is_none());
        assert!(config.devices.is_empty());
    }

    #[test]
    fn parse_files() {
        let config = parse_config_sys(b"FILES=30\n");
        assert_eq!(config.files, 30);
    }

    #[test]
    fn parse_files_below_minimum_ignored() {
        let config = parse_config_sys(b"FILES=5\n");
        assert_eq!(config.files, 20); // default, 5 < 8
    }

    #[test]
    fn parse_buffers() {
        let config = parse_config_sys(b"BUFFERS=40\n");
        assert_eq!(config.buffers, 40);
    }

    #[test]
    fn parse_buffers_out_of_range_ignored() {
        let config = parse_config_sys(b"BUFFERS=0\n");
        assert_eq!(config.buffers, 15);
        let config = parse_config_sys(b"BUFFERS=100\n");
        assert_eq!(config.buffers, 15);
    }

    #[test]
    fn parse_lastdrive() {
        let config = parse_config_sys(b"LASTDRIVE=E\n");
        assert_eq!(config.lastdrive, 5); // E = 5th letter
    }

    #[test]
    fn parse_lastdrive_lowercase() {
        let config = parse_config_sys(b"LASTDRIVE=e\n");
        assert_eq!(config.lastdrive, 5);
    }

    #[test]
    fn parse_country() {
        let config = parse_config_sys(b"COUNTRY=001\n");
        assert_eq!(config.country, 1);
    }

    #[test]
    fn parse_break_on() {
        let config = parse_config_sys(b"BREAK=ON\n");
        assert!(config.ctrl_break);
    }

    #[test]
    fn parse_break_off() {
        let config = parse_config_sys(b"BREAK=OFF\n");
        assert!(!config.ctrl_break);
    }

    #[test]
    fn parse_shell() {
        let config = parse_config_sys(b"SHELL=C:\\COMMAND.COM /P\n");
        let shell = config.shell.expect("SHELL= should parse");
        assert_eq!(shell.path, b"C:\\COMMAND.COM");
        assert_eq!(shell.raw_arguments, b"/P");
        assert_eq!(shell.initial_drive, None);
        assert_eq!(shell.environment_size_bytes, None);
        assert!(shell.permanent);
    }

    #[test]
    fn parse_shell_command_com_contract() {
        let config = parse_config_sys(b"SHELL = A:COMMAND.COM A: /E:384 /P\n");
        let shell = config.shell.expect("SHELL= should parse");
        assert_eq!(shell.path, b"A:COMMAND.COM");
        assert_eq!(shell.raw_arguments, b"A: /E:384 /P");
        assert_eq!(shell.initial_drive, Some(0));
        assert_eq!(shell.environment_size_bytes, Some(384));
        assert!(shell.permanent);
    }

    #[test]
    fn parse_device_neccd() {
        let config = parse_config_sys(b"DEVICE=A:\\NECCD.SYS /D:CD001\n");
        assert_eq!(config.cdrom_device_name.as_deref(), Some(b"CD001".as_ref()));
        assert_eq!(config.devices.len(), 1);
        assert_eq!(config.devices[0].directive, DeviceDirective::Device);
        assert_eq!(config.devices[0].path, b"A:\\NECCD.SYS");
        assert_eq!(config.devices[0].arguments, b"/D:CD001");
    }

    #[test]
    fn parse_device_neccdd() {
        let config = parse_config_sys(b"DEVICE=A:\\DOS\\NECCDD.SYS /D:MYCDROM\n");
        assert_eq!(
            config.cdrom_device_name.as_deref(),
            Some(b"MYCDROM".as_ref())
        );
    }

    #[test]
    fn parse_device_neccd_no_d_param() {
        let config = parse_config_sys(b"DEVICE=NECCD.SYS\n");
        assert_eq!(
            config.cdrom_device_name.as_deref(),
            Some(b"MSCD001".as_ref())
        );
    }

    #[test]
    fn parse_devicehigh() {
        let config = parse_config_sys(b"DEVICEHIGH=A:\\NECCD.SYS /D:CD002\n");
        assert_eq!(config.cdrom_device_name.as_deref(), Some(b"CD002".as_ref()));
        assert_eq!(config.devices.len(), 1);
        assert_eq!(config.devices[0].directive, DeviceDirective::DeviceHigh);
    }

    #[test]
    fn unknown_device_preserved() {
        let config = parse_config_sys(b"DEVICE=MOUSE.SYS\n");
        assert!(config.cdrom_device_name.is_none());
        assert_eq!(config.devices.len(), 1);
        assert_eq!(config.devices[0].path, b"MOUSE.SYS");
    }

    #[test]
    fn device_lines_preserve_order() {
        let config =
            parse_config_sys(b"DEVICE=FIRST.SYS /A\nDEVICEHIGH=C:\\DOS\\SECOND.SYS /B /C\n");
        assert_eq!(config.devices.len(), 2);
        assert_eq!(config.devices[0].directive, DeviceDirective::Device);
        assert_eq!(config.devices[0].raw_value, b"FIRST.SYS /A");
        assert_eq!(config.devices[0].path, b"FIRST.SYS");
        assert_eq!(config.devices[0].arguments, b"/A");
        assert_eq!(config.devices[1].directive, DeviceDirective::DeviceHigh);
        assert_eq!(config.devices[1].path, b"C:\\DOS\\SECOND.SYS");
        assert_eq!(config.devices[1].arguments, b"/B /C");
    }

    #[test]
    fn comments_and_blank_lines_skipped() {
        let input = b"; This is a comment\n\nREM Another comment\nFILES=25\n";
        let config = parse_config_sys(input);
        assert_eq!(config.files, 25);
    }

    #[test]
    fn case_insensitive_directives() {
        let config = parse_config_sys(b"files=30\nbuffers=20\nbreak=on\n");
        assert_eq!(config.files, 30);
        assert_eq!(config.buffers, 20);
        assert!(config.ctrl_break);
    }

    #[test]
    fn mixed_valid_and_invalid_lines() {
        let input = b"FILES=30\nGARBAGE LINE\nBUFFERS=20\nNOEQUALS\n";
        let config = parse_config_sys(input);
        assert_eq!(config.files, 30);
        assert_eq!(config.buffers, 20);
    }

    #[test]
    fn crlf_line_endings() {
        let config = parse_config_sys(b"FILES=25\r\nBUFFERS=10\r\n");
        assert_eq!(config.files, 25);
        assert_eq!(config.buffers, 10);
    }

    #[test]
    fn last_value_wins() {
        let config = parse_config_sys(b"FILES=30\nFILES=40\n");
        assert_eq!(config.files, 40);
    }
}
