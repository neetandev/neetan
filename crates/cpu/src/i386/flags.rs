const PARITY_TABLE: [bool; 256] = {
    let mut table = [false; 256];
    let mut i = 0u16;
    while i < 256 {
        let mut bits = 0u32;
        let mut v = i;
        while v > 0 {
            bits += (v & 1) as u32;
            v >>= 1;
        }
        table[i as usize] = (bits & 1) == 0;
        i += 1;
    }
    table
};

save_state::runtime_state! {
    /// i386 CPU flags register state.
    #[derive(Debug, Clone)]
    pub struct I386Flags {
        /// Lazily computed sign flag value (negative means SF=1).
        pub sign_val: i32,
        /// Lazily computed zero flag value (zero means ZF=1).
        pub zero_val: u32,
        /// Lazily computed carry flag value (non-zero means CF=1).
        pub carry_val: u32,
        /// Lazily computed overflow flag value (non-zero means OF=1).
        pub overflow_val: u32,
        /// Lazily computed auxiliary carry flag value (non-zero means AF=1).
        pub aux_val: u32,
        /// Lazily computed parity flag value (low byte used for lookup).
        pub parity_val: u32,
        /// Trap flag.
        pub tf: bool,
        /// Interrupt enable flag.
        pub if_flag: bool,
        /// Direction flag.
        pub df: bool,
        /// I/O Privilege Level (bits 12-13 of FLAGS).
        pub iopl: u8,
        /// Nested Task flag (bit 14 of FLAGS).
        pub nt: bool,
    }
}

impl I386Flags {
    /// Creates flags with default power-on state.
    pub const fn new() -> Self {
        Self {
            sign_val: 0,
            zero_val: 1,
            carry_val: 0,
            overflow_val: 0,
            aux_val: 0,
            parity_val: 0,
            tf: false,
            if_flag: false,
            df: false,
            iopl: 0,
            nt: false,
        }
    }

    /// Returns the carry flag.
    #[inline(always)]
    pub fn cf(&self) -> bool {
        self.carry_val != 0
    }

    /// Returns the sign flag.
    #[inline(always)]
    pub fn sf(&self) -> bool {
        self.sign_val < 0
    }

    /// Returns the zero flag.
    #[inline(always)]
    pub fn zf(&self) -> bool {
        self.zero_val == 0
    }

    /// Returns the parity flag.
    #[inline(always)]
    pub fn pf(&self) -> bool {
        PARITY_TABLE[(self.parity_val & 0xFF) as usize]
    }

    /// Returns the auxiliary carry flag.
    #[inline(always)]
    pub fn af(&self) -> bool {
        self.aux_val != 0
    }

    /// Returns the overflow flag.
    #[inline(always)]
    pub fn of(&self) -> bool {
        self.overflow_val != 0
    }

    /// Returns the carry flag as a 0/1 integer.
    #[inline(always)]
    pub fn cf_val(&self) -> u32 {
        u32::from(self.carry_val != 0)
    }

    /// Packs all flags into a 16-bit FLAGS register value.
    pub fn compress(&self) -> u16 {
        (self.cf() as u16)
            | 0x0002
            | ((self.pf() as u16) << 2)
            | ((self.af() as u16) << 4)
            | ((self.zf() as u16) << 6)
            | ((self.sf() as u16) << 7)
            | ((self.tf as u16) << 8)
            | ((self.if_flag as u16) << 9)
            | ((self.df as u16) << 10)
            | ((self.of() as u16) << 11)
            | (((self.iopl & 3) as u16) << 12)
            | ((self.nt as u16) << 14)
    }

    /// Unpacks a 16-bit FLAGS register value into individual flags.
    pub fn expand(&mut self, value: u16) {
        self.carry_val = (value & 0x0001) as u32;
        self.parity_val = if value & 0x0004 != 0 { 0 } else { 1 };
        self.aux_val = (value & 0x0010) as u32;
        self.zero_val = if value & 0x0040 != 0 { 0 } else { 1 };
        self.sign_val = if value & 0x0080 != 0 { -1 } else { 0 };
        self.tf = value & 0x0100 != 0;
        self.if_flag = value & 0x0200 != 0;
        self.df = value & 0x0400 != 0;
        self.overflow_val = (value & 0x0800) as u32;
        self.iopl = ((value >> 12) & 3) as u8;
        self.nt = value & 0x4000 != 0;
    }

    /// Loads flags with privilege-level masking.
    pub fn load_flags(&mut self, value: u16, cpl: u16, is_protected: bool) {
        if is_protected {
            let mut masked = value;
            if cpl > 0 {
                masked = (masked & !0x3000) | ((self.iopl as u16 & 3) << 12);
                if cpl > self.iopl as u16 {
                    masked = (masked & !0x0200) | (self.compress() & 0x0200);
                }
            }
            self.expand(masked);
        } else {
            self.expand(value & !0x8000);
        }
    }

    /// Sets carry flag from a byte-sized result (bit 8).
    #[inline(always)]
    pub fn set_cf_byte(&mut self, result: u32) {
        self.carry_val = result & 0x100;
    }

    /// Sets carry flag from a word-sized result (bit 16).
    #[inline(always)]
    pub fn set_cf_word(&mut self, result: u32) {
        self.carry_val = result & 0x10000;
    }

    /// Sets carry flag from a dword-sized result (bit 32).
    #[inline(always)]
    pub fn set_cf_dword(&mut self, result: u64) {
        self.carry_val = ((result >> 32) & 1) as u32;
    }

    /// Sets auxiliary carry flag from XOR of result, source, and destination.
    #[inline(always)]
    pub fn set_af(&mut self, result: u32, src: u32, dst: u32) {
        self.aux_val = (result ^ (src ^ dst)) & 0x10;
    }

    /// Sets overflow flag for byte addition.
    #[inline(always)]
    pub fn set_of_add_byte(&mut self, result: u32, src: u32, dst: u32) {
        self.overflow_val = (result ^ src) & (result ^ dst) & 0x80;
    }

    /// Sets overflow flag for word addition.
    #[inline(always)]
    pub fn set_of_add_word(&mut self, result: u32, src: u32, dst: u32) {
        self.overflow_val = (result ^ src) & (result ^ dst) & 0x8000;
    }

    /// Sets overflow flag for dword addition.
    #[inline(always)]
    pub fn set_of_add_dword(&mut self, result: u32, src: u32, dst: u32) {
        self.overflow_val = (result ^ src) & (result ^ dst) & 0x8000_0000;
    }

    /// Sets overflow flag for byte subtraction.
    #[inline(always)]
    pub fn set_of_sub_byte(&mut self, result: u32, src: u32, dst: u32) {
        self.overflow_val = (dst ^ src) & (dst ^ result) & 0x80;
    }

    /// Sets overflow flag for word subtraction.
    #[inline(always)]
    pub fn set_of_sub_word(&mut self, result: u32, src: u32, dst: u32) {
        self.overflow_val = (dst ^ src) & (dst ^ result) & 0x8000;
    }

    /// Sets overflow flag for dword subtraction.
    #[inline(always)]
    pub fn set_of_sub_dword(&mut self, result: u32, src: u32, dst: u32) {
        self.overflow_val = (dst ^ src) & (dst ^ result) & 0x8000_0000;
    }

    /// Sets sign, zero, and parity flags from a byte result.
    #[inline(always)]
    pub fn set_szpf_byte(&mut self, result: u32) {
        self.sign_val = result as u8 as i8 as i32;
        self.zero_val = result & 0xFF;
        self.parity_val = result;
    }

    /// Sets sign, zero, and parity flags from a word result.
    #[inline(always)]
    pub fn set_szpf_word(&mut self, result: u32) {
        self.sign_val = result as u16 as i16 as i32;
        self.zero_val = result & 0xFFFF;
        self.parity_val = result;
    }

    /// Sets sign, zero, and parity flags from a dword result.
    #[inline(always)]
    pub fn set_szpf_dword(&mut self, result: u32) {
        self.sign_val = result as i32;
        self.zero_val = result;
        self.parity_val = result;
    }
}

impl Default for I386Flags {
    fn default() -> Self {
        Self::new()
    }
}

impl PartialEq for I386Flags {
    fn eq(&self, other: &Self) -> bool {
        self.compress() == other.compress()
    }
}

impl Eq for I386Flags {}
