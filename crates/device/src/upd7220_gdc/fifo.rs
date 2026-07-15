use super::{STATUS_DATA_READY, STATUS_FIFO_EMPTY, STATUS_FIFO_FULL};

save_state::runtime_state! {
/// Authoritative read FIFO state for the GDC.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FifoState {
    /// FIFO buffer.
    pub buffer: [u8; 16],
    /// Number of valid bytes in the FIFO.
    pub count: u8,
    /// Current read position.
    pub read_pos: u8,
}}

impl Default for FifoState {
    fn default() -> Self {
        Self::new()
    }
}

impl FifoState {
    /// Creates a new empty FIFO.
    pub fn new() -> Self {
        Self {
            buffer: [0; 16],
            count: 0,
            read_pos: 0,
        }
    }

    /// Clears the FIFO.
    pub fn clear(&mut self) {
        self.count = 0;
        self.read_pos = 0;
    }

    /// Enqueues a byte into the FIFO.
    pub fn queue_byte(&mut self, value: u8) {
        if self.count < 16 {
            let write_pos = (self.read_pos + self.count) & 0x0F;
            self.buffer[write_pos as usize] = value;
            self.count += 1;
        }
    }

    /// Dequeues a byte from the FIFO. Returns 0xFF if empty.
    pub fn dequeue_byte(&mut self) -> u8 {
        if self.count == 0 {
            return 0xFF;
        }
        let value = self.buffer[self.read_pos as usize];
        self.read_pos = (self.read_pos + 1) & 0x0F;
        self.count -= 1;
        value
    }

    /// Returns true if the FIFO is empty.
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Returns true if the FIFO has data available.
    pub fn data_available(&self) -> bool {
        self.count > 0
    }

    /// Returns the FIFO status bits for the status register.
    pub fn status_bits(&self) -> u8 {
        let mut bits = 0;
        if self.count == 0 {
            bits |= STATUS_FIFO_EMPTY;
        }
        if self.count > 0 {
            bits |= STATUS_DATA_READY;
        }
        if self.count >= 16 {
            bits |= STATUS_FIFO_FULL;
        }
        bits
    }
}
