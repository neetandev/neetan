//! V9938 asynchronous command engine.

use super::{MsxVdpDisplayMode, STATUS_COMMAND_EXECUTE, STATUS_TRANSFER_READY, V9938_STATUS_COUNT};

/// STOP command number.
const COMMAND_STOP: u8 = 0x0;
/// POINT command number.
const COMMAND_POINT: u8 = 0x4;
/// PSET command number.
const COMMAND_PSET: u8 = 0x5;
/// SRCH command number.
const COMMAND_SRCH: u8 = 0x6;
/// LINE command number.
const COMMAND_LINE: u8 = 0x7;
/// LMMV command number.
const COMMAND_LMMV: u8 = 0x8;
/// LMMM command number.
const COMMAND_LMMM: u8 = 0x9;
/// LMCM command number.
const COMMAND_LMCM: u8 = 0xA;
/// LMMC command number.
const COMMAND_LMMC: u8 = 0xB;
/// HMMV command number.
const COMMAND_HMMV: u8 = 0xC;
/// HMMM command number.
const COMMAND_HMMM: u8 = 0xD;
/// YMMM command number.
const COMMAND_YMMM: u8 = 0xE;
/// HMMC command number.
const COMMAND_HMMC: u8 = 0xF;
/// Decrement-X direction bit in R#45.
const ARGUMENT_DECREMENT_X: u8 = 0x04;
/// Decrement-Y direction bit in R#45.
const ARGUMENT_DECREMENT_Y: u8 = 0x08;
/// LINE major-axis bit in R#45.
const ARGUMENT_MAJOR_Y: u8 = 0x01;
/// SRCH inequality bit in R#45.
const ARGUMENT_NOT_EQUAL: u8 = 0x02;
/// Source expanded-VRAM bit in R#45.
const ARGUMENT_SOURCE_EXPANDED: u8 = 0x10;
/// Destination expanded-VRAM bit in R#45.
const ARGUMENT_DESTINATION_EXPANDED: u8 = 0x20;
/// Expanded-VRAM address offset.
const EXPANDED_VRAM_OFFSET: usize = 0x20000;
/// Maximum VDP command Y coordinate.
const MAXIMUM_Y: i32 = 1023;

/// Master-tick costs for blanked, sprite-free, and sprite-active display states.
type CommandTiming = [u64; 3];

/// POINT uses one VRAM transaction.
const POINT_TIMING: CommandTiming = [9, 16, 44];
/// PSET uses a read followed by a write.
const PSET_TIMING: CommandTiming = [18, 32, 88];
/// SRCH timing per compared pixel.
const SEARCH_TIMING: CommandTiming = [82, 83, 103];
/// LINE timing per plotted pixel.
const LINE_TIMING: CommandTiming = [106, 116, 126];
/// LMMV, LMCM, and LMMC timing per pixel.
const LOGICAL_FILL_TIMING: CommandTiming = [87, 106, 114];
/// LMMM timing per pixel.
const LOGICAL_COPY_TIMING: CommandTiming = [116, 117, 160];
/// HMMV and HMMC timing per packed byte.
const HIGH_FILL_TIMING: CommandTiming = [44, 53, 55];
/// HMMM timing per packed byte.
const HIGH_COPY_TIMING: CommandTiming = [82, 86, 111];
/// YMMM timing per packed byte.
const VERTICAL_COPY_TIMING: CommandTiming = [59, 61, 95];

/// Active command execution state.
struct ActiveCommand {
    kind: u8,
    mode: MsxVdpDisplayMode,
    source_x: i32,
    source_y: i32,
    destination_x: i32,
    destination_y: i32,
    start_source_x: i32,
    start_destination_x: i32,
    remaining_x: u16,
    width: u16,
    horizontal_step: i32,
    remaining_y: u16,
    color: u8,
    argument: u8,
    operation: u8,
    line_error: i32,
    line_major: i32,
    line_minor: i32,
}

save_state::runtime_state! {
/// Active V9938 command progress.
#[derive(Clone)]
pub(super) struct ActiveCommandState {
    kind: u8,
    mode: u8,
    source_x: i32,
    source_y: i32,
    destination_x: i32,
    destination_y: i32,
    start_source_x: i32,
    start_destination_x: i32,
    remaining_x: u16,
    width: u16,
    horizontal_step: i32,
    remaining_y: u16,
    color: u8,
    argument: u8,
    operation: u8,
    line_error: i32,
    line_major: i32,
    line_minor: i32,
}}

save_state::runtime_state! {
/// Complete asynchronous V9938 command-engine state.
#[derive(Clone)]
pub(super) struct V9938CommandEngineState {
    registers: [u8; 15],
    next_step_tick: u64,
    active: Option<crate::video_msx::command::ActiveCommandState>,
    waiting_for_cpu: bool,
    color_transfer_pending: bool,
}}

/// V9938 command engine synchronized by absolute master ticks.
pub(super) struct V9938CommandEngine {
    registers: [u8; 15],
    next_step_tick: u64,
    active: Option<ActiveCommand>,
    waiting_for_cpu: bool,
    color_transfer_pending: bool,
}

/// Encodes a display mode with a stable save-state tag.
const fn mode_to_tag(mode: MsxVdpDisplayMode) -> u8 {
    match mode {
        MsxVdpDisplayMode::Graphics1 => 0,
        MsxVdpDisplayMode::Text1 => 1,
        MsxVdpDisplayMode::Multicolor => 2,
        MsxVdpDisplayMode::Graphics2 => 3,
        MsxVdpDisplayMode::Graphics3 => 4,
        MsxVdpDisplayMode::Text2 => 5,
        MsxVdpDisplayMode::Graphics4 => 6,
        MsxVdpDisplayMode::Graphics5 => 7,
        MsxVdpDisplayMode::Graphics6 => 8,
        MsxVdpDisplayMode::Graphics7 => 9,
        MsxVdpDisplayMode::Unsupported => 10,
    }
}

/// Decodes a display mode from its stable save-state tag.
fn tag_to_mode(tag: u8) -> Result<MsxVdpDisplayMode, save_state::StateValidationError> {
    Ok(match tag {
        0 => MsxVdpDisplayMode::Graphics1,
        1 => MsxVdpDisplayMode::Text1,
        2 => MsxVdpDisplayMode::Multicolor,
        3 => MsxVdpDisplayMode::Graphics2,
        4 => MsxVdpDisplayMode::Graphics3,
        5 => MsxVdpDisplayMode::Text2,
        6 => MsxVdpDisplayMode::Graphics4,
        7 => MsxVdpDisplayMode::Graphics5,
        8 => MsxVdpDisplayMode::Graphics6,
        9 => MsxVdpDisplayMode::Graphics7,
        10 => MsxVdpDisplayMode::Unsupported,
        _ => {
            return Err(save_state::StateValidationError::new(
                "V9938 command display mode is invalid",
            ));
        }
    })
}

impl V9938CommandEngine {
    /// Creates an idle command engine.
    pub(super) const fn new() -> Self {
        Self {
            registers: [0; 15],
            next_step_tick: 0,
            active: None,
            waiting_for_cpu: false,
            color_transfer_pending: false,
        }
    }

    /// Captures active command progress and transfer handshakes.
    pub(super) fn capture_state(&self) -> V9938CommandEngineState {
        V9938CommandEngineState {
            registers: self.registers,
            next_step_tick: self.next_step_tick,
            active: self.active.as_ref().map(|active| ActiveCommandState {
                kind: active.kind,
                mode: mode_to_tag(active.mode),
                source_x: active.source_x,
                source_y: active.source_y,
                destination_x: active.destination_x,
                destination_y: active.destination_y,
                start_source_x: active.start_source_x,
                start_destination_x: active.start_destination_x,
                remaining_x: active.remaining_x,
                width: active.width,
                horizontal_step: active.horizontal_step,
                remaining_y: active.remaining_y,
                color: active.color,
                argument: active.argument,
                operation: active.operation,
                line_error: active.line_error,
                line_major: active.line_major,
                line_minor: active.line_minor,
            }),
            waiting_for_cpu: self.waiting_for_cpu,
            color_transfer_pending: self.color_transfer_pending,
        }
    }

    /// Restores active command progress and transfer handshakes.
    pub(super) fn restore_state(
        &mut self,
        state: V9938CommandEngineState,
    ) -> Result<(), save_state::StateValidationError> {
        let active = state
            .active
            .map(|active| {
                if active.kind > COMMAND_HMMC || !(1..=4).contains(&active.horizontal_step) {
                    return Err(save_state::StateValidationError::new(
                        "V9938 active command state is invalid",
                    ));
                }
                Ok(ActiveCommand {
                    kind: active.kind,
                    mode: tag_to_mode(active.mode)?,
                    source_x: active.source_x,
                    source_y: active.source_y,
                    destination_x: active.destination_x,
                    destination_y: active.destination_y,
                    start_source_x: active.start_source_x,
                    start_destination_x: active.start_destination_x,
                    remaining_x: active.remaining_x,
                    width: active.width,
                    horizontal_step: active.horizontal_step,
                    remaining_y: active.remaining_y,
                    color: active.color,
                    argument: active.argument,
                    operation: active.operation,
                    line_error: active.line_error,
                    line_major: active.line_major,
                    line_minor: active.line_minor,
                })
            })
            .transpose()?;
        self.registers = state.registers;
        self.next_step_tick = state.next_step_tick;
        self.active = active;
        self.waiting_for_cpu = state.waiting_for_cpu;
        self.color_transfer_pending = state.color_transfer_pending;
        Ok(())
    }

    /// Writes one command register.
    pub(super) fn write_register(
        &mut self,
        index: usize,
        value: u8,
        tick: u64,
        mode: MsxVdpDisplayMode,
        statuses: &mut [u8; V9938_STATUS_COUNT],
    ) {
        if index >= self.registers.len() {
            return;
        }
        self.registers[index] = value;
        if index == 12 {
            self.color_transfer_pending = true;
            let cpu_to_vram = self
                .active
                .as_ref()
                .is_some_and(|active| matches!(active.kind, COMMAND_LMMC | COMMAND_HMMC));
            if cpu_to_vram && self.waiting_for_cpu {
                self.waiting_for_cpu = false;
                statuses[2] &= !STATUS_TRANSFER_READY;
            } else if self.active.is_none() {
                statuses[2] &= !STATUS_TRANSFER_READY;
            }
        }
        if index == 14 {
            self.start_command(tick, mode, statuses);
        }
    }

    /// Advances command work to an absolute VDP tick.
    pub(super) fn advance_to(
        &mut self,
        tick: u64,
        display_active: bool,
        sprites_enabled: bool,
        _mode: MsxVdpDisplayMode,
        vram: &mut [u8],
        statuses: &mut [u8; V9938_STATUS_COUNT],
    ) {
        while self.active.is_some() && !self.waiting_for_cpu {
            let step_ticks = self.step_ticks(display_active, sprites_enabled);
            if self.next_step_tick.saturating_add(step_ticks) > tick {
                break;
            }
            self.next_step_tick = self.next_step_tick.saturating_add(step_ticks);
            self.execute_step(vram, statuses);
        }
    }

    /// Updates transfer-derived status before a status read.
    pub(super) fn prepare_status(
        &mut self,
        _selected: usize,
        _statuses: &mut [u8; V9938_STATUS_COUNT],
    ) {
    }

    /// Acknowledges a command color transfer read.
    pub(super) fn color_read(&mut self, statuses: &mut [u8; V9938_STATUS_COUNT]) {
        let Some(active) = self.active.as_mut() else {
            statuses[2] &= !STATUS_TRANSFER_READY;
            return;
        };
        if active.kind != COMMAND_LMCM || !self.waiting_for_cpu {
            return;
        }
        self.waiting_for_cpu = false;
        statuses[2] &= !STATUS_TRANSFER_READY;
    }

    /// Defers an active command when the CPU consumes a VRAM service slot.
    pub(super) fn defer_for_cpu_vram_access(&mut self, ticks: u64) {
        if self.active.is_some() {
            self.next_step_tick = self.next_step_tick.saturating_add(ticks);
        }
    }

    /// Starts or aborts the command selected by R#46.
    fn start_command(
        &mut self,
        tick: u64,
        mode: MsxVdpDisplayMode,
        statuses: &mut [u8; V9938_STATUS_COUNT],
    ) {
        let kind = self.registers[14] >> 4;
        if kind == COMMAND_STOP || !mode.is_bitmap() {
            self.finish(statuses);
            return;
        }
        let pixels_per_line = pixel_format(mode).pixels_per_line;
        let mut source_x = i32::from(register_word(&self.registers, 0, 1) & 0x01FF);
        let source_y = i32::from(register_word(&self.registers, 2, 3) & 0x03FF);
        let destination_x = i32::from(register_word(&self.registers, 4, 5) & 0x01FF);
        let destination_y = i32::from(register_word(&self.registers, 6, 7) & 0x03FF);
        let requested_width = register_word(&self.registers, 8, 9) & 0x03FF;
        let requested_height = register_word(&self.registers, 10, 11) & 0x03FF;
        let mut width = requested_width;
        let mut height = requested_height;
        if height == 0 {
            height = 1024;
        }
        let argument = self.registers[13];
        let high_speed = matches!(
            kind,
            COMMAND_HMMV | COMMAND_HMMM | COMMAND_YMMM | COMMAND_HMMC
        );
        let pixels_per_byte = 1u16 << pixel_format(mode).pixels_per_byte_shift;
        let horizontal_step = if high_speed {
            i32::from(pixels_per_byte)
        } else {
            1
        };
        if high_speed {
            width >>= pixel_format(mode).pixels_per_byte_shift;
            if width == 0 {
                width = pixels_per_line / pixels_per_byte;
            }
        } else if kind == COMMAND_LINE {
            width = requested_width.saturating_add(1);
        } else if width == 0 {
            width = pixels_per_line;
        }
        if kind == COMMAND_YMMM {
            source_x = destination_x;
            let available = if argument & ARGUMENT_DECREMENT_X != 0 {
                destination_x + 1
            } else {
                i32::from(pixels_per_line) - destination_x
            };
            width = u16::try_from(available.max(0))
                .unwrap_or(0)
                .div_ceil(pixels_per_byte);
        }
        let line_major = i32::from(requested_width);
        let line_minor = i32::from(height);
        self.active = Some(ActiveCommand {
            kind,
            mode,
            source_x,
            source_y,
            destination_x,
            destination_y,
            start_source_x: source_x,
            start_destination_x: destination_x,
            remaining_x: width,
            width,
            horizontal_step,
            remaining_y: height,
            color: self.registers[12],
            argument,
            operation: self.registers[14] & 0x0F,
            line_error: i32::from(requested_width.saturating_sub(1)) / 2,
            line_major,
            line_minor: if kind == COMMAND_LINE {
                i32::from(requested_height)
            } else {
                line_minor
            },
        });
        self.next_step_tick = tick;
        self.waiting_for_cpu =
            matches!(kind, COMMAND_LMMC | COMMAND_HMMC) && !self.color_transfer_pending;
        statuses[2] |= STATUS_COMMAND_EXECUTE;
        if matches!(kind, COMMAND_LMCM | COMMAND_LMMC | COMMAND_HMMC) {
            statuses[2] |= STATUS_TRANSFER_READY;
        }
    }

    /// Executes one pixel or byte operation.
    fn execute_step(&mut self, vram: &mut [u8], statuses: &mut [u8; V9938_STATUS_COUNT]) {
        let kind = self.active.as_ref().unwrap().kind;
        match kind {
            COMMAND_POINT => self.execute_point(vram, statuses),
            COMMAND_PSET => self.execute_pset(vram, statuses),
            COMMAND_SRCH => self.execute_search(vram, statuses),
            COMMAND_LINE => self.execute_line(vram, statuses),
            COMMAND_LMMV => self.execute_logical_fill(vram, statuses),
            COMMAND_LMMM => self.execute_logical_copy(vram, statuses),
            COMMAND_LMCM => self.execute_vram_to_cpu(vram, statuses),
            COMMAND_LMMC => self.execute_cpu_to_vram(vram, statuses),
            COMMAND_HMMV => self.execute_high_fill(vram, statuses),
            COMMAND_HMMM | COMMAND_YMMM => self.execute_high_copy(vram, statuses),
            COMMAND_HMMC => self.execute_cpu_to_vram_high(vram, statuses),
            _ => self.finish(statuses),
        }
    }

    /// Executes POINT.
    fn execute_point(&mut self, vram: &[u8], statuses: &mut [u8; V9938_STATUS_COUNT]) {
        let active = self.active.as_ref().unwrap();
        statuses[7] = read_pixel(
            vram,
            active.mode,
            active.source_x,
            active.source_y,
            active.argument & ARGUMENT_SOURCE_EXPANDED != 0,
        );
        self.finish(statuses);
    }

    /// Executes PSET.
    fn execute_pset(&mut self, vram: &mut [u8], statuses: &mut [u8; V9938_STATUS_COUNT]) {
        let active = self.active.as_ref().unwrap();
        write_pixel(
            vram,
            active.mode,
            active.destination_x,
            active.destination_y,
            active.argument & ARGUMENT_DESTINATION_EXPANDED != 0,
            active.color,
            active.operation,
        );
        self.finish(statuses);
    }

    /// Executes one SRCH comparison.
    fn execute_search(&mut self, vram: &[u8], statuses: &mut [u8; V9938_STATUS_COUNT]) {
        let active = self.active.as_mut().unwrap();
        let format = pixel_format(active.mode);
        let color = read_pixel(
            vram,
            active.mode,
            active.source_x,
            active.source_y,
            active.argument & ARGUMENT_SOURCE_EXPANDED != 0,
        );
        let equal = color == active.color & format.color_mask;
        let objective_is_unequal = active.argument & ARGUMENT_NOT_EQUAL != 0;
        if equal != objective_is_unequal {
            statuses[2] |= 0x10;
            set_search_coordinate(statuses, active.source_x);
            self.finish(statuses);
            return;
        }
        active.source_x += direction_x(active.argument);
        if !(0..i32::from(format.pixels_per_line)).contains(&active.source_x) {
            statuses[2] &= !0x10;
            set_search_coordinate(statuses, active.source_x);
            self.finish(statuses);
        }
    }

    /// Executes one LINE pixel.
    fn execute_line(&mut self, vram: &mut [u8], statuses: &mut [u8; V9938_STATUS_COUNT]) {
        let active = self.active.as_mut().unwrap();
        let format = pixel_format(active.mode);
        if !(0..=MAXIMUM_Y).contains(&active.destination_y) || active.remaining_x == 0 {
            self.finish(statuses);
            return;
        }
        write_pixel(
            vram,
            active.mode,
            active.destination_x,
            active.destination_y,
            active.argument & ARGUMENT_DESTINATION_EXPANDED != 0,
            active.color,
            active.operation,
        );
        active.remaining_x -= 1;
        let major_y = active.argument & ARGUMENT_MAJOR_Y != 0;
        if major_y {
            active.destination_y += direction_y(active.argument);
        } else {
            active.destination_x += direction_x(active.argument);
        }
        active.line_error -= active.line_minor;
        if active.line_error < 0 {
            if major_y {
                active.destination_x += direction_x(active.argument);
            } else {
                active.destination_y += direction_y(active.argument);
            }
            active.line_error += active.line_major;
        }
        if active.remaining_x == 0
            || !(0..i32::from(format.pixels_per_line)).contains(&active.destination_x)
        {
            self.finish(statuses);
        }
    }

    /// Executes one LMMV pixel.
    fn execute_logical_fill(&mut self, vram: &mut [u8], statuses: &mut [u8; V9938_STATUS_COUNT]) {
        let active = self.active.as_ref().unwrap();
        write_pixel(
            vram,
            active.mode,
            active.destination_x,
            active.destination_y,
            active.argument & ARGUMENT_DESTINATION_EXPANDED != 0,
            active.color,
            active.operation,
        );
        self.advance_rectangle(false, true, true, statuses);
    }

    /// Executes one LMMM pixel.
    fn execute_logical_copy(&mut self, vram: &mut [u8], statuses: &mut [u8; V9938_STATUS_COUNT]) {
        let active = self.active.as_ref().unwrap();
        let color = read_pixel(
            vram,
            active.mode,
            active.source_x,
            active.source_y,
            active.argument & ARGUMENT_SOURCE_EXPANDED != 0,
        );
        write_pixel(
            vram,
            active.mode,
            active.destination_x,
            active.destination_y,
            active.argument & ARGUMENT_DESTINATION_EXPANDED != 0,
            color,
            active.operation,
        );
        self.advance_rectangle(true, true, true, statuses);
    }

    /// Executes one LMCM transfer.
    fn execute_vram_to_cpu(&mut self, vram: &[u8], statuses: &mut [u8; V9938_STATUS_COUNT]) {
        let active = self.active.as_ref().unwrap();
        statuses[7] = read_pixel(
            vram,
            active.mode,
            active.source_x,
            active.source_y,
            active.argument & ARGUMENT_SOURCE_EXPANDED != 0,
        );
        let finished = self.advance_rectangle(true, false, false, statuses);
        statuses[2] |= STATUS_TRANSFER_READY;
        if finished {
            self.finish(statuses);
        } else {
            self.waiting_for_cpu = true;
        }
    }

    /// Executes one LMMC transfer.
    fn execute_cpu_to_vram(&mut self, vram: &mut [u8], statuses: &mut [u8; V9938_STATUS_COUNT]) {
        self.color_transfer_pending = false;
        let active = self.active.as_ref().unwrap();
        write_pixel(
            vram,
            active.mode,
            active.destination_x,
            active.destination_y,
            active.argument & ARGUMENT_DESTINATION_EXPANDED != 0,
            self.registers[12],
            active.operation,
        );
        if !self.advance_rectangle(false, true, true, statuses) {
            self.waiting_for_cpu = true;
            statuses[2] |= STATUS_TRANSFER_READY;
        }
    }

    /// Executes one HMMV byte.
    fn execute_high_fill(&mut self, vram: &mut [u8], statuses: &mut [u8; V9938_STATUS_COUNT]) {
        let active = self.active.as_ref().unwrap();
        let value = duplicate_color(active.mode, active.color);
        write_command_byte(
            vram,
            active.mode,
            active.destination_x,
            active.destination_y,
            active.argument & ARGUMENT_DESTINATION_EXPANDED != 0,
            value,
        );
        self.advance_rectangle(false, true, true, statuses);
    }

    /// Executes one HMMM or YMMM byte.
    fn execute_high_copy(&mut self, vram: &mut [u8], statuses: &mut [u8; V9938_STATUS_COUNT]) {
        let active = self.active.as_ref().unwrap();
        let source_x = if active.kind == COMMAND_YMMM {
            active.destination_x
        } else {
            active.source_x
        };
        let source_expanded = if active.kind == COMMAND_YMMM {
            active.argument & ARGUMENT_DESTINATION_EXPANDED != 0
        } else {
            active.argument & ARGUMENT_SOURCE_EXPANDED != 0
        };
        let value = read_command_byte(
            vram,
            active.mode,
            source_x,
            active.source_y,
            source_expanded,
        );
        write_command_byte(
            vram,
            active.mode,
            active.destination_x,
            active.destination_y,
            active.argument & ARGUMENT_DESTINATION_EXPANDED != 0,
            value,
        );
        self.advance_rectangle(true, true, true, statuses);
    }

    /// Executes one HMMC transfer.
    fn execute_cpu_to_vram_high(
        &mut self,
        vram: &mut [u8],
        statuses: &mut [u8; V9938_STATUS_COUNT],
    ) {
        self.color_transfer_pending = false;
        let active = self.active.as_ref().unwrap();
        write_command_byte(
            vram,
            active.mode,
            active.destination_x,
            active.destination_y,
            active.argument & ARGUMENT_DESTINATION_EXPANDED != 0,
            self.registers[12],
        );
        if !self.advance_rectangle(false, true, true, statuses) {
            self.waiting_for_cpu = true;
            statuses[2] |= STATUS_TRANSFER_READY;
        }
    }

    /// Advances a rectangular command to its next coordinate.
    fn advance_rectangle(
        &mut self,
        source: bool,
        destination: bool,
        complete: bool,
        statuses: &mut [u8; V9938_STATUS_COUNT],
    ) -> bool {
        let active = self.active.as_mut().unwrap();
        active.remaining_x = active.remaining_x.saturating_sub(1);
        if source {
            active.source_x += direction_x(active.argument) * active.horizontal_step;
        }
        if destination {
            active.destination_x += direction_x(active.argument) * active.horizontal_step;
        }
        let format = pixel_format(active.mode);
        let source_out =
            source && !(0..i32::from(format.pixels_per_line)).contains(&active.source_x);
        let destination_out =
            destination && !(0..i32::from(format.pixels_per_line)).contains(&active.destination_x);
        if active.remaining_x == 0 || source_out || destination_out {
            active.remaining_y = active.remaining_y.saturating_sub(1);
            if active.remaining_y == 0 {
                if complete {
                    self.finish(statuses);
                }
                return true;
            }
            active.remaining_x = active.width;
            active.source_x = active.start_source_x;
            active.destination_x = active.start_destination_x;
            if source {
                active.source_y += direction_y(active.argument);
            }
            if destination {
                active.destination_y += direction_y(active.argument);
            }
            if (source && active.source_y < 0) || (destination && active.destination_y < 0) {
                if complete {
                    self.finish(statuses);
                }
                return true;
            }
            active.source_y &= MAXIMUM_Y;
            active.destination_y &= MAXIMUM_Y;
        }
        false
    }

    /// Returns the current command step cost.
    fn step_ticks(&self, display_active: bool, sprites_enabled: bool) -> u64 {
        let kind = self.active.as_ref().unwrap().kind;
        let timing = match kind {
            COMMAND_POINT => POINT_TIMING,
            COMMAND_PSET => PSET_TIMING,
            COMMAND_HMMV | COMMAND_HMMC => HIGH_FILL_TIMING,
            COMMAND_HMMM => HIGH_COPY_TIMING,
            COMMAND_YMMM => VERTICAL_COPY_TIMING,
            COMMAND_LMMV | COMMAND_LMCM | COMMAND_LMMC => LOGICAL_FILL_TIMING,
            COMMAND_LMMM => LOGICAL_COPY_TIMING,
            COMMAND_SRCH => SEARCH_TIMING,
            COMMAND_LINE => LINE_TIMING,
            _ => POINT_TIMING,
        };
        let state = if !display_active {
            0
        } else if sprites_enabled {
            2
        } else {
            1
        };
        timing[state]
    }

    /// Completes the current command and clears CE.
    fn finish(&mut self, statuses: &mut [u8; V9938_STATUS_COUNT]) {
        self.active = None;
        self.waiting_for_cpu = false;
        statuses[2] &= !STATUS_COMMAND_EXECUTE;
    }
}

/// Bitmap pixel format used by commands.
#[derive(Clone, Copy)]
struct PixelFormat {
    pixels_per_line: u16,
    pixels_per_byte_shift: u8,
    color_mask: u8,
}

/// Returns the pixel packing for one bitmap mode.
const fn pixel_format(mode: MsxVdpDisplayMode) -> PixelFormat {
    match mode {
        MsxVdpDisplayMode::Graphics4 => PixelFormat {
            pixels_per_line: 256,
            pixels_per_byte_shift: 1,
            color_mask: 0x0F,
        },
        MsxVdpDisplayMode::Graphics5 => PixelFormat {
            pixels_per_line: 512,
            pixels_per_byte_shift: 2,
            color_mask: 0x03,
        },
        MsxVdpDisplayMode::Graphics6 => PixelFormat {
            pixels_per_line: 512,
            pixels_per_byte_shift: 1,
            color_mask: 0x0F,
        },
        MsxVdpDisplayMode::Graphics7 => PixelFormat {
            pixels_per_line: 256,
            pixels_per_byte_shift: 0,
            color_mask: 0xFF,
        },
        _ => PixelFormat {
            pixels_per_line: 256,
            pixels_per_byte_shift: 0,
            color_mask: 0xFF,
        },
    }
}

/// Reads one little-endian command register pair.
fn register_word(registers: &[u8; 15], low: usize, high: usize) -> u16 {
    u16::from(registers[low]) | u16::from(registers[high]) << 8
}

/// Returns the signed X command direction.
const fn direction_x(argument: u8) -> i32 {
    if argument & ARGUMENT_DECREMENT_X != 0 {
        -1
    } else {
        1
    }
}

/// Returns the signed Y command direction.
const fn direction_y(argument: u8) -> i32 {
    if argument & ARGUMENT_DECREMENT_Y != 0 {
        -1
    } else {
        1
    }
}

/// Reads one command pixel.
pub(super) fn read_pixel(
    vram: &[u8],
    mode: MsxVdpDisplayMode,
    x: i32,
    y: i32,
    expanded: bool,
) -> u8 {
    let format = pixel_format(mode);
    if !(0..=MAXIMUM_Y).contains(&y) {
        return format.color_mask;
    }
    let address = pixel_address(mode, x as usize, y as usize, expanded);
    let value = vram.get(address).copied().unwrap_or(0xFF);
    let pixel = x as usize & (usize::from(1u8 << format.pixels_per_byte_shift) - 1);
    let shift = ((1usize << format.pixels_per_byte_shift) - 1 - pixel)
        * (8 >> format.pixels_per_byte_shift);
    (value >> shift) & format.color_mask
}

/// Writes one command pixel through a logical operation.
#[allow(clippy::too_many_arguments)]
fn write_pixel(
    vram: &mut [u8],
    mode: MsxVdpDisplayMode,
    x: i32,
    y: i32,
    expanded: bool,
    color: u8,
    operation: u8,
) {
    let format = pixel_format(mode);
    if !(0..=MAXIMUM_Y).contains(&y) {
        return;
    }
    let address = pixel_address(mode, x as usize, y as usize, expanded);
    let Some(value) = vram.get_mut(address) else {
        return;
    };
    let pixel = x as usize & (usize::from(1u8 << format.pixels_per_byte_shift) - 1);
    let shift = ((1usize << format.pixels_per_byte_shift) - 1 - pixel)
        * (8 >> format.pixels_per_byte_shift);
    let mask = format.color_mask << shift;
    let destination = (*value >> shift) & format.color_mask;
    let source = color & format.color_mask;
    let result = logical_operation(source, destination, format.color_mask, operation);
    *value = (*value & !mask) | (result << shift);
}

/// Applies one V9938 logical operation.
fn logical_operation(source: u8, destination: u8, mask: u8, operation: u8) -> u8 {
    if operation & 0x08 != 0 && source == 0 {
        return destination;
    }
    match operation & 0x07 {
        0 => source,
        1 => source & destination,
        2 => source | destination,
        3 => source ^ destination,
        4 => !source & mask,
        _ => destination,
    }
}

/// Returns one mode-specific pixel address.
fn pixel_address(mode: MsxVdpDisplayMode, x: usize, y: usize, expanded: bool) -> usize {
    let base = if expanded { EXPANDED_VRAM_OFFSET } else { 0 };
    base | match mode {
        MsxVdpDisplayMode::Graphics4 => (y & 1023) << 7 | (x & 255) >> 1,
        MsxVdpDisplayMode::Graphics5 => (y & 1023) << 7 | (x & 511) >> 2,
        MsxVdpDisplayMode::Graphics6 => (x & 2) << 15 | (y & 511) << 7 | (x & 511) >> 2,
        MsxVdpDisplayMode::Graphics7 => (x & 1) << 16 | (y & 511) << 7 | (x & 255) >> 1,
        _ => 0,
    }
}

/// Reads one high-speed command byte.
fn read_command_byte(vram: &[u8], mode: MsxVdpDisplayMode, x: i32, y: i32, expanded: bool) -> u8 {
    let format = pixel_format(mode);
    let byte_x = x >> format.pixels_per_byte_shift;
    if byte_x < 0 || y < 0 {
        return 0xFF;
    }
    let pixel_x = byte_x << format.pixels_per_byte_shift;
    let address = pixel_address(mode, pixel_x as usize, y as usize, expanded);
    vram.get(address).copied().unwrap_or(0xFF)
}

/// Writes one high-speed command byte.
fn write_command_byte(
    vram: &mut [u8],
    mode: MsxVdpDisplayMode,
    x: i32,
    y: i32,
    expanded: bool,
    value: u8,
) {
    let format = pixel_format(mode);
    let byte_x = x >> format.pixels_per_byte_shift;
    if byte_x < 0 || y < 0 {
        return;
    }
    let pixel_x = byte_x << format.pixels_per_byte_shift;
    let address = pixel_address(mode, pixel_x as usize, y as usize, expanded);
    if let Some(destination) = vram.get_mut(address) {
        *destination = value;
    }
}

/// Duplicates a color across one packed byte.
fn duplicate_color(mode: MsxVdpDisplayMode, color: u8) -> u8 {
    match mode {
        MsxVdpDisplayMode::Graphics4 | MsxVdpDisplayMode::Graphics6 => {
            let color = color & 0x0F;
            color | color << 4
        }
        MsxVdpDisplayMode::Graphics5 => {
            let color = color & 0x03;
            color | color << 2 | color << 4 | color << 6
        }
        MsxVdpDisplayMode::Graphics7 => color,
        _ => color,
    }
}

/// Stores the SRCH result in S#8 and S#9.
fn set_search_coordinate(statuses: &mut [u8; V9938_STATUS_COUNT], x: i32) {
    let x = x as u16;
    statuses[8] = x as u8;
    statuses[9] = 0xFE | ((x >> 8) as u8 & 1);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    /// YMMM ignores SX when copying from DX to the horizontal border.
    fn vertical_copy_uses_destination_x_for_source_clipping() {
        let mut engine = V9938CommandEngine::new();
        let mut statuses = [0; V9938_STATUS_COUNT];
        let mut vram = vec![0; 0x20000];
        let source_y = 278usize;
        let destination_y = 22usize;
        let first_byte = 194usize / 2;
        for byte in first_byte..128 {
            vram[source_y * 128 + byte] = byte as u8;
        }
        let registers = [224, 0, 22, 1, 194, 0, 22, 0, 0, 0, 1, 0, 0, 0, 0xE0];
        for (index, value) in registers.into_iter().enumerate() {
            engine.write_register(index, value, 0, MsxVdpDisplayMode::Graphics4, &mut statuses);
        }

        engine.advance_to(
            100_000,
            false,
            false,
            MsxVdpDisplayMode::Graphics4,
            &mut vram,
            &mut statuses,
        );

        assert_eq!(
            &vram[destination_y * 128 + first_byte..destination_y * 128 + 128],
            &vram[source_y * 128 + first_byte..source_y * 128 + 128]
        );
        assert_eq!(statuses[2] & STATUS_COMMAND_EXECUTE, 0);
    }
}
