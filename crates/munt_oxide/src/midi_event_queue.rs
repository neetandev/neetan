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

// Simple queue implementation using a ring buffer to store incoming MIDI event before the synth
// actually processes it.
// It is intended to:
// - get rid of prerenderer while retaining graceful partial abortion
// - add fair emulation of the MIDI interface delays
// - extend the synth interface with the default implementation of a typical rendering loop.

use crate::state::{MAX_STREAM_BUFFER_SIZE, MidiEvent, MidiEventQueueState};

impl MidiEventQueueState {
    /// Must be called once after creating MidiEventQueueState.
    /// `ring_buffer_size` must be a power of 2.
    pub(crate) fn init(&mut self, ring_buffer_size: u32) {
        self.ring_buffer_mask = ring_buffer_size - 1;
        self.ring_buffer = vec![MidiEvent::default(); ring_buffer_size as usize];
        self.sysex_buffer = vec![0; MAX_STREAM_BUFFER_SIZE];
        self.reset();
    }

    pub(crate) fn reset(&mut self) {
        self.start_position = 0;
        self.end_position = 0;
        self.sysex_write_position = 0;
        self.sysex_used = 0;
    }

    pub(crate) fn push_short_message(&mut self, short_message_data: u32, timestamp: u32) -> bool {
        let new_end_position = (self.end_position + 1) & self.ring_buffer_mask;
        if self.start_position == new_end_position {
            return false;
        }
        let new_event = &mut self.ring_buffer[self.end_position as usize];
        new_event.is_sysex = false;
        new_event.sysex_offset = 0;
        new_event.sysex_length = 0;
        new_event.short_message_data = short_message_data;
        new_event.timestamp = timestamp;
        self.end_position = new_end_position;
        true
    }

    pub(crate) fn push_sysex(&mut self, sysex_data: &[u8], timestamp: u32) -> bool {
        let new_end_position = (self.end_position + 1) & self.ring_buffer_mask;
        if self.start_position == new_end_position {
            return false;
        }
        if sysex_data.len() > self.sysex_buffer.len() - self.sysex_used as usize {
            return false;
        }
        let sysex_offset = self.sysex_write_position as usize;
        let first_length = sysex_data.len().min(self.sysex_buffer.len() - sysex_offset);
        self.sysex_buffer[sysex_offset..sysex_offset + first_length]
            .copy_from_slice(&sysex_data[..first_length]);
        let remaining = sysex_data.len() - first_length;
        self.sysex_buffer[..remaining].copy_from_slice(&sysex_data[first_length..]);
        let new_event = &mut self.ring_buffer[self.end_position as usize];
        new_event.is_sysex = true;
        new_event.sysex_offset = self.sysex_write_position;
        new_event.sysex_length = sysex_data.len() as u32;
        new_event.short_message_data = sysex_data.len() as u32;
        new_event.timestamp = timestamp;
        self.sysex_write_position =
            (self.sysex_write_position + sysex_data.len() as u32) % self.sysex_buffer.len() as u32;
        self.sysex_used += sysex_data.len() as u32;
        self.end_position = new_end_position;
        true
    }

    pub(crate) fn copy_front_sysex(&self, target: &mut [u8]) -> Option<usize> {
        let event = self.peek()?;
        if !event.is_sysex || target.len() < event.sysex_length as usize {
            return None;
        }
        let sysex_offset = event.sysex_offset as usize;
        let sysex_length = event.sysex_length as usize;
        let first_length = sysex_length.min(self.sysex_buffer.len() - sysex_offset);
        target[..first_length]
            .copy_from_slice(&self.sysex_buffer[sysex_offset..sysex_offset + first_length]);
        let remaining = sysex_length - first_length;
        target[first_length..sysex_length].copy_from_slice(&self.sysex_buffer[..remaining]);
        Some(sysex_length)
    }

    pub(crate) fn peek(&self) -> Option<&MidiEvent> {
        if self.is_empty() {
            None
        } else {
            Some(&self.ring_buffer[self.start_position as usize])
        }
    }

    pub(crate) fn drop_front(&mut self) {
        if self.is_empty() {
            return;
        }
        let event = &self.ring_buffer[self.start_position as usize];
        if event.is_sysex {
            self.sysex_used -= event.sysex_length;
        }
        self.start_position = (self.start_position + 1) & self.ring_buffer_mask;
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.start_position == self.end_position
    }
}

#[cfg(test)]
mod tests {
    use crate::state::MidiEventQueueState;

    fn make_queue(size: u32) -> MidiEventQueueState {
        let mut state = MidiEventQueueState {
            ring_buffer: Vec::new(),
            ring_buffer_mask: 0,
            start_position: 0,
            end_position: 0,
            sysex_buffer: Vec::new(),
            sysex_write_position: 0,
            sysex_used: 0,
        };
        state.init(size);
        state
    }

    #[test]
    fn empty_on_creation() {
        let state = make_queue(8);
        assert!(state.is_empty());
        assert!(state.peek().is_none());
    }

    #[test]
    fn push_peek_drop_fifo() {
        let mut state = make_queue(8);
        assert!(state.push_short_message(0x007F3C90, 100));
        assert!(state.push_short_message(0x00003C90, 200));
        assert!(!state.is_empty());

        let event = state.peek().unwrap();
        assert_eq!(event.short_message_data, 0x007F3C90);
        assert_eq!(event.timestamp, 100);
        assert!(!event.is_sysex);

        state.drop_front();

        let event = state.peek().unwrap();
        assert_eq!(event.short_message_data, 0x00003C90);
        assert_eq!(event.timestamp, 200);

        state.drop_front();
        assert!(state.is_empty());
    }

    #[test]
    fn sysex_data_preserved() {
        let mut state = make_queue(8);
        let sysex = vec![0xF0, 0x41, 0x10, 0x16, 0x12, 0xF7];
        assert!(state.push_sysex(&sysex, 300));

        let event = state.peek().unwrap();
        assert!(event.is_sysex);
        let mut copied = vec![0; sysex.len()];
        assert_eq!(state.copy_front_sysex(&mut copied), Some(sysex.len()));
        assert_eq!(copied, sysex);
        assert_eq!(event.short_message_data, sysex.len() as u32);
        assert_eq!(event.timestamp, 300);

        state.drop_front();
        assert!(state.is_empty());
    }

    #[test]
    fn full_queue_rejects() {
        let mut state = make_queue(4); // capacity 4, usable slots = 3
        assert!(state.push_short_message(1, 0));
        assert!(state.push_short_message(2, 0));
        assert!(state.push_short_message(3, 0));
        assert!(!state.push_short_message(4, 0)); // full
    }

    #[test]
    fn wrap_around() {
        let mut state = make_queue(4);
        for round in 0..3u32 {
            for i in 0..3u32 {
                assert!(state.push_short_message(round * 10 + i, 0));
            }
            for i in 0..3u32 {
                let event = state.peek().unwrap();
                assert_eq!(event.short_message_data, round * 10 + i);
                state.drop_front();
            }
            assert!(state.is_empty());
        }
    }

    #[test]
    fn reset_clears() {
        let mut state = make_queue(8);
        state.push_short_message(1, 0);
        state.push_short_message(2, 0);
        state.reset();
        assert!(state.is_empty());
    }

    #[test]
    fn drop_on_empty_is_noop() {
        let mut state = make_queue(8);
        state.drop_front(); // should not panic
        assert!(state.is_empty());
    }
}
