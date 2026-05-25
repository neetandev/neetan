use common::is_shift_jis_lead_byte;

#[derive(Debug, Clone)]
pub(crate) struct ByteField {
    bytes: Vec<u8>,
    pub(crate) cursor: usize,
}

impl ByteField {
    pub(crate) fn new(bytes: Vec<u8>) -> Self {
        let cursor = bytes.len();
        Self { bytes, cursor }
    }

    pub(crate) fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub(crate) fn set_bytes(&mut self, bytes: Vec<u8>) {
        self.bytes = bytes;
        self.cursor = self.bytes.len();
    }

    pub(crate) fn insert_byte(&mut self, byte: u8) {
        self.bytes.insert(self.cursor, byte);
        self.cursor += 1;
    }

    pub(crate) fn backspace(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
            self.bytes.remove(self.cursor);
        }
    }

    pub(crate) fn delete(&mut self) {
        if self.cursor < self.bytes.len() {
            self.bytes.remove(self.cursor);
        }
    }

    pub(crate) fn move_left(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
        }
    }

    pub(crate) fn move_right(&mut self) {
        if self.cursor < self.bytes.len() {
            self.cursor += 1;
        }
    }

    pub(crate) fn move_home(&mut self) {
        self.cursor = 0;
    }

    pub(crate) fn move_end(&mut self) {
        self.cursor = self.bytes.len();
    }
}

pub(crate) const SCAN_ROLL_UP: u8 = 0x36;
pub(crate) const SCAN_ROLL_DOWN: u8 = 0x37;
pub(crate) const SCAN_INSERT: u8 = 0x38;
pub(crate) const SCAN_DELETE: u8 = 0x39;
pub(crate) const SCAN_UP: u8 = 0x3A;
pub(crate) const SCAN_LEFT: u8 = 0x3B;
pub(crate) const SCAN_RIGHT: u8 = 0x3C;
pub(crate) const SCAN_DOWN: u8 = 0x3D;
pub(crate) const SCAN_HOME: u8 = 0x3E;
pub(crate) const SCAN_END: u8 = 0x3F;
pub(crate) const SCAN_F1: u8 = 0x62;
pub(crate) const SCAN_F2: u8 = 0x63;
pub(crate) const SCAN_F3: u8 = 0x64;
pub(crate) const SCAN_F4: u8 = 0x65;
pub(crate) const SCAN_F5: u8 = 0x66;
pub(crate) const SCAN_F6: u8 = 0x67;
pub(crate) const SCAN_F7: u8 = 0x68;

pub(crate) fn is_text_input_byte(byte: u8) -> bool {
    byte >= 0x20 || byte >= 0xA1 || is_shift_jis_lead_byte(byte)
}
