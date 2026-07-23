//! INT 08h timer tick bookkeeping, INT 1Ah time-of-day services and the
//! INT 70h RTC interrupt work.

use common::{Cpu, TraceSink};

use super::{AtBus, METADATA_RTC_ALARM_HELPER};

/// BIOS data area: timer tick count (dword).
const BDA_TIMER_COUNT: u32 = 0x46C;
/// BIOS data area: timer 24-hour rollover flag.
const BDA_TIMER_OVERFLOW: u32 = 0x470;
/// BIOS data area: diskette motor status.
const BDA_FLOPPY_MOTOR: u32 = 0x43F;
/// BIOS data area: diskette motor shutoff counter.
const BDA_FLOPPY_MOTOR_COUNT: u32 = 0x440;
/// Timer ticks in 24 hours at the 18.2 Hz tick rate.
const TICKS_PER_DAY: u32 = 0x0018_00B0;
/// FDC digital output register port.
const FDC_DOR_PORT: u16 = 0x3F2;
/// DOR bits 7:4: the four drive motor enables.
const DOR_MOTOR_MASK: u8 = 0xF0;
/// BDA 40:3F bits 3:0: drive motor running flags.
const MOTOR_STATUS_RUNNING_MASK: u8 = 0x0F;
/// CMOS register: RTC seconds (BCD).
const CMOS_RTC_SECONDS: usize = 0x00;
/// CMOS register: RTC minutes (BCD).
const CMOS_RTC_MINUTES: usize = 0x02;
/// CMOS register: RTC hours (BCD).
const CMOS_RTC_HOURS: usize = 0x04;
/// CMOS register: RTC day of month (BCD).
const CMOS_RTC_DAY: usize = 0x07;
/// CMOS register: RTC month (BCD).
const CMOS_RTC_MONTH: usize = 0x08;
/// CMOS register: RTC year (BCD, two digits).
const CMOS_RTC_YEAR: usize = 0x09;
/// CMOS register: RTC control register B.
const CMOS_RTC_REG_B: usize = 0x0B;
/// CMOS register: century (BCD).
const CMOS_RTC_CENTURY: usize = 0x32;
/// RTC register B bit 0: daylight savings enable.
const RTC_REG_B_DSE: u8 = 0x01;
/// RTC address port; bit 7 keeps NMI disabled like the POST exit state.
const RTC_ADDRESS_PORT: u16 = 0x70;
/// RTC data port.
const RTC_DATA_PORT: u16 = 0x71;
/// RTC address bit 7: NMI disable.
const RTC_NMI_DISABLE: u8 = 0x80;
/// RTC register index: seconds alarm (BCD).
const RTC_REG_SECONDS_ALARM: u8 = 0x01;
/// RTC register index: minutes alarm (BCD).
const RTC_REG_MINUTES_ALARM: u8 = 0x03;
/// RTC register index: hours alarm (BCD).
const RTC_REG_HOURS_ALARM: u8 = 0x05;
/// RTC register index: control register A.
const RTC_REG_A: u8 = 0x0A;
/// RTC register index: control register B.
const RTC_REG_B: u8 = 0x0B;
/// RTC register index: control register C (interrupt flags, read clears).
const RTC_REG_C: u8 = 0x0C;
/// RTC register A bits 3:0: periodic interrupt rate select.
const RTC_REG_A_RATE_MASK: u8 = 0x0F;
/// RTC register A rate select for 1024 Hz.
const RTC_REG_A_RATE_1024HZ: u8 = 0x06;
/// RTC register B bit 6: periodic interrupt enable.
pub(super) const RTC_REG_B_PIE: u8 = 0x40;
/// RTC register B bit 5: alarm interrupt enable.
const RTC_REG_B_AIE: u8 = 0x20;
/// RTC register C bit 6: periodic interrupt flag.
const RTC_REG_C_PERIODIC: u8 = 0x40;
/// RTC register C bit 5: alarm interrupt flag.
const RTC_REG_C_ALARM: u8 = 0x20;
/// Slave PIC interrupt mask register port.
const SLAVE_IMR_PORT: u16 = 0xA1;
/// Slave PIC IMR bit 0: IRQ 8 (RTC).
const IRQ8_MASK_BIT: u8 = 0x01;
/// Microseconds per RTC periodic tick at the 1024 Hz rate.
const WAIT_MICROSECONDS_PER_TICK: u32 = 976;
/// BIOS data area: event wait user flag pointer offset (word).
const BDA_WAIT_POINTER_OFFSET: u32 = 0x498;
/// BIOS data area: event wait user flag pointer segment (word).
const BDA_WAIT_POINTER_SEGMENT: u32 = 0x49A;
/// BIOS data area: event wait microsecond count (dword).
const BDA_WAIT_COUNT: u32 = 0x49C;
/// BIOS data area: event wait active flag.
const BDA_WAIT_ACTIVE: u32 = 0x4A0;
/// BDA 40:A0 bit 0: an event wait interval is armed.
const WAIT_ACTIVE_BIT: u8 = 0x01;
/// User flag bit 7: the event wait interval elapsed.
const WAIT_ELAPSED_BIT: u8 = 0x80;

impl<T: TraceSink> AtBus<T> {
    /// INT 08h tick work: advances the BDA tick count with the midnight
    /// rollover and runs the floppy motor shutoff countdown. Never touches
    /// CPU registers or the IRET frame; the ROM stub chains INT 1Ch and
    /// sends the EOI after the trap returns.
    pub(super) fn hle_int08h(&mut self) {
        let count = self.read_mem_dword(BDA_TIMER_COUNT).wrapping_add(1);
        if count >= TICKS_PER_DAY {
            self.write_mem_dword(BDA_TIMER_COUNT, 0);
            self.write_mem_byte(BDA_TIMER_OVERFLOW, 1);
        } else {
            self.write_mem_dword(BDA_TIMER_COUNT, count);
        }

        let motor_ticks = self.read_mem_byte(BDA_FLOPPY_MOTOR_COUNT);
        if motor_ticks != 0 {
            let remaining = motor_ticks - 1;
            self.write_mem_byte(BDA_FLOPPY_MOTOR_COUNT, remaining);
            if remaining == 0 {
                let status = self.read_mem_byte(BDA_FLOPPY_MOTOR);
                self.write_mem_byte(BDA_FLOPPY_MOTOR, status & !MOTOR_STATUS_RUNNING_MASK);
                let dor = self.fdc.read_dor();
                self.fdc_io_write(FDC_DOR_PORT, dor & !DOR_MOTOR_MASK);
            }
        }

        self.tick_teletype_beep();
    }

    /// INT 70h (IRQ 8) RTC interrupt work: consumes the register C flags,
    /// runs the INT 15h AH=83h event-wait countdown on the periodic tick and
    /// chains the guest INT 4Ah hook on the alarm. Never touches CPU
    /// registers or the original IRET frame; the ROM stub sends the EOIs
    /// after the trap returns.
    pub(super) fn hle_int70h(&mut self, cpu: &mut impl Cpu) {
        let flags = self.rtc_read_register(RTC_REG_C);

        if flags & RTC_REG_C_PERIODIC != 0
            && self.read_mem_byte(BDA_WAIT_ACTIVE) & WAIT_ACTIVE_BIT != 0
        {
            let remaining = self.read_mem_dword(BDA_WAIT_COUNT);
            if remaining > WAIT_MICROSECONDS_PER_TICK {
                self.write_mem_dword(BDA_WAIT_COUNT, remaining - WAIT_MICROSECONDS_PER_TICK);
            } else {
                let offset = self.read_mem_word(BDA_WAIT_POINTER_OFFSET);
                let segment = self.read_mem_word(BDA_WAIT_POINTER_SEGMENT);
                let user_flag = (u32::from(segment) << 4).wrapping_add(u32::from(offset));
                let value = self.read_mem_byte(user_flag);
                self.write_mem_byte(user_flag, value | WAIT_ELAPSED_BIT);
                self.write_mem_dword(BDA_WAIT_COUNT, 0);
                self.write_mem_byte(BDA_WAIT_ACTIVE, 0);
                self.rtc_update_reg_b(0, RTC_REG_B_PIE);
            }
        }

        if flags & RTC_REG_C_ALARM != 0 {
            self.retarget_frame_to_helper(cpu, METADATA_RTC_ALARM_HELPER);
        }
    }

    /// Reads an RTC register through the guest-visible ports, keeping NMI
    /// disabled like the POST exit state.
    pub(super) fn rtc_read_register(&mut self, register: u8) -> u8 {
        self.io_write(RTC_ADDRESS_PORT, register | RTC_NMI_DISABLE);
        self.io_read(RTC_DATA_PORT).0
    }

    /// Writes an RTC register through the guest-visible ports so the write
    /// side effects (event rescheduling) fire like a hardware access.
    pub(super) fn rtc_write_register(&mut self, register: u8, value: u8) {
        self.io_write(RTC_ADDRESS_PORT, register | RTC_NMI_DISABLE);
        self.io_write(RTC_DATA_PORT, value);
    }

    /// Forces the RTC periodic rate to the standard BIOS 1024 Hz select.
    pub(super) fn rtc_select_periodic_rate_1024hz(&mut self) {
        let register_a = self.rtc_read_register(RTC_REG_A);
        self.rtc_write_register(
            RTC_REG_A,
            (register_a & !RTC_REG_A_RATE_MASK) | RTC_REG_A_RATE_1024HZ,
        );
    }

    /// Updates RTC register B bits and keeps the slave PIC IRQ 8 mask in
    /// sync: unmasked while either RTC interrupt source is enabled, masked
    /// again (the POST state) when both are off.
    pub(super) fn rtc_update_reg_b(&mut self, set: u8, clear: u8) {
        let value = (self.rtc_read_register(RTC_REG_B) | set) & !clear;
        self.rtc_write_register(RTC_REG_B, value);
        let interrupts_enabled = value & (RTC_REG_B_PIE | RTC_REG_B_AIE) != 0;
        let imr = self.io_read(SLAVE_IMR_PORT).0;
        let new_imr = if interrupts_enabled {
            imr & !IRQ8_MASK_BIT
        } else {
            imr | IRQ8_MASK_BIT
        };
        if new_imr != imr {
            self.io_write(SLAVE_IMR_PORT, new_imr);
        }
    }

    /// INT 1Ah time services dispatch.
    pub(super) fn hle_int1ah(&mut self, cpu: &mut impl Cpu) {
        match cpu.ah() {
            0x00 => self.int1ah_read_tick_count(cpu),
            0x01 => self.int1ah_set_tick_count(cpu),
            0x02 => self.int1ah_read_rtc_time(cpu),
            0x03 => self.int1ah_set_rtc_time(cpu),
            0x04 => self.int1ah_read_rtc_date(cpu),
            0x05 => self.int1ah_set_rtc_date(cpu),
            0x06 => self.int1ah_set_alarm(cpu),
            0x07 => self.int1ah_reset_alarm(cpu),
            _ => self.set_iret_cf(cpu, true),
        }
    }

    /// AH=06h: arms the RTC alarm at the BCD time in CH/CL/DH. The INT 70h
    /// handler chains the guest INT 4Ah hook when it fires. Fails with the
    /// carry flag when an alarm is already armed.
    fn int1ah_set_alarm(&mut self, cpu: &mut impl Cpu) {
        if self.rtc_read_register(RTC_REG_B) & RTC_REG_B_AIE != 0 {
            self.set_iret_cf(cpu, true);
            return;
        }
        self.rtc_write_register(RTC_REG_HOURS_ALARM, cpu.ch());
        self.rtc_write_register(RTC_REG_MINUTES_ALARM, cpu.cl());
        self.rtc_write_register(RTC_REG_SECONDS_ALARM, cpu.dh());
        self.rtc_update_reg_b(RTC_REG_B_AIE, 0);
        self.set_iret_cf(cpu, false);
    }

    /// AH=07h: disarms the RTC alarm.
    fn int1ah_reset_alarm(&mut self, cpu: &mut impl Cpu) {
        self.rtc_update_reg_b(0, RTC_REG_B_AIE);
        self.set_iret_cf(cpu, false);
    }

    /// AH=00h: returns the tick count in CX:DX and the midnight flag in AL,
    /// clearing the flag.
    fn int1ah_read_tick_count(&mut self, cpu: &mut impl Cpu) {
        let count = self.read_mem_dword(BDA_TIMER_COUNT);
        let overflow = self.read_mem_byte(BDA_TIMER_OVERFLOW);
        self.write_mem_byte(BDA_TIMER_OVERFLOW, 0);
        cpu.set_cx((count >> 16) as u16);
        cpu.set_dx(count as u16);
        cpu.set_al(overflow);
        self.set_iret_cf(cpu, false);
    }

    /// AH=01h: sets the tick count from CX:DX and clears the midnight flag.
    fn int1ah_set_tick_count(&mut self, cpu: &mut impl Cpu) {
        let count = (u32::from(cpu.cx()) << 16) | u32::from(cpu.dx());
        self.write_mem_dword(BDA_TIMER_COUNT, count);
        self.write_mem_byte(BDA_TIMER_OVERFLOW, 0);
        self.set_iret_cf(cpu, false);
    }

    /// AH=02h: reads the RTC time as BCD into CH/CL/DH plus the daylight
    /// savings flag in DL.
    fn int1ah_read_rtc_time(&mut self, cpu: &mut impl Cpu) {
        cpu.set_ch(self.rtc.cmos[CMOS_RTC_HOURS]);
        cpu.set_cl(self.rtc.cmos[CMOS_RTC_MINUTES]);
        cpu.set_dh(self.rtc.cmos[CMOS_RTC_SECONDS]);
        cpu.set_dl(self.rtc.cmos[CMOS_RTC_REG_B] & RTC_REG_B_DSE);
        self.set_iret_cf(cpu, false);
    }

    /// AH=03h: sets the RTC time from the BCD values in CH/CL/DH and the
    /// daylight savings flag in DL.
    fn int1ah_set_rtc_time(&mut self, cpu: &mut impl Cpu) {
        self.rtc.cmos[CMOS_RTC_HOURS] = cpu.ch();
        self.rtc.cmos[CMOS_RTC_MINUTES] = cpu.cl();
        self.rtc.cmos[CMOS_RTC_SECONDS] = cpu.dh();
        if cpu.dl() & RTC_REG_B_DSE != 0 {
            self.rtc.cmos[CMOS_RTC_REG_B] |= RTC_REG_B_DSE;
        } else {
            self.rtc.cmos[CMOS_RTC_REG_B] &= !RTC_REG_B_DSE;
        }
        self.set_iret_cf(cpu, false);
    }

    /// AH=04h: reads the RTC date as BCD into CH (century), CL (year),
    /// DH (month) and DL (day).
    fn int1ah_read_rtc_date(&mut self, cpu: &mut impl Cpu) {
        cpu.set_ch(self.rtc.cmos[CMOS_RTC_CENTURY]);
        cpu.set_cl(self.rtc.cmos[CMOS_RTC_YEAR]);
        cpu.set_dh(self.rtc.cmos[CMOS_RTC_MONTH]);
        cpu.set_dl(self.rtc.cmos[CMOS_RTC_DAY]);
        self.set_iret_cf(cpu, false);
    }

    /// AH=05h: sets the RTC date from the BCD values in CH (century),
    /// CL (year), DH (month) and DL (day).
    fn int1ah_set_rtc_date(&mut self, cpu: &mut impl Cpu) {
        self.rtc.cmos[CMOS_RTC_CENTURY] = cpu.ch();
        self.rtc.cmos[CMOS_RTC_YEAR] = cpu.cl();
        self.rtc.cmos[CMOS_RTC_MONTH] = cpu.dh();
        self.rtc.cmos[CMOS_RTC_DAY] = cpu.dl();
        self.set_iret_cf(cpu, false);
    }
}
