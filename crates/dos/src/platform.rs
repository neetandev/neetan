//! Platform description of a NeetanDos instance.
//!
//! A `DosPlatform` is plain data. It names the DOS version that NeetanDos
//! reports, the data area of the DOS data segment with the structure format
//! of the DOS internal tables, the memory placement of the DOS kernel, the
//! optional EMS and UMB windows, the default country and the interrupt
//! vectors that belong to the guest. The machine crate builds the platform
//! and hands it to `NeetanDos::new`.
//!
//! The placement can come from the guest at run time. A foreign `IO.SYS`
//! passes the kernel segment and the memory top to the DOS kernel entry, and
//! the kernel entry builds the platform from these values.

use crate::layout::DataArea;

/// Size of one paragraph in bytes.
const PARAGRAPH_BYTES: u32 = 16;

save_state::runtime_state! {
    /// DOS version that `INT 21h AH=30h` reports.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct DosVersion {
        /// Major version number (`AL`).
        pub major: u8,
        /// Minor version number (`AH`).
        pub minor: u8,
        /// OEM number (`BH`).
        pub oem_number: u8,
    }
}

save_state::runtime_state! {
    /// Memory placement of the DOS kernel.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct DosPlacement {
        /// Segment of the DOS data area. The list of lists, the SFT and the
        /// swappable data area live in this segment.
        pub dos_data_segment: u16,
        /// Segment of the first memory control block.
        pub first_mcb_segment: u16,
        /// Top of conventional memory in paragraphs. The MCB chain ends below it.
        pub memory_top_segment: u16,
    }
}

impl DosPlacement {
    /// Returns the linear address of the DOS data segment.
    pub const fn dos_data_base(&self) -> u32 {
        (self.dos_data_segment as u32) * PARAGRAPH_BYTES
    }

    /// Returns the linear address of an offset in the DOS data segment.
    pub const fn dos_data_address(&self, offset: u16) -> u32 {
        self.dos_data_base() + offset as u32
    }

    /// Returns the number of bytes between the DOS data segment and the first MCB.
    pub const fn dos_data_size(&self) -> u32 {
        (self.first_mcb_segment as u32 - self.dos_data_segment as u32) * PARAGRAPH_BYTES
    }

    /// Returns true if the DOS data segment, the first MCB and the memory top
    /// are in ascending order.
    pub const fn is_ordered(&self) -> bool {
        self.dos_data_segment < self.first_mcb_segment
            && self.first_mcb_segment < self.memory_top_segment
    }
}

save_state::runtime_state! {
    /// EMS page frame window in the upper memory area.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct EmsWindow {
        /// Segment of physical page 0 of the page frame.
        pub page_frame_segment: u16,
    }
}

impl EmsWindow {
    /// Number of physical pages in the page frame.
    pub const PHYSICAL_PAGE_COUNT: usize = 4;
    /// Size of one EMS page in bytes.
    pub const PAGE_SIZE: u32 = 0x4000;
    /// Size of one EMS page in paragraphs.
    pub const PAGE_PARAGRAPHS: u16 = 0x0400;

    /// Returns the linear address of physical page 0.
    pub const fn base_address(&self) -> u32 {
        (self.page_frame_segment as u32) * PARAGRAPH_BYTES
    }

    /// Returns the size of the whole page frame in bytes.
    pub const fn size(&self) -> u32 {
        Self::PHYSICAL_PAGE_COUNT as u32 * Self::PAGE_SIZE
    }

    /// Returns the physical page that starts at `segment`.
    pub fn physical_page_for_segment(&self, segment: u16) -> Option<usize> {
        (0..Self::PHYSICAL_PAGE_COUNT).find(|&page| {
            self.page_frame_segment
                .wrapping_add(page as u16 * Self::PAGE_PARAGRAPHS)
                == segment
        })
    }
}

save_state::runtime_state_enum! {
    /// Use of one block of the UMB chain.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum UmbBlockKind {
        /// The block is free for allocation.
        Free = 0,
        /// The block belongs to DOS and covers memory that no program may use.
        Reserved = 1,
    }
}

save_state::runtime_state! {
    /// One memory control block of the initial UMB chain.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct UmbBlock {
        /// Segment of the MCB header.
        pub mcb_segment: u16,
        /// Size of the block behind the header in paragraphs.
        pub paragraphs: u16,
        /// Use of the block.
        pub kind: UmbBlockKind,
    }
}

save_state::runtime_state! {
    /// Upper memory window and the initial MCB chain inside it.
    ///
    /// The chain starts with a leading free block. The EMS page frame takes the
    /// start of the window when EMS is active. The leading block then starts
    /// behind the page frame.
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct UmbWindow {
        /// Leading free block when the EMS page frame is active.
        pub leading_block_with_ems: UmbBlock,
        /// Leading free block without an EMS page frame.
        pub leading_block_without_ems: UmbBlock,
        /// Blocks behind the leading block in chain order. The last block ends
        /// the chain.
        pub trailing_blocks: Vec<UmbBlock>,
    }
}

impl UmbWindow {
    /// Returns the leading block for the given EMS state.
    pub fn leading_block(&self, ems_active: bool) -> UmbBlock {
        if ems_active {
            self.leading_block_with_ems
        } else {
            self.leading_block_without_ems
        }
    }
}

save_state::runtime_state! {
    /// Set of interrupt vectors.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct VectorSet {
        /// One bit per vector, vector 0 in bit 0 of word 0.
        bits: [u64; 4],
    }
}

impl VectorSet {
    /// Number of vectors in one word of the bit set.
    const VECTORS_PER_WORD: usize = 64;

    /// Returns an empty set.
    pub const fn empty() -> Self {
        Self { bits: [0; 4] }
    }

    /// Returns a set that holds the given vectors.
    pub const fn from_vectors(vectors: &[u8]) -> Self {
        let mut set = Self::empty();
        let mut index = 0;
        while index < vectors.len() {
            set = set.with(vectors[index]);
            index += 1;
        }
        set
    }

    /// Returns a copy of the set that also holds `vector`.
    pub const fn with(mut self, vector: u8) -> Self {
        let vector = vector as usize;
        self.bits[vector / Self::VECTORS_PER_WORD] |= 1 << (vector % Self::VECTORS_PER_WORD);
        self
    }

    /// Returns true if the set holds `vector`.
    pub const fn contains(&self, vector: u8) -> bool {
        let vector = vector as usize;
        self.bits[vector / Self::VECTORS_PER_WORD] & (1 << (vector % Self::VECTORS_PER_WORD)) != 0
    }

    /// Returns true if the set holds no vector.
    pub const fn is_empty(&self) -> bool {
        self.bits[0] == 0 && self.bits[1] == 0 && self.bits[2] == 0 && self.bits[3] == 0
    }
}

/// DOS version of NEC MS-DOS 6.20 on the PC-98.
const PC98_VERSION: DosVersion = DosVersion {
    major: 6,
    minor: 20,
    oem_number: 0x00,
};

/// Kernel placement of NeetanDos on the PC-98. The boot places the first MCB
/// behind the disk buffer and the DPBs, at this segment or above.
const PC98_PLACEMENT: DosPlacement = DosPlacement {
    dos_data_segment: 0x0200,
    first_mcb_segment: 0x0370,
    memory_top_segment: 0xA000,
};

/// EMS page frame of the PC-98 at `C0000h`.
const PC98_EMS_WINDOW: EmsWindow = EmsWindow {
    page_frame_segment: 0xC000,
};

/// Country code of Japan.
const COUNTRY_CODE_JAPAN: u16 = 81;

/// DOS version of the MS-DOS 3.10 kernel under the FM Towns `IO.SYS`.
const FM_TOWNS_VERSION: DosVersion = DosVersion {
    major: 3,
    minor: 10,
    oem_number: 0xFF,
};

/// Vectors that the FM Towns `IO.SYS`, TBIOS and the drivers own. NeetanDos
/// leaves them alone.
const FM_TOWNS_GUEST_VECTORS: &[u8] = &[
    0x29, 0x33, 0x67, 0x8E, 0x90, 0x91, 0x92, 0x93, 0x94, 0x95, 0x96, 0x97, 0x98, 0x99, 0x9A, 0x9B,
    0x9C, 0x9D, 0x9E, 0x9F, 0xA0, 0xA1, 0xA2, 0xA3, 0xA4, 0xA5, 0xA6, 0xA7, 0xA8, 0xA9, 0xAA, 0xAB,
    0xAC, 0xAD, 0xAE, 0xAF, 0xDC, 0xE4, 0xFD, 0xFE,
];

save_state::runtime_state_enum! {
    /// Machine family of a NeetanDos instance. It selects the machine
    /// specific rules: the native devices behind the block units of a
    /// foreign `IO.SYS` and the CD-ROM device that NeetanDos serves itself.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum DosMachine {
        /// NEC PC-98 with the self boot of NeetanDos.
        Pc98 = 0,
        /// Fujitsu FM Towns with the `IO.SYS` of the boot medium.
        FmTowns = 1,
    }
}

/// Device name of the CD-ROM driver of the FM Towns `IO.SYS`.
const FM_TOWNS_CDROM_DEVICE_NAME: [u8; 8] = *b"TOWNS_CD";

impl DosMachine {
    /// Returns true for a machine with the PC-98 text VRAM and the cursor
    /// of the `IO.SYS` work area.
    pub const fn has_text_vram(self) -> bool {
        match self {
            Self::Pc98 => true,
            Self::FmTowns => false,
        }
    }

    /// Returns true when the machine CD-ROM drive is an MSCDEX drive without
    /// an MSCDEX installation.
    pub const fn has_implicit_mscdex(self) -> bool {
        match self {
            Self::Pc98 => true,
            Self::FmTowns => false,
        }
    }

    /// Returns the name of the guest CD-ROM device whose drive NeetanDos
    /// serves through `CdromIo`. Every other CD-ROM device of an MSCDEX
    /// installation goes through guest calls of its driver.
    pub const fn native_cdrom_device_name(self) -> Option<[u8; 8]> {
        match self {
            Self::Pc98 => None,
            Self::FmTowns => Some(FM_TOWNS_CDROM_DEVICE_NAME),
        }
    }
}

/// Machine description of a NeetanDos instance.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DosPlatform {
    /// Machine family.
    pub machine: DosMachine,
    /// DOS version that `INT 21h AH=30h` reports.
    pub version: DosVersion,
    /// Places of the DOS tables in the DOS data segment. The data area also
    /// names the structure format of the DOS internal tables.
    pub data_area: DataArea,
    /// Memory placement of the DOS kernel.
    pub placement: DosPlacement,
    /// EMS page frame window, if the machine has one.
    pub ems_window: Option<EmsWindow>,
    /// Upper memory window, if the machine has one.
    pub umb_window: Option<UmbWindow>,
    /// Country code before `CONFIG.SYS` applies `COUNTRY=`.
    pub country_code: u16,
    /// Interrupt vectors that belong to the guest. NeetanDos installs no
    /// handler on these vectors.
    pub guest_vectors: VectorSet,
}

state_struct_codec!(DosPlatform {
    machine,
    version,
    data_area,
    placement,
    ems_window,
    umb_window,
    country_code,
    guest_vectors,
});

impl DosPlatform {
    /// Returns the platform of the PC-98: MS-DOS 6.20 with the DOS 6.20
    /// format and the PC-98 6.20 data area, the DOS data segment at `0200h`,
    /// conventional memory up to `A000h`, the EMS page frame at `C000h` and
    /// the UMB chain in `C000h`-`DFFFh`.
    pub fn pc98() -> Self {
        Self {
            machine: DosMachine::Pc98,
            version: PC98_VERSION,
            data_area: DataArea::Pc98Dos620,
            placement: PC98_PLACEMENT,
            ems_window: Some(PC98_EMS_WINDOW),
            umb_window: Some(pc98_umb_window()),
            country_code: COUNTRY_CODE_JAPAN,
            guest_vectors: VectorSet::empty(),
        }
    }

    /// Returns the platform of the DOS kernel under the FM Towns `IO.SYS`:
    /// MS-DOS 3.10 with the DOS 3.10 format and the FM Towns data area, no
    /// EMS and no UMB window. The kernel entry supplies the placement from
    /// the DOSINIT registers.
    pub fn fm_towns(placement: DosPlacement) -> Self {
        Self {
            machine: DosMachine::FmTowns,
            version: FM_TOWNS_VERSION,
            data_area: DataArea::FmTowns,
            placement,
            ems_window: None,
            umb_window: None,
            country_code: COUNTRY_CODE_JAPAN,
            guest_vectors: VectorSet::from_vectors(FM_TOWNS_GUEST_VECTORS),
        }
    }

    /// Returns true if both platforms agree in every field except the
    /// placement. The placement may come from the guest at run time.
    pub fn has_same_configuration(&self, other: &Self) -> bool {
        self.machine == other.machine
            && self.version == other.version
            && self.data_area == other.data_area
            && self.ems_window == other.ems_window
            && self.umb_window == other.umb_window
            && self.country_code == other.country_code
            && self.guest_vectors == other.guest_vectors
    }

    /// Returns true if a restored instance of this platform may hold the
    /// placement `restored`. The PC-98 self boot keeps the DOS data segment
    /// and the memory top and puts the first MCB at the configured segment or
    /// above. A foreign `IO.SYS` supplies an ordered placement at DOSINIT. A
    /// kernel that waits for its DOSINIT call holds the configured placement.
    pub fn accepts_placement(&self, restored: &DosPlacement) -> bool {
        match self.machine {
            DosMachine::Pc98 => {
                restored.dos_data_segment == self.placement.dos_data_segment
                    && restored.memory_top_segment == self.placement.memory_top_segment
                    && restored.first_mcb_segment >= self.placement.first_mcb_segment
                    && restored.is_ordered()
            }
            DosMachine::FmTowns => restored.is_ordered() || *restored == self.placement,
        }
    }

    /// Returns true if NeetanDos may install a handler on `vector`.
    pub const fn installs_vector(&self, vector: u8) -> bool {
        !self.guest_vectors.contains(vector)
    }
}

/// Returns the UMB window of the PC-98.
///
/// With EMS the page frame takes `C000h`-`CFFFh` and the chain starts at
/// `D000h`. Without EMS the leading block also covers `C000h`-`CFFFh`. Two
/// reserved blocks keep `D701h`-`D7FEh` and `DF00h`-`DFFFh` away from programs.
fn pc98_umb_window() -> UmbWindow {
    UmbWindow {
        leading_block_with_ems: UmbBlock {
            mcb_segment: 0xD000,
            paragraphs: 0x0700,
            kind: UmbBlockKind::Free,
        },
        leading_block_without_ems: UmbBlock {
            mcb_segment: 0xC000,
            paragraphs: 0x1700,
            kind: UmbBlockKind::Free,
        },
        trailing_blocks: vec![
            UmbBlock {
                mcb_segment: 0xD701,
                paragraphs: 0x00FD,
                kind: UmbBlockKind::Reserved,
            },
            UmbBlock {
                mcb_segment: 0xD7FF,
                paragraphs: 0x0700,
                kind: UmbBlockKind::Free,
            },
            UmbBlock {
                mcb_segment: 0xDF00,
                paragraphs: 0x00FF,
                kind: UmbBlockKind::Reserved,
            },
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pc98_platform_keeps_the_pc98_values() {
        let platform = DosPlatform::pc98();
        assert_eq!(
            platform.version,
            DosVersion {
                major: 6,
                minor: 20,
                oem_number: 0x00,
            }
        );
        assert_eq!(platform.data_area, DataArea::Pc98Dos620);
        assert_eq!(platform.placement.dos_data_segment, 0x0200);
        assert_eq!(platform.placement.dos_data_base(), 0x2000);
        assert_eq!(platform.placement.first_mcb_segment, 0x0370);
        assert_eq!(platform.placement.dos_data_size(), 0x1700);
        assert_eq!(platform.placement.memory_top_segment, 0xA000);
        assert_eq!(
            platform.ems_window,
            Some(EmsWindow {
                page_frame_segment: 0xC000,
            })
        );
        assert_eq!(platform.country_code, 81);
        assert!(platform.guest_vectors.is_empty());
    }

    #[test]
    fn pc98_umb_window_keeps_the_pc98_chain() {
        let window = DosPlatform::pc98().umb_window.expect("PC-98 has UMBs");
        let free = UmbBlockKind::Free;
        let reserved = UmbBlockKind::Reserved;
        let block = |mcb_segment, paragraphs, kind| UmbBlock {
            mcb_segment,
            paragraphs,
            kind,
        };
        assert_eq!(window.leading_block(true), block(0xD000, 0x0700, free));
        assert_eq!(window.leading_block(false), block(0xC000, 0x1700, free));
        assert_eq!(
            window.trailing_blocks,
            vec![
                block(0xD701, 0x00FD, reserved),
                block(0xD7FF, 0x0700, free),
                block(0xDF00, 0x00FF, reserved),
            ]
        );
    }

    #[test]
    fn ems_window_maps_page_segments() {
        let window = EmsWindow {
            page_frame_segment: 0xC000,
        };
        assert_eq!(window.base_address(), 0xC0000);
        assert_eq!(window.size(), 0x10000);
        assert_eq!(window.physical_page_for_segment(0xC000), Some(0));
        assert_eq!(window.physical_page_for_segment(0xC400), Some(1));
        assert_eq!(window.physical_page_for_segment(0xC800), Some(2));
        assert_eq!(window.physical_page_for_segment(0xCC00), Some(3));
        assert_eq!(window.physical_page_for_segment(0xD000), None);
        assert_eq!(window.physical_page_for_segment(0xC200), None);
    }

    #[test]
    fn fm_towns_platform_reports_dos_310_and_takes_the_placement() {
        let placement = DosPlacement {
            dos_data_segment: 0x1076,
            first_mcb_segment: 0x1800,
            memory_top_segment: 0xC000,
        };
        let platform = DosPlatform::fm_towns(placement);
        assert_eq!(
            platform.version,
            DosVersion {
                major: 3,
                minor: 10,
                oem_number: 0xFF,
            }
        );
        assert_eq!(platform.data_area, DataArea::FmTowns);
        assert_eq!(platform.placement, placement);
        assert!(platform.ems_window.is_none());
        assert!(platform.umb_window.is_none());
        assert!(placement.is_ordered());
    }

    #[test]
    fn fm_towns_platform_leaves_the_io_sys_vectors_alone() {
        let platform = DosPlatform::fm_towns(DosPlacement {
            dos_data_segment: 0x1076,
            first_mcb_segment: 0x1800,
            memory_top_segment: 0xC000,
        });
        for vector in [0x29, 0x33, 0x67, 0x8E, 0x90, 0xAF, 0xDC, 0xE4, 0xFD, 0xFE] {
            assert!(!platform.installs_vector(vector), "{vector:#04X}");
        }
        for vector in [0x20, 0x21, 0x24, 0x28, 0x2F, 0x5C, 0x8F, 0xB0] {
            assert!(platform.installs_vector(vector), "{vector:#04X}");
        }
    }

    #[test]
    fn pc98_placement_keeps_the_data_segment_and_the_memory_top() {
        let platform = DosPlatform::pc98();
        let placement = platform.placement;
        assert!(platform.accepts_placement(&placement));
        let booted = DosPlacement {
            first_mcb_segment: 0x0400,
            ..placement
        };
        assert!(platform.accepts_placement(&booted));
        for restored in [
            DosPlacement {
                dos_data_segment: 0x0100,
                ..booted
            },
            DosPlacement {
                memory_top_segment: 0x9000,
                ..booted
            },
            DosPlacement {
                first_mcb_segment: 0x0300,
                ..placement
            },
            DosPlacement {
                first_mcb_segment: 0xA000,
                ..placement
            },
        ] {
            assert!(!platform.accepts_placement(&restored), "{restored:?}");
        }
    }

    #[test]
    fn fm_towns_placement_comes_from_dosinit() {
        let configured = DosPlacement {
            dos_data_segment: 0,
            first_mcb_segment: 0,
            memory_top_segment: 0,
        };
        let platform = DosPlatform::fm_towns(configured);
        assert!(platform.accepts_placement(&configured));
        assert!(platform.accepts_placement(&DosPlacement {
            dos_data_segment: 0x1076,
            first_mcb_segment: 0x1800,
            memory_top_segment: 0xC000,
        }));
        assert!(!platform.accepts_placement(&DosPlacement {
            dos_data_segment: 0x1800,
            first_mcb_segment: 0x1076,
            memory_top_segment: 0xC000,
        }));
    }

    #[test]
    fn vector_set_holds_every_vector_word() {
        let set = VectorSet::from_vectors(&[0x00, 0x3F, 0x40, 0x7F, 0x80, 0xBF, 0xC0, 0xFF]);
        for vector in [0x00, 0x3F, 0x40, 0x7F, 0x80, 0xBF, 0xC0, 0xFF] {
            assert!(set.contains(vector));
        }
        for vector in [0x01, 0x41, 0x81, 0xC1, 0xFE] {
            assert!(!set.contains(vector));
        }
        assert!(!set.is_empty());
        assert!(VectorSet::empty().is_empty());
    }

    #[test]
    fn only_the_pc98_data_area_has_self_boot_regions() {
        let fm_towns = DosPlatform::fm_towns(DosPlacement {
            dos_data_segment: 0x1076,
            first_mcb_segment: 0x1800,
            memory_top_segment: 0xC000,
        });
        for platform in [DosPlatform::pc98(), fm_towns] {
            assert_eq!(
                platform.machine == DosMachine::Pc98,
                platform.data_area.layout().self_boot.is_some(),
                "{:?}",
                platform.machine
            );
        }
    }

    #[test]
    fn same_configuration_ignores_the_placement() {
        let first = DosPlatform::fm_towns(DosPlacement {
            dos_data_segment: 0x1076,
            first_mcb_segment: 0x1800,
            memory_top_segment: 0xC000,
        });
        let second = DosPlatform::fm_towns(DosPlacement {
            dos_data_segment: 0x0FC0,
            first_mcb_segment: 0x1700,
            memory_top_segment: 0xC000,
        });
        assert!(first.has_same_configuration(&second));
        assert!(!first.has_same_configuration(&DosPlatform::pc98()));
    }
}
