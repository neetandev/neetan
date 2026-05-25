use crate::{CpuAccess, MemoryAccess};

pub(crate) struct MockCpu {
    pub(crate) eax: u32,
    pub(crate) ebx: u32,
    pub(crate) ecx: u32,
    pub(crate) edx: u32,
    pub(crate) si: u16,
    pub(crate) di: u16,
    pub(crate) ds: u16,
    pub(crate) es: u16,
    pub(crate) ss: u16,
    pub(crate) sp: u16,
    pub(crate) cs: u16,
    pub(crate) carry: bool,
}

impl Default for MockCpu {
    fn default() -> Self {
        Self {
            eax: 0,
            ebx: 0,
            ecx: 0,
            edx: 0,
            si: 0,
            di: 0,
            ds: 0,
            es: 0,
            ss: 0x2000,
            sp: 0x0100,
            cs: 0,
            carry: false,
        }
    }
}

impl MockCpu {
    pub(crate) fn iret_flags_addr(&self) -> u32 {
        ((self.ss as u32) << 4) + self.sp as u32 + 4
    }
}

impl CpuAccess for MockCpu {
    fn ax(&self) -> u16 {
        self.eax as u16
    }

    fn set_ax(&mut self, value: u16) {
        self.eax = (self.eax & 0xFFFF_0000) | value as u32;
    }

    fn bx(&self) -> u16 {
        self.ebx as u16
    }

    fn set_bx(&mut self, value: u16) {
        self.ebx = (self.ebx & 0xFFFF_0000) | value as u32;
    }

    fn cx(&self) -> u16 {
        self.ecx as u16
    }

    fn set_cx(&mut self, value: u16) {
        self.ecx = (self.ecx & 0xFFFF_0000) | value as u32;
    }

    fn dx(&self) -> u16 {
        self.edx as u16
    }

    fn set_dx(&mut self, value: u16) {
        self.edx = (self.edx & 0xFFFF_0000) | value as u32;
    }

    fn si(&self) -> u16 {
        self.si
    }

    fn set_si(&mut self, value: u16) {
        self.si = value;
    }

    fn di(&self) -> u16 {
        self.di
    }

    fn set_di(&mut self, value: u16) {
        self.di = value;
    }

    fn ds(&self) -> u16 {
        self.ds
    }

    fn set_ds(&mut self, value: u16) {
        self.ds = value;
    }

    fn es(&self) -> u16 {
        self.es
    }

    fn set_es(&mut self, value: u16) {
        self.es = value;
    }

    fn ss(&self) -> u16 {
        self.ss
    }

    fn set_ss(&mut self, value: u16) {
        self.ss = value;
    }

    fn sp(&self) -> u16 {
        self.sp
    }

    fn set_sp(&mut self, value: u16) {
        self.sp = value;
    }

    fn cs(&self) -> u16 {
        self.cs
    }

    fn set_carry(&mut self, carry: bool) {
        self.carry = carry;
    }

    fn eax(&self) -> u32 {
        self.eax
    }

    fn set_eax(&mut self, value: u32) {
        self.eax = value;
    }

    fn ebx(&self) -> u32 {
        self.ebx
    }

    fn set_ebx(&mut self, value: u32) {
        self.ebx = value;
    }

    fn ecx(&self) -> u32 {
        self.ecx
    }

    fn set_ecx(&mut self, value: u32) {
        self.ecx = value;
    }

    fn edx(&self) -> u32 {
        self.edx
    }

    fn set_edx(&mut self, value: u32) {
        self.edx = value;
    }
}

pub(crate) struct MockMemory {
    data: Vec<u8>,
    extended_memory_size: u32,
    ems_page_frame_slot_mappings: [Option<u32>; 4],
}

impl MockMemory {
    pub(crate) fn with_extended_memory(size: usize, extended_memory_size: u32) -> Self {
        Self {
            data: vec![0; size],
            extended_memory_size,
            ems_page_frame_slot_mappings: [None; 4],
        }
    }
}

impl MemoryAccess for MockMemory {
    fn read_byte(&self, address: u32) -> u8 {
        if (0xC0000..=0xCFFFF).contains(&address) {
            let slot = ((address - 0xC0000) / 0x4000) as usize;
            let slot_offset = (address - 0xC0000) % 0x4000;
            if let Some(base) = self.ems_page_frame_slot_mappings[slot] {
                return self.data[(base + slot_offset) as usize];
            }
        }
        self.data[address as usize]
    }

    fn write_byte(&mut self, address: u32, value: u8) {
        if (0xC0000..=0xCFFFF).contains(&address) {
            let slot = ((address - 0xC0000) / 0x4000) as usize;
            let slot_offset = (address - 0xC0000) % 0x4000;
            if let Some(base) = self.ems_page_frame_slot_mappings[slot] {
                self.data[(base + slot_offset) as usize] = value;
                return;
            }
        }
        self.data[address as usize] = value;
    }

    fn read_word(&self, address: u32) -> u16 {
        let low = self.read_byte(address) as u16;
        let high = self.read_byte(address + 1) as u16;
        low | (high << 8)
    }

    fn write_word(&mut self, address: u32, value: u16) {
        self.write_byte(address, value as u8);
        self.write_byte(address + 1, (value >> 8) as u8);
    }

    fn read_block(&self, address: u32, buffer: &mut [u8]) {
        for (index, byte) in buffer.iter_mut().enumerate() {
            *byte = self.read_byte(address + index as u32);
        }
    }

    fn write_block(&mut self, address: u32, data: &[u8]) {
        for (index, byte) in data.iter().enumerate() {
            self.write_byte(address + index as u32, *byte);
        }
    }

    fn extended_memory_size(&self) -> u32 {
        self.extended_memory_size
    }

    fn map_ems_page_frame_slot(&mut self, physical_page: u8, backing_linear_addr: Option<u32>) {
        let physical_page = usize::from(physical_page);
        if physical_page < self.ems_page_frame_slot_mappings.len() {
            self.ems_page_frame_slot_mappings[physical_page] = backing_linear_addr;
        }
    }
}
