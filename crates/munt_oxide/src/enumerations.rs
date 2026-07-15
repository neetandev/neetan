// Copyright (C) 2003, 2004, 2005, 2006, 2008, 2009 Dean Beeler, Jerome Fisher
// Copyright (C) 2011-2022 Dean Beeler, Jerome Fisher, Sergey V. Mikayev
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

/// Methods for emulating the connection between the LA32 and the DAC, which involves
/// some hacks in the real devices for doubling the volume.
/// See also http://en.wikipedia.org/wiki/Roland_MT-32#Digital_overflow
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub(crate) enum DacInputMode {
    /// Produces samples at double the volume, without tricks.
    /// Nicer overdrive characteristics than the DAC hacks (it simply clips samples within range)
    /// Higher quality than the real devices
    Nice = 0,
}

/// Methods for emulating the effective delay of incoming MIDI messages introduced by a MIDI interface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub(crate) enum MidiDelayMode {
    /// Process incoming MIDI events immediately.
    Immediate = 0,

    /// Delay incoming short MIDI messages as if they were transferred via a MIDI cable
    /// to a real hardware unit and immediate sysex processing.
    /// This ensures more accurate timing of simultaneous NoteOn messages.
    DelayShortMessagesOnly = 1,

    /// Delay all incoming MIDI events as if they were transferred via a MIDI cable
    /// to a real hardware unit.
    DelayAll = 2,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[repr(i32)]
/// Playback lifecycle of one allocated poly.
pub(crate) enum PolyState {
    Playing = 0,
    // This marks keys that have been released on the keyboard, but are being held by the pedal
    Held = 1,
    Releasing = 2,
    #[default]
    Inactive = 3,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
/// Selected MT-32 reverb algorithm.
pub(crate) enum ReverbMode {
    Room = 0,
    Hall = 1,
    Plate = 2,
    TapDelay = 3,
}

/// Adds stable save-state tags matching an existing MT-32 enum representation.
///
/// Use this only for fieldless implementation enums whose numeric values are
/// already defined by the synth core. Unknown values must remain invalid.
macro_rules! impl_enum_codec {
    ($state:ty { $($tag:literal => $variant:path),+ $(,)? }) => {
        impl save_state::StateEncode for $state {
            fn encode_state(&self, output: &mut Vec<u8>) {
                save_state::StateEncode::encode_state(&(*self as i32), output);
            }
        }

        impl save_state::StateDecode for $state {
            fn decode_state(
                decoder: &mut save_state::StateDecoder<'_>,
            ) -> Result<Self, save_state::StateDecodeError> {
                match <i32 as save_state::StateDecode>::decode_state(decoder)? {
                    $($tag => Ok($variant),)+
                    _ => Err(save_state::StateDecodeError::InvalidTag),
                }
            }
        }
    };
}

impl_enum_codec!(DacInputMode { 0 => DacInputMode::Nice });
impl_enum_codec!(MidiDelayMode {
    0 => MidiDelayMode::Immediate,
    1 => MidiDelayMode::DelayShortMessagesOnly,
    2 => MidiDelayMode::DelayAll,
});
impl_enum_codec!(PolyState {
    0 => PolyState::Playing,
    1 => PolyState::Held,
    2 => PolyState::Releasing,
    3 => PolyState::Inactive,
});
impl_enum_codec!(ReverbMode {
    0 => ReverbMode::Room,
    1 => ReverbMode::Hall,
    2 => ReverbMode::Plate,
    3 => ReverbMode::TapDelay,
});
