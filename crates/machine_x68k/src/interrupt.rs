//! X68000 motherboard interrupt arbitration and IOC routing.

/// Motherboard interrupt input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InterruptSource {
    /// Non-maskable interrupt at level 7.
    Nmi,
    /// MC68901 MFP at level 6.
    Mfp,
    /// Z8530 SCC at level 5.
    Scc,
    /// Expansion or MIDI interrupt at level 4.
    ExpansionLevel4,
    /// HD63450 DMAC at level 3.
    Dmac,
    /// Expansion interrupt at level 2.
    ExpansionLevel2,
}

impl InterruptSource {
    /// Returns the wired interrupt level.
    const fn level(self) -> u8 {
        match self {
            Self::Nmi => 7,
            Self::Mfp => 6,
            Self::Scc => 5,
            Self::ExpansionLevel4 => 4,
            Self::Dmac => 3,
            Self::ExpansionLevel2 => 2,
        }
    }
}

/// Interrupt source routed through the X68000 IOC at level 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IocSource {
    /// Internal SCSI controller.
    Spc,
    /// Floppy controller.
    Fdc,
    /// Floppy drive-change input.
    Fdd,
    /// SASI hard-disk controller.
    Hdc,
    /// Printer interface.
    Printer,
}

impl IocSource {
    /// Returns the IOC request bit.
    const fn bit(self) -> u8 {
        match self {
            Self::Printer => 0x01,
            Self::Fdd => 0x02,
            Self::Fdc => 0x04,
            Self::Hdc => 0x08,
            Self::Spc => 0x10,
        }
    }

    /// Returns the programmed-vector offset.
    const fn vector_offset(self) -> u8 {
        match self {
            Self::Fdc => 0,
            Self::Fdd => 1,
            Self::Hdc => 2,
            Self::Printer => 3,
            Self::Spc => 0,
        }
    }
}

save_state::runtime_state! {
/// Authoritative X68000 I/O controller interrupt state.
#[derive(Debug, Clone, Default)]
pub(crate) struct Ioc {
    pending: u8,
    mask: u8,
    vector_base: u8,
}}

impl Ioc {
    /// Latches a request edge.
    pub(crate) fn signal(&mut self, source: IocSource) {
        self.pending |= source.bit();
    }

    /// Clears a request.
    pub(crate) fn clear(&mut self, source: IocSource) {
        self.pending &= !source.bit();
    }

    /// Returns pending status bits.
    pub(crate) fn status(&self) -> u8 {
        let signals = (if self.pending & IocSource::Fdc.bit() != 0 {
            0x80
        } else {
            0
        }) | (if self.pending & IocSource::Fdd.bit() != 0 {
            0x40
        } else {
            0
        }) | (if self.pending & IocSource::Printer.bit() != 0 {
            0x20
        } else {
            0
        }) | (if self.pending & IocSource::Hdc.bit() != 0 {
            0x10
        } else {
            0
        });
        signals | self.mask
    }

    /// Sets the legacy-source mask.
    pub(crate) fn set_mask(&mut self, mask: u8) {
        self.mask = mask & 0x0F;
    }

    /// Sets the aligned vector base.
    pub(crate) fn set_vector_base(&mut self, vector: u8) {
        self.vector_base = vector & 0xFC;
    }

    /// Reports an acknowledgeable request.
    pub(crate) fn pending(&self) -> bool {
        self.selected().is_some()
    }

    /// Acknowledges the highest request.
    pub(crate) fn acknowledge(&mut self) -> Option<u8> {
        let source = self.selected()?;
        self.clear(source);
        Some(if source == IocSource::Spc {
            0x6C
        } else {
            self.vector_base | source.vector_offset()
        })
    }

    /// Selects the highest request.
    fn selected(&self) -> Option<IocSource> {
        if self.pending & IocSource::Spc.bit() != 0 {
            return Some(IocSource::Spc);
        }
        [
            IocSource::Fdc,
            IocSource::Fdd,
            IocSource::Hdc,
            IocSource::Printer,
        ]
        .into_iter()
        .find(|source| self.pending & self.mask & source.bit() != 0)
    }
}

save_state::runtime_state! {
/// Authoritative interrupt routing and pending-source state.
#[derive(Debug, Clone, Default)]
pub(crate) struct InterruptRouter {
    asserted: [bool; 8],
    vectors: [u8; 8],
    pub(crate) ioc: Ioc,
}}

impl InterruptRouter {
    /// Resets all interrupt inputs.
    pub(crate) fn reset(&mut self) {
        *self = Self::default();
    }

    /// Asserts a vectored input.
    pub(crate) fn assert(&mut self, source: InterruptSource, vector: u8) {
        let level = usize::from(source.level());
        self.asserted[level] = true;
        self.vectors[level] = vector;
    }

    /// Clears an input.
    pub(crate) fn clear(&mut self, source: InterruptSource) {
        self.asserted[usize::from(source.level())] = false;
    }

    /// Returns the highest asserted level.
    pub(crate) fn level(&self) -> u8 {
        if self.asserted[7] {
            return 7;
        }
        for level in (2..=6).rev() {
            if self.asserted[level] {
                return level as u8;
            }
        }
        u8::from(self.ioc.pending())
    }

    /// Acknowledges one interrupt level.
    pub(crate) fn acknowledge(&mut self, level: u8) -> u8 {
        if level == 1 {
            return self.ioc.acknowledge().unwrap_or(0x18);
        }
        if level == 7 && self.asserted[7] {
            self.asserted[7] = false;
            return 0x1F;
        }
        let index = usize::from(level.min(7));
        if level >= 2 && self.asserted[index] {
            self.asserted[index] = false;
            return self.vectors[index];
        }
        0x18
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn highest_level_wins_and_missing_ack_is_spurious() {
        let mut router = InterruptRouter::default();
        router.assert(InterruptSource::Dmac, 0x44);
        router.assert(InterruptSource::Mfp, 0x46);
        assert_eq!(router.level(), 6);
        assert_eq!(router.acknowledge(6), 0x46);
        assert_eq!(router.level(), 3);
        assert_eq!(router.acknowledge(6), 0x18);
    }

    #[test]
    fn ioc_prioritizes_spc_and_uses_programmed_vectors() {
        let mut ioc = Ioc::default();
        ioc.set_mask(0x0F);
        ioc.set_vector_base(0x43);
        ioc.signal(IocSource::Printer);
        ioc.signal(IocSource::Fdc);
        ioc.signal(IocSource::Spc);
        assert_eq!(ioc.acknowledge(), Some(0x6C));
        assert_eq!(ioc.acknowledge(), Some(0x40));
        assert_eq!(ioc.acknowledge(), Some(0x43));
    }
}
