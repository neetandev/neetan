//! INT 08h system timer tick and INT 1Ch date/time + interval timer service.

use common::{Cpu, SegmentRegister};
use device::i8253_pit::PIT_FLAG_I;

use super::{
    super::Pc9801Bus, InterruptMaskChange, PIT_CLOCK_8MHZ_LINEAGE, iret_stack_base,
    request_interrupt_mask_change,
};
use crate::TraceSink;

impl<T: TraceSink> Pc9801Bus<T> {
    /// INT 08h timer tick. Counts down the interval timer and returns the IR0
    /// mask change in CF and ZF: CF clear keeps the IMR, CF set with ZF clear
    /// unmasks IR0 and CF set with ZF set masks IR0.
    pub(super) fn hle_int08h(&mut self, cpu: &mut impl Cpu) {
        let change = self.int08h_interval_timer_tick(cpu);
        request_interrupt_mask_change(cpu, change);
    }

    /// Counts down the interval timer. Chains the INT 07h callback on expiry
    /// and reloads the timer on every other tick.
    fn int08h_interval_timer_tick(&mut self, cpu: &mut impl Cpu) -> InterruptMaskChange {
        if !self.bios_interval_timer_active {
            return InterruptMaskChange::Keep;
        }

        // IRET frame layout at SS:SP: [IP +0] [CS +2] [FLAGS +4].
        let iret_base = iret_stack_base(cpu);

        let count = self.ram_read_u16(0x058A);
        let new_count = count.wrapping_sub(1);
        self.ram_write_u16(0x058A, new_count);

        if new_count == 0 && count > 0 {
            self.bios_interval_timer_active = false;
            // The real BIOS masks IR0 before it calls the callback. The ROM
            // stub masks it with guest I/O before its IRET enters the callback.

            // Fire the user callback by chaining the IRET frame.
            // The real BIOS invokes `INT 07H` which pushes FLAGS and
            // clears IF, so the callback runs with interrupts disabled.
            let callback_offset = self.ram_read_u16(0x001C);
            let callback_segment = self.ram_read_u16(0x001E);

            if callback_offset != 0 || callback_segment != 0 {
                let orig_flags = self.read_mem_word(iret_base + 0x04);
                let callback_base = iret_base.wrapping_sub(6);
                self.write_mem_word(callback_base, callback_offset);
                self.write_mem_word(callback_base + 0x02, callback_segment);
                self.write_mem_word(callback_base + 0x04, orig_flags & !0x0200);
                cpu.set_sp(cpu.sp().wrapping_sub(6));
            }
            return InterruptMaskChange::Mask;
        }

        // Non-zero result: reload PIT counter via INT 1CH AH=03H. The real BIOS
        // issues `MOV AH,03H; INT 1CH` so software hooks on INT 1CH can intercept.
        // We call the handler directly since the IVT points to our stub.
        if new_count != 0 {
            return self.int1ch_continue_interval_timer();
        }
        InterruptMaskChange::Keep
    }

    /// INT 1Ch timer and calendar service. Returns the IR0 mask change in CF
    /// and ZF: CF clear keeps the IMR, CF set with ZF clear unmasks IR0 and CF
    /// set with ZF set masks IR0.
    pub(super) fn hle_int1ch(&mut self, cpu: &mut impl Cpu) {
        let change = match cpu.ah() {
            0x00 => {
                self.int1ch_get_datetime(cpu);
                InterruptMaskChange::Keep
            }
            0x01 => {
                self.int1ch_set_datetime(cpu);
                InterruptMaskChange::Keep
            }
            0x02 => self.int1ch_set_interval_timer(cpu),
            0x03 => self.int1ch_continue_interval_timer(),
            _ => InterruptMaskChange::Keep,
        };
        request_interrupt_mask_change(cpu, change);
    }

    fn int1ch_get_datetime(&mut self, cpu: &mut impl Cpu) {
        let time = self.host_date_time_source.now().to_bcd_bytes();
        for (i, &byte) in time.iter().enumerate() {
            self.hle_write_byte(
                cpu,
                SegmentRegister::ES,
                u32::from(cpu.bx()) + i as u32,
                byte,
            );
        }
    }

    fn int1ch_set_datetime(&mut self, cpu: &mut impl Cpu) {
        let mut buf = [0u8; 6];
        for (i, buf_byte) in buf.iter_mut().enumerate() {
            *buf_byte =
                self.hle_read_byte(cpu, SegmentRegister::ES, u32::from(cpu.bx()) + i as u32);
        }

        // Store year in Memory Switch 8 (text VRAM offset 0x3FFE).
        self.memory.state.text_vram[0x3FFE] = buf[0];

        self.rtc.load_shift_register(&buf);
    }

    /// AH=02h: stores the INT 07h callback and the count, starts the 10 ms
    /// interval timer and requests the IR0 unmask.
    fn int1ch_set_interval_timer(&mut self, cpu: &mut impl Cpu) -> InterruptMaskChange {
        let callback_seg = cpu.es();
        let callback_off = cpu.bx();
        let count = cpu.cx();

        // Store callback address in IVT vector 0x07 (at 0x001C).
        self.ram_write_u16(0x001C, callback_off);
        self.ram_write_u16(0x001E, callback_seg);

        // Store counter at CA_TIM_CNT (0x058A).
        self.ram_write_u16(0x058A, count);
        self.bios_interval_timer_active = true;

        // Program PIT channel 0 for 10ms interval timer.
        let divider: u16 = if self.clocks.pit_clock_hz == PIT_CLOCK_8MHZ_LINEAGE {
            0x4E00
        } else {
            0x6000
        };

        self.pit.write_control(
            0,
            0x36,
            self.current_cycle,
            self.clocks.cpu_clock_hz,
            self.clocks.pit_clock_hz,
        );
        self.pit.write_counter(0, divider as u8);
        self.pit.write_counter(0, (divider >> 8) as u8);
        self.pit.state.channels[0].last_load_cycle = self.current_cycle;
        self.clear_pic_irq(0);
        self.pit.state.channels[0].flag |= PIT_FLAG_I;
        let cpu_cycles = self
            .pit
            .timer0_period_cycles(self.clocks.cpu_clock_hz, self.clocks.pit_clock_hz);
        self.scheduler
            .schedule(Event98::PitTimer0, self.current_cycle + cpu_cycles);
        self.update_next_event_cycle();
        InterruptMaskChange::Unmask
    }

    /// AH=03h: reloads the 10 ms interval timer and requests the IR0 unmask.
    fn int1ch_continue_interval_timer(&mut self) -> InterruptMaskChange {
        // The real BIOS calls this on every non-expired INT 08H tick.
        let divider: u16 = if self.clocks.pit_clock_hz == PIT_CLOCK_8MHZ_LINEAGE {
            0x4E00
        } else {
            0x6000
        };
        self.pit.write_counter(0, divider as u8);
        self.pit.write_counter(0, (divider >> 8) as u8);
        self.pit.state.channels[0].last_load_cycle = self.current_cycle;
        self.clear_pic_irq(0);
        self.pit.state.channels[0].flag |= PIT_FLAG_I;
        let cpu_cycles = self
            .pit
            .timer0_period_cycles(self.clocks.cpu_clock_hz, self.clocks.pit_clock_hz);
        self.scheduler
            .schedule(Event98::PitTimer0, self.current_cycle + cpu_cycles);
        self.update_next_event_cycle();
        InterruptMaskChange::Unmask
    }
}
use crate::scheduler::Event98;
