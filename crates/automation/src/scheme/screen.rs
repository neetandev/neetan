//! Screen read, hash, screenshot, and comparison natives.

use std::{cell::RefCell, rc::Rc};

use r7rs::{Engine, Error, LibraryName, NativeContext, Value};

use super::support::{machine_id, make_alist, make_list, op_error_value, to_count, to_u32};
use crate::session::AutomationSession;

/// Builds the artifact alist describing a written or recorded artifact path.
fn artifact_alist(
    context: &mut NativeContext,
    path: &str,
    bytes: Option<usize>,
) -> Result<Value, Error> {
    let path_value = context.string_utf8(path.to_owned())?;
    let mut entries = vec![("path", path_value)];
    if let Some(bytes) = bytes {
        let bytes_value = context.integer(i128::try_from(bytes).unwrap_or(i128::MAX))?;
        entries.push(("bytes", bytes_value));
    }
    make_alist(context, entries)
}

/// Derives the artifact-relative comparison image name from an expected path.
///
/// The result is the expected file stem plus `-compare.png`, so it lands
/// directly under the artifact root regardless of where the expected image is.
fn comparison_output_name(expected_path: &str) -> String {
    let stem = std::path::Path::new(expected_path)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("screen");
    format!("{stem}-compare.png")
}

/// Returns the on-disk byte length of a written artifact, or 0 when unknown.
fn written_len(path: &std::path::Path) -> usize {
    std::fs::metadata(path)
        .map(|metadata| usize::try_from(metadata.len()).unwrap_or(usize::MAX))
        .unwrap_or(0)
}

/// Registers the screen read, hash, screenshot, and comparison natives.
pub(super) fn register_screen_natives(
    engine: &mut Engine,
    session: &Rc<RefCell<AutomationSession>>,
    internal: &LibraryName,
) -> Result<(), Error> {
    let screen_available = Rc::clone(session);
    engine.register_library_fn(
        internal,
        "%screen-available?",
        1..=1,
        move |context, args| {
            if let Err(value) = machine_id(context, &screen_available, args[0])? {
                return Ok(value);
            }
            Ok(Value::boolean(screen_available.borrow().screen_available()))
        },
    )?;

    let screen_size = Rc::clone(session);
    engine.register_library_fn(internal, "%screen-size", 1..=1, move |context, args| {
        if let Err(value) = machine_id(context, &screen_size, args[0])? {
            return Ok(value);
        }
        match screen_size.borrow().screen_size() {
            Ok((width, height)) => {
                let width = context.integer(i128::from(width))?;
                let height = context.integer(i128::from(height))?;
                make_list(context, vec![width, height])
            }
            Err(error) => op_error_value(context, &error),
        }
    })?;

    let screen_rgba = Rc::clone(session);
    engine.register_library_fn(internal, "%screen-rgba", 1..=1, move |context, args| {
        if let Err(value) = machine_id(context, &screen_rgba, args[0])? {
            return Ok(value);
        }
        match screen_rgba.borrow().screen_rgba() {
            Ok(bytes) => context.bytevector(bytes),
            Err(error) => op_error_value(context, &error),
        }
    })?;

    let screen_pixel = Rc::clone(session);
    engine.register_library_fn(internal, "%screen-pixel", 3..=3, move |context, args| {
        if let Err(value) = machine_id(context, &screen_pixel, args[0])? {
            return Ok(value);
        }
        let x = match to_u32(context, args[1])? {
            Ok(x) => x,
            Err(value) => return Ok(value),
        };
        let y = match to_u32(context, args[2])? {
            Ok(y) => y,
            Err(value) => return Ok(value),
        };
        match screen_pixel.borrow().screen_pixel(x, y) {
            Ok((red, green, blue, alpha)) => {
                let red = context.integer(i128::from(red))?;
                let green = context.integer(i128::from(green))?;
                let blue = context.integer(i128::from(blue))?;
                let alpha = context.integer(i128::from(alpha))?;
                make_list(context, vec![red, green, blue, alpha])
            }
            Err(error) => op_error_value(context, &error),
        }
    })?;

    let screen_hash = Rc::clone(session);
    engine.register_library_fn(internal, "%screen-hash", 1..=1, move |context, args| {
        if let Err(value) = machine_id(context, &screen_hash, args[0])? {
            return Ok(value);
        }
        match screen_hash.borrow().screen_hash() {
            Ok(hash) => context.string_utf8(hash),
            Err(error) => op_error_value(context, &error),
        }
    })?;

    let save_screenshot = Rc::clone(session);
    engine.register_library_fn(internal, "%save-screenshot", 2..=2, move |context, args| {
        if let Err(value) = machine_id(context, &save_screenshot, args[0])? {
            return Ok(value);
        }
        let path = context.to_str(args[1])?.to_owned();
        match save_screenshot.borrow_mut().save_screenshot(&path) {
            Ok(written) => artifact_alist(context, &path, Some(written_len(&written))),
            Err(error) => op_error_value(context, &error),
        }
    })?;

    let screen_matches = Rc::clone(session);
    engine.register_library_fn(internal, "%screen-matches", 3..=3, move |context, args| {
        if let Err(value) = machine_id(context, &screen_matches, args[0])? {
            return Ok(value);
        }
        let path = context.to_str(args[1])?.to_owned();
        let tolerance = context.to_f64(args[2])?;
        match screen_matches.borrow().screen_matches(&path, tolerance) {
            Ok(matched) => Ok(Value::boolean(matched)),
            Err(error) => op_error_value(context, &error),
        }
    })?;

    let wait_for_screen = Rc::clone(session);
    engine.register_library_fn(internal, "%wait-for-screen", 5..=5, move |context, args| {
        if let Err(value) = machine_id(context, &wait_for_screen, args[0])? {
            return Ok(value);
        }
        let path = context.to_str(args[1])?.to_owned();
        let tolerance = context.to_f64(args[2])?;
        let maximum_frames = match to_count(context, args[3])? {
            Ok(value) => value,
            Err(value) => return Ok(value),
        };
        let maximum_ticks = match to_count(context, args[4])? {
            Ok(value) => value,
            Err(value) => return Ok(value),
        };
        match wait_for_screen.borrow_mut().wait_for_screen(
            &path,
            tolerance,
            maximum_frames,
            maximum_ticks,
        ) {
            Ok(matched) => Ok(Value::boolean(matched)),
            Err(error) => op_error_value(context, &error),
        }
    })?;

    let region_matches = Rc::clone(session);
    engine.register_library_fn(
        internal,
        "%screen-region-matches",
        7..=7,
        move |context, args| {
            if let Err(value) = machine_id(context, &region_matches, args[0])? {
                return Ok(value);
            }
            let path = context.to_str(args[1])?.to_owned();
            let x = match to_u32(context, args[2])? {
                Ok(x) => x,
                Err(value) => return Ok(value),
            };
            let y = match to_u32(context, args[3])? {
                Ok(y) => y,
                Err(value) => return Ok(value),
            };
            let width = match to_u32(context, args[4])? {
                Ok(width) => width,
                Err(value) => return Ok(value),
            };
            let height = match to_u32(context, args[5])? {
                Ok(height) => height,
                Err(value) => return Ok(value),
            };
            let tolerance = context.to_f64(args[6])?;
            match region_matches
                .borrow()
                .screen_region_matches(&path, x, y, width, height, tolerance)
            {
                Ok(matched) => Ok(Value::boolean(matched)),
                Err(error) => op_error_value(context, &error),
            }
        },
    )?;

    let comparison_image = Rc::clone(session);
    engine.register_library_fn(
        internal,
        "%screen-comparison-image",
        2..=2,
        move |context, args| {
            if let Err(value) = machine_id(context, &comparison_image, args[0])? {
                return Ok(value);
            }
            let expected = context.to_str(args[1])?.to_owned();
            let out_path = comparison_output_name(&expected);
            match comparison_image
                .borrow_mut()
                .screen_comparison_image(&expected, &out_path)
            {
                Ok(written) => artifact_alist(context, &out_path, Some(written_len(&written))),
                Err(error) => op_error_value(context, &error),
            }
        },
    )?;

    Ok(())
}
