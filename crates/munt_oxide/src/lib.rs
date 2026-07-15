//! Roland MT-32 emulation in pure Rust.
//!
//! Provides a render thread that accepts MIDI bytes and produces
//! resampled 48 kHz stereo f32 audio chunks for mixing into the
//! emulator's audio output.

#![forbid(unsafe_code)]

/// Adds save-state codecs to existing MT-32 implementation structures.
///
/// Listed fields are authoritative and retain their written order. The
/// `defaults` form omits immutable ROMs and lookup tables, creates temporary
/// placeholders while decoding, and requires restore preparation to reattach
/// the active resources before validation.
macro_rules! impl_state_codec {
    ($state:ty { $($field:ident),* $(,)? } defaults { $($default_field:ident: $default:expr),* $(,)? }) => {
        impl save_state::StateEncode for $state {
            fn encode_state(&self, output: &mut Vec<u8>) {
                $(save_state::StateEncode::encode_state(&self.$field, output);)*
            }
        }

        impl save_state::StateDecode for $state {
            fn decode_state(
                decoder: &mut save_state::StateDecoder<'_>,
            ) -> Result<Self, save_state::StateDecodeError> {
                Ok(Self {
                    $($field: save_state::StateDecode::decode_state(decoder)?,)*
                    $($default_field: $default,)*
                })
            }
        }
    };
    ($state:ty { $($field:ident),* $(,)? }) => {
        impl save_state::StateEncode for $state {
            fn encode_state(&self, output: &mut Vec<u8>) {
                $(save_state::StateEncode::encode_state(&self.$field, output);)*
            }
        }

        impl save_state::StateDecode for $state {
            fn decode_state(
                decoder: &mut save_state::StateDecoder<'_>,
            ) -> Result<Self, save_state::StateDecodeError> {
                Ok(Self {
                    $($field: save_state::StateDecode::decode_state(decoder)?,)*
                })
            }
        }
    };
}

pub(crate) use impl_state_codec;

mod analog;
mod blake3_digest;
mod breverb;
mod context;
mod enumerations;
mod la32_float_wave_generator;
mod la32_ramp;
mod memory_region;
mod midi_event_queue;
mod midi_stream_parser;
mod part;
mod partial;
mod partial_manager;
mod poly;
mod rom_info;
mod state;
mod structures;
mod synth;
mod tables;
mod thread;
mod tva;
mod tvf;
mod tvp;

pub use thread::{MuntActor, MuntActorState, MuntError, MuntWorkerState};
