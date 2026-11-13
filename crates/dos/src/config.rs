//! CONFIG.SYS and AUTOEXEC.BAT parsing.

use crate::platform::DosVersion;

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
    /// Parsed SHELL= configuration.
    pub shell: Option<ShellConfig>,
    /// MSCDEX device name extracted from DEVICE=NECCD.SYS /D:name.
    pub cdrom_device_name: Option<Vec<u8>>,
    /// Ordered DEVICE= and DEVICEHIGH= lines from CONFIG.SYS.
    pub devices: Vec<ConfigDeviceLine>,
    /// Line numbers (1-based) of the lines that print the error message of
    /// the dialect.
    pub error_lines: Vec<usize>,
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
            error_lines: Vec::new(),
        }
    }
}

/// Keyword matching of the CONFIG.SYS parser of a DOS version.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ConfigDialect {
    /// DOS 3: a directive names the first keyword that starts its name.
    /// `DEVICEHIGH=` names `DEVICE` and `FILESXYZ=` names `FILES`.
    KeywordPrefix,
    /// DOS 6: a directive names the keyword that equals its name.
    ExactKeyword,
}

impl ConfigDialect {
    /// Returns the dialect of the DOS kernel `version`.
    pub(crate) fn of(version: DosVersion) -> Self {
        if version.is_dos3() {
            Self::KeywordPrefix
        } else {
            Self::ExactKeyword
        }
    }

    /// Returns the directive of the keyword `name` in upper case, or `None`
    /// for an unknown keyword.
    fn directive(self, name: &[u8]) -> Option<Directive> {
        if self.has_remarks() && name == REMARK_KEYWORD {
            return Some(Directive::Remark);
        }
        match self {
            Self::KeywordPrefix => DOS310_KEYWORDS
                .iter()
                .find(|(keyword, _)| name.starts_with(keyword))
                .map(|&(_, directive)| directive),
            Self::ExactKeyword => DOS620_KEYWORDS
                .iter()
                .find(|(keyword, _)| name == *keyword)
                .map(|&(_, directive)| directive),
        }
    }

    /// Returns true when the dialect knows the remark keyword `REM` and the
    /// comment lines that start with `;`.
    fn has_remarks(self) -> bool {
        match self {
            Self::KeywordPrefix => false,
            Self::ExactKeyword => true,
        }
    }

    /// Returns true when the dialect knows the configuration blocks of the
    /// DOS 6 start menu. A block header line starts with `[`.
    fn has_configuration_blocks(self) -> bool {
        match self {
            Self::KeywordPrefix => false,
            Self::ExactKeyword => true,
        }
    }

    /// Returns true when a line with a single token continues with the next
    /// line that holds at least one byte. The kernel appends that line
    /// without a separator.
    fn joins_single_token_lines(self) -> bool {
        match self {
            Self::KeywordPrefix => true,
            Self::ExactKeyword => false,
        }
    }

    /// Returns the content of `line` without its leading and trailing
    /// whitespace. The DOS 3 kernel also skips the `;` bytes in front of the
    /// keyword.
    fn line_content(self, line: &[u8]) -> &[u8] {
        match self {
            Self::KeywordPrefix => skip_leading_delimiters(line).trim_ascii_end(),
            Self::ExactKeyword => line.trim_ascii(),
        }
    }

    /// Returns the Shift-JIS error message of the line `line_number`
    /// (1-based).
    pub(crate) fn error_message(self, line_number: usize) -> Vec<u8> {
        let mut message = NEW_LINE.to_vec();
        match self {
            Self::KeywordPrefix => {
                message.extend(shift_jis_bytes(DOS310_UNKNOWN_DIRECTIVE_MESSAGE));
            }
            Self::ExactKeyword => {
                message.extend(shift_jis_bytes(DOS620_UNKNOWN_DIRECTIVE_MESSAGE));
                message.extend_from_slice(NEW_LINE);
                message.extend(shift_jis_bytes(DOS620_ERROR_LINE_MESSAGE));
                message.extend_from_slice(line_number.to_string().as_bytes());
            }
        }
        message.extend_from_slice(NEW_LINE);
        message
    }
}

/// Line break of a console message.
const NEW_LINE: &[u8] = b"\r\n";

/// Message of NEC MS-DOS 3.x for a line without a known keyword.
const DOS310_UNKNOWN_DIRECTIVE_MESSAGE: &str = "CONFIG.SYSに無効なコマンドかパラメータがあります.";

/// First message line of NEC MS-DOS 6.20 for a line without a known keyword.
const DOS620_UNKNOWN_DIRECTIVE_MESSAGE: &str = "CONFIG.SYS に不明なコマンドがあります.";

/// Second message line of NEC MS-DOS 6.20 for a line without a known
/// keyword. The line number follows it.
const DOS620_ERROR_LINE_MESSAGE: &str = "CONFIG.SYS のエラーです: 行";

/// Keyword of a remark line.
const REMARK_KEYWORD: &[u8] = b"REM";

/// First byte of a comment line.
const COMMENT_PREFIX: u8 = b';';

/// First byte of a configuration block header of the DOS 6 start menu.
const BLOCK_HEADER_PREFIX: u8 = b'[';

/// End of file mark of a text file.
const END_OF_FILE_MARK: u8 = 0x1A;

/// Directive of a CONFIG.SYS line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Directive {
    /// `BREAK=`.
    Break,
    /// `BUFFERS=`.
    Buffers,
    /// `COUNTRY=`.
    Country,
    /// `DEVICE=`.
    Device,
    /// `DEVICEHIGH=`.
    DeviceHigh,
    /// `FILES=`.
    Files,
    /// `LASTDRIVE=`.
    LastDrive,
    /// `SHELL=`.
    Shell,
    /// A remark line.
    Remark,
    /// A known keyword without an effect on the HLE DOS.
    Accepted,
}

/// Keywords of MS-DOS 3.10. No keyword starts with another one. The order of
/// the prefix tests does not change the match.
const DOS310_KEYWORDS: &[(&[u8], Directive)] = &[
    (b"BREAK", Directive::Break),
    (b"BUFFERS", Directive::Buffers),
    (b"COUNTRY", Directive::Country),
    (b"DEVICE", Directive::Device),
    (b"FCBS", Directive::Accepted),
    (b"FILES", Directive::Files),
    (b"LASTDRIVE", Directive::LastDrive),
    (b"SHELL", Directive::Shell),
];

/// Keywords of MS-DOS 6.20.
const DOS620_KEYWORDS: &[(&[u8], Directive)] = &[
    (b"BREAK", Directive::Break),
    (b"BUFFERS", Directive::Buffers),
    (b"COUNTRY", Directive::Country),
    (b"DEVICE", Directive::Device),
    (b"DEVICEHIGH", Directive::DeviceHigh),
    (b"DOS", Directive::Accepted),
    (b"DRIVPARM", Directive::Accepted),
    (b"FCBS", Directive::Accepted),
    (b"FILES", Directive::Files),
    (b"INCLUDE", Directive::Accepted),
    (b"INSTALL", Directive::Accepted),
    (b"LASTDRIVE", Directive::LastDrive),
    (b"MENUCOLOR", Directive::Accepted),
    (b"MENUDEFAULT", Directive::Accepted),
    (b"MENUITEM", Directive::Accepted),
    (b"NUMLOCK", Directive::Accepted),
    (b"SET", Directive::Accepted),
    (b"SHELL", Directive::Shell),
    (b"STACKS", Directive::Accepted),
    (b"SUBMENU", Directive::Accepted),
    (b"SWITCHES", Directive::Accepted),
];

/// Returns true for a byte that ends the keyword of a directive.
fn ends_keyword(byte: u8) -> bool {
    byte.is_ascii_whitespace() || matches!(byte, b'=' | b'?' | b',' | b';' | b'/')
}

/// Returns the Shift-JIS bytes of `text`.
fn shift_jis_bytes(text: &str) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(text.len() * 2);
    for character in common::str_to_jis(text) {
        if let Some(encoded) = common::jis_to_shift_jis(character) {
            encoded.write_to(&mut bytes);
        }
    }
    bytes
}

/// Parses CONFIG.SYS file content into a `ConfigSys` struct with the keywords
/// of `dialect`.
pub(crate) fn parse_config_sys(data: &[u8], dialect: ConfigDialect) -> ConfigSys {
    let mut config = ConfigSys::default();

    for (line_number, line) in logical_lines(&split_lines(data), dialect) {
        let trimmed = dialect.line_content(&line);
        if trimmed.is_empty() {
            continue;
        }
        if dialect.has_remarks() && trimmed[0] == COMMENT_PREFIX {
            continue;
        }
        if dialect.has_configuration_blocks() && trimmed[0] == BLOCK_HEADER_PREFIX {
            continue;
        }
        let upper: Vec<u8> = trimmed.iter().map(|b| b.to_ascii_uppercase()).collect();
        let keyword_length = upper
            .iter()
            .position(|&byte| ends_keyword(byte))
            .unwrap_or(upper.len());
        let Some(directive) = dialect.directive(&upper[..keyword_length]) else {
            config.error_lines.push(line_number);
            continue;
        };

        let Some(eq_pos) = trimmed.iter().position(|&b| b == b'=') else {
            continue;
        };
        let value_trimmed = trimmed[eq_pos + 1..].trim_ascii();

        match directive {
            Directive::Files => {
                if let Some(n) = parse_u16(value_trimmed)
                    && n >= 8
                {
                    config.files = n;
                }
            }
            Directive::Buffers => {
                if let Some(n) = parse_u16(value_trimmed)
                    && (1..=99).contains(&n)
                {
                    config.buffers = n;
                }
            }
            Directive::LastDrive if value_trimmed.len() == 1 => {
                let ch = value_trimmed[0].to_ascii_uppercase();
                if ch.is_ascii_uppercase() {
                    config.lastdrive = ch - b'A' + 1;
                }
            }
            Directive::Country => {
                if let Some(n) = parse_u16(value_trimmed) {
                    config.country = n;
                }
            }
            Directive::Break => {
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
            Directive::Shell if !value_trimmed.is_empty() => {
                config.shell = ShellConfig::parse(value_trimmed);
            }
            Directive::Device => {
                parse_device_line(DeviceDirective::Device, value_trimmed, &mut config);
            }
            Directive::DeviceHigh => {
                parse_device_line(DeviceDirective::DeviceHigh, value_trimmed, &mut config);
            }
            Directive::LastDrive | Directive::Shell | Directive::Remark | Directive::Accepted => {}
        }
    }

    config
}

/// Returns `line` behind its leading whitespace and `;` bytes.
fn skip_leading_delimiters(line: &[u8]) -> &[u8] {
    let start = line
        .iter()
        .position(|&byte| !byte.is_ascii_whitespace() && byte != COMMENT_PREFIX)
        .unwrap_or(line.len());
    &line[start..]
}

/// Returns true for a line that holds a single token behind its leading
/// whitespace and `;` bytes.
fn is_single_token(line: &[u8]) -> bool {
    let token = skip_leading_delimiters(line);
    !token.is_empty() && !token.iter().any(|&byte| ends_keyword(byte))
}

/// Returns the logical lines of `lines` with the 1-based number of their
/// first physical line. Under `dialect` with joined single token lines, a
/// line with a single token continues with the next line that holds at
/// least one byte. The joining repeats while the appended line holds a
/// single token.
fn logical_lines(lines: &[Vec<u8>], dialect: ConfigDialect) -> Vec<(usize, Vec<u8>)> {
    let mut logical = Vec::with_capacity(lines.len());
    let mut next_line = 0;
    while next_line < lines.len() {
        let line_number = next_line + 1;
        let mut line = lines[next_line].clone();
        let mut joins_next = dialect.joins_single_token_lines() && is_single_token(&line);
        next_line += 1;
        while joins_next {
            while lines.get(next_line).is_some_and(|line| line.is_empty()) {
                next_line += 1;
            }
            let Some(appended) = lines.get(next_line) else {
                break;
            };
            line.extend_from_slice(appended);
            joins_next = is_single_token(appended);
            next_line += 1;
        }
        logical.push((line_number, line));
    }
    logical
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

/// Splits raw file data into lines on \r\n or \n. The end of file mark ends
/// the data.
fn split_lines(data: &[u8]) -> Vec<Vec<u8>> {
    let mut lines = Vec::new();
    let mut current = Vec::new();
    for &byte in data {
        if byte == END_OF_FILE_MARK {
            break;
        }
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

    /// Parses `data` with the keywords of MS-DOS 6.20.
    fn parse_dos620(data: &[u8]) -> ConfigSys {
        parse_config_sys(data, ConfigDialect::ExactKeyword)
    }

    /// Parses `data` with the keywords of MS-DOS 3.10.
    fn parse_dos310(data: &[u8]) -> ConfigSys {
        parse_config_sys(data, ConfigDialect::KeywordPrefix)
    }

    #[test]
    fn empty_input_returns_defaults() {
        let config = parse_dos620(b"");
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
        let config = parse_dos620(b"FILES=30\n");
        assert_eq!(config.files, 30);
    }

    #[test]
    fn parse_files_below_minimum_ignored() {
        let config = parse_dos620(b"FILES=5\n");
        assert_eq!(config.files, 20); // default, 5 < 8
    }

    #[test]
    fn parse_buffers() {
        let config = parse_dos620(b"BUFFERS=40\n");
        assert_eq!(config.buffers, 40);
    }

    #[test]
    fn parse_buffers_out_of_range_ignored() {
        let config = parse_dos620(b"BUFFERS=0\n");
        assert_eq!(config.buffers, 15);
        let config = parse_dos620(b"BUFFERS=100\n");
        assert_eq!(config.buffers, 15);
    }

    #[test]
    fn parse_lastdrive() {
        let config = parse_dos620(b"LASTDRIVE=E\n");
        assert_eq!(config.lastdrive, 5); // E = 5th letter
    }

    #[test]
    fn parse_lastdrive_lowercase() {
        let config = parse_dos620(b"LASTDRIVE=e\n");
        assert_eq!(config.lastdrive, 5);
    }

    #[test]
    fn parse_country() {
        let config = parse_dos620(b"COUNTRY=001\n");
        assert_eq!(config.country, 1);
    }

    #[test]
    fn parse_break_on() {
        let config = parse_dos620(b"BREAK=ON\n");
        assert!(config.ctrl_break);
    }

    #[test]
    fn parse_break_off() {
        let config = parse_dos620(b"BREAK=OFF\n");
        assert!(!config.ctrl_break);
    }

    #[test]
    fn parse_shell() {
        let config = parse_dos620(b"SHELL=C:\\COMMAND.COM /P\n");
        let shell = config.shell.expect("SHELL= should parse");
        assert_eq!(shell.path, b"C:\\COMMAND.COM");
        assert_eq!(shell.raw_arguments, b"/P");
        assert_eq!(shell.initial_drive, None);
        assert_eq!(shell.environment_size_bytes, None);
        assert!(shell.permanent);
    }

    #[test]
    fn parse_shell_command_com_contract() {
        let config = parse_dos620(b"SHELL = A:COMMAND.COM A: /E:384 /P\n");
        let shell = config.shell.expect("SHELL= should parse");
        assert_eq!(shell.path, b"A:COMMAND.COM");
        assert_eq!(shell.raw_arguments, b"A: /E:384 /P");
        assert_eq!(shell.initial_drive, Some(0));
        assert_eq!(shell.environment_size_bytes, Some(384));
        assert!(shell.permanent);
    }

    #[test]
    fn parse_device_neccd() {
        let config = parse_dos620(b"DEVICE=A:\\NECCD.SYS /D:CD001\n");
        assert_eq!(config.cdrom_device_name.as_deref(), Some(b"CD001".as_ref()));
        assert_eq!(config.devices.len(), 1);
        assert_eq!(config.devices[0].directive, DeviceDirective::Device);
        assert_eq!(config.devices[0].path, b"A:\\NECCD.SYS");
        assert_eq!(config.devices[0].arguments, b"/D:CD001");
    }

    #[test]
    fn parse_device_neccdd() {
        let config = parse_dos620(b"DEVICE=A:\\DOS\\NECCDD.SYS /D:MYCDROM\n");
        assert_eq!(
            config.cdrom_device_name.as_deref(),
            Some(b"MYCDROM".as_ref())
        );
    }

    #[test]
    fn parse_device_neccd_no_d_param() {
        let config = parse_dos620(b"DEVICE=NECCD.SYS\n");
        assert_eq!(
            config.cdrom_device_name.as_deref(),
            Some(b"MSCD001".as_ref())
        );
    }

    #[test]
    fn parse_devicehigh() {
        let config = parse_dos620(b"DEVICEHIGH=A:\\NECCD.SYS /D:CD002\n");
        assert_eq!(config.cdrom_device_name.as_deref(), Some(b"CD002".as_ref()));
        assert_eq!(config.devices.len(), 1);
        assert_eq!(config.devices[0].directive, DeviceDirective::DeviceHigh);
    }

    #[test]
    fn unknown_device_preserved() {
        let config = parse_dos620(b"DEVICE=MOUSE.SYS\n");
        assert!(config.cdrom_device_name.is_none());
        assert_eq!(config.devices.len(), 1);
        assert_eq!(config.devices[0].path, b"MOUSE.SYS");
    }

    #[test]
    fn device_lines_preserve_order() {
        let config = parse_dos620(b"DEVICE=FIRST.SYS /A\nDEVICEHIGH=C:\\DOS\\SECOND.SYS /B /C\n");
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
        let config = parse_dos620(input);
        assert_eq!(config.files, 25);
    }

    #[test]
    fn case_insensitive_directives() {
        let config = parse_dos620(b"files=30\nbuffers=20\nbreak=on\n");
        assert_eq!(config.files, 30);
        assert_eq!(config.buffers, 20);
        assert!(config.ctrl_break);
    }

    #[test]
    fn mixed_valid_and_invalid_lines() {
        let input = b"FILES=30\nGARBAGE LINE\nBUFFERS=20\nNOEQUALS\n";
        let config = parse_dos620(input);
        assert_eq!(config.files, 30);
        assert_eq!(config.buffers, 20);
    }

    #[test]
    fn crlf_line_endings() {
        let config = parse_dos620(b"FILES=25\r\nBUFFERS=10\r\n");
        assert_eq!(config.files, 25);
        assert_eq!(config.buffers, 10);
    }

    #[test]
    fn last_value_wins() {
        let config = parse_dos620(b"FILES=30\nFILES=40\n");
        assert_eq!(config.files, 40);
    }

    #[test]
    fn dos310_matches_a_directive_by_its_keyword_prefix() {
        let config =
            parse_dos310(b"FILESXYZ=20\r\nBUFFERSX=5\r\nDEVICEHIGH=A:\\DOS\\RAMDISK.SYS\r\n");
        assert_eq!(config.files, 20);
        assert_eq!(config.buffers, 5);
        assert_eq!(config.devices.len(), 1);
        assert_eq!(config.devices[0].directive, DeviceDirective::Device);
        assert_eq!(config.devices[0].path, b"A:\\DOS\\RAMDISK.SYS");
        assert!(config.error_lines.is_empty());
    }

    #[test]
    fn dos310_prefix_keywords_cover_every_dos_310_directive() {
        let config = parse_dos310(
            b"BREAKX=ON\nCOUNTRYX=001\nFCBSX=4,0\nLASTDRIVEX=E\nSHELLX=A:\\COMMAND.COM\n",
        );
        assert!(config.ctrl_break);
        assert_eq!(config.country, 1);
        assert_eq!(config.lastdrive, 5);
        assert_eq!(
            config.shell.map(|shell| shell.path),
            Some(b"A:\\COMMAND.COM".to_vec())
        );
        assert!(config.error_lines.is_empty());
    }

    #[test]
    fn dos310_keywords_start_with_no_other_keyword() {
        for (keyword, _) in DOS310_KEYWORDS {
            let matches = DOS310_KEYWORDS
                .iter()
                .filter(|(other, _)| keyword.starts_with(other))
                .count();
            assert_eq!(matches, 1, "{}", String::from_utf8_lossy(keyword));
        }
    }

    #[test]
    fn dos310_reports_the_keywords_of_later_versions() {
        let config = parse_dos310(
            b"FOOBAR=1\nDOS=HIGH\nSTACKS=9,256\nDRIVPARM=/D:0\nINSTALL=A:\\X.EXE\n[MENU]\n",
        );
        assert_eq!(config.error_lines, [1, 2, 3, 4, 5, 6]);
    }

    #[test]
    fn dos310_reports_remarks_and_comments() {
        let config =
            parse_dos310(b"REM a remark\n; a comment\nREM\n\n   \nFILES=30\n;comment\nREMXYZ=1\n");
        assert_eq!(config.files, 30);
        assert_eq!(config.error_lines, [1, 2, 3, 7]);
    }

    #[test]
    fn dos310_single_token_line_continues_with_the_next_line() {
        let config = parse_dos310(
            b"REM\r\nFILES=11\r\n;comment\r\nBUFFERS=13\r\n[MENU]\r\nLASTDRIVE=K\r\nFILES\r\nFCBS=7,0\r\n",
        );
        assert_eq!(config.files, 20);
        assert_eq!(config.buffers, 15);
        assert_eq!(config.lastdrive, 26);
        assert_eq!(config.error_lines, [1, 3, 5]);
    }

    #[test]
    fn dos310_delimiter_behind_the_keyword_keeps_the_next_line() {
        let config = parse_dos310(
            b"FILES=\r\nLASTDRIVE=K\r\nBUFFERS   \r\nFILES=11\r\n;two words\r\nBUFFERS=13\r\nFILES =\t\r\nBREAK=ON\r\n",
        );
        assert_eq!(config.lastdrive, 11);
        assert_eq!(config.files, 11);
        assert_eq!(config.buffers, 13);
        assert!(config.ctrl_break);
        assert_eq!(config.error_lines, [5]);
    }

    #[test]
    fn dos310_joined_line_skips_empty_lines_and_takes_whitespace_lines() {
        let config = parse_dos310(b"REM\r\n\r\nFILES=11\r\nREM\r\n   \r\nLASTDRIVE=M\r\n");
        assert_eq!(config.files, 20);
        assert_eq!(config.lastdrive, 13);
        assert_eq!(config.error_lines, [1, 4]);
    }

    #[test]
    fn dos310_joined_lines_run_as_one_directive() {
        let config = parse_dos310(b"FILES\r\n30\r\nLASTDRIVE\r\nM\r\nBUFFERS=13\r\n");
        assert_eq!(config.files, 13);
        assert_eq!(config.buffers, 15);
        assert_eq!(config.lastdrive, 26);
        assert!(config.error_lines.is_empty());
    }

    #[test]
    fn dos310_skips_leading_semicolons() {
        let config = parse_dos310(b";FILES=11\r\nBUFFERS=13\r\n;\r\nLASTDRIVE=K\r\n");
        assert_eq!(config.files, 11);
        assert_eq!(config.buffers, 13);
        assert_eq!(config.lastdrive, 11);
        assert!(config.error_lines.is_empty());
    }

    #[test]
    fn dos620_single_token_line_keeps_the_next_line() {
        let config = parse_dos620(b"REM\r\nFILES=11\r\nFOOBAR\r\nLASTDRIVE=K\r\n");
        assert_eq!(config.files, 11);
        assert_eq!(config.lastdrive, 11);
        assert_eq!(config.error_lines, [3]);
    }

    #[test]
    fn dos620_keeps_remarks_and_comments_silent() {
        let config = parse_dos620(b"; comment\nREM x\nREM\nREM:x\n");
        assert_eq!(config.error_lines, [4]);
    }

    #[test]
    fn dos620_matches_the_whole_keyword() {
        let config = parse_dos620(b"FILESXYZ=20\nBUFFERSX=5\nFILES=30\n");
        assert_eq!(config.files, 30);
        assert_eq!(config.buffers, 15);
        assert_eq!(config.error_lines, [1, 2]);
    }

    #[test]
    fn dos620_knows_every_dos_620_directive() {
        let input = b"BREAK=ON\r\nBUFFERS=20\r\nCOUNTRY=081\r\nDEVICE=A:\\X.SYS\r\n\
DEVICEHIGH /L:1 =A:\\Y.SYS\r\nDOS=HIGH,UMB\r\nDRIVPARM=/D:0 /F:2\r\nFCBS=4,0\r\nFILES=30\r\n\
INCLUDE=COMMON\r\nINSTALL=A:\\SHARE.EXE\r\nLASTDRIVE=Z\r\nMENUCOLOR=7,0\r\n\
MENUDEFAULT=COMMON,5\r\nMENUITEM=COMMON\r\nNUMLOCK=OFF\r\nREM remark\r\nSET TEMP=A:\\\r\n\
SHELL=A:\\COMMAND.COM\r\nSTACKS=9,256\r\nSUBMENU=MORE\r\nSWITCHES=/F\r\n[COMMON]\r\n\
; comment\r\nDEVICE?=A:\\Z.SYS\r\n\r\n";
        let config = parse_dos620(input);
        assert!(config.error_lines.is_empty(), "{:?}", config.error_lines);
        assert_eq!(config.devices.len(), 3);
        assert_eq!(config.devices[1].directive, DeviceDirective::DeviceHigh);
        assert_eq!(config.devices[2].path, b"A:\\Z.SYS");
    }

    #[test]
    fn dos620_counts_every_physical_line() {
        let config = parse_dos620(b"FILES=30\r\n\r\n; comment\r\nFOOBAR=1\r\nGARBAGE LINE\r\n");
        assert_eq!(config.error_lines, [4, 5]);
    }

    #[test]
    fn end_of_file_mark_ends_the_directives() {
        let config = parse_dos620(b"FILES=30\r\n\x1A\r\nFOOBAR=1\r\n");
        assert_eq!(config.files, 30);
        assert!(config.error_lines.is_empty());
    }

    #[test]
    fn dialect_follows_the_kernel_version() {
        let version = |major, minor| DosVersion {
            major,
            minor,
            oem_number: 0xFF,
        };
        assert_eq!(
            ConfigDialect::of(version(3, 10)),
            ConfigDialect::KeywordPrefix
        );
        assert_eq!(
            ConfigDialect::of(version(6, 20)),
            ConfigDialect::ExactKeyword
        );
    }

    #[test]
    fn dos310_message_is_one_line_without_a_line_number() {
        let mut expected = b"\r\nCONFIG.SYS".to_vec();
        expected.extend_from_slice(&[
            0x82, 0xC9, 0x96, 0xB3, 0x8C, 0xF8, 0x82, 0xC8, 0x83, 0x52, 0x83, 0x7D, 0x83, 0x93,
            0x83, 0x68, 0x82, 0xA9, 0x83, 0x70, 0x83, 0x89, 0x83, 0x81, 0x81, 0x5B, 0x83, 0x5E,
            0x82, 0xAA, 0x82, 0xA0, 0x82, 0xE8, 0x82, 0xDC, 0x82, 0xB7,
        ]);
        expected.extend_from_slice(b".\r\n");
        assert_eq!(ConfigDialect::KeywordPrefix.error_message(7), expected);
    }

    #[test]
    fn dos620_message_names_the_line_number() {
        let mut expected = b"\r\nCONFIG.SYS ".to_vec();
        expected.extend_from_slice(&[
            0x82, 0xC9, 0x95, 0x73, 0x96, 0xBE, 0x82, 0xC8, 0x83, 0x52, 0x83, 0x7D, 0x83, 0x93,
            0x83, 0x68, 0x82, 0xAA, 0x82, 0xA0, 0x82, 0xE8, 0x82, 0xDC, 0x82, 0xB7,
        ]);
        expected.extend_from_slice(b".\r\nCONFIG.SYS ");
        expected.extend_from_slice(&[
            0x82, 0xCC, 0x83, 0x47, 0x83, 0x89, 0x81, 0x5B, 0x82, 0xC5, 0x82, 0xB7,
        ]);
        expected.extend_from_slice(b": ");
        expected.extend_from_slice(&[0x8D, 0x73]);
        expected.extend_from_slice(b"12\r\n");
        assert_eq!(ConfigDialect::ExactKeyword.error_message(12), expected);
    }
}
