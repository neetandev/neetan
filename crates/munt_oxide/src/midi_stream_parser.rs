// Copyright (C) 2003, 2004, 2005, 2006, 2008, 2009 Dean Beeler, Jerome Fisher
// Copyright (C) 2011-2026 Dean Beeler, Jerome Fisher, Sergey V. Mikayev
//
//  This program is free software: you can redistribute it and/or modify
//  it under the terms of the GNU Lesser General Public License as published by
//  the Free Software Foundation, either version 2.1 of the License, or
//  (at your option) any later version.
//
//  This program is distributed in the hope that it will be useful,
//  but WITHOUT ANY WARRANTY; without even the implied warranty of
//  MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
//  GNU Lesser General Public License for more details.
//
//  You should have received a copy of the GNU Lesser General Public License
//  along with this program.  If not, see <http://www.gnu.org/licenses/>.

#[cfg(test)]
use crate::state::MAX_STREAM_BUFFER_SIZE;
use crate::state::MidiStreamParserState;

/// Returns the expected length of a short MIDI message given its status byte.
pub(crate) fn get_short_message_length(msg: u32) -> u32 {
    if (msg & 0xF0) == 0xF0 {
        match msg & 0xFF {
            0xF1 | 0xF3 => 2,
            0xF2 => 3,
            _ => 1,
        }
    } else {
        // NOTE: This calculation isn't quite correct
        // as it doesn't consider the running status byte
        if (msg & 0xE0) == 0xC0 { 2 } else { 3 }
    }
}

impl MidiStreamParserState {
    /// Parses a block of raw MIDI bytes. All the parsed MIDI messages are sent in sequence to the
    /// user-supplied callbacks for further processing.
    /// SysEx messages are allowed to be fragmented across several calls to this method. Running
    /// status is also handled for short messages.
    /// NOTE: the total length of a SysEx message being fragmented shall not exceed
    /// MAX_STREAM_BUFFER_SIZE (32768 bytes).
    pub(crate) fn parse_stream(
        &mut self,
        stream: &[u8],
        on_short_message: &mut dyn FnMut(u32),
        on_sysex: &mut dyn FnMut(&[u8]),
        on_system_realtime: &mut dyn FnMut(u8),
    ) {
        let mut offset: usize = 0;
        let mut length = stream.len() as u32;
        while length > 0 {
            let parsed_message_length;
            if 0xF8 <= stream[offset] {
                // Process System Realtime immediately and go on
                on_system_realtime(stream[offset]);
                parsed_message_length = 1;
                // No effect on the running status
            } else if self.stream_buffer_size > 0 {
                // Check if there is something in streamBuffer waiting for being processed
                if self.stream_buffer[0] == 0xF0 {
                    parsed_message_length = self.parse_sysex_fragment(
                        &stream[offset..],
                        length,
                        on_sysex,
                        on_system_realtime,
                    );
                } else {
                    parsed_message_length = self.parse_short_message_data_bytes(
                        &stream[offset..],
                        length,
                        on_short_message,
                        on_system_realtime,
                    );
                }
            } else if stream[offset] == 0xF0 {
                self.running_status = 0; // SysEx clears the running status
                parsed_message_length = self.parse_sysex(&stream[offset..], length, on_sysex);
            } else {
                parsed_message_length = self.parse_short_message_status(&stream[offset..]);
            }

            // Parsed successfully
            offset += parsed_message_length as usize;
            length -= parsed_message_length;
        }
    }

    // We deal with SysEx messages below 512 bytes long in most cases. Nevertheless, it seems
    // reasonable to support a possibility to load bulk dumps using a single message. However,
    // this is known to fail with a real device due to limited input buffer size.
    fn check_stream_buffer_capacity(&mut self, preserve_content: bool) -> bool {
        if (self.stream_buffer_size as usize) < self.stream_buffer.len() {
            return true;
        }
        let _ = preserve_content;
        false
    }

    // Checks input byte whether it is a status byte. If not, replaces it with running status
    // when available. Returns true if the input byte was changed to running status.
    fn process_status_byte(&mut self, status: &mut u8) -> bool {
        if *status < 0x80 {
            // First byte isn't status, try running status
            if self.running_status < 0x80 {
                // No running status available yet
                return false;
            }
            *status = self.running_status;
            return true;
        } else if *status < 0xF0 {
            // Store current status as running for a Voice message
            self.running_status = *status;
        } else if *status < 0xF8 {
            // System Common clears running status
            self.running_status = 0;
        } // System Realtime doesn't affect running status
        false
    }

    fn parse_short_message_status(&mut self, stream: &[u8]) -> u32 {
        let mut status = stream[0];
        let parsed_length = if self.process_status_byte(&mut status) {
            0
        } else {
            1
        };
        if 0x80 <= status {
            // If no running status available yet, skip one byte
            self.stream_buffer[0] = status;
            self.stream_buffer_size += 1;
        }
        parsed_length
    }

    fn parse_short_message_data_bytes(
        &mut self,
        stream: &[u8],
        length: u32,
        on_short_message: &mut dyn FnMut(u32),
        on_system_realtime: &mut dyn FnMut(u8),
    ) -> u32 {
        let short_message_length = get_short_message_length(self.stream_buffer[0] as u32);
        let mut parsed_length: u32 = 0;
        let mut remaining = length;
        let mut stream_offset: usize = 0;

        // Append incoming bytes to streamBuffer
        while (self.stream_buffer_size < short_message_length) && (remaining > 0) {
            remaining -= 1;
            let data_byte = stream[stream_offset];
            stream_offset += 1;
            if data_byte < 0x80 {
                // Add data byte to streamBuffer
                self.stream_buffer[self.stream_buffer_size as usize] = data_byte;
                self.stream_buffer_size += 1;
            } else if data_byte < 0xF8 {
                // Discard invalid bytes and start over
                self.stream_buffer_size = 0; // Clear streamBuffer
                return parsed_length;
            } else {
                // Bypass System Realtime message
                on_system_realtime(data_byte);
            }
            parsed_length += 1;
        }
        if self.stream_buffer_size < short_message_length {
            return parsed_length; // Still lacks data bytes
        }

        // Assemble short message
        let mut short_message = self.stream_buffer[0] as u32;
        for i in 1..short_message_length {
            short_message |= (self.stream_buffer[i as usize] as u32) << (i << 3);
        }
        on_short_message(short_message);
        self.stream_buffer_size = 0; // Clear streamBuffer
        parsed_length
    }

    fn parse_sysex(&mut self, stream: &[u8], length: u32, on_sysex: &mut dyn FnMut(&[u8])) -> u32 {
        // Find SysEx length
        let mut sysex_length: u32 = 1;
        while sysex_length < length {
            let next_byte = stream[sysex_length as usize];
            sysex_length += 1;
            if 0x80 <= next_byte {
                if next_byte == 0xF7 {
                    // End of SysEx
                    on_sysex(&stream[..sysex_length as usize]);
                    return sysex_length;
                }
                if 0xF8 <= next_byte {
                    // The System Realtime message must be processed right after return
                    // but the SysEx is actually fragmented and to be reconstructed in
                    // streamBuffer
                    sysex_length -= 1;
                    break;
                }
                // Illegal status byte in SysEx message, aborting
                // Continue parsing from that point
                return sysex_length - 1;
            }
        }

        // Store incomplete SysEx message for further processing
        self.stream_buffer_size = sysex_length;
        if self.check_stream_buffer_capacity(false) {
            self.stream_buffer[..sysex_length as usize]
                .copy_from_slice(&stream[..sysex_length as usize]);
        } else {
            // Not enough buffer capacity, don't care about the real buffer content, just mark
            // the first byte
            self.stream_buffer[0] = stream[0];
            self.stream_buffer_size = self.stream_buffer.len() as u32;
        }
        sysex_length
    }

    fn parse_sysex_fragment(
        &mut self,
        stream: &[u8],
        length: u32,
        on_sysex: &mut dyn FnMut(&[u8]),
        on_system_realtime: &mut dyn FnMut(u8),
    ) -> u32 {
        let mut parsed_length: u32 = 0;
        while parsed_length < length {
            let next_byte = stream[parsed_length as usize];
            parsed_length += 1;
            if next_byte < 0x80 {
                // Add SysEx data byte to streamBuffer
                if self.check_stream_buffer_capacity(true) {
                    self.stream_buffer[self.stream_buffer_size as usize] = next_byte;
                    self.stream_buffer_size += 1;
                }
                continue;
            }
            if 0xF8 <= next_byte {
                // Bypass System Realtime message
                on_system_realtime(next_byte);
                continue;
            }
            if next_byte != 0xF7 {
                // Illegal status byte in SysEx message, aborting
                // Clear streamBuffer and continue parsing from that point
                self.stream_buffer_size = 0;
                parsed_length -= 1;
                break;
            }
            // End of SysEx
            if self.check_stream_buffer_capacity(true) {
                self.stream_buffer[self.stream_buffer_size as usize] = next_byte;
                self.stream_buffer_size += 1;
                on_sysex(&self.stream_buffer[..self.stream_buffer_size as usize]);
                self.stream_buffer_size = 0; // Clear streamBuffer
                break;
            }
            // Encountered streamBuffer overrun
            self.stream_buffer_size = 0; // Clear streamBuffer
            break;
        }
        parsed_length
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct ParseResult {
        short_messages: Vec<u32>,
        sysex_messages: Vec<Vec<u8>>,
        realtime_messages: Vec<u8>,
    }

    fn parse_rust(stream: &[u8]) -> ParseResult {
        let mut state = MidiStreamParserState {
            running_status: 0,
            stream_buffer: vec![0u8; MAX_STREAM_BUFFER_SIZE],
            stream_buffer_size: 0,
        };
        let mut short_messages = Vec::new();
        let mut sysex_messages = Vec::new();
        let mut realtime_messages = Vec::new();
        state.parse_stream(
            stream,
            &mut |msg| short_messages.push(msg),
            &mut |data| sysex_messages.push(data.to_vec()),
            &mut |rt| realtime_messages.push(rt),
        );
        ParseResult {
            short_messages,
            sysex_messages,
            realtime_messages,
        }
    }

    fn parse_rust_fragmented(stream1: &[u8], stream2: &[u8]) -> ParseResult {
        let mut state = MidiStreamParserState {
            running_status: 0,
            stream_buffer: vec![0u8; MAX_STREAM_BUFFER_SIZE],
            stream_buffer_size: 0,
        };
        let mut short_messages = Vec::new();
        let mut sysex_messages = Vec::new();
        let mut realtime_messages = Vec::new();
        state.parse_stream(
            stream1,
            &mut |msg| short_messages.push(msg),
            &mut |data| sysex_messages.push(data.to_vec()),
            &mut |rt| realtime_messages.push(rt),
        );
        state.parse_stream(
            stream2,
            &mut |msg| short_messages.push(msg),
            &mut |data| sysex_messages.push(data.to_vec()),
            &mut |rt| realtime_messages.push(rt),
        );
        ParseResult {
            short_messages,
            sysex_messages,
            realtime_messages,
        }
    }

    #[test]
    fn midi_note_on() {
        let r = parse_rust(&[0x90, 0x3C, 0x7F]);
        assert_eq!(r.short_messages, [0x007F3C90]);
    }

    #[test]
    fn midi_running_status() {
        let r = parse_rust(&[0x90, 0x3C, 0x7F, 0x3C, 0x00]);
        assert_eq!(r.short_messages, [0x007F3C90, 0x00003C90]);
    }

    #[test]
    fn midi_program_change() {
        let r = parse_rust(&[0xC0, 0x05]);
        assert_eq!(r.short_messages, [0x000005C0]);
    }

    #[test]
    fn midi_pitch_bend() {
        let r = parse_rust(&[0xE0, 0x00, 0x40]);
        assert_eq!(r.short_messages, [0x004000E0]);
    }

    #[test]
    fn midi_complete_sysex() {
        let r = parse_rust(&[0xF0, 0x41, 0x10, 0x16, 0x12, 0x08, 0x00, 0x00, 0xF7]);
        assert_eq!(r.short_messages, Vec::<u32>::new());
        assert_eq!(
            r.sysex_messages,
            [vec![0xF0, 0x41, 0x10, 0x16, 0x12, 0x08, 0x00, 0x00, 0xF7]]
        );
    }

    #[test]
    fn midi_fragmented_sysex() {
        let r = parse_rust_fragmented(&[0xF0, 0x41, 0x10, 0x16], &[0x12, 0x08, 0x00, 0x00, 0xF7]);
        assert_eq!(r.short_messages, Vec::<u32>::new());
        assert_eq!(
            r.sysex_messages,
            [vec![0xF0, 0x41, 0x10, 0x16, 0x12, 0x08, 0x00, 0x00, 0xF7]]
        );
    }

    #[test]
    fn midi_realtime_in_voice_message() {
        let r = parse_rust(&[0x90, 0xFE, 0x3C, 0x7F]);
        assert_eq!(r.short_messages, [0x007F3C90]);
        assert_eq!(r.realtime_messages, [0xFE]);
    }

    #[test]
    fn midi_realtime_in_sysex() {
        let r = parse_rust(&[0xF0, 0x41, 0xF8, 0x10, 0xF7]);
        assert_eq!(r.sysex_messages, [vec![0xF0, 0x41, 0x10, 0xF7]]);
        assert_eq!(r.realtime_messages, [0xF8]);
    }

    #[test]
    fn midi_no_running_status_bare_data() {
        let r = parse_rust(&[0x3C, 0x7F]);
        assert_eq!(r.short_messages, Vec::<u32>::new());
    }

    #[test]
    fn midi_multiple_message_types() {
        let r = parse_rust(&[
            0x90, 0x3C, 0x7F, // note on
            0xC0, 0x05, // program change
            0xB0, 0x07, 0x64, // control change
            0xE0, 0x00, 0x40, // pitch bend
            0xF0, 0x41, 0x10, 0xF7, // short sysex
            0x90, 0x40, 0x50, // note on (new status)
        ]);
        assert_eq!(
            r.short_messages,
            [0x007F3C90, 0x000005C0, 0x006407B0, 0x004000E0, 0x00504090]
        );
        assert_eq!(r.sysex_messages, [vec![0xF0, 0x41, 0x10, 0xF7]]);
    }

    #[test]
    fn midi_system_common_clears_running_status() {
        // 0xF1 is system common (MTC quarter frame), clears running status.
        // After that, bare data bytes 0x3C, 0x7F are ignored (no running status).
        let r = parse_rust(&[0x90, 0x3C, 0x7F, 0xF1, 0x00, 0x3C, 0x7F]);
        assert_eq!(r.short_messages, [0x007F3C90, 0x000000F1]);
    }

    #[test]
    fn midi_get_short_message_length_matches() {
        for status in 0x80u32..=0xFF {
            let rust_len = get_short_message_length(status);
            let expected = if (status & 0xF0) == 0xF0 {
                match status & 0xFF {
                    0xF1 | 0xF3 => 2,
                    0xF2 => 3,
                    _ => 1,
                }
            } else if (status & 0xE0) == 0xC0 {
                2
            } else {
                3
            };
            assert_eq!(
                rust_len, expected,
                "short message length mismatch for status 0x{status:02X}"
            );
        }
    }
}
