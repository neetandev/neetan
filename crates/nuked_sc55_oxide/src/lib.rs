//! Nuked-SC55 emulation in pure Rust.
//!
//! Provides a render thread that accepts MIDI bytes and produces
//! resampled 48 kHz stereo f32 audio chunks for mixing into the
//! emulator's audio output.

#![forbid(unsafe_code)]

/// Adds save-state codecs to existing SC-55 implementation structures.
///
/// Listed fields are authoritative and retain their written order. The
/// `defaults` form omits immutable ROM allocations, creates temporary
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

mod context;
mod mcu;
mod mcu_interrupt;
mod mcu_opcodes;
mod mcu_timer;
mod pcm;
mod state;
mod submcu;
mod thread;

use state::Sc55State;
pub use thread::{Sc55Actor, Sc55ActorState, Sc55Error, Sc55WorkerState};
