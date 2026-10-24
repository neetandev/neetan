//! Catches a panic of emulator code and turns it into a message.

use std::{
    any::Any,
    panic::{AssertUnwindSafe, catch_unwind},
};

/// The message of a panic payload that carries no text.
const UNKNOWN_PANIC_MESSAGE: &str = "unknown panic payload";

/// Runs `call` and returns its result, or the message of the panic it raised.
/// The default panic hook still prints the panic.
///
/// `call` runs under `AssertUnwindSafe`. A caller that gets the message treats
/// the state that `call` touched as broken. It ends the session and only
/// flushes the disk images of the machine, each flush under its own guard.
pub(crate) fn catch_panic<R>(call: impl FnOnce() -> R) -> Result<R, String> {
    catch_unwind(AssertUnwindSafe(call)).map_err(|payload| panic_message(payload.as_ref()))
}

/// The text of a panic payload.
fn panic_message(payload: &(dyn Any + Send)) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).to_owned()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        UNKNOWN_PANIC_MESSAGE.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::{UNKNOWN_PANIC_MESSAGE, catch_panic};

    #[test]
    fn returns_the_result_without_a_panic() {
        assert_eq!(catch_panic(|| 7), Ok(7));
    }

    #[test]
    fn returns_the_message_of_a_static_panic() {
        let result: Result<(), String> = catch_panic(|| panic!("core halted"));
        assert_eq!(result, Err("core halted".to_owned()));
    }

    #[test]
    fn returns_the_message_of_a_formatted_panic() {
        let address = 0x1234;
        let result: Result<(), String> = catch_panic(|| panic!("bad address {address:#06X}"));
        assert_eq!(result, Err("bad address 0x1234".to_owned()));
    }

    #[test]
    fn names_a_payload_without_text() {
        let result: Result<(), String> = catch_panic(|| std::panic::panic_any(42_u32));
        assert_eq!(result, Err(UNKNOWN_PANIC_MESSAGE.to_owned()));
    }
}
