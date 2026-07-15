//! PC-88VA SGP bus wiring and interrupt scheduling.

use device::sgp_pc88va::SgpMemory;
pub(crate) use device::sgp_pc88va::SgpState;

use super::Pc88VaBus;
use crate::{memory::Pc88VaMemory, scheduler::Event88Va};

impl SgpMemory for Pc88VaMemory {
    fn read_word(&self, address: u32) -> u16 {
        self.sgp_read_word(address)
    }

    fn write_word(&mut self, address: u32, value: u16) {
        self.sgp_write_word(address, value);
    }
}

fn sgp_not_active(port: u16) -> u8 {
    if port & 0x0F == 0x0A { 0xFA } else { 0xFE }
}

fn sgp_not_implemented(port: u16) -> u8 {
    if port & 1 != 0 {
        if port == 0x501 || port == 0x503 {
            0xFF
        } else if port & 0x02 != 0 {
            0xFD
        } else {
            0xFF
        }
    } else if port & 0x0F == 0x0A {
        0xFA
    } else {
        0xFE
    }
}

impl<T: common::TraceSink> Pc88VaBus<T> {
    fn sgp_active(&self) -> bool {
        self.memory.gmsp_bit() != 0
    }

    pub(crate) fn sgp_io_read(&self, port: u16) -> u8 {
        if !self.sgp_active() {
            return sgp_not_active(port);
        }
        match port {
            0x504 => self.sgp.control(),
            0x506 => self.sgp.busy(),
            0x508 => 1,
            _ => sgp_not_implemented(port),
        }
    }

    pub(crate) fn sgp_io_write(&mut self, port: u16, value: u8) {
        match port {
            0x500..=0x503 => self.sgp.write_address_byte(port, value),
            0x504 => self.write_sgp_control(value),
            0x506 if self.sgp.write_trigger(value) => self.start_sgp(),
            _ => {}
        }
    }

    fn write_sgp_control(&mut self, value: u8) {
        let effect = self.sgp.write_control(value);
        if effect.abort {
            self.scheduler.cancel(Event88Va::SgpComplete);
            self.update_next_event_cycle();
        }
        if effect.raise_interrupt {
            self.pic.set_irq(8);
        }
        if effect.clear_interrupt {
            self.pic.clear_irq(8);
        }
    }

    fn start_sgp(&mut self) {
        if !self.sgp_active() {
            return;
        }
        let cycles = self.sgp.execute(&mut self.memory);
        self.scheduler
            .schedule(Event88Va::SgpComplete, self.current_cycle + cycles.max(1));
        self.update_next_event_cycle();
    }

    pub(crate) fn on_sgp_complete(&mut self) {
        if self.sgp.complete() {
            self.pic.set_irq(8);
        }
    }
}
