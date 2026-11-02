//! Kernel entry of NeetanDos under a foreign `IO.SYS` (MS-DOS 3.x DOSINIT).
//!
//! # Protocol
//!
//! SYSINIT of `IO.SYS` copies the kernel image to the segment behind the
//! resident `IO.SYS` and far-calls `kernel:0000` with:
//!
//! * `CS` = `ES` = the kernel segment. It becomes the DOS data segment.
//! * `DS:SI` = the first device header of the `IO.SYS` device chain.
//! * `DX` = the top of conventional memory in paragraphs.
//! * a far return address on the stack of SYSINIT, interrupts enabled.
//!
//! The kernel image enters NeetanDos with `PUSHF`, `PUSH CS`, `CALL` to an
//! interrupt trap stub with the trap code `KernelInit` and returns to SYSINIT
//! with `RETF`. The trap code is the entry of this module.
//!
//! # Work
//!
//! 1. The NUL header of the list of lists takes `DS:SI` as its next pointer.
//!    The chain stays as `IO.SYS` built it.
//! 2. Every device of the chain gets INIT (request length `1Ah`, all other
//!    fields zero) in chain order through guest calls. A block device writes
//!    its unit count to header byte `0Ah` and supplies one BPB per unit.
//! 3. The drive letters count up over the block units in chain order. The
//!    machine rule names the native device behind each unit of the first
//!    block device (`DriveMap::from_chain_block_devices`). On the FM Towns
//!    this is the `IO.SYS` block driver: floppy BPB units are floppy drives
//!    `20h`+, the OS ROM BPB unit is the OS ROM drive `42h`, every other
//!    unit `n` is the SCSI partition of slot `n` of the CMOS drive
//!    assignment. NeetanDos serves these through `DiskIo`. Every other unit
//!    goes through guest calls of its driver. The DPB of every unit names its
//!    chain driver. A DPB that SYSINIT links later for a `CONFIG.SYS` driver
//!    becomes a guest unit at the first DOS call after SYSINIT links the
//!    driver.
//! 4. The first disk buffer (largest sector) and the DPBs follow at the
//!    system data offset of the layout. The first device is CON: SFT entry 0
//!    (three references) and `LoL+0Ch`. The CLOCK device (attribute bit 3)
//!    serves the date and time and fills `LoL+08h`. CON serves the console.
//! 5. The vectors `00h`, `20h`-`28h` and `2Ah`-`3Fh` point into the private
//!    area of the layout. `INT 29h` stays with `IO.SYS`.
//! 6. One free `Z` MCB covers the paragraphs from the end of the DPBs to
//!    `DX`. A PSP at the MCB + 1 holds handles 0-2 on SFT entry 0 and is the
//!    current PSP.
//!
//! # Return
//!
//! `ES:DI` = the list of lists, `DS` = `DX` = the PSP of step 6, the stack
//! of the caller and its flags restored. SYSINIT reads `LoL+20h` (block
//! units), copies the PSP and `IO.SYS` walks the MCBs from `DS - 1`. The
//! other registers and flags hold the values of the MS-DOS 3.10 kernel:
//! `AX=0003h`, `BX=F01Dh`, `CX=0004h`, `SI=0094h`, `BP` = the kernel
//! segment, CF set and ZF clear. `IO.SYS` closes `BP` as a file handle after
//! a failed open of `KEYSET.TBL`. That close fails with error `06h`.
//!
//! # SYSINIT work
//!
//! SYSINIT parses `CONFIG.SYS` and starts the shell through `INT 21h`.
//! SYSINIT and `IO.SYS` allocate the CDS array, the `FILES=` SFT block and
//! the `BUFFERS=` buffers. NeetanDos reads them through the list of lists.
//! The `IO.SYS` work area and the drive discovery from the BIOS data area
//! belong to the PC-98 self boot.
//!
//! # DOS 3.10 data segment
//!
//! `0000h`-`0025h` header of the kernel image, `0026h` list of lists with the
//! NUL header at `0048h`, `0098h` SFT, `02CEh`-`0A05h` SDA with the guest call
//! area from `0586h`, `0A10h`-`0B7Fh` private stubs and tables, `11D2h` the
//! `INT 21h` entry pattern that CTL.SYS searches (kernel image), `425Eh` a word
//! that MENU_DRV of `IO.SYS` 1989-12-16 changes (kept free), `4300h`-`4AFFh`
//! the CD transfer buffer, `6AA0h` the first disk buffer and the DPBs.

use common::warn;

use crate::{
    CdromIo, CpuAccess, DiskIo, DosPlacement, END_OF_CHAIN, MemoryAccess, NeetanDos, device_chain,
    device_request::{
        COMMAND_INIT, REQUEST_OFF_BPB_POINTER, REQUEST_OFF_MEDIA, request_status, status_is_error,
        write_request_header,
    },
    drive_map::{self, ChainBlockDevice, DriveMap, GuestBlockUnit},
    filesystem::fat_bpb::{Bpb, PARAMETER_BLOCK_LENGTH},
    guest_call::{FarPointer, GuestCall, GuestCallArea, GuestStep},
    layout::DpbGeometry,
    memory::MCB_TYPE_Z,
    process,
    system_data::SystemDataArea,
    tables::{self, write_far_ptr},
    trap,
};

/// Length of the INIT request that the kernel sends.
const INIT_REQUEST_LENGTH: u8 = 0x1A;

/// Offset of the unit count byte in a block device header.
const DEVHDR_OFF_UNIT_COUNT: u32 = 0x0A;

/// Length of the DOS 3 BPB that a driver returns.
const DOS3_BPB_LENGTH: usize = 13;

/// Number of SFT entries of the first SFT block.
const FIRST_SFT_BLOCK_ENTRIES: u16 = 5;

/// `AX` that the MS-DOS 3.10 kernel leaves at the DOSINIT return.
const DOSINIT_RETURN_AX: u16 = 0x0003;

/// `BX` that the MS-DOS 3.10 kernel leaves at the DOSINIT return.
const DOSINIT_RETURN_BX: u16 = 0xF01D;

/// `CX` that the MS-DOS 3.10 kernel leaves at the DOSINIT return.
const DOSINIT_RETURN_CX: u16 = 0x0004;

/// `SI` that the MS-DOS 3.10 kernel leaves at the DOSINIT return.
const DOSINIT_RETURN_SI: u16 = 0x0094;

/// Reference count of SFT entry 0: handles 0, 1 and 2 of the first PSP.
const CON_REFERENCE_COUNT: u16 = 3;

/// Open mode of SFT entry 0: read and write.
const CON_OPEN_MODE: u16 = 0x0002;

/// Device information word of SFT entry 0: character device, not at end of
/// file, standard output and standard input.
const CON_DEVICE_INFO: u16 = 0x00C3;

/// Attribute bits of the standard input and output device.
const STANDARD_DEVICE_BITS: u16 = tables::DEVATTR_STDIN | tables::DEVATTR_STDOUT;

/// Handles of the first PSP that name SFT entry 0.
const STANDARD_HANDLES: u32 = 3;

/// Handle table entry of a closed handle.
const CLOSED_HANDLE: u8 = 0xFF;

/// Number of handles of a PSP.
const PSP_HANDLE_COUNT: u32 = 20;

/// Offset of the terminate address that `INT 22h` names in the first PSP.
const TERMINATE_ADDRESS_OFFSET: u16 = 0x0100;

/// Interrupt vectors whose kernel stub traps into NeetanDos: divide error,
/// terminate, DOS function, absolute disk read and write, keep process and
/// multiplex.
pub const KERNEL_INTERRUPT_VECTORS: [u8; 7] = [0x00, 0x20, 0x21, 0x25, 0x26, 0x27, 0x2F];

/// Interrupt vectors with an `IRET` stub of the kernel.
const KERNEL_IRET_VECTORS: [u8; 21] = [
    0x23, 0x28, 0x2A, 0x2B, 0x2C, 0x2D, 0x2E, 0x32, 0x33, 0x34, 0x35, 0x36, 0x37, 0x38, 0x39, 0x3A,
    0x3B, 0x3C, 0x3D, 0x3E, 0x3F,
];

/// Critical error vector.
const CRITICAL_ERROR_VECTOR: u8 = 0x24;

/// Terminate address vector.
const TERMINATE_ADDRESS_VECTOR: u8 = 0x22;

/// Address of the CP/M call entry: a `JMP FAR` over vector `30h` and the
/// first byte of vector `31h`.
const CPM_ENTRY_ADDRESS: u32 = 0x00C0;

/// Opcode of `JMP FAR ptr16:16`.
const JMP_FAR: u8 = 0xEA;

/// Offset of the NUL device routine in the kernel stubs: sets the done bit
/// of the request and returns (`OR BYTE ES:[BX+4], 1`, `RETF`).
const NUL_ROUTINE_OFFSET: u16 = 0x00;

/// Code of the NUL device routine.
const NUL_ROUTINE: [u8; 6] = [0x26, 0x80, 0x4F, 0x04, 0x01, 0xCB];

/// Offset of the `IRET` stub in the kernel stubs.
const IRET_STUB_OFFSET: u16 = 0x08;

/// Offset of the CP/M call handler in the kernel stubs.
const CPM_HANDLER_OFFSET: u16 = 0x10;

/// CP/M call handler. The caller reached it through `CALL 0005h` of its PSP
/// and the `CALL FAR` there. The handler drops the far return into the PSP,
/// turns the near return into a far return to the caller, runs the function
/// of `CL` through `INT 21h` and returns.
const CPM_HANDLER: [u8; 18] = [
    0x83, 0xC4, 0x02, // ADD SP, 2
    0x55, // PUSH BP
    0x8B, 0xEC, // MOV BP, SP
    0x8B, 0x46, 0x02, // MOV AX, [BP+2]
    0x87, 0x46, 0x04, // XCHG AX, [BP+4]
    0x89, 0x46, 0x02, // MOV [BP+2], AX
    0x5D, // POP BP
    0x8A, 0xE1, // MOV AH, CL
];

/// Tail of the CP/M call handler: `INT 21h`, `RETF`.
const CPM_HANDLER_TAIL: [u8; 3] = [0xCD, 0x21, 0xCB];

/// Offset of the first interrupt trap stub in the kernel stubs.
const TRAP_STUBS_OFFSET: u16 = 0x30;

/// Size of the kernel stubs.
pub(crate) const KERNEL_STUBS_SIZE: u16 =
    TRAP_STUBS_OFFSET + KERNEL_INTERRUPT_VECTORS.len() as u16 * trap::INTERRUPT_TRAP_STUB_LENGTH;

save_state::runtime_state! {
    /// A device of the chain with its INIT answer.
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub(crate) struct InitializedDevice {
        /// Device header.
        pub(crate) header: FarPointer,
        /// Attribute word of the header.
        pub(crate) attribute: u16,
        /// BPB of every unit of a block device. Empty for a character device.
        pub(crate) unit_bpbs: Vec<Bpb>,
    }
}

save_state::runtime_state! {
    /// Progress of the kernel entry across the INIT calls of the chain.
    #[derive(Clone, Debug, Default, PartialEq, Eq)]
    pub(crate) struct KernelInit {
        /// Devices whose INIT returned, in chain order.
        pub(crate) devices: Vec<InitializedDevice>,
        /// True after the INIT of the last device of the chain.
        pub(crate) devices_initialized: bool,
    }
}

impl InitializedDevice {
    /// Returns true for a character device.
    fn is_character(&self) -> bool {
        self.attribute & tables::DEVATTR_CHAR != 0
    }
}

/// Reads the DOS 3 BPB at `address`.
fn read_driver_bpb(memory: &dyn MemoryAccess, address: u32) -> Option<Bpb> {
    let mut block = [0u8; PARAMETER_BLOCK_LENGTH];
    memory.read_block(address, &mut block[..DOS3_BPB_LENGTH]);
    Bpb::from_parameter_block(&block)
}

impl NeetanDos {
    /// Trap `KernelInit`: the DOSINIT entry. The first run writes the fixed
    /// DOS data and starts the INIT calls of the chain. The run after the
    /// last INIT builds the drives, the memory and the return registers.
    pub(crate) fn init_as_kernel(
        &mut self,
        cpu: &mut dyn CpuAccess,
        memory: &mut dyn MemoryAccess,
        device: &mut (impl DiskIo + CdromIo),
    ) {
        match &self.kernel_init {
            Some(progress) if progress.devices_initialized => {
                self.finish_kernel_init(cpu, memory, device);
            }
            Some(_) => {
                warn!("NeetanDos kernel entry runs again before the chain is initialized");
                self.start_kernel_init(cpu, memory, device);
            }
            None => self.start_kernel_init(cpu, memory, device),
        }
    }

    /// Takes the placement from the entry registers, writes the fixed DOS
    /// data and queues the INIT of the first device.
    fn start_kernel_init(
        &mut self,
        cpu: &mut dyn CpuAccess,
        memory: &mut dyn MemoryAccess,
        device: &mut (impl DiskIo + CdromIo),
    ) {
        let dos_data_segment = cpu.cs();
        let chain = FarPointer::new(cpu.ds(), cpu.si());
        let provisional_area = SystemDataArea::new(self.state.structures(), 0, 0);
        self.state.platform.placement = DosPlacement {
            dos_data_segment,
            first_mcb_segment: provisional_area.first_mcb_segment(dos_data_segment),
            memory_top_segment: cpu.dx(),
        };
        self.write_kernel_data(memory, chain);
        self.guest_call_area().write_stub(memory);
        self.kernel_init = Some(KernelInit::default());

        if (chain.segment, chain.offset) == END_OF_CHAIN {
            self.kernel_init = Some(KernelInit {
                devices: Vec::new(),
                devices_initialized: true,
            });
            self.finish_kernel_init(cpu, memory, device);
            return;
        }
        self.state
            .guest_requests
            .steps
            .push(GuestStep::KernelDeviceInit(chain));
        self.state.guest_requests.redispatch = true;
    }

    /// Writes the list of lists, the NUL header, the first SFT block, the
    /// SDA flags and the private stubs and tables of the kernel.
    fn write_kernel_data(&self, memory: &mut dyn MemoryAccess, chain: FarPointer) {
        let structures = self.state.structures();
        let dos_data_segment = self.state.dos_data_segment();
        let list_of_lists = self.state.list_of_lists_address();

        let list_start = list_of_lists - tables::SYSVARS_FIRST_MCB_DISTANCE;
        let list_end = list_of_lists + tables::SYSVARS_OFF_NUL_HEADER + tables::DEVHDR_SIZE as u32;
        memory.write_block(list_start, &vec![0u8; (list_end - list_start) as usize]);
        write_far_ptr(
            memory,
            list_of_lists + tables::SYSVARS_OFF_SFT_PTR,
            dos_data_segment,
            structures.system_file_table.first_block_offset,
        );
        let nul = self.state.dos_data_address(self.state.nul_device_offset());
        tables::write_device_header(
            memory,
            nul,
            chain.segment,
            chain.offset,
            tables::DEVATTR_CHAR | tables::DEVATTR_NUL,
            b"NUL     ",
        );
        let stubs = self.kernel_stubs_offset();
        memory.write_word(
            nul + tables::DEVHDR_OFF_STRATEGY,
            stubs + NUL_ROUTINE_OFFSET,
        );
        memory.write_word(
            nul + tables::DEVHDR_OFF_INTERRUPT,
            stubs + NUL_ROUTINE_OFFSET,
        );

        let sft_block = self.state.first_sft_block_address();
        let sft_size = tables::SFT_HEADER_SIZE
            + u32::from(FIRST_SFT_BLOCK_ENTRIES) * structures.system_file_table.entry_size;
        memory.write_block(sft_block, &vec![0u8; sft_size as usize]);
        write_far_ptr(
            memory,
            sft_block + tables::SFT_BLOCK_OFF_NEXT,
            END_OF_CHAIN.0,
            END_OF_CHAIN.1,
        );
        memory.write_word(
            sft_block + tables::SFT_BLOCK_OFF_COUNT,
            FIRST_SFT_BLOCK_ENTRIES,
        );

        let sda = &structures.swappable_data_area;
        memory.write_block(
            self.state.sda_address(),
            &vec![0u8; usize::from(sda.private_tail_offset - sda.data_offset)],
        );
        memory.write_byte(
            self.state.sda_address() + u32::from(tables::SDA_OFF_ERROR_DRIVE),
            0xFF,
        );

        let private_area = &structures.private_area;
        memory.write_block(
            self.state.dos_data_address(private_area.dbcs_table),
            &crate::country::DBCS_LEAD_BYTES,
        );
        memory.write_block(
            self.state
                .dos_data_address(private_area.critical_error_stub),
            &crate::CRITICAL_ERROR_FAIL_STUB,
        );
        trap::write_far_trap_stub(
            memory,
            self.state
                .dos_data_address(private_area.error_retriever_stub),
            crate::DosTrapCode::ErrorMessageRetriever,
        );
        self.write_kernel_stubs(memory);
    }

    /// Returns the offset of the kernel stubs of the layout.
    fn kernel_stubs_offset(&self) -> u16 {
        self.state
            .structures()
            .private_area
            .kernel_stubs
            .expect("the layout of the kernel entry has kernel stubs")
    }

    /// Writes the NUL device routine, the `IRET` stub, the CP/M call handler
    /// and the interrupt trap stubs.
    fn write_kernel_stubs(&self, memory: &mut dyn MemoryAccess) {
        let base = self.kernel_stubs_offset();
        let address = |offset: u16| self.state.dos_data_address(base + offset);
        memory.write_block(address(0), &vec![0u8; usize::from(KERNEL_STUBS_SIZE)]);
        memory.write_block(address(NUL_ROUTINE_OFFSET), &NUL_ROUTINE);
        memory.write_byte(address(IRET_STUB_OFFSET), 0xCF);
        memory.write_block(address(CPM_HANDLER_OFFSET), &CPM_HANDLER);
        memory.write_block(
            address(CPM_HANDLER_OFFSET + CPM_HANDLER.len() as u16),
            &CPM_HANDLER_TAIL,
        );
        for (index, vector) in KERNEL_INTERRUPT_VECTORS.iter().enumerate() {
            trap::write_interrupt_trap_stub(memory, address(trap_stub_offset(index)), *vector);
        }
    }

    /// Writes the INIT request for `device` and returns its guest call.
    pub(crate) fn prepare_kernel_device_init(
        &mut self,
        device: FarPointer,
        area: &GuestCallArea,
        memory: &mut dyn MemoryAccess,
    ) -> GuestCall {
        write_request_header(
            memory,
            area.request_address(),
            INIT_REQUEST_LENGTH,
            0,
            COMMAND_INIT,
        );
        GuestCall::DeviceRequest {
            device,
            data_segment: self.state.dos_data_segment(),
        }
    }

    /// Records the INIT answer of `device` and returns the INIT step of the
    /// next device of the chain.
    pub(crate) fn finish_kernel_device_init(
        &mut self,
        device: FarPointer,
        area: &GuestCallArea,
        memory: &mut dyn MemoryAccess,
    ) -> Vec<GuestStep> {
        let header = device.linear();
        let request = area.request_address();
        let attribute = memory.read_word(header + tables::DEVHDR_OFF_ATTRIBUTE);
        let mut unit_bpbs = Vec::new();
        if attribute & tables::DEVATTR_CHAR == 0 {
            let status = request_status(memory, request);
            let units = if status_is_error(status) {
                warn!(
                    "NeetanDos: INIT of the block device at {:04X}:{:04X} failed ({status:04X})",
                    device.segment, device.offset
                );
                0
            } else {
                memory.read_byte(request + REQUEST_OFF_MEDIA)
            };
            let array = FarPointer::read(memory, request + REQUEST_OFF_BPB_POINTER);
            for unit in 0..u32::from(units) {
                let offset = memory.read_word(array.linear() + unit * 2);
                let bpb_address = FarPointer::new(array.segment, offset).linear();
                match read_driver_bpb(memory, bpb_address) {
                    Some(bpb) => unit_bpbs.push(bpb),
                    None => {
                        warn!("NeetanDos: unit {unit} of a block device has no valid BPB");
                        break;
                    }
                }
            }
            memory.write_byte(header + DEVHDR_OFF_UNIT_COUNT, unit_bpbs.len() as u8);
        }

        let progress = self.kernel_init.get_or_insert_with(KernelInit::default);
        progress.devices.push(InitializedDevice {
            header: device,
            attribute,
            unit_bpbs,
        });
        let next = FarPointer::read(memory, header + tables::DEVHDR_OFF_NEXT_PTR);
        if (next.segment, next.offset) == END_OF_CHAIN {
            progress.devices_initialized = true;
            Vec::new()
        } else {
            vec![GuestStep::KernelDeviceInit(next)]
        }
    }

    /// Builds the drives, the console, the vectors, the MCB and the PSP and
    /// sets the return registers.
    fn finish_kernel_init(
        &mut self,
        cpu: &mut dyn CpuAccess,
        memory: &mut dyn MemoryAccess,
        disk: &mut dyn DiskIo,
    ) {
        let progress = self.kernel_init.take().unwrap_or_default();
        let dos_data_segment = self.state.dos_data_segment();
        let list_of_lists = self.state.list_of_lists_address();

        let block_devices: Vec<ChainBlockDevice> = progress
            .devices
            .iter()
            .filter(|device| !device.is_character())
            .map(|device| ChainBlockDevice {
                header_segment: device.header.segment,
                header_offset: device.header.offset,
                unit_bpbs: device.unit_bpbs.clone(),
            })
            .collect();
        self.state.drive_map = DriveMap::from_chain_block_devices(
            self.state.platform.machine,
            &block_devices,
            memory,
            disk,
        );
        self.state.fat_volumes.fill(None);
        self.state.mount_drive_volumes(disk);

        let system_data = self.state.system_data_area();
        let first_mcb_segment = system_data.first_mcb_segment(dos_data_segment);
        self.state.platform.placement.first_mcb_segment = first_mcb_segment;
        memory.write_word(
            list_of_lists - tables::SYSVARS_FIRST_MCB_DISTANCE,
            first_mcb_segment,
        );
        system_data.write_buffer_and_pointers(memory, dos_data_segment, list_of_lists);
        self.write_drive_structures(memory, &system_data);
        link_parameter_blocks_to_drivers(&self.state, memory, &block_devices, &system_data);
        self.state.stale_parameter_blocks = 0;

        let console = progress
            .devices
            .first()
            .filter(|device| device.is_character())
            .map(|device| device.header);
        if let Some(console) = console {
            self.install_kernel_console(memory, console);
        }
        if let Some(clock) = progress
            .devices
            .iter()
            .find(|device| device.is_character() && device.attribute & tables::DEVATTR_CLOCK != 0)
        {
            write_far_ptr(
                memory,
                list_of_lists + tables::SYSVARS_OFF_CLOCK_PTR,
                clock.header.segment,
                clock.header.offset,
            );
            self.attach_guest_clock(clock.header);
        }

        let psp_segment = self.write_kernel_process(memory);
        self.install_kernel_vectors(memory, psp_segment);
        self.state.dbcs_table_addr = self
            .state
            .dos_data_address(self.state.structures().private_area.dbcs_table);
        self.write_swappable_data_area(memory);

        cpu.set_es(dos_data_segment);
        cpu.set_di(self.state.structures().list_of_lists.data_offset);
        cpu.set_ds(psp_segment);
        cpu.set_dx(psp_segment);
        cpu.set_ax(DOSINIT_RETURN_AX);
        cpu.set_bx(DOSINIT_RETURN_BX);
        cpu.set_cx(DOSINIT_RETURN_CX);
        cpu.set_si(DOSINIT_RETURN_SI);
        cpu.set_bp(dos_data_segment);
        crate::write_iret_carry(cpu, memory, true);
        crate::set_iret_zf(cpu, memory, false);
    }

    /// Makes `console` the CON device: SFT entry 0, `LoL+0Ch`, the standard
    /// device bits of its attribute and the console backend.
    fn install_kernel_console(&mut self, memory: &mut dyn MemoryAccess, console: FarPointer) {
        let header = console.linear();
        let attribute = memory.read_word(header + tables::DEVHDR_OFF_ATTRIBUTE);
        memory.write_word(
            header + tables::DEVHDR_OFF_ATTRIBUTE,
            attribute | STANDARD_DEVICE_BITS,
        );
        write_far_ptr(
            memory,
            self.state.list_of_lists_address() + tables::SYSVARS_OFF_CON_PTR,
            console.segment,
            console.offset,
        );

        let entry = self.state.first_sft_block_address() + tables::SFT_HEADER_SIZE;
        memory.write_word(entry + tables::SFT_ENT_REF_COUNT, CON_REFERENCE_COUNT);
        memory.write_word(entry + tables::SFT_ENT_OPEN_MODE, CON_OPEN_MODE);
        memory.write_word(entry + tables::SFT_ENT_DEV_INFO, CON_DEVICE_INFO);
        write_far_ptr(
            memory,
            entry + tables::SFT_ENT_DEV_PTR,
            console.segment,
            console.offset,
        );
        let mut name = [b' '; 11];
        memory.read_block(header + tables::DEVHDR_OFF_NAME, &mut name[..8]);
        memory.write_block(entry + tables::SFT_ENT_NAME, &name);

        self.attach_guest_console(memory, console);
    }

    /// Writes the free `Z` MCB behind the system data and the first PSP at the MCB
    /// + 1. Returns the PSP segment.
    fn write_kernel_process(&mut self, memory: &mut dyn MemoryAccess) -> u16 {
        let placement = self.state.platform.placement;
        let mcb = u32::from(placement.first_mcb_segment) << 4;
        memory.write_block(mcb, &[0u8; tables::MCB_HEADER_SIZE as usize]);
        memory.write_byte(mcb + tables::MCB_OFF_TYPE, MCB_TYPE_Z);
        memory.write_word(mcb + tables::MCB_OFF_OWNER, tables::MCB_OWNER_FREE);
        memory.write_word(
            mcb + tables::MCB_OFF_SIZE,
            placement.memory_top_segment - placement.first_mcb_segment - 1,
        );

        let psp_segment = placement.first_mcb_segment + 1;
        process::write_psp(
            memory,
            psp_segment,
            psp_segment,
            0,
            placement.memory_top_segment,
        );
        let handles = (u32::from(psp_segment) << 4) + tables::PSP_OFF_JFT;
        for handle in 0..PSP_HANDLE_COUNT {
            let entry = if handle < STANDARD_HANDLES {
                0
            } else {
                CLOSED_HANDLE
            };
            memory.write_byte(handles + handle, entry);
        }

        self.state.current_psp = psp_segment;
        self.state.dta_segment = psp_segment;
        self.state.dta_offset = 0x0080;
        self.state.dta_address = (u32::from(psp_segment) << 4) + 0x0080;
        psp_segment
    }

    /// Points the kernel vectors at the kernel stubs. `INT 22h` names the
    /// first PSP, `INT 30h` and `31h` hold the CP/M call entry.
    fn install_kernel_vectors(&self, memory: &mut dyn MemoryAccess, psp_segment: u16) {
        let stubs = self.kernel_stubs_offset();
        for vector in KERNEL_IRET_VECTORS {
            self.install_vector(memory, vector, stubs + IRET_STUB_OFFSET);
        }
        for (index, vector) in KERNEL_INTERRUPT_VECTORS.iter().enumerate() {
            self.install_vector(memory, *vector, stubs + trap_stub_offset(index));
        }
        self.install_vector(
            memory,
            CRITICAL_ERROR_VECTOR,
            self.state.structures().private_area.critical_error_stub,
        );
        write_far_ptr(
            memory,
            u32::from(TERMINATE_ADDRESS_VECTOR) * 4,
            psp_segment,
            TERMINATE_ADDRESS_OFFSET,
        );
        memory.write_byte(CPM_ENTRY_ADDRESS, JMP_FAR);
        write_far_ptr(
            memory,
            CPM_ENTRY_ADDRESS + 1,
            self.state.dos_data_segment(),
            stubs + CPM_HANDLER_OFFSET,
        );
    }
}

impl NeetanDos {
    /// Adds a guest unit for every DPB of the DPB chain whose drive has no
    /// entry in the drive map and whose driver is a block device of the
    /// device chain. SYSINIT raises `LoL+20h` and links the DPB of a
    /// `CONFIG.SYS` block driver, calls `AH=53h` for its geometry, then
    /// writes the driver pointer and links the driver behind NUL. The driver
    /// test skips a DPB until SYSINIT completes it. A placement that is not
    /// ordered (the kernel entry did not run yet) has no list of lists to
    /// read.
    pub(crate) fn adopt_guest_parameter_blocks(&mut self, memory: &dyn MemoryAccess) {
        if !self.state.platform.placement.is_ordered() {
            return;
        }
        let list_of_lists = self.state.list_of_lists_address();
        let block_units =
            usize::from(memory.read_byte(list_of_lists + tables::SYSVARS_OFF_BLOCK_DEVS));
        if block_units <= self.state.drive_map.drives_with_parameter_block().count() {
            return;
        }
        let structures = self.state.structures();
        let dpb_layout = &structures.drive_parameter_block;
        let nul = FarPointer::new(
            self.state.dos_data_segment(),
            self.state.nul_device_offset(),
        );
        let mut parameter_block =
            FarPointer::read(memory, list_of_lists + tables::SYSVARS_OFF_DPB_PTR);
        for _ in 0..drive_map::DRIVE_LETTER_COUNT {
            if (parameter_block.segment, parameter_block.offset) == END_OF_CHAIN {
                break;
            }
            let address = parameter_block.linear();
            let drive_index = memory.read_byte(address + tables::DPB_OFF_DRIVE_NUM);
            let driver = FarPointer::read(memory, address + dpb_layout.device);
            if usize::from(drive_index) < drive_map::DRIVE_LETTER_COUNT
                && self.state.drive_map.entry(drive_index).is_none()
                && device_chain::is_chain_block_device(memory, nul, driver)
            {
                let unit = GuestBlockUnit {
                    header_segment: driver.segment,
                    header_offset: driver.offset,
                    unit: memory.read_byte(address + tables::DPB_OFF_UNIT_NUM),
                };
                let bpb = DpbGeometry::read(memory, address, structures).to_bpb();
                self.state.drive_map.set(
                    drive_index,
                    drive_map::fm_towns::guest_unit_entry(unit, bpb),
                );
                self.state.guest_volumes.drop_image(drive_index);
            }
            parameter_block = FarPointer::read(memory, address + dpb_layout.next);
        }
    }
}

/// Points the DPB of every drive at the chain driver of its unit. The drive
/// letters count up over the units of `block_devices` in chain order.
fn link_parameter_blocks_to_drivers(
    state: &crate::DosState,
    memory: &mut dyn MemoryAccess,
    block_devices: &[ChainBlockDevice],
    system_data: &SystemDataArea,
) {
    let unit_drivers: Vec<FarPointer> = block_devices
        .iter()
        .flat_map(|device| {
            std::iter::repeat_n(
                FarPointer::new(device.header_segment, device.header_offset),
                device.unit_bpbs.len(),
            )
        })
        .collect();
    let device_field = state.structures().drive_parameter_block.device;
    for (chain_index, (drive_index, _)) in state.drive_map.drives_with_parameter_block().enumerate()
    {
        let Some(driver) = unit_drivers.get(usize::from(drive_index)) else {
            continue;
        };
        let parameter_block =
            state.dos_data_address(system_data.parameter_block_offset(chain_index));
        write_far_ptr(
            memory,
            parameter_block + device_field,
            driver.segment,
            driver.offset,
        );
    }
}

/// Returns the offset of the trap stub of the vector at `index` of
/// `KERNEL_INTERRUPT_VECTORS` in the kernel stubs.
const fn trap_stub_offset(index: usize) -> u16 {
    TRAP_STUBS_OFFSET + index as u16 * trap::INTERRUPT_TRAP_STUB_LENGTH
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::DOS310_LAYOUT;

    /// End of the private window `0A10h`-`0B7Fh` of the DOS 3.10 data
    /// segment.
    const PRIVATE_WINDOW_END: u16 = 0x0B80;

    #[test]
    fn kernel_stub_parts_do_not_overlap() {
        const {
            assert!(NUL_ROUTINE_OFFSET + NUL_ROUTINE.len() as u16 <= IRET_STUB_OFFSET);
            assert!(IRET_STUB_OFFSET < CPM_HANDLER_OFFSET);
            assert!(
                CPM_HANDLER_OFFSET + (CPM_HANDLER.len() + CPM_HANDLER_TAIL.len()) as u16
                    <= TRAP_STUBS_OFFSET
            );
        }
        assert_eq!(
            trap_stub_offset(KERNEL_INTERRUPT_VECTORS.len()),
            KERNEL_STUBS_SIZE
        );
    }

    #[test]
    fn dos310_kernel_stubs_fit_into_the_private_window() {
        let private_area = DOS310_LAYOUT.private_area;
        let stubs = private_area.kernel_stubs.expect("3.10 has kernel stubs");
        let message_buffer_end =
            private_area.error_message_buffer + tables::ERROR_MESSAGE_BUFFER_SIZE as u16;
        assert!(message_buffer_end <= stubs);
        let stubs_end = stubs + KERNEL_STUBS_SIZE;
        let transfer_buffer = private_area
            .transfer_buffer
            .expect("3.10 has a transfer buffer");
        assert!(stubs_end <= PRIVATE_WINDOW_END);
        assert!(stubs_end <= transfer_buffer);
        assert!(private_area.system_data > transfer_buffer);
    }

    #[test]
    fn iret_vectors_and_trap_vectors_are_disjoint_and_leave_int_29h() {
        for vector in KERNEL_IRET_VECTORS {
            assert!(!KERNEL_INTERRUPT_VECTORS.contains(&vector), "{vector:02X}");
        }
        assert!(!KERNEL_IRET_VECTORS.contains(&0x29));
        assert!(!KERNEL_INTERRUPT_VECTORS.contains(&0x29));
        assert!(!KERNEL_IRET_VECTORS.contains(&CRITICAL_ERROR_VECTOR));
    }
}
