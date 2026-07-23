//! INT 09h keyboard interrupt and INT 16h keyboard service handlers.

use common::{Cpu, TraceSink};

use super::{
    AtBus, BIOS_CODE_SEGMENT, METADATA_COLD_ENTRY, METADATA_CONTROL_BREAK_HELPER,
    METADATA_PAUSE_WAIT_LOOP, iret_stack_base,
};

/// Linear base address of the BIOS data area segment 0x40.
const BDA_SEGMENT_BASE: u32 = 0x400;
/// BIOS data area: keyboard shift flags byte 1.
const BDA_KEYBOARD_FLAGS_1: u32 = 0x417;
/// BIOS data area: keyboard shift flags byte 2.
const BDA_KEYBOARD_FLAGS_2: u32 = 0x418;
/// BIOS data area: Alt-numpad decimal entry accumulator.
const BDA_ALT_NUMPAD_ACCUMULATOR: u32 = 0x419;
/// BIOS data area: keyboard buffer head pointer (word).
const BDA_KEYBOARD_HEAD: u32 = 0x41A;
/// BIOS data area: keyboard buffer tail pointer (word).
const BDA_KEYBOARD_TAIL: u32 = 0x41C;
/// BIOS data area: break flag.
const BDA_BREAK_FLAG: u32 = 0x471;
/// BIOS data area: reset flag (word, 0x1234 requests a warm boot).
const BDA_RESET_FLAG: u32 = 0x472;
/// BIOS data area: keyboard buffer start offset (word).
const BDA_KEYBOARD_BUFFER_START: u32 = 0x480;
/// BIOS data area: keyboard buffer end offset (word).
const BDA_KEYBOARD_BUFFER_END: u32 = 0x482;
/// BIOS data area: keyboard mode/type flags.
const BDA_KEYBOARD_MODE: u32 = 0x496;
/// BIOS data area: keyboard LED flags.
const BDA_KEYBOARD_LEDS: u32 = 0x497;

/// Shift flags 1: right shift key pressed.
const FLAG1_RIGHT_SHIFT: u8 = 0x01;
/// Shift flags 1: left shift key pressed.
const FLAG1_LEFT_SHIFT: u8 = 0x02;
/// Shift flags 1: either control key pressed.
const FLAG1_CONTROL: u8 = 0x04;
/// Shift flags 1: either alt key pressed.
const FLAG1_ALT: u8 = 0x08;
/// Shift flags 1: scroll lock active.
const FLAG1_SCROLL_ACTIVE: u8 = 0x10;
/// Shift flags 1: num lock active.
const FLAG1_NUM_ACTIVE: u8 = 0x20;
/// Shift flags 1: caps lock active.
const FLAG1_CAPS_ACTIVE: u8 = 0x40;
/// Shift flags 1: insert active.
const FLAG1_INSERT_ACTIVE: u8 = 0x80;

/// Shift flags 2: left control key pressed.
const FLAG2_LEFT_CONTROL: u8 = 0x01;
/// Shift flags 2: left alt key pressed.
const FLAG2_LEFT_ALT: u8 = 0x02;
/// Shift flags 2: SysReq key pressed.
const FLAG2_SYSREQ_PRESSED: u8 = 0x04;
/// Shift flags 2: pause state active.
const FLAG2_PAUSE_ACTIVE: u8 = 0x08;
/// Shift flags 2: scroll lock key pressed.
const FLAG2_SCROLL_PRESSED: u8 = 0x10;
/// Shift flags 2: num lock key pressed.
const FLAG2_NUM_PRESSED: u8 = 0x20;
/// Shift flags 2: caps lock key pressed.
const FLAG2_CAPS_PRESSED: u8 = 0x40;
/// Shift flags 2: insert key pressed.
const FLAG2_INSERT_PRESSED: u8 = 0x80;

/// Keyboard mode: last scancode was the E1 pause prefix.
const MODE_LAST_E1: u8 = 0x01;
/// Keyboard mode: last scancode was the E0 extended prefix.
const MODE_LAST_E0: u8 = 0x02;
/// Keyboard mode: right control key pressed.
const MODE_RIGHT_CONTROL: u8 = 0x04;
/// Keyboard mode: right alt key pressed.
const MODE_RIGHT_ALT: u8 = 0x08;

/// Keyboard LED flags: scroll lock.
const LED_SCROLL: u8 = 0x01;
/// Keyboard LED flags: num lock.
const LED_NUM: u8 = 0x02;
/// Keyboard LED flags: caps lock.
const LED_CAPS: u8 = 0x04;
/// Keyboard LED flags: the three lock LED mirror bits.
const LED_STATE_MASK: u8 = 0x07;
/// Keyboard LED flags: the keyboard acknowledged the LED command.
const LED_ACKNOWLEDGED: u8 = 0x10;
/// Keyboard LED flags: the keyboard asked for a resend.
const LED_RESEND_SEEN: u8 = 0x20;
/// Keyboard LED flags: an LED update is in progress.
const LED_UPDATE_IN_PROGRESS: u8 = 0x40;

/// KBC data port, the keyboard command and response path.
const KBC_DATA_PORT: u16 = 0x60;
/// Keyboard command: set the lock LEDs, one parameter byte follows.
const KEYBOARD_COMMAND_SET_LEDS: u8 = 0xED;
/// Keyboard response: the command was accepted.
const KEYBOARD_RESPONSE_ACK: u8 = 0xFA;
/// Keyboard response: resend the last command.
const KEYBOARD_RESPONSE_RESEND: u8 = 0xFE;

/// Break flag: Ctrl-Break was pressed.
const BREAK_FLAG_PRESSED: u8 = 0x80;
/// Reset flag value requesting a warm boot.
const WARM_BOOT_FLAG: u16 = 0x1234;
/// FLAGS register: interrupt enable.
const FLAG_INTERRUPT_ENABLE: u16 = 0x0200;

/// Release bit of a set-1 scancode.
const KEY_RELEASE_FLAG: u8 = 0x80;
/// Set-1 scancode: control (right control when E0-prefixed).
const SCANCODE_CONTROL: u8 = 0x1D;
/// Set-1 scancode: left shift (fake shift when E0-prefixed).
const SCANCODE_LEFT_SHIFT: u8 = 0x2A;
/// Set-1 scancode: right shift (fake shift when E0-prefixed).
const SCANCODE_RIGHT_SHIFT: u8 = 0x36;
/// Set-1 scancode: alt (right alt when E0-prefixed).
const SCANCODE_ALT: u8 = 0x38;
/// Set-1 scancode: caps lock.
const SCANCODE_CAPS_LOCK: u8 = 0x3A;
/// Set-1 scancode: num lock.
const SCANCODE_NUM_LOCK: u8 = 0x45;
/// Set-1 scancode: scroll lock (Ctrl-Break when E0-prefixed).
const SCANCODE_SCROLL_LOCK: u8 = 0x46;
/// Set-1 scancode: keypad 0 / insert.
const SCANCODE_KEYPAD_INSERT: u8 = 0x52;
/// Set-1 scancode: keypad period / delete.
const SCANCODE_KEYPAD_DELETE: u8 = 0x53;
/// Set-1 scancode: SysReq.
const SCANCODE_SYSTEM_REQUEST: u8 = 0x54;
/// Prefix byte marking an E0-extended key sequence.
const SCANCODE_EXTENDED_PREFIX: u8 = 0xE0;
/// Prefix byte starting the E1 pause key sequence.
const SCANCODE_PAUSE_PREFIX: u8 = 0xE1;

/// Buffer entry marker: this key and modifier combination produces no
/// keystroke, matching the fillers of the real AT BIOS key tables.
const NO_KEYSTROKE: u16 = 0xFFFF;

/// INT 16h AH=09h functionality bitmap: AH=10h-12h and AX=0305h supported.
const KEYBOARD_FUNCTIONALITY: u8 = 0x24;
/// INT 16h AH=03h subfunction: set typematic rate and delay.
const TYPEMATIC_SET_RATE_AND_DELAY: u8 = 0x05;

/// Buffer entries for one scancode across the four modifier columns of the
/// AT BIOS key tables (scan code high byte, ASCII low byte).
struct KeyTranslation {
    normal: u16,
    shift: u16,
    control: u16,
    alt: u16,
}

/// Builds one translation table row.
const fn key(normal: u16, shift: u16, control: u16, alt: u16) -> KeyTranslation {
    KeyTranslation {
        normal,
        shift,
        control,
        alt,
    }
}

/// Builds a row for a key that never produces a keystroke by itself
/// (modifiers, lock keys, and unused scancodes).
const fn no_key() -> KeyTranslation {
    key(NO_KEYSTROKE, NO_KEYSTROKE, NO_KEYSTROKE, NO_KEYSTROKE)
}

/// US layout translation for set-1 make codes 0x00-0x58, indexed by
/// scancode. Values follow the standard AT BIOS scan code tables.
#[rustfmt::skip]
const KEY_TRANSLATIONS: [KeyTranslation; 0x59] = [
    no_key(),                                   // 0x00 (none)
    key(0x011B, 0x011B, 0x011B, 0x0100),        // 0x01 Esc
    key(0x0231, 0x0221, NO_KEYSTROKE, 0x7800),  // 0x02 1
    key(0x0332, 0x0340, 0x0300, 0x7900),        // 0x03 2
    key(0x0433, 0x0423, NO_KEYSTROKE, 0x7A00),  // 0x04 3
    key(0x0534, 0x0524, NO_KEYSTROKE, 0x7B00),  // 0x05 4
    key(0x0635, 0x0625, NO_KEYSTROKE, 0x7C00),  // 0x06 5
    key(0x0736, 0x075E, 0x071E, 0x7D00),        // 0x07 6
    key(0x0837, 0x0826, NO_KEYSTROKE, 0x7E00),  // 0x08 7
    key(0x0938, 0x092A, NO_KEYSTROKE, 0x7F00),  // 0x09 8
    key(0x0A39, 0x0A28, NO_KEYSTROKE, 0x8000),  // 0x0A 9
    key(0x0B30, 0x0B29, NO_KEYSTROKE, 0x8100),  // 0x0B 0
    key(0x0C2D, 0x0C5F, 0x0C1F, 0x8200),        // 0x0C -
    key(0x0D3D, 0x0D2B, NO_KEYSTROKE, 0x8300),  // 0x0D =
    key(0x0E08, 0x0E08, 0x0E7F, 0x0E00),        // 0x0E Backspace
    key(0x0F09, 0x0F00, 0x9400, 0xA500),        // 0x0F Tab
    key(0x1071, 0x1051, 0x1011, 0x1000),        // 0x10 Q
    key(0x1177, 0x1157, 0x1117, 0x1100),        // 0x11 W
    key(0x1265, 0x1245, 0x1205, 0x1200),        // 0x12 E
    key(0x1372, 0x1352, 0x1312, 0x1300),        // 0x13 R
    key(0x1474, 0x1454, 0x1414, 0x1400),        // 0x14 T
    key(0x1579, 0x1559, 0x1519, 0x1500),        // 0x15 Y
    key(0x1675, 0x1655, 0x1615, 0x1600),        // 0x16 U
    key(0x1769, 0x1749, 0x1709, 0x1700),        // 0x17 I
    key(0x186F, 0x184F, 0x180F, 0x1800),        // 0x18 O
    key(0x1970, 0x1950, 0x1910, 0x1900),        // 0x19 P
    key(0x1A5B, 0x1A7B, 0x1A1B, 0x1A00),        // 0x1A [
    key(0x1B5D, 0x1B7D, 0x1B1D, 0x1B00),        // 0x1B ]
    key(0x1C0D, 0x1C0D, 0x1C0A, 0xA600),        // 0x1C Enter
    no_key(),                                   // 0x1D Ctrl
    key(0x1E61, 0x1E41, 0x1E01, 0x1E00),        // 0x1E A
    key(0x1F73, 0x1F53, 0x1F13, 0x1F00),        // 0x1F S
    key(0x2064, 0x2044, 0x2004, 0x2000),        // 0x20 D
    key(0x2166, 0x2146, 0x2106, 0x2100),        // 0x21 F
    key(0x2267, 0x2247, 0x2207, 0x2200),        // 0x22 G
    key(0x2368, 0x2348, 0x2308, 0x2300),        // 0x23 H
    key(0x246A, 0x244A, 0x240A, 0x2400),        // 0x24 J
    key(0x256B, 0x254B, 0x250B, 0x2500),        // 0x25 K
    key(0x266C, 0x264C, 0x260C, 0x2600),        // 0x26 L
    key(0x273B, 0x273A, NO_KEYSTROKE, 0x2700),  // 0x27 ;
    key(0x2827, 0x2822, NO_KEYSTROKE, NO_KEYSTROKE), // 0x28 '
    key(0x2960, 0x297E, NO_KEYSTROKE, NO_KEYSTROKE), // 0x29 `
    no_key(),                                   // 0x2A Left shift
    key(0x2B5C, 0x2B7C, 0x2B1C, 0x2600),        // 0x2B backslash
    key(0x2C7A, 0x2C5A, 0x2C1A, 0x2C00),        // 0x2C Z
    key(0x2D78, 0x2D58, 0x2D18, 0x2D00),        // 0x2D X
    key(0x2E63, 0x2E43, 0x2E03, 0x2E00),        // 0x2E C
    key(0x2F76, 0x2F56, 0x2F16, 0x2F00),        // 0x2F V
    key(0x3062, 0x3042, 0x3002, 0x3000),        // 0x30 B
    key(0x316E, 0x314E, 0x310E, 0x3100),        // 0x31 N
    key(0x326D, 0x324D, 0x320D, 0x3200),        // 0x32 M
    key(0x332C, 0x333C, NO_KEYSTROKE, NO_KEYSTROKE), // 0x33 ,
    key(0x342E, 0x343E, NO_KEYSTROKE, NO_KEYSTROKE), // 0x34 .
    key(0x352F, 0x353F, NO_KEYSTROKE, NO_KEYSTROKE), // 0x35 /
    no_key(),                                   // 0x36 Right shift
    key(0x372A, 0x372A, 0x9600, 0x3700),        // 0x37 Keypad *
    no_key(),                                   // 0x38 Alt
    key(0x3920, 0x3920, 0x3920, 0x3920),        // 0x39 Space
    no_key(),                                   // 0x3A Caps lock
    key(0x3B00, 0x5400, 0x5E00, 0x6800),        // 0x3B F1
    key(0x3C00, 0x5500, 0x5F00, 0x6900),        // 0x3C F2
    key(0x3D00, 0x5600, 0x6000, 0x6A00),        // 0x3D F3
    key(0x3E00, 0x5700, 0x6100, 0x6B00),        // 0x3E F4
    key(0x3F00, 0x5800, 0x6200, 0x6C00),        // 0x3F F5
    key(0x4000, 0x5900, 0x6300, 0x6D00),        // 0x40 F6
    key(0x4100, 0x5A00, 0x6400, 0x6E00),        // 0x41 F7
    key(0x4200, 0x5B00, 0x6500, 0x6F00),        // 0x42 F8
    key(0x4300, 0x5C00, 0x6600, 0x7000),        // 0x43 F9
    key(0x4400, 0x5D00, 0x6700, 0x7100),        // 0x44 F10
    no_key(),                                   // 0x45 Num lock
    no_key(),                                   // 0x46 Scroll lock
    key(0x4700, 0x4737, 0x7700, NO_KEYSTROKE),  // 0x47 Keypad 7 / Home
    key(0x4800, 0x4838, 0x8D00, NO_KEYSTROKE),  // 0x48 Keypad 8 / Up
    key(0x4900, 0x4939, 0x8400, NO_KEYSTROKE),  // 0x49 Keypad 9 / PgUp
    key(0x4A2D, 0x4A2D, 0x8E00, 0x4A00),        // 0x4A Keypad -
    key(0x4B00, 0x4B34, 0x7300, NO_KEYSTROKE),  // 0x4B Keypad 4 / Left
    key(NO_KEYSTROKE, 0x4C35, 0x8F00, NO_KEYSTROKE), // 0x4C Keypad 5
    key(0x4D00, 0x4D36, 0x7400, NO_KEYSTROKE),  // 0x4D Keypad 6 / Right
    key(0x4E2B, 0x4E2B, NO_KEYSTROKE, 0x4E00),  // 0x4E Keypad +
    key(0x4F00, 0x4F31, 0x7500, NO_KEYSTROKE),  // 0x4F Keypad 1 / End
    key(0x5000, 0x5032, 0x9100, NO_KEYSTROKE),  // 0x50 Keypad 2 / Down
    key(0x5100, 0x5133, 0x7600, NO_KEYSTROKE),  // 0x51 Keypad 3 / PgDn
    key(0x5200, 0x5230, 0x9200, NO_KEYSTROKE),  // 0x52 Keypad 0 / Ins
    key(0x5300, 0x532E, 0x9300, NO_KEYSTROKE),  // 0x53 Keypad . / Del
    no_key(),                                   // 0x54 SysReq
    no_key(),                                   // 0x55 (none)
    no_key(),                                   // 0x56 (102-key only)
    key(0x8500, 0x8700, 0x8900, 0x8B00),        // 0x57 F11
    key(0x8600, 0x8800, 0x8A00, 0x8C00),        // 0x58 F12
];

/// The digit a keypad scancode enters into the Alt-numpad accumulator.
fn keypad_digit(key_code: u8) -> Option<u8> {
    match key_code {
        0x47 => Some(7),
        0x48 => Some(8),
        0x49 => Some(9),
        0x4B => Some(4),
        0x4C => Some(5),
        0x4D => Some(6),
        0x4F => Some(1),
        0x50 => Some(2),
        0x51 => Some(3),
        0x52 => Some(0),
        _ => None,
    }
}

/// Whether the scancode belongs to the numeric keypad block affected by
/// the NumLock state.
fn is_keypad_key(key_code: u8) -> bool {
    (0x47..=0x53).contains(&key_code)
}

/// Whether the scancode is a letter key affected by CapsLock.
fn is_letter_key(key_code: u8) -> bool {
    matches!(key_code, 0x10..=0x19 | 0x1E..=0x26 | 0x2C..=0x32)
}

/// Whether the scancode is one of the 106-key Japanese keys, buffered with
/// a zero ASCII byte for the DOS/V keyboard drivers.
fn is_japanese_key(key_code: u8) -> bool {
    matches!(key_code, 0x70 | 0x73 | 0x79 | 0x7B | 0x7D)
}

/// Rewrites an enhanced-only buffer entry into its 83/84-key compatible
/// form for INT 16h AH=00h/01h, or drops it entirely.
fn compatibility_filter(entry: u16) -> Option<u16> {
    let scan = entry >> 8;
    let ascii = entry & 0x00FF;
    if scan == 0x00E0 && ascii != 0 {
        // Keypad Enter and divide: substitute the classic scan code.
        let classic_scan = if ascii == 0x2F { 0x3500 } else { 0x1C00 };
        return Some(classic_scan | ascii);
    }
    if ascii == 0x00E0 && scan != 0 {
        return Some(scan << 8);
    }
    if (0x0085..=0x008C).contains(&scan) {
        return None;
    }
    Some(entry)
}

/// How the INT 09h handler leaves the interrupted program: continue where
/// it was, or retarget the IRET frame first.
enum KeyboardInterruptAction {
    Continue,
    WarmReboot,
    ControlBreak,
    PauseLoop,
}

impl<T: TraceSink> AtBus<T> {
    /// INT 09h: translates the scancode latched through port 0x07F1 into the
    /// BDA keyboard state and sends the EOI for IRQ 1.
    pub(super) fn hle_int09h(&mut self, cpu: &mut impl Cpu) {
        let raw_code = self.hle_scancode;

        if raw_code == KEYBOARD_RESPONSE_ACK || raw_code == KEYBOARD_RESPONSE_RESEND {
            self.handle_keyboard_response(raw_code);
            self.pic.write_port0(0, 0x20);
            return;
        }

        if raw_code == SCANCODE_EXTENDED_PREFIX || raw_code == SCANCODE_PAUSE_PREFIX {
            let prefix_flag = if raw_code == SCANCODE_EXTENDED_PREFIX {
                MODE_LAST_E0
            } else {
                MODE_LAST_E1
            };
            let mode = self.read_mem_byte(BDA_KEYBOARD_MODE);
            self.write_mem_byte(BDA_KEYBOARD_MODE, mode | prefix_flag);
            self.pic.write_port0(0, 0x20);
            return;
        }

        let mode = self.read_mem_byte(BDA_KEYBOARD_MODE);
        let extended = mode & MODE_LAST_E0 != 0;
        let pause_prefixed = mode & MODE_LAST_E1 != 0;
        self.write_mem_byte(BDA_KEYBOARD_MODE, mode & !(MODE_LAST_E0 | MODE_LAST_E1));

        let action = self.process_keyboard_scancode(raw_code, extended, pause_prefixed);
        self.pic.write_port0(0, 0x20);

        match action {
            KeyboardInterruptAction::Continue => {}
            KeyboardInterruptAction::WarmReboot => self.retarget_frame_to_cold_entry(cpu),
            KeyboardInterruptAction::ControlBreak => {
                self.retarget_frame_to_helper(cpu, METADATA_CONTROL_BREAK_HELPER);
            }
            KeyboardInterruptAction::PauseLoop => {
                self.retarget_frame_to_helper(cpu, METADATA_PAUSE_WAIT_LOOP);
            }
        }
    }

    /// Updates all BDA keyboard state for one scancode and decides how the
    /// interrupted program resumes.
    fn process_keyboard_scancode(
        &mut self,
        raw_code: u8,
        extended: bool,
        pause_prefixed: bool,
    ) -> KeyboardInterruptAction {
        let key_code = raw_code & !KEY_RELEASE_FLAG;
        let release = raw_code & KEY_RELEASE_FLAG != 0;

        if pause_prefixed {
            return self.process_pause_sequence(key_code, release);
        }

        // The keyboard brackets grey keys with fake shifts when NumLock or
        // Shift interact with them; they carry no state of their own.
        if extended && (key_code == SCANCODE_LEFT_SHIFT || key_code == SCANCODE_RIGHT_SHIFT) {
            return KeyboardInterruptAction::Continue;
        }

        // An active pause ends on the next make code, which is consumed.
        if !release {
            let flags2 = self.read_mem_byte(BDA_KEYBOARD_FLAGS_2);
            if flags2 & FLAG2_PAUSE_ACTIVE != 0 {
                self.write_mem_byte(BDA_KEYBOARD_FLAGS_2, flags2 & !FLAG2_PAUSE_ACTIVE);
                return KeyboardInterruptAction::Continue;
            }
        }

        match key_code {
            SCANCODE_LEFT_SHIFT => {
                self.update_shift_flag1(FLAG1_LEFT_SHIFT, !release);
                return KeyboardInterruptAction::Continue;
            }
            SCANCODE_RIGHT_SHIFT => {
                self.update_shift_flag1(FLAG1_RIGHT_SHIFT, !release);
                return KeyboardInterruptAction::Continue;
            }
            SCANCODE_CONTROL => {
                self.update_control_keys(extended, release);
                return KeyboardInterruptAction::Continue;
            }
            SCANCODE_ALT => {
                self.update_alt_keys(extended, release);
                return KeyboardInterruptAction::Continue;
            }
            SCANCODE_CAPS_LOCK if !extended => {
                self.handle_lock_key(release, FLAG2_CAPS_PRESSED, FLAG1_CAPS_ACTIVE, LED_CAPS);
                return KeyboardInterruptAction::Continue;
            }
            SCANCODE_NUM_LOCK if !extended => {
                let flags1 = self.read_mem_byte(BDA_KEYBOARD_FLAGS_1);
                if !release && flags1 & FLAG1_CONTROL != 0 {
                    let flags2 = self.read_mem_byte(BDA_KEYBOARD_FLAGS_2);
                    self.write_mem_byte(BDA_KEYBOARD_FLAGS_2, flags2 | FLAG2_PAUSE_ACTIVE);
                    return KeyboardInterruptAction::PauseLoop;
                }
                self.handle_lock_key(release, FLAG2_NUM_PRESSED, FLAG1_NUM_ACTIVE, LED_NUM);
                return KeyboardInterruptAction::Continue;
            }
            SCANCODE_SCROLL_LOCK => {
                let flags1 = self.read_mem_byte(BDA_KEYBOARD_FLAGS_1);
                if extended || flags1 & FLAG1_CONTROL != 0 {
                    if release {
                        return KeyboardInterruptAction::Continue;
                    }
                    return self.control_break_action();
                }
                self.handle_lock_key(
                    release,
                    FLAG2_SCROLL_PRESSED,
                    FLAG1_SCROLL_ACTIVE,
                    LED_SCROLL,
                );
                return KeyboardInterruptAction::Continue;
            }
            SCANCODE_SYSTEM_REQUEST => {
                self.update_shift_flag2(FLAG2_SYSREQ_PRESSED, !release);
                return KeyboardInterruptAction::Continue;
            }
            _ => {}
        }

        let flags1 = self.read_mem_byte(BDA_KEYBOARD_FLAGS_1);

        if key_code == SCANCODE_KEYPAD_DELETE
            && !release
            && flags1 & FLAG1_CONTROL != 0
            && flags1 & FLAG1_ALT != 0
        {
            self.write_mem_word(BDA_RESET_FLAG, WARM_BOOT_FLAG);
            self.needs_full_reinit = true;
            return KeyboardInterruptAction::WarmReboot;
        }

        if release {
            if key_code == SCANCODE_KEYPAD_INSERT {
                self.update_shift_flag2(FLAG2_INSERT_PRESSED, false);
            }
            return KeyboardInterruptAction::Continue;
        }

        if !extended
            && flags1 & FLAG1_ALT != 0
            && let Some(digit) = keypad_digit(key_code)
        {
            let accumulator = self.read_mem_byte(BDA_ALT_NUMPAD_ACCUMULATOR);
            self.write_mem_byte(
                BDA_ALT_NUMPAD_ACCUMULATOR,
                accumulator.wrapping_mul(10).wrapping_add(digit),
            );
            return KeyboardInterruptAction::Continue;
        }

        let entry = if extended {
            self.translate_extended_key(key_code)
        } else if is_japanese_key(key_code) {
            u16::from(key_code) << 8
        } else if usize::from(key_code) < KEY_TRANSLATIONS.len() {
            self.translate_regular_key(key_code)
        } else {
            NO_KEYSTROKE
        };

        if key_code == SCANCODE_KEYPAD_INSERT && (entry == 0x5200 || entry == 0x52E0) {
            let flags1 = self.read_mem_byte(BDA_KEYBOARD_FLAGS_1);
            self.write_mem_byte(BDA_KEYBOARD_FLAGS_1, flags1 ^ FLAG1_INSERT_ACTIVE);
            self.update_shift_flag2(FLAG2_INSERT_PRESSED, true);
        }

        if entry != NO_KEYSTROKE {
            self.buffer_key_entry(entry);
        }
        KeyboardInterruptAction::Continue
    }

    /// Handles the scancodes following an E1 pause prefix. The 101-key Pause
    /// key sends E1 1D 45 (and E1 9D C5 on release); pause activates on the
    /// final 45 make and every byte of the sequence is consumed.
    fn process_pause_sequence(&mut self, key_code: u8, release: bool) -> KeyboardInterruptAction {
        if key_code == SCANCODE_CONTROL {
            let mode = self.read_mem_byte(BDA_KEYBOARD_MODE);
            self.write_mem_byte(BDA_KEYBOARD_MODE, mode | MODE_LAST_E1);
            return KeyboardInterruptAction::Continue;
        }
        if key_code == SCANCODE_NUM_LOCK && !release {
            let flags2 = self.read_mem_byte(BDA_KEYBOARD_FLAGS_2);
            self.write_mem_byte(BDA_KEYBOARD_FLAGS_2, flags2 | FLAG2_PAUSE_ACTIVE);
            return KeyboardInterruptAction::PauseLoop;
        }
        KeyboardInterruptAction::Continue
    }

    /// Sets or clears a shift flags 1 bit.
    fn update_shift_flag1(&mut self, flag: u8, pressed: bool) {
        let flags1 = self.read_mem_byte(BDA_KEYBOARD_FLAGS_1);
        let flags1 = if pressed {
            flags1 | flag
        } else {
            flags1 & !flag
        };
        self.write_mem_byte(BDA_KEYBOARD_FLAGS_1, flags1);
    }

    /// Sets or clears a shift flags 2 bit.
    fn update_shift_flag2(&mut self, flag: u8, pressed: bool) {
        let flags2 = self.read_mem_byte(BDA_KEYBOARD_FLAGS_2);
        let flags2 = if pressed {
            flags2 | flag
        } else {
            flags2 & !flag
        };
        self.write_mem_byte(BDA_KEYBOARD_FLAGS_2, flags2);
    }

    /// Tracks a left or right control key and recomputes the combined flag.
    fn update_control_keys(&mut self, extended: bool, release: bool) {
        if extended {
            let mode = self.read_mem_byte(BDA_KEYBOARD_MODE);
            let mode = if release {
                mode & !MODE_RIGHT_CONTROL
            } else {
                mode | MODE_RIGHT_CONTROL
            };
            self.write_mem_byte(BDA_KEYBOARD_MODE, mode);
        } else {
            self.update_shift_flag2(FLAG2_LEFT_CONTROL, !release);
        }
        let left = self.read_mem_byte(BDA_KEYBOARD_FLAGS_2) & FLAG2_LEFT_CONTROL != 0;
        let right = self.read_mem_byte(BDA_KEYBOARD_MODE) & MODE_RIGHT_CONTROL != 0;
        self.update_shift_flag1(FLAG1_CONTROL, left || right);
    }

    /// Tracks a left or right alt key, recomputes the combined flag, and
    /// flushes the Alt-numpad accumulator when the last alt key goes up.
    fn update_alt_keys(&mut self, extended: bool, release: bool) {
        if extended {
            let mode = self.read_mem_byte(BDA_KEYBOARD_MODE);
            let mode = if release {
                mode & !MODE_RIGHT_ALT
            } else {
                mode | MODE_RIGHT_ALT
            };
            self.write_mem_byte(BDA_KEYBOARD_MODE, mode);
        } else {
            self.update_shift_flag2(FLAG2_LEFT_ALT, !release);
        }
        let left = self.read_mem_byte(BDA_KEYBOARD_FLAGS_2) & FLAG2_LEFT_ALT != 0;
        let right = self.read_mem_byte(BDA_KEYBOARD_MODE) & MODE_RIGHT_ALT != 0;
        self.update_shift_flag1(FLAG1_ALT, left || right);

        if release && !left && !right {
            let accumulator = self.read_mem_byte(BDA_ALT_NUMPAD_ACCUMULATOR);
            if accumulator != 0 {
                self.buffer_key_entry(u16::from(accumulator));
                self.write_mem_byte(BDA_ALT_NUMPAD_ACCUMULATOR, 0);
            }
        }
    }

    /// Lock key handling: toggle the active flag and its LED bit on the
    /// first make, using the pressed bit as the typematic debounce, then
    /// program the new LED state into the keyboard.
    fn handle_lock_key(&mut self, release: bool, pressed_flag: u8, active_flag: u8, led_flag: u8) {
        if release {
            self.update_shift_flag2(pressed_flag, false);
            return;
        }
        let flags2 = self.read_mem_byte(BDA_KEYBOARD_FLAGS_2);
        if flags2 & pressed_flag != 0 {
            return;
        }
        self.write_mem_byte(BDA_KEYBOARD_FLAGS_2, flags2 | pressed_flag);
        let flags1 = self.read_mem_byte(BDA_KEYBOARD_FLAGS_1) ^ active_flag;
        self.write_mem_byte(BDA_KEYBOARD_FLAGS_1, flags1);
        let leds = self.read_mem_byte(BDA_KEYBOARD_LEDS) & !(LED_ACKNOWLEDGED | LED_RESEND_SEEN);
        let leds = if flags1 & active_flag != 0 {
            leds | led_flag
        } else {
            leds & !led_flag
        };
        self.write_mem_byte(BDA_KEYBOARD_LEDS, leds);
        self.send_keyboard_led_command();
    }

    /// Programs the three lock LEDs from the BDA mirror through the keyboard
    /// 0xED command.
    ///
    /// The acknowledge bytes are left in the controller queue and arrive as
    /// IRQ 1 interrupts, which `handle_keyboard_response` consumes. Draining
    /// them here would swallow the host scancodes queued ahead of them.
    fn send_keyboard_led_command(&mut self) {
        let leds = self.read_mem_byte(BDA_KEYBOARD_LEDS);
        self.write_mem_byte(BDA_KEYBOARD_LEDS, leds | LED_UPDATE_IN_PROGRESS);
        self.io_write(KBC_DATA_PORT, KEYBOARD_COMMAND_SET_LEDS);
        self.io_write(KBC_DATA_PORT, leds & LED_STATE_MASK);
    }

    /// Consumes a keyboard response to the LED command. Neither byte may
    /// reach the translation path, where 0xFA and 0xFE would look like
    /// releases of the non-existent keys 0x7A and 0x7E.
    ///
    /// The basic assurance test result 0xAA is deliberately not filtered: in
    /// the translated set-1 stream it is indistinguishable from a left shift
    /// release, and the HLE BIOS only resets the keyboard inside the POST,
    /// which consumes the responses itself.
    fn handle_keyboard_response(&mut self, response: u8) {
        let leds = self.read_mem_byte(BDA_KEYBOARD_LEDS);
        if response == KEYBOARD_RESPONSE_ACK {
            let leds = (leds & !LED_UPDATE_IN_PROGRESS) | LED_ACKNOWLEDGED;
            self.write_mem_byte(BDA_KEYBOARD_LEDS, leds);
            return;
        }
        // A resend request is honored once per LED update, tracked in the
        // mirror byte so save states need no extra field.
        if leds & LED_RESEND_SEEN == 0 {
            self.write_mem_byte(BDA_KEYBOARD_LEDS, leds | LED_RESEND_SEEN);
            self.send_keyboard_led_command();
        }
    }

    /// Ctrl-Break: flush the buffer, store the null keystroke, raise the
    /// break flag, and run the guest INT 1Bh handler through the ROM helper.
    fn control_break_action(&mut self) -> KeyboardInterruptAction {
        self.flush_keyboard_buffer();
        self.buffer_key_entry(0x0000);
        let break_flag = self.read_mem_byte(BDA_BREAK_FLAG);
        self.write_mem_byte(BDA_BREAK_FLAG, break_flag | BREAK_FLAG_PRESSED);
        KeyboardInterruptAction::ControlBreak
    }

    /// Translates a non-extended key through the modifier columns.
    fn translate_regular_key(&mut self, key_code: u8) -> u16 {
        let translation = &KEY_TRANSLATIONS[usize::from(key_code)];
        let flags1 = self.read_mem_byte(BDA_KEYBOARD_FLAGS_1);
        let shift_held = flags1 & (FLAG1_LEFT_SHIFT | FLAG1_RIGHT_SHIFT) != 0;

        if flags1 & FLAG1_ALT != 0 {
            translation.alt
        } else if flags1 & FLAG1_CONTROL != 0 {
            translation.control
        } else if is_keypad_key(key_code) {
            let digits = (flags1 & FLAG1_NUM_ACTIVE != 0) != shift_held;
            if digits {
                translation.shift
            } else {
                translation.normal
            }
        } else if is_letter_key(key_code) {
            let caps = flags1 & FLAG1_CAPS_ACTIVE != 0;
            if caps != shift_held {
                translation.shift
            } else {
                translation.normal
            }
        } else if shift_held {
            translation.shift
        } else {
            translation.normal
        }
    }

    /// Translates an E0-prefixed key through the modifier columns.
    fn translate_extended_key(&mut self, key_code: u8) -> u16 {
        let (normal, control, alt) = match key_code {
            0x1C => (0xE00D, 0xE00A, 0xA600),             // Keypad Enter
            0x35 => (0xE02F, 0x9500, 0xA400),             // Keypad /
            0x37 => (NO_KEYSTROKE, 0x7200, NO_KEYSTROKE), // Print screen
            0x47 => (0x47E0, 0x77E0, 0x9700),             // Home
            0x48 => (0x48E0, 0x8DE0, 0x9800),             // Up
            0x49 => (0x49E0, 0x84E0, 0x9900),             // PgUp
            0x4B => (0x4BE0, 0x73E0, 0x9B00),             // Left
            0x4D => (0x4DE0, 0x74E0, 0x9D00),             // Right
            0x4F => (0x4FE0, 0x75E0, 0x9F00),             // End
            0x50 => (0x50E0, 0x91E0, 0xA000),             // Down
            0x51 => (0x51E0, 0x76E0, 0xA100),             // PgDn
            0x52 => (0x52E0, 0x92E0, 0xA200),             // Insert
            0x53 => (0x53E0, 0x93E0, 0xA300),             // Delete
            _ => (NO_KEYSTROKE, NO_KEYSTROKE, NO_KEYSTROKE),
        };
        let flags1 = self.read_mem_byte(BDA_KEYBOARD_FLAGS_1);
        if flags1 & FLAG1_ALT != 0 {
            alt
        } else if flags1 & FLAG1_CONTROL != 0 {
            control
        } else {
            normal
        }
    }

    /// Inserts one entry at the buffer tail. Returns false when the ring is
    /// full and the entry was dropped.
    fn buffer_key_entry(&mut self, entry: u16) -> bool {
        let start = self.read_mem_word(BDA_KEYBOARD_BUFFER_START);
        let end = self.read_mem_word(BDA_KEYBOARD_BUFFER_END);
        let head = self.read_mem_word(BDA_KEYBOARD_HEAD);
        let tail = self.read_mem_word(BDA_KEYBOARD_TAIL);

        let mut next_tail = tail.wrapping_add(2);
        if next_tail >= end {
            next_tail = start;
        }
        if next_tail == head {
            return false;
        }
        self.write_mem_word(BDA_SEGMENT_BASE + u32::from(tail), entry);
        self.write_mem_word(BDA_KEYBOARD_TAIL, next_tail);
        true
    }

    /// Removes and returns the entry at the buffer head.
    fn keyboard_buffer_pop(&mut self) -> Option<u16> {
        let entry = self.keyboard_buffer_peek()?;
        let start = self.read_mem_word(BDA_KEYBOARD_BUFFER_START);
        let end = self.read_mem_word(BDA_KEYBOARD_BUFFER_END);
        let head = self.read_mem_word(BDA_KEYBOARD_HEAD);
        let mut next_head = head.wrapping_add(2);
        if next_head >= end {
            next_head = start;
        }
        self.write_mem_word(BDA_KEYBOARD_HEAD, next_head);
        Some(entry)
    }

    /// Returns the entry at the buffer head without removing it.
    fn keyboard_buffer_peek(&mut self) -> Option<u16> {
        let head = self.read_mem_word(BDA_KEYBOARD_HEAD);
        let tail = self.read_mem_word(BDA_KEYBOARD_TAIL);
        if head == tail {
            return None;
        }
        Some(self.read_mem_word(BDA_SEGMENT_BASE + u32::from(head)))
    }

    /// Empties the keyboard buffer.
    fn flush_keyboard_buffer(&mut self) {
        let start = self.read_mem_word(BDA_KEYBOARD_BUFFER_START);
        self.write_mem_word(BDA_KEYBOARD_HEAD, start);
        self.write_mem_word(BDA_KEYBOARD_TAIL, start);
    }

    /// Rewrites the IRET frame in place so the stub's IRET enters the ROM
    /// cold entry, re-running POST and the bootstrap.
    fn retarget_frame_to_cold_entry(&mut self, cpu: &mut impl Cpu) {
        let cold_entry = self.stub_rom_metadata_word(METADATA_COLD_ENTRY);
        let frame_base = iret_stack_base(cpu);
        let flags = self.read_mem_word(frame_base.wrapping_add(4));
        self.write_mem_word(frame_base, cold_entry);
        self.write_mem_word(frame_base.wrapping_add(2), BIOS_CODE_SEGMENT);
        self.write_mem_word(frame_base.wrapping_add(4), flags & !FLAG_INTERRUPT_ENABLE);
    }

    /// Pushes a fabricated IRET frame entering a ROM helper routine. The
    /// helper frame goes below SP into the scratch space the popped AX/DX
    /// saves vacated, so the helper's final IRET consumes the untouched
    /// original frame and the interrupted program resumes with its exact
    /// stack pointer.
    pub(super) fn retarget_frame_to_helper(&mut self, cpu: &mut impl Cpu, metadata_offset: usize) {
        let helper = self.stub_rom_metadata_word(metadata_offset);
        let flags = self.read_mem_word(iret_stack_base(cpu).wrapping_add(4));
        cpu.set_sp(cpu.sp().wrapping_sub(6));
        let helper_base = iret_stack_base(cpu);
        self.write_mem_word(helper_base, helper);
        self.write_mem_word(helper_base.wrapping_add(2), BIOS_CODE_SEGMENT);
        self.write_mem_word(helper_base.wrapping_add(4), flags & !FLAG_INTERRUPT_ENABLE);
    }

    /// INT 16h keyboard services dispatch.
    pub(super) fn hle_int16h(&mut self, cpu: &mut impl Cpu) {
        match cpu.ah() {
            0x00 => self.int16h_read(cpu, true),
            0x01 => self.int16h_peek(cpu, true),
            0x02 => self.int16h_shift_status(cpu),
            0x03 => self.int16h_set_typematic(cpu),
            0x05 => self.int16h_push_key(cpu),
            0x09 => self.int16h_functionality(cpu),
            0x10 => self.int16h_read(cpu, false),
            0x11 => self.int16h_peek(cpu, false),
            0x12 => self.int16h_extended_shift_status(cpu),
            _ => {}
        }
    }

    /// AH=00h/10h: removes and returns the next keystroke in AX. Blocks on
    /// an empty buffer by rewinding the caller's IP to re-execute the INT
    /// 16h instruction until a key arrives.
    fn int16h_read(&mut self, cpu: &mut impl Cpu, filter_enhanced: bool) {
        loop {
            let Some(entry) = self.keyboard_buffer_pop() else {
                let frame_base = iret_stack_base(cpu);
                let caller_ip = self.read_mem_word(frame_base);
                self.write_mem_word(frame_base, caller_ip.wrapping_sub(2));

                // Force IF on so IRQ 1 can fire during the wait loop, and
                // burn cycles so the timeslice ends.
                let flags = self.read_mem_word(frame_base.wrapping_add(4));
                self.write_mem_word(frame_base.wrapping_add(4), flags | FLAG_INTERRUPT_ENABLE);
                self.pending_wait_cycles += 2000;
                return;
            };
            let entry = if filter_enhanced {
                match compatibility_filter(entry) {
                    Some(entry) => entry,
                    None => continue,
                }
            } else {
                entry
            };
            cpu.set_ax(entry);
            return;
        }
    }

    /// AH=01h/11h: returns the next keystroke in AX without removing it,
    /// with ZF set in the returned FLAGS when the buffer is empty.
    fn int16h_peek(&mut self, cpu: &mut impl Cpu, filter_enhanced: bool) {
        loop {
            let Some(entry) = self.keyboard_buffer_peek() else {
                self.set_iret_zf(cpu, true);
                return;
            };
            let entry = if filter_enhanced {
                match compatibility_filter(entry) {
                    Some(entry) => entry,
                    None => {
                        // Enhanced-only keystrokes are consumed while
                        // checking, like the IBM BIOS does.
                        self.keyboard_buffer_pop();
                        continue;
                    }
                }
            } else {
                entry
            };
            cpu.set_ax(entry);
            self.set_iret_zf(cpu, false);
            return;
        }
    }

    /// AH=02h: returns shift flags 1 in AL.
    fn int16h_shift_status(&mut self, cpu: &mut impl Cpu) {
        let flags1 = self.read_mem_byte(BDA_KEYBOARD_FLAGS_1);
        cpu.set_al(flags1);
    }

    /// AH=03h AL=05h: sets the keyboard typematic rate and delay. The real
    /// BIOS sends the 0xF3 command and consumes the ACK bytes itself, so
    /// the faithful net effect is the updated keyboard configuration.
    fn int16h_set_typematic(&mut self, cpu: &mut impl Cpu) {
        if cpu.al() != TYPEMATIC_SET_RATE_AND_DELAY {
            return;
        }
        let delay = ((cpu.bx() >> 8) as u8 & 0x03) << 5;
        let rate = cpu.bx() as u8 & 0x1F;
        self.kbc.keyboard.typematic = delay | rate;
    }

    /// AH=05h: pushes CX into the keyboard buffer. AL=0 on success, AL=1
    /// when the buffer is full.
    fn int16h_push_key(&mut self, cpu: &mut impl Cpu) {
        let stored = self.buffer_key_entry(cpu.cx());
        cpu.set_al(if stored { 0x00 } else { 0x01 });
    }

    /// AH=09h: returns the supported keyboard functionality bitmap in AL.
    fn int16h_functionality(&mut self, cpu: &mut impl Cpu) {
        cpu.set_al(KEYBOARD_FUNCTIONALITY);
    }

    /// AH=12h: returns shift flags 1 in AL and the extended shift states
    /// in AH.
    fn int16h_extended_shift_status(&mut self, cpu: &mut impl Cpu) {
        let flags1 = self.read_mem_byte(BDA_KEYBOARD_FLAGS_1);
        let flags2 = self.read_mem_byte(BDA_KEYBOARD_FLAGS_2);
        let mode = self.read_mem_byte(BDA_KEYBOARD_MODE);

        let mut extended_flags = 0u8;
        if flags2 & FLAG2_LEFT_CONTROL != 0 {
            extended_flags |= 0x01;
        }
        if flags2 & FLAG2_LEFT_ALT != 0 {
            extended_flags |= 0x02;
        }
        if mode & MODE_RIGHT_CONTROL != 0 {
            extended_flags |= 0x04;
        }
        if mode & MODE_RIGHT_ALT != 0 {
            extended_flags |= 0x08;
        }
        if flags2 & FLAG2_SCROLL_PRESSED != 0 {
            extended_flags |= 0x10;
        }
        if flags2 & FLAG2_NUM_PRESSED != 0 {
            extended_flags |= 0x20;
        }
        if flags2 & FLAG2_CAPS_PRESSED != 0 {
            extended_flags |= 0x40;
        }
        if flags2 & FLAG2_SYSREQ_PRESSED != 0 {
            extended_flags |= 0x80;
        }
        cpu.set_al(flags1);
        cpu.set_ah(extended_flags);
    }
}
