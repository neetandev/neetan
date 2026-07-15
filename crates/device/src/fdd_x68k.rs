//! X68000 floppy drive control block.
//!
//! Models the two drive-control registers that sit next to the uPD72065 in
//! the X68000 FDC window: the drive option control register (guest address
//! 0xE94005, write: eject and LED control per drive mask, read: media
//! status of the last targeted drive) and the drive select register
//! (0xE94007: motor, data rate, and unit select). Media presence changes
//! latch a status-change flag the machine turns into an IOC FDD interrupt.

/// Number of drive positions in the control block.
const DRIVE_COUNT: usize = 4;

/// Drive option control bit 7: blink the access LED (no-disk indicator).
const CONTROL_LED_BLINK: u8 = 0x80;

/// Drive option control bit 6: eject-prevent (locks the eject button).
const CONTROL_EJECT_PREVENT: u8 = 0x40;

/// Drive option control bit 5: eject the media.
const CONTROL_EJECT: u8 = 0x20;

/// Drive select bit 7: spindle motor on.
const SELECT_MOTOR_ON: u8 = 0x80;

/// Drive select bit 4: 2DD data rate (300 kbps) instead of 2HD (500 kbps).
const SELECT_DATA_RATE_2DD: u8 = 0x10;

/// Drive status read bit 7: media inserted.
const STATUS_INSERTED: u8 = 0x80;

/// Drive status read bit 6: media mis-inserted.
const STATUS_MIS_INSERTED: u8 = 0x40;

/// Effects of a drive option control register write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FddControlEffects {
    /// Drives whose media must be ejected by the machine.
    pub eject_request_mask: u8,
    /// The write clears the latched IOC FDD interrupt.
    pub clear_fdd_interrupt: bool,
}

/// Effects of a drive select register write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FddSelectEffects {
    /// The write clears the latched IOC FDD interrupt.
    pub clear_fdd_interrupt: bool,
    /// The motor line changed state with this write.
    pub motor_changed: bool,
}

save_state::runtime_state! {
/// Per-drive state of the X68000 drive control block.
#[derive(Debug, Clone, Copy, Default)]
struct FddDrive {
    /// Media is inserted.
    inserted: bool,
    /// Access LED blink mode (no-disk indicator).
    led_blink: bool,
    /// Eject button locked.
    eject_prevented: bool,
}}

save_state::runtime_state! {
/// X68000 floppy drive control block.
#[derive(Debug, Clone, Default)]
pub struct FddX68k {
    drives: [FddDrive; DRIVE_COUNT],
    /// Drive mask of the last option-control write (status read target).
    control_target_mask: u8,
    /// Spindle motor line shared by the drives.
    motor_on: bool,
    /// Data rate selection: true selects the 300 kbps 2DD rate.
    data_rate_2dd: bool,
    /// Unit selected by the drive select register.
    selected_unit: usize,
    /// A media insert or eject happened since the last acknowledge.
    status_changed: bool,
}}

impl FddX68k {
    /// Captures drive control, motor, and media-change latches.
    pub fn capture_state(&self) -> Self {
        self.clone()
    }

    /// Validates drive control state against retained mounted media.
    pub fn validate_state(
        &self,
        mounted: [bool; DRIVE_COUNT],
    ) -> Result<(), save_state::StateValidationError> {
        if self.selected_unit >= DRIVE_COUNT
            || self.control_target_mask & !0x0F != 0
            || self
                .drives
                .iter()
                .zip(mounted)
                .any(|(drive, present)| drive.inserted != present)
        {
            return Err(save_state::StateValidationError::new(
                "X68000 floppy drive state is invalid",
            ));
        }
        Ok(())
    }

    /// Restores drive control, motor, and media-change latches.
    pub fn restore_state(&mut self, state: Self) {
        *self = state;
    }
}

impl FddX68k {
    /// Creates a drive control block with no media and the motor off.
    pub fn new() -> Self {
        Self::default()
    }

    /// Resets the control block; inserted media stays inserted.
    pub fn reset(&mut self) {
        let drives = self.drives;
        *self = Self::default();
        for (index, drive) in drives.iter().enumerate() {
            self.drives[index].inserted = drive.inserted;
        }
    }

    /// Writes the drive option control register (guest 0xE94005).
    pub fn write_control(&mut self, value: u8) -> FddControlEffects {
        let mask = value & 0x0F;
        self.control_target_mask = mask;
        let mut eject_request_mask = 0u8;
        for (index, drive) in self.drives.iter_mut().enumerate() {
            if mask & (1 << index) == 0 {
                continue;
            }
            drive.led_blink = value & CONTROL_LED_BLINK != 0;
            drive.eject_prevented = value & CONTROL_EJECT_PREVENT != 0;
            // The eject fires only when this write asserts EJT with PRV clear.
            let eject = value & (CONTROL_EJECT | CONTROL_EJECT_PREVENT) == CONTROL_EJECT;
            if eject && drive.inserted {
                eject_request_mask |= 1 << index;
            }
        }
        FddControlEffects {
            eject_request_mask,
            clear_fdd_interrupt: true,
        }
    }

    /// Reads the drive status register (guest 0xE94005).
    pub fn read_status(&self) -> u8 {
        let target = (0..DRIVE_COUNT).find(|&index| self.control_target_mask & (1 << index) != 0);
        let Some(index) = target else {
            return 0;
        };
        let mut status = 0;
        if self.drives[index].inserted {
            status |= STATUS_INSERTED;
        }
        status & !STATUS_MIS_INSERTED
    }

    /// Writes the drive select register (guest 0xE94007).
    pub fn write_select(&mut self, value: u8) -> FddSelectEffects {
        let motor = value & SELECT_MOTOR_ON != 0;
        let motor_changed = motor != self.motor_on;
        self.motor_on = motor;
        self.data_rate_2dd = value & SELECT_DATA_RATE_2DD != 0;
        self.selected_unit = usize::from(value & 0x03);
        FddSelectEffects {
            clear_fdd_interrupt: true,
            motor_changed,
        }
    }

    /// Updates media presence; a change latches the status-change flag.
    pub fn set_inserted(&mut self, drive: usize, inserted: bool) {
        if drive >= DRIVE_COUNT {
            return;
        }
        if self.drives[drive].inserted != inserted {
            self.drives[drive].inserted = inserted;
            self.status_changed = true;
        }
    }

    /// Returns whether media is inserted in `drive`.
    pub fn inserted(&self, drive: usize) -> bool {
        drive < DRIVE_COUNT && self.drives[drive].inserted
    }

    /// Returns whether the eject button of `drive` is locked.
    pub fn eject_prevented(&self, drive: usize) -> bool {
        drive < DRIVE_COUNT && self.drives[drive].eject_prevented
    }

    /// Returns whether the access LED of `drive` blinks.
    pub fn led_blink(&self, drive: usize) -> bool {
        drive < DRIVE_COUNT && self.drives[drive].led_blink
    }

    /// Returns whether the spindle motor is on.
    pub fn motor_on(&self) -> bool {
        self.motor_on
    }

    /// Returns the unit selected by the drive select register.
    pub fn selected_drive(&self) -> usize {
        self.selected_unit
    }

    /// Returns the selected media data rate in bits per second.
    pub fn data_rate_hz(&self) -> u32 {
        if self.data_rate_2dd { 300_000 } else { 500_000 }
    }

    /// Returns the ready mask: drives with media inserted while the motor runs.
    pub fn ready_mask(&self) -> u8 {
        if !self.motor_on {
            return 0;
        }
        let mut mask = 0;
        for (index, drive) in self.drives.iter().enumerate() {
            if drive.inserted {
                mask |= 1 << index;
            }
        }
        mask
    }

    /// Consumes the latched media status-change flag (IOC FDD edge).
    pub fn take_status_changed(&mut self) -> bool {
        std::mem::replace(&mut self.status_changed, false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_write_updates_led_and_prevent_state_per_mask() {
        let mut fdd = FddX68k::new();
        let effects = fdd.write_control(0x03);
        assert!(effects.clear_fdd_interrupt);
        assert_eq!(effects.eject_request_mask, 0);
        let effects = fdd.write_control(0xC3);
        assert!(effects.clear_fdd_interrupt);
        assert!(fdd.led_blink(0));
        assert!(fdd.led_blink(1));
        assert!(!fdd.led_blink(2));
        assert!(fdd.eject_prevented(0));

        let effects = fdd.write_control(0x01);
        assert!(effects.clear_fdd_interrupt);
        fdd.write_control(0x00);
        assert!(!fdd.led_blink(0), "cleared for masked drive");
        assert!(fdd.led_blink(1), "unmasked drive keeps its state");
        assert!(!fdd.eject_prevented(0));
    }

    #[test]
    fn eject_requests_only_inserted_unlocked_drives() {
        let mut fdd = FddX68k::new();
        fdd.set_inserted(0, true);
        fdd.set_inserted(1, true);
        fdd.take_status_changed();

        // Drive 2 is empty, so only drives 0 and 1 report eject requests.
        fdd.write_control(0x07);
        let effects = fdd.write_control(CONTROL_EJECT | 0x07);
        assert_eq!(effects.eject_request_mask, 0x03);

        let mut fdd = FddX68k::new();
        fdd.set_inserted(0, true);
        fdd.write_control(0x01);
        let effects = fdd.write_control(CONTROL_EJECT | CONTROL_EJECT_PREVENT | 0x01);
        assert_eq!(
            effects.eject_request_mask, 0,
            "eject-prevent in the same write blocks the eject request"
        );
        fdd.write_control(0x01);
        let effects = fdd.write_control(CONTROL_EJECT | 0x01);
        assert_eq!(
            effects.eject_request_mask, 0x01,
            "a later write without eject-prevent ejects"
        );
    }

    #[test]
    fn eject_targets_the_drive_selected_by_the_same_write() {
        let mut fdd = FddX68k::new();
        fdd.set_inserted(0, true);
        fdd.take_status_changed();

        let triggered = fdd.write_control(CONTROL_EJECT | 0x01);
        assert_eq!(triggered.eject_request_mask, 0x01);
        let no_target = fdd.write_control(CONTROL_EJECT);
        assert_eq!(no_target.eject_request_mask, 0);
    }

    #[test]
    fn status_read_reports_the_control_target() {
        let mut fdd = FddX68k::new();
        fdd.set_inserted(1, true);

        fdd.write_control(0x01);
        assert_eq!(fdd.read_status(), 0x00);
        fdd.write_control(0x02);
        assert_eq!(fdd.read_status(), STATUS_INSERTED);

        fdd.write_control(0x00);
        assert_eq!(fdd.read_status(), 0x00, "no target selected");
    }

    #[test]
    fn select_write_sets_motor_rate_and_unit() {
        let mut fdd = FddX68k::new();
        let effects = fdd.write_select(SELECT_MOTOR_ON | SELECT_DATA_RATE_2DD | 0x02);
        assert!(effects.clear_fdd_interrupt);
        assert!(effects.motor_changed);
        assert!(fdd.motor_on());
        assert_eq!(fdd.data_rate_hz(), 300_000);
        assert_eq!(fdd.selected_drive(), 2);

        let effects = fdd.write_select(SELECT_MOTOR_ON | 0x01);
        assert!(!effects.motor_changed);
        assert_eq!(fdd.data_rate_hz(), 500_000);
        assert_eq!(fdd.selected_drive(), 1);

        let effects = fdd.write_select(0x00);
        assert!(effects.motor_changed);
        assert!(!fdd.motor_on());
    }

    #[test]
    fn insert_and_eject_latch_the_status_change() {
        let mut fdd = FddX68k::new();
        assert!(!fdd.take_status_changed());

        fdd.set_inserted(0, true);
        assert!(fdd.take_status_changed());
        assert!(!fdd.take_status_changed(), "flag is consumed");

        fdd.set_inserted(0, true);
        assert!(!fdd.take_status_changed(), "no change, no latch");

        fdd.set_inserted(0, false);
        assert!(fdd.take_status_changed());
    }

    #[test]
    fn ready_mask_requires_motor_and_media() {
        let mut fdd = FddX68k::new();
        fdd.set_inserted(0, true);
        fdd.set_inserted(1, true);
        assert_eq!(fdd.ready_mask(), 0, "motor off");

        fdd.write_select(SELECT_MOTOR_ON);
        assert_eq!(fdd.ready_mask(), 0x03);

        fdd.set_inserted(1, false);
        assert_eq!(fdd.ready_mask(), 0x01);
    }

    #[test]
    fn reset_keeps_media_but_clears_control_state() {
        let mut fdd = FddX68k::new();
        fdd.set_inserted(0, true);
        fdd.write_select(SELECT_MOTOR_ON | SELECT_DATA_RATE_2DD | 0x03);
        fdd.write_control(CONTROL_LED_BLINK | 0x01);

        fdd.reset();
        assert!(fdd.inserted(0));
        assert!(!fdd.motor_on());
        assert!(!fdd.led_blink(0));
        assert_eq!(fdd.data_rate_hz(), 500_000);
        assert_eq!(fdd.selected_drive(), 0);
    }
}
