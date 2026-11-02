#[path = "guest/harness.rs"]
mod harness;

#[path = "guest/calls.rs"]
mod calls;

#[path = "guest/console.rs"]
mod console;

#[path = "guest/clock.rs"]
mod clock;

#[path = "guest/kernel_harness.rs"]
mod kernel_harness;

#[path = "guest/kernel.rs"]
mod kernel;

#[path = "guest/kernel_block_device.rs"]
mod kernel_block_device;

#[path = "guest/critical_error.rs"]
mod critical_error;

#[path = "guest/mscdex.rs"]
mod mscdex;

#[path = "guest/os_rom.rs"]
mod os_rom;
