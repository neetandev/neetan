//! FM-7 cassette (data recorder) glue.
//!
//! The FM-7 cassette interface is minimal: `0xFD00` bit 1 gates the tape motor,
//! `0xFD00` bit 0 drives the MIC (record) output, and `0xFD02` bit 7 samples the
//! EAR (playback) input. The same `0xFD00` write also carries the parallel
//! printer strobe (bit 6) and select (bit 7) lines, and the same `0xFD02` read
//! reports the printer status in bits 0-5. With no printer attached those status
//! lines idle high (ready). The deck plays a bit-per-sample waveform and is
//! advanced lazily against `current_cycle`, mirroring the X1.

use common::TraceSink;
use device::cassette::{CassetteError, load_cassette};

use super::Fm7Bus;

/// `0xFD00` write bit driving the cassette MIC (record) output.
const CASSETTE_MIC_BIT: u8 = 0x01;
/// `0xFD00` write bit gating the cassette motor.
const CASSETTE_MOTOR_BIT: u8 = 0x02;
/// `0xFD00` write bit pulsing the printer data strobe.
const PRINTER_STROBE_BIT: u8 = 0x40;
/// `0xFD00` write bit asserting the printer select line.
const PRINTER_SELECT_BIT: u8 = 0x80;
/// `0xFD02` read bit 7 carrying the cassette EAR (playback) level.
const CASSETTE_EAR_BIT: u8 = 0x80;
/// `0xFD02` read value with the EAR line low and the printer status lines idle:
/// bits 0-5 (BUSY, ERROR, ACK, PE, DET1, DET2) all read high with no printer
/// attached, so the byte reads `0x7F` until the EAR bit is overlaid.
const CASSETTE_PRINTER_IDLE: u8 = 0x7F;

impl<T: TraceSink> Fm7Bus<T> {
    /// Loads a cassette image (chosen by file extension) into the deck.
    pub fn insert_cassette(&mut self, extension: &str, image: &[u8]) -> Result<(), CassetteError> {
        let media = load_cassette(extension, image)?;
        self.cassette.insert_media(media);
        Ok(())
    }

    /// Loads a cassette image and records its configured source path.
    pub fn insert_cassette_from_path(
        &mut self,
        extension: &str,
        image: &[u8],
        path: &std::path::Path,
    ) -> Result<(), CassetteError> {
        let media = load_cassette(extension, image)?;
        self.cassette.insert_media_from_path(media, path);
        Ok(())
    }

    /// Removes the loaded cassette from the deck.
    pub fn eject_cassette(&mut self) {
        self.cassette.eject();
    }

    /// Advances the cassette waveform to the current cycle.
    pub(crate) fn advance_cassette(&mut self) {
        self.cassette
            .advance(self.current_cycle, self.clocks.main_clock_hz);
    }

    /// Handles a `0xFD00` write: cassette motor gating plus the accepted-but-idle
    /// MIC and printer strobe/select lines.
    ///
    /// Only the motor line has an effect. Tape recording (MIC out) and printer
    /// output (strobe/select/data) are intentionally not implemented; the deck
    /// plays back only, matching the other machines.
    pub(crate) fn write_system_port(&mut self, value: u8) {
        self.advance_cassette();
        self.cassette.set_motor(value & CASSETTE_MOTOR_BIT != 0);
        let _accepted_but_ignored =
            value & (CASSETTE_MIC_BIT | PRINTER_STROBE_BIT | PRINTER_SELECT_BIT);
    }

    /// Reads `0xFD02`: the cassette EAR level in bit 7 over the idle printer
    /// status in bits 0-5.
    pub(crate) fn read_cassette_printer_status(&mut self) -> u8 {
        self.advance_cassette();
        let mut value = CASSETTE_PRINTER_IDLE;
        if self.cassette.ear_level() {
            value |= CASSETTE_EAR_BIT;
        }
        value
    }
}
