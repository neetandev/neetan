//! Text entry: `type-text!` types every printable ASCII character with the key
//! and shift state of the keyboard of each machine family.
//!
//! Every family types the 95 printable ASCII characters one at a time through
//! the session API. A recording wrapper captures the native key codes the
//! machine receives, and the test decodes them with a key legend table taken
//! from the references of the family. A character without a key on that
//! keyboard must fail with `neetan/unsupported` and inject no key.

#[path = "common/harness.rs"]
mod harness;

use std::{cell::RefCell, rc::Rc, sync::Arc};

use automation::{AutomationSession, OpError};
use common::{
    AutomatedMachine, AutomationDescriptor, AutomationTimeline, CpuMode, FixedHostDateTime,
    FlushError, HostKey, KeyModifiers, Machine, MediaBacking, MediaImage, RunOutcome, RunRequest,
    TraceCatalog, temporary::TemporaryFolder,
};
use harness::{
    build_fm7, build_msx, build_pc98, build_x68k, fixed_date_time, make_session, trace_sink,
};

/// Audio sample rate of the machines built here.
const SAMPLE_RATE: u32 = 48000;

/// Frames each character key is held. The machines run without firmware, so
/// the test checks the key codes and runs no frames.
const SPACING_FRAMES: u64 = 0;

/// Tick budget for one character.
const MAX_TICKS_PER_CHARACTER: u64 = 1_000_000;

/// Release flag of a native key code.
const RELEASE_FLAG: u8 = 0x80;

/// One reference key: native code, unshifted character, shifted character.
type KeyLegend = (u8, Option<char>, Option<char>);

/// The native keys and characters of one machine keyboard.
struct KeyboardReference {
    /// Machine family name used in failure messages.
    family: &'static str,
    /// Native code of the Shift key, or `None` for a keyboard that composes
    /// the shifted code itself.
    shift_code: Option<u8>,
    /// Every key that types a printable ASCII character.
    legends: Vec<KeyLegend>,
}

impl KeyboardReference {
    /// Returns the legend of the key with the native code `code`.
    fn legend(&self, code: u8) -> Option<&KeyLegend> {
        self.legends.iter().find(|legend| legend.0 == code)
    }

    /// Whether some key of this keyboard types `character`.
    fn types(&self, character: char) -> bool {
        self.legends
            .iter()
            .any(|&(_, normal, shifted)| normal == Some(character) || shifted == Some(character))
    }

    /// Decodes the native codes of one typed character into the character the
    /// keyboard produces, or `None` for a sequence that is not one keystroke.
    fn decode(&self, codes: &[u8]) -> Option<char> {
        match (codes, self.shift_code) {
            ([code, release], _) if *release == code | RELEASE_FLAG => self.legend(*code)?.1,
            ([shift, code, release, shift_release], Some(shift_code))
                if *shift == shift_code
                    && *release == code | RELEASE_FLAG
                    && *shift_release == shift_code | RELEASE_FLAG =>
            {
                self.legend(*code)?.2
            }
            _ => None,
        }
    }
}

/// Appends the letter keys of a keyboard whose letters sit on consecutive
/// codes along the QWERTY rows.
fn qwerty_letters(legends: &mut Vec<KeyLegend>, q_row: u8, a_row: u8, z_row: u8) {
    for (start, letters) in [
        (q_row, "qwertyuiop"),
        (a_row, "asdfghjkl"),
        (z_row, "zxcvbnm"),
    ] {
        for (offset, letter) in (0u8..).zip(letters.chars()) {
            legends.push((
                start + offset,
                Some(letter),
                Some(letter.to_ascii_uppercase()),
            ));
        }
    }
}

/// Appends the digit keys `1`..`9` on consecutive codes with their JIS shifted
/// symbols.
fn jis_digits(legends: &mut Vec<KeyLegend>, one: u8) {
    for (offset, (digit, symbol)) in (0u8..).zip("123456789".chars().zip("!\"#$%&'()".chars())) {
        legends.push((one + offset, Some(digit), Some(symbol)));
    }
}

/// PC-98 key codes and key data in normal mode (undoc98 int18kb.txt, table
/// "キーコード・キーデータ"; the `^` key follows note *2 like the built-in BIOS).
fn pc98_reference() -> KeyboardReference {
    let mut legends = vec![
        (0x0A, Some('0'), None),
        (0x0B, Some('-'), Some('=')),
        (0x0C, Some('^'), Some('^')),
        (0x0D, Some('\\'), Some('|')),
        (0x1A, Some('@'), Some('~')),
        (0x1B, Some('['), Some('{')),
        (0x26, Some(';'), Some('+')),
        (0x27, Some(':'), Some('*')),
        (0x28, Some(']'), Some('}')),
        (0x30, Some(','), Some('<')),
        (0x31, Some('.'), Some('>')),
        (0x32, Some('/'), Some('?')),
        (0x33, None, Some('_')),
        (0x34, Some(' '), Some(' ')),
    ];
    jis_digits(&mut legends, 0x01);
    qwerty_letters(&mut legends, 0x10, 0x1D, 0x29);
    KeyboardReference {
        family: "pc98",
        shift_code: Some(0x70),
        legends,
    }
}

/// PC-88VA scan codes (PC-88VA Technical Manual 5.6.3, scan code table) with
/// the internal codes of the 88M/F series (manual 6.3.2).
fn pc88va_reference() -> KeyboardReference {
    let mut legends = vec![
        (0x0A, Some('0'), None),
        (0x0B, Some('-'), Some('=')),
        (0x0C, Some('^'), None),
        (0x0D, Some('\\'), Some('|')),
        (0x1A, Some('@'), None),
        (0x1B, Some('['), Some('{')),
        (0x26, Some(';'), Some('+')),
        (0x27, Some(':'), Some('*')),
        (0x28, Some(']'), Some('}')),
        (0x30, Some(','), Some('<')),
        (0x31, Some('.'), Some('>')),
        (0x32, Some('/'), Some('?')),
        (0x33, None, Some('_')),
        (0x34, Some(' '), Some(' ')),
    ];
    jis_digits(&mut legends, 0x01);
    qwerty_letters(&mut legends, 0x10, 0x1D, 0x29);
    KeyboardReference {
        family: "pc88va",
        shift_code: Some(0x70),
        legends,
    }
}

/// PC-8801 key matrix cells as `row << 3 | column` (the PC-8801 keyboard
/// matrix of the PC-88 references; the yen key types 5Ch).
fn pc88_reference() -> KeyboardReference {
    let mut legends = vec![
        (0x10, Some('@'), None),
        (0x2B, Some('['), Some('{')),
        (0x2C, Some('\\'), Some('|')),
        (0x2D, Some(']'), Some('}')),
        (0x2E, Some('^'), None),
        (0x2F, Some('-'), Some('=')),
        (0x30, Some('0'), None),
        (0x3A, Some(':'), Some('*')),
        (0x3B, Some(';'), Some('+')),
        (0x3C, Some(','), Some('<')),
        (0x3D, Some('.'), Some('>')),
        (0x3E, Some('/'), Some('?')),
        (0x3F, None, Some('_')),
        (0x4E, Some(' '), Some(' ')),
    ];
    jis_digits(&mut legends, 0x31);
    for (offset, letter) in (0u8..).zip('a'..='z') {
        legends.push((
            0x11 + offset,
            Some(letter),
            Some(letter.to_ascii_uppercase()),
        ));
    }
    KeyboardReference {
        family: "pc88",
        shift_code: Some(0x46),
        legends,
    }
}

/// MSX keys by the key code that the MSX keyboard places in the Sony matrix,
/// with the characters of openMSX `unicodemap.jp_jis` (matrix cell noted).
fn msx_reference() -> KeyboardReference {
    let mut legends = vec![
        // (0,0)
        (0x0A, Some('0'), None),
        // (1,2)
        (0x0B, Some('-'), Some('=')),
        // (1,3)
        (0x0C, Some('^'), Some('~')),
        // (1,4)
        (0x0D, Some('\\'), Some('|')),
        // (1,5)
        (0x1A, Some('@'), Some('`')),
        // (1,6)
        (0x1B, Some('['), Some('{')),
        // (1,7)
        (0x26, Some(';'), Some('+')),
        // (2,0)
        (0x27, Some(':'), Some('*')),
        // (2,1)
        (0x28, Some(']'), Some('}')),
        // (2,2)
        (0x30, Some(','), Some('<')),
        // (2,3)
        (0x31, Some('.'), Some('>')),
        // (2,4)
        (0x32, Some('/'), Some('?')),
        // (2,5)
        (0x33, None, Some('_')),
        // (8,0)
        (0x34, Some(' '), Some(' ')),
    ];
    jis_digits(&mut legends, 0x01);
    qwerty_letters(&mut legends, 0x10, 0x1D, 0x29);
    KeyboardReference {
        family: "msx",
        shift_code: Some(0x70),
        legends,
    }
}

/// FM-7 scan codes with the characters of the F-BASIC keycode tables (CSCP
/// `vm/fm7/keyboard_tables.h`, normal and shift tables).
fn fm7_reference() -> KeyboardReference {
    let mut legends = jis_scan_code_symbols(Some('"'));
    jis_digits(&mut legends, 0x02);
    qwerty_letters(&mut legends, 0x11, 0x1E, 0x2A);
    KeyboardReference {
        family: "fm7",
        shift_code: Some(0x53),
        legends,
    }
}

/// FM Towns scan codes with the key legends of the FM Towns keyboard (MAME
/// `fmtowns.cpp` input ports; the shifted `^` is the JIS X 0201 overline 7Eh).
fn towns_reference() -> KeyboardReference {
    let mut legends = jis_scan_code_symbols(Some('"'));
    jis_digits(&mut legends, 0x02);
    qwerty_letters(&mut legends, 0x11, 0x1E, 0x2A);
    KeyboardReference {
        family: "towns",
        shift_code: Some(0x53),
        legends,
    }
}

/// X68000 scan codes with the key legends of XEiJ `Keyboard.java`
/// (`KBD_EO_TO_XO` key top table).
fn x68k_reference() -> KeyboardReference {
    let mut legends = jis_scan_code_symbols(None);
    jis_digits(&mut legends, 0x02);
    qwerty_letters(&mut legends, 0x11, 0x1E, 0x2A);
    KeyboardReference {
        family: "x68k",
        shift_code: Some(0x70),
        legends,
    }
}

/// Symbol keys shared by the FM-7, FM Towns and X68000 scan codes. The
/// unshifted character of the `_` key differs between the families.
fn jis_scan_code_symbols(underscore_key_normal: Option<char>) -> Vec<KeyLegend> {
    vec![
        (0x0B, Some('0'), None),
        (0x0C, Some('-'), Some('=')),
        (0x0D, Some('^'), Some('~')),
        (0x0E, Some('\\'), Some('|')),
        (0x1B, Some('@'), Some('`')),
        (0x1C, Some('['), Some('{')),
        (0x27, Some(';'), Some('+')),
        (0x28, Some(':'), Some('*')),
        (0x29, Some(']'), Some('}')),
        (0x31, Some(','), Some('<')),
        (0x32, Some('.'), Some('>')),
        (0x33, Some('/'), Some('?')),
        (0x34, underscore_key_normal, Some('_')),
        (0x35, Some(' '), Some(' ')),
    ]
}

/// X1 keys by host sub-id with the X1 character codes of their virtual keys
/// (CSCP `vm/x1/psub.cpp`, tables `keycode` and `keycode_s`; virtual key noted).
fn x1_reference() -> KeyboardReference {
    let mut legends = vec![
        // VK 0x30
        (0x30, Some('0'), Some('0')),
        // VK 0xBA
        (0x01, Some(':'), Some('*')),
        // VK 0xBB
        (0x02, Some(';'), Some('+')),
        // VK 0xBC
        (0x04, Some(','), Some('<')),
        // VK 0xBD
        (0x05, Some('-'), Some('=')),
        // VK 0xBE
        (0x06, Some('.'), Some('>')),
        // VK 0xBF
        (0x07, Some('/'), Some('?')),
        // VK 0xC0
        (0x0A, Some('@'), Some('~')),
        // VK 0xDB
        (0x0B, Some('['), Some('{')),
        // VK 0xDC
        (0x0C, Some('\\'), Some('|')),
        // VK 0xDD
        (0x0E, Some(']'), Some('}')),
        // VK 0xDE
        (0x0F, Some('^'), Some('`')),
        // VK 0xE2
        (0x17, None, Some('_')),
        // VK 0x20
        (0x20, Some(' '), Some(' ')),
    ];
    jis_digits(&mut legends, 0x31);
    for letter in 'a'..='z' {
        legends.push((
            letter.to_ascii_uppercase() as u8,
            Some(letter),
            Some(letter.to_ascii_uppercase()),
        ));
    }
    KeyboardReference {
        family: "x1",
        shift_code: Some(0x10),
        legends,
    }
}

/// PC/AT set-1 scan codes with the key legends of the OADG 106-key keyboard
/// IBM 5576-A01 (DOS/V Technical Reference Manual, section 2.5, figure 2-3).
fn at_reference() -> KeyboardReference {
    let mut legends = vec![
        (0x0B, Some('0'), Some('~')),
        (0x0C, Some('-'), Some('=')),
        (0x0D, Some('^'), Some('~')),
        (0x7D, Some('\\'), Some('|')),
        (0x1A, Some('@'), Some('`')),
        (0x1B, Some('['), Some('{')),
        (0x27, Some(';'), Some('+')),
        (0x28, Some(':'), Some('*')),
        (0x2B, Some(']'), Some('}')),
        (0x33, Some(','), Some('<')),
        (0x34, Some('.'), Some('>')),
        (0x35, Some('/'), Some('?')),
        (0x73, Some('\\'), Some('_')),
        (0x39, Some(' '), Some(' ')),
    ];
    jis_digits(&mut legends, 0x02);
    qwerty_letters(&mut legends, 0x10, 0x1E, 0x2C);
    KeyboardReference {
        family: "at",
        shift_code: Some(0x2A),
        legends,
    }
}

/// PC-6001 sub-controller codes. The code is the character itself; the set
/// follows the key scan of the PC-6001 keyboard (MAME `pc6001.cpp`,
/// `check_keyboard_press`), where a letter key types the capital letter and
/// Shift selects the small letter.
fn pc60_reference() -> KeyboardReference {
    let characters = ('A'..='Z')
        .chain('a'..='z')
        .chain('0'..='9')
        .chain("!\"#$%&'()= -,.;:/?[]^".chars());
    KeyboardReference {
        family: "pc60",
        shift_code: None,
        legends: characters
            .map(|character| (character as u8, Some(character), None))
            .collect(),
    }
}

/// A machine wrapper that records every native key code it receives.
struct RecordingMachine {
    inner: Box<dyn AutomatedMachine>,
    codes: Rc<RefCell<Vec<u8>>>,
}

impl Machine for RecordingMachine {
    fn cpu_clock_hz(&self) -> f64 {
        self.inner.cpu_clock_hz()
    }

    fn run_for(&mut self, budget: u64) -> u64 {
        self.inner.run_for(budget)
    }

    fn shutdown_requested(&self) -> bool {
        self.inner.shutdown_requested()
    }

    fn display_framebuffer(&self) -> &[u8] {
        self.inner.display_framebuffer()
    }

    fn display_dimensions(&self) -> (u32, u32) {
        self.inner.display_dimensions()
    }

    fn push_keyboard_scancode(&mut self, code: u8) {
        self.codes.borrow_mut().push(code);
        self.inner.push_keyboard_scancode(code);
    }

    fn translate_host_key(&self, key: HostKey, modifiers: KeyModifiers) -> Option<u8> {
        self.inner.translate_host_key(key, modifiers)
    }

    fn generate_audio_samples(&mut self, volume: f32, output: &mut [f32]) -> usize {
        self.inner.generate_audio_samples(volume, output)
    }

    fn insert_floppy(
        &mut self,
        drive: usize,
        image: MediaImage<'_>,
        backing: MediaBacking,
    ) -> Result<String, String> {
        self.inner.insert_floppy(drive, image, backing)
    }

    fn eject_floppy(&mut self, drive: usize) {
        self.inner.eject_floppy(drive);
    }

    fn flush_floppies(&mut self) -> Result<(), FlushError> {
        self.inner.flush_floppies()
    }
}

impl AutomatedMachine for RecordingMachine {
    fn automation_descriptor(&self) -> AutomationDescriptor {
        self.inner.automation_descriptor()
    }

    fn automation_timeline(&self) -> AutomationTimeline {
        self.inner.automation_timeline()
    }

    fn run_automation(&mut self, request: RunRequest) -> RunOutcome {
        self.inner.run_automation(request)
    }

    fn trace_catalog(&self) -> TraceCatalog {
        self.inner.trace_catalog()
    }
}

/// Builds an automated PC-8801 machine without ROMs.
fn build_pc88() -> Box<dyn AutomatedMachine> {
    let bus = machine_88::Pc8801Bus::new_with_trace_sink(
        machine_88::Pc8801Model::PC8801MC,
        machine_88::ClockSelect::EightMhz,
        SAMPLE_RATE,
        trace_sink(),
    );
    machine_88::build_automated_machine(bus)
}

/// Builds an automated PC-88VA machine with empty ROMs.
fn build_pc88va() -> Box<dyn AutomatedMachine> {
    let roms = machine_88va::LoadedRoms {
        rom00: vec![0; 0x8_0000],
        rom08: vec![0; 0x2_0000],
        rom1: vec![0; 0x2_0000],
        font: vec![0; 0x5_0000],
        dictionary: vec![0; 0x8_0000],
        subsys: vec![0; 0x2000],
    };
    let bus = machine_88va::Pc88VaBus::new_with_trace_sink(
        machine_88va::Pc88VaModel::PC88VA2,
        roms,
        SAMPLE_RATE,
        trace_sink(),
    );
    machine_88va::build_automated_machine(bus)
}

/// Builds an automated PC-6001 machine without ROMs.
fn build_pc60() -> Box<dyn AutomatedMachine> {
    let bus = machine_60::Pc6000Bus::new_with_trace_sink(
        machine_60::Pc6000Model::Pc6001,
        SAMPLE_RATE,
        trace_sink(),
    );
    machine_60::build_automated_machine(bus)
}

/// Builds an automated X1 machine without ROMs.
fn build_x1() -> Box<dyn AutomatedMachine> {
    let bus =
        machine_x1::X1Bus::new_with_trace_sink(machine_x1::X1Model::X1, SAMPLE_RATE, trace_sink());
    machine_x1::build_automated_machine(bus)
}

/// Builds an automated FM Towns machine with the built-in HLE ROMs.
fn build_towns() -> Box<dyn AutomatedMachine> {
    let model = machine_towns::TownsModel::FmTowns;
    let bus = machine_towns::TownsBus::new_with_trace_sink(
        model,
        CpuMode::High,
        machine_towns::LoadedRoms::builtin_hle_set(model),
        SAMPLE_RATE,
        trace_sink(),
    );
    let mut machine = machine_towns::build_automated_machine(
        model,
        bus,
        machine_towns::TownsBootDevice::Auto,
        machine_towns::TownsPadType::default(),
        false,
    );
    machine.set_host_date_time_source(Arc::new(FixedHostDateTime(fixed_date_time())));
    machine
}

/// Builds an automated PC/AT machine with the built-in HLE BIOS.
fn build_at() -> Box<dyn AutomatedMachine> {
    let model = machine_at::AtModel::At486Dx66;
    let bus = machine_at::AtBus::new_with_trace_sink(
        model.cpu_clock_hz(CpuMode::High),
        model.ram_size(),
        machine_at::LoadedRoms::hle_stub_set(),
        SAMPLE_RATE,
        trace_sink(),
    );
    let mut machine =
        machine_at::build_automated_machine(bus, machine_at::AtBootDevice::FloppyFirst);
    machine.set_host_date_time_source(Arc::new(FixedHostDateTime(fixed_date_time())));
    machine
}

/// Types every printable ASCII character on a fresh machine and returns one
/// line per character whose received key codes disagree with `reference`.
fn type_printable_ascii(
    build: fn() -> Box<dyn AutomatedMachine>,
    reference: &KeyboardReference,
) -> Vec<String> {
    let root = TemporaryFolder::new(&format!("auto_type-text-{}", reference.family));
    let (mut session, _receiver) = make_session(root.path(), root.path());
    let codes = Rc::new(RefCell::new(Vec::new()));
    session.install_machine(Box::new(RecordingMachine {
        inner: build(),
        codes: Rc::clone(&codes),
    }));

    let mut mismatches = Vec::new();
    for character in ' '..='~' {
        codes.borrow_mut().clear();
        let result = type_character(&mut session, character);
        let received = codes.borrow().clone();
        let expected_typeable = reference.types(character);
        match result {
            Ok(()) if expected_typeable => {
                let decoded = reference.decode(&received);
                if decoded != Some(character) {
                    mismatches.push(format!(
                        "{character:?}: codes {received:02X?} decode to {decoded:?}"
                    ));
                }
            }
            Ok(()) => mismatches.push(format!(
                "{character:?}: has no key but was typed as codes {received:02X?}"
            )),
            Err(OpError::Unsupported(_)) if !expected_typeable && received.is_empty() => {}
            Err(error) => mismatches.push(format!(
                "{character:?}: failed with {} ({}) after codes {received:02X?}",
                error.symbol(),
                error.message()
            )),
        }
    }
    mismatches
}

/// Types one character through the session.
fn type_character(session: &mut AutomationSession, character: char) -> Result<(), OpError> {
    session.type_text(
        &character.to_string(),
        SPACING_FRAMES,
        MAX_TICKS_PER_CHARACTER,
    )
}

/// Asserts that every printable ASCII character reaches the machine as the key
/// of its reference table.
fn assert_types_printable_ascii(
    build: fn() -> Box<dyn AutomatedMachine>,
    reference: KeyboardReference,
) {
    let mismatches = type_printable_ascii(build, &reference);
    assert!(
        mismatches.is_empty(),
        "{}: {} characters reach the guest wrongly:\n{}",
        reference.family,
        mismatches.len(),
        mismatches.join("\n")
    );
}

#[test]
fn pc98_types_printable_ascii() {
    assert_types_printable_ascii(build_pc98, pc98_reference());
}

#[test]
fn pc88_types_printable_ascii() {
    assert_types_printable_ascii(build_pc88, pc88_reference());
}

#[test]
fn pc88va_types_printable_ascii() {
    assert_types_printable_ascii(build_pc88va, pc88va_reference());
}

#[test]
fn pc60_types_printable_ascii() {
    assert_types_printable_ascii(build_pc60, pc60_reference());
}

#[test]
fn msx_types_printable_ascii() {
    assert_types_printable_ascii(build_msx, msx_reference());
}

#[test]
fn fm7_types_printable_ascii() {
    assert_types_printable_ascii(build_fm7, fm7_reference());
}

#[test]
fn towns_types_printable_ascii() {
    assert_types_printable_ascii(build_towns, towns_reference());
}

#[test]
fn x68k_types_printable_ascii() {
    assert_types_printable_ascii(build_x68k, x68k_reference());
}

#[test]
fn x1_types_printable_ascii() {
    assert_types_printable_ascii(build_x1, x1_reference());
}

#[test]
fn at_types_printable_ascii() {
    assert_types_printable_ascii(build_at, at_reference());
}

#[test]
fn non_text_character_is_an_argument_error() {
    let root = TemporaryFolder::new("auto_type-text-argument");
    let (mut session, _receiver) = make_session(root.path(), root.path());
    session.install_machine(build_pc98());
    for character in ['\u{3042}', '\u{7F}', '\u{1B}'] {
        let result = type_character(&mut session, character);
        assert!(
            matches!(result, Err(OpError::Argument(_))),
            "{character:?}: expected an argument error, got {result:?}"
        );
    }
}
