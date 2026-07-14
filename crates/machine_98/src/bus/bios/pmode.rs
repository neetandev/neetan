//! INT 1Fh protected-mode extension service (286/386 memory copy, PCI BIOS check).

use common::{Cpu, MachineModel};

use super::super::Pc9801Bus;
use crate::Tracing;

impl<T: Tracing> Pc9801Bus<T> {
    pub(super) fn hle_int1fh(&mut self, cpu: &mut impl Cpu) {
        let ah = cpu.ah();

        // Bit 7 must be set for valid extended functions.
        // On F and VM (expansion ROM handler), CF is cleared even for AH < 0x80.
        if ah & 0x80 == 0 {
            if matches!(
                self.machine_model,
                MachineModel::PC9801F | MachineModel::PC9801VM
            ) {
                self.set_iret_cf(cpu, false);
            }
            return;
        }

        match ah {
            0xCC => {
                // PCI BIOS check: no PCI on VM-era machines. Set CF = 1.
                self.set_iret_cf(cpu, true);
            }
            0x90 => {
                // Protected-mode memory copy (286/386 only).
                if matches!(
                    self.machine_model,
                    MachineModel::PC9801F | MachineModel::PC9801VM
                ) {
                    self.set_iret_cf(cpu, false);
                    return;
                }

                let desc_seg = cpu.es();
                let desc_off = cpu.bx();
                let desc_base = (u32::from(desc_seg) << 4).wrapping_add(u32::from(desc_off));

                // Read source descriptor (at desc_base + 0x10).
                let src_base_addr = self.parse_gdt_descriptor_base(desc_base + 0x10);

                // Read destination descriptor (at desc_base + 0x18).
                let dst_base_addr = self.parse_gdt_descriptor_base(desc_base + 0x18);

                let mut src_off = u32::from(cpu.si());
                let mut dst_off = u32::from(cpu.di());

                // CX=0 means 65536 bytes.
                let remaining = if cpu.cx() == 0 {
                    0x10000u32
                } else {
                    cpu.cx() as u32
                };

                // The real BIOS enters protected mode (which ignores A20)
                // and disables A20 when returning to real mode.
                self.a20_enabled = true;
                for _ in 0..remaining {
                    let byte = self.read_byte_direct(src_base_addr.wrapping_add(src_off));
                    self.memory
                        .write_byte(dst_base_addr.wrapping_add(dst_off), byte);
                    src_off = (src_off + 1) & 0xFFFF;
                    dst_off = (dst_off + 1) & 0xFFFF;
                }
                self.a20_enabled = false;

                self.set_iret_cf(cpu, false);
            }
            _ => {
                // AH with bit 4 clear (0x80..0x8F excl. handled above): clear CF.
                // AH with bit 4 set (0x91..0x9F, 0xB0..0xBF, etc.): leave CF unchanged.
                if ah & 0x10 == 0 {
                    self.set_iret_cf(cpu, false);
                }
            }
        }
    }

    fn parse_gdt_descriptor_base(&self, addr: u32) -> u32 {
        let base_lo = self.read_byte_direct(addr + 2) as u32;
        let base_mid = self.read_byte_direct(addr + 3) as u32;
        let base_hi = self.read_byte_direct(addr + 4) as u32;
        base_lo | (base_mid << 8) | (base_hi << 16)
    }
}
