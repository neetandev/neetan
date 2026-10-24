//! Machine construction, configuration, capability, and timeline natives.

use std::{cell::RefCell, collections::BTreeSet, rc::Rc};

use machine_factory::config::{
    EmulatorConfig, MachineSelection,
    settings::{
        ExplicitSettings, MACHINE_KEY, ROMS_FOLDER_KEY, Scope, Setting, ValueKind,
        apply_derived_defaults, find_setting,
    },
    systems::rom_folders,
    target_from_name, target_symbol_name,
};
use r7rs::{Engine, Error, LibraryName, NativeContext, NativeValues, Value};

use super::{
    API_VERSION_MAJOR, API_VERSION_MINOR,
    support::{error_value, machine_id, make_alist, op_error_value},
};
use crate::{
    config::CommonConfig,
    media::{MediaKind, MediaRequest, media_kind_from_name},
    protocol::{ExecutionResult, TestCaseOutcome},
    session::AutomationSession,
};

/// Returns whether the named capability is present on the current machine.
fn capability_present(session: &Rc<RefCell<AutomationSession>>, name: &str) -> bool {
    let mut borrowed = session.borrow_mut();
    let media = borrowed.media_capabilities().unwrap_or_default();
    let input = borrowed.input_capabilities().unwrap_or_default();
    match name {
        "keyboard" => input.keyboard,
        "mouse" => input.mouse_buttons > 0,
        "joystick" => input.joystick_ports > 0,
        "cartridge" => media.cartridge,
        "cassette" => media.cassette,
        "hard-disk" => media.hard_disk,
        "printer" => media.printer,
        "mt32" => media.mt32,
        "sc55" => media.sc55,
        "floppy-drives" => media.floppy_drives > 0,
        "cdrom" => media.cdrom,
        "save-state" => borrowed.has_machine(),
        "inspect" => borrowed.supports_inspection(),
        "mutate" => borrowed.supports_mutation(),
        "trace" => borrowed.supports_tracing(),
        _ => false,
    }
}

/// Builds the capability alist from the machine descriptor and media slots.
fn capabilities_value(
    context: &mut NativeContext,
    session: &Rc<RefCell<AutomationSession>>,
) -> Result<Value, Error> {
    let (media, input) = {
        let borrowed = session.borrow();
        (
            borrowed.media_capabilities().unwrap_or_default(),
            borrowed.input_capabilities().unwrap_or_default(),
        )
    };
    let supports_tracing = session.borrow().supports_tracing();
    let mouse_buttons = context.integer(i128::from(input.mouse_buttons))?;
    let joystick_ports = context.integer(i128::from(input.joystick_ports))?;
    let floppy_drives = context.integer(i128::from(media.floppy_drives))?;
    let trace_schema_version = context.integer(i128::from(common::TRACE_SCHEMA_VERSION))?;
    make_alist(
        context,
        vec![
            ("keyboard", Value::boolean(input.keyboard)),
            ("mouse", Value::boolean(input.mouse_buttons > 0)),
            ("mouse-buttons", mouse_buttons),
            ("joystick", Value::boolean(input.joystick_ports > 0)),
            ("joystick-ports", joystick_ports),
            ("cartridge", Value::boolean(media.cartridge)),
            ("cassette", Value::boolean(media.cassette)),
            ("hard-disk", Value::boolean(media.hard_disk)),
            ("printer", Value::boolean(media.printer)),
            ("mt32", Value::boolean(media.mt32)),
            ("sc55", Value::boolean(media.sc55)),
            ("floppy-drives", floppy_drives),
            ("cdrom", Value::boolean(media.cdrom)),
            ("trace", Value::boolean(supports_tracing)),
            ("trace-schema-version", trace_schema_version),
        ],
    )
}

/// Builds the machine-info alist, or `#f` when no machine is present.
fn machine_info_value(
    context: &mut NativeContext,
    session: &Rc<RefCell<AutomationSession>>,
) -> Result<Value, Error> {
    let (descriptor, timeline) = {
        let borrowed = session.borrow();
        (borrowed.descriptor(), borrowed.timeline())
    };
    let Some(descriptor) = descriptor else {
        return Ok(Value::boolean(false));
    };
    let capabilities = capabilities_value(context, session)?;
    let numerator = context.integer(i128::from(descriptor.timebase.ticks_per_second_numerator))?;
    let denominator =
        context.integer(i128::from(descriptor.timebase.ticks_per_second_denominator))?;
    let timebase = make_alist(
        context,
        vec![
            ("ticks-per-second-numerator", numerator),
            ("ticks-per-second-denominator", denominator),
        ],
    )?;
    let target = context.intern_symbol(descriptor.target)?;
    let model = context.intern_symbol(descriptor.model)?;
    let api_version = context.integer(API_VERSION_MAJOR)?;
    let epoch = context.integer(i128::from(timeline.epoch))?;
    let audio_sample_rate = context.integer(i128::from(descriptor.audio_sample_rate))?;
    make_alist(
        context,
        vec![
            ("target", target),
            ("model", model),
            ("api-version", api_version),
            ("epoch", epoch),
            ("timebase", timebase),
            ("audio-sample-rate", audio_sample_rate),
            ("capabilities", capabilities),
        ],
    )
}

/// Builds the host-config alist from the exposed common settings.
fn host_config_value(
    context: &mut NativeContext,
    session: &Rc<RefCell<AutomationSession>>,
) -> Result<Value, Error> {
    let common = session.borrow().common_config().clone();
    let mut entries: Vec<(&str, Value)> = Vec::new();
    let directories = rom_folders()
        .map(|folder| (folder.key, common.rom_folders.get(folder.key)))
        .chain([
            (ROMS_FOLDER_KEY, common.roms_folder.as_ref()),
            ("artifact-root", common.artifact_root.as_ref()),
        ]);
    for (key, directory) in directories {
        if let Some(path) = directory {
            let value = context.string_utf8(path.display().to_string())?;
            entries.push((key, value));
        }
    }
    let guest_time = context.string_utf8(format_guest_time(&common))?;
    let timeout = context.integer(i128::from(common.timeout_seconds))?;
    let audio_sample_rate = context.integer(i128::from(common.audio_sample_rate()))?;
    entries.push(("guest-time", guest_time));
    entries.push(("timeout", timeout));
    entries.push(("audio-sample-rate", audio_sample_rate));
    make_alist(context, entries)
}

/// Formats the guest real-time-clock value as `YYYY-MM-DDThh:mm:ss`.
fn format_guest_time(common: &CommonConfig) -> String {
    let time = common.guest_time;
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
        time.year, time.month, time.day, time.hour, time.minute, time.second
    )
}

/// Copies the common ROM directories into the configuration through the settings table.
fn seed_rom_directories(config: &mut EmulatorConfig, common: &CommonConfig) -> Result<(), String> {
    for (key, directory) in &common.rom_folders {
        let Some(setting) = find_setting(key) else {
            return Err(format!("internal error: '{key}' is not a settings key"));
        };
        (setting.apply)(config, &directory.to_string_lossy())?;
    }
    Ok(())
}

/// Validates the parsed media entries into mount requests for the session.
///
/// Slot ranges are checked against each kind and duplicate `(kind, slot)`
/// entries are rejected. The session mounts these after the machine is built.
fn collect_media(
    media: Vec<(String, i128, String)>,
) -> Result<Vec<MediaRequest>, (String, String)> {
    let mut requests: Vec<MediaRequest> = Vec::new();
    let mut seen: BTreeSet<(MediaKind, usize)> = BTreeSet::new();
    for (media_type, slot, path) in media {
        let Some(kind) = media_kind_from_name(&media_type) else {
            return Err((
                "neetan/argument".to_owned(),
                format!("unknown media type '{media_type}'"),
            ));
        };
        let Ok(slot_index) = usize::try_from(slot) else {
            return Err((
                "neetan/argument".to_owned(),
                format!("{} slot {slot} is out of range", kind.symbol()),
            ));
        };
        if slot_index >= kind.slot_count() {
            return Err((
                "neetan/argument".to_owned(),
                format!(
                    "{} slot {slot} is out of range, expected 0..{}",
                    kind.symbol(),
                    kind.slot_count()
                ),
            ));
        }
        if !seen.insert((kind, slot_index)) {
            return Err((
                "neetan/argument".to_owned(),
                format!("duplicate media entry for {} slot {slot}", kind.symbol()),
            ));
        }
        requests.push(MediaRequest {
            kind,
            slot: slot_index,
            source: path,
        });
    }
    Ok(requests)
}

/// Reads the symbol value of a choice or on/off setting from a specification entry.
///
/// The inner `Err` reports a value kind that `specification_setting` rejects.
fn setting_value(
    context: &NativeContext,
    setting: &Setting,
    value: Value,
) -> Result<Result<String, String>, Error> {
    match setting.kind {
        ValueKind::Choice(_) | ValueKind::OnOff => {
            Ok(Ok(context.to_symbol_name(value)?.to_owned()))
        }
        ValueKind::Number(_) | ValueKind::Path(_) | ValueKind::PathList => Ok(Err(format!(
            "internal error: the machine specification reads no value of '{}'",
            setting.key
        ))),
    }
}

/// Looks up a machine setting of the specification.
///
/// The specification accepts the settings of the emulated machine with a
/// choice or on/off value. The `model` key selects the machine. The
/// `media` entry mounts every image, the printer output included.
fn specification_setting(key: &str) -> Result<&'static Setting, String> {
    let Some(setting) = find_setting(key) else {
        return Err(format!("unknown machine specification key '{key}'"));
    };
    if setting.key == MACHINE_KEY {
        return Err(format!(
            "unknown machine specification key '{key}', the model key selects the machine"
        ));
    }
    match setting.scope {
        Scope::System => {}
        Scope::Host => {
            return Err(format!(
                "'{key}' is a host setting, a machine specification cannot set it"
            ));
        }
        Scope::Game => {
            return Err(format!(
                "'{key}' is a media setting, mount media with the media entry"
            ));
        }
    }
    if setting.frontend_only {
        return Err(format!("'{key}' has no effect in a headless session"));
    }
    match setting.kind {
        ValueKind::Path(_) | ValueKind::PathList if key == PRINTER_KEY => {
            return Err(format!(
                "'{key}' is a media setting, mount the output with the printer media kind"
            ));
        }
        ValueKind::Path(_) | ValueKind::PathList => {
            return Err(format!(
                "'{key}' takes a path, a machine specification cannot set it"
            ));
        }
        ValueKind::Number(_) => {
            return Err(format!(
                "'{key}' takes a number, a machine specification cannot set it"
            ));
        }
        ValueKind::Choice(_) | ValueKind::OnOff => {}
    }
    Ok(setting)
}

/// Key of the printer output setting in configuration files.
const PRINTER_KEY: &str = "printer";

/// Parses a `call-with-machine` specification alist into an `EmulatorConfig` plus
/// the list of media mount requests.
///
/// Returns the inner `Err` for a domain error whose stable symbol and message
/// the caller re-raises. The outer `Err` is a structural r7rs type error.
#[allow(clippy::type_complexity)]
fn parse_specification(
    context: &NativeContext,
    specification: Value,
    common: &CommonConfig,
) -> Result<Result<(EmulatorConfig, Vec<MediaRequest>), (String, String)>, Error> {
    let mut target_name: Option<String> = None;
    let mut model_name: Option<String> = None;
    let mut settings: Vec<(&'static Setting, String)> = Vec::new();
    let mut media: Vec<(String, i128, String)> = Vec::new();
    let mut keys = BTreeSet::new();

    for entry in context.to_list(specification)? {
        let (key_value, value) = context.to_pair(entry)?;
        let key = context.to_symbol_name(key_value)?.to_owned();
        if !keys.insert(key.clone()) {
            return Ok(Err((
                "neetan/argument".to_owned(),
                format!("duplicate machine specification key '{key}'"),
            )));
        }
        match key.as_str() {
            "media" => {
                for media_entry in context.to_list(value)? {
                    let parts = context.to_list(media_entry)?;
                    if parts.len() != 3 {
                        return Ok(Err((
                            "neetan/argument".to_owned(),
                            "media entry must be (type slot path)".to_owned(),
                        )));
                    }
                    let media_type = context.to_symbol_name(parts[0])?.to_owned();
                    let slot = context.to_i128(parts[1])?;
                    let path = context.to_str(parts[2])?.to_owned();
                    media.push((media_type, slot, path));
                }
            }
            "target" => target_name = Some(context.to_symbol_name(value)?.to_owned()),
            "model" => model_name = Some(context.to_symbol_name(value)?.to_owned()),
            _ => {
                let setting = match specification_setting(&key) {
                    Ok(setting) => setting,
                    Err(message) => return Ok(Err(("neetan/argument".to_owned(), message))),
                };
                match setting_value(context, setting, value)? {
                    Ok(text) => settings.push((setting, text)),
                    Err(message) => return Ok(Err(("neetan/argument".to_owned(), message))),
                }
            }
        }
    }

    let model = match (&target_name, &model_name) {
        (_, Some(model)) => match model.parse::<MachineSelection>() {
            Ok(model) => model,
            Err(message) => return Ok(Err(("neetan/argument".to_owned(), message))),
        },
        (Some(target), None) => match target_from_name(target) {
            Ok(target) => MachineSelection::default_of(target),
            Err(message) => return Ok(Err(("neetan/argument".to_owned(), message))),
        },
        (None, None) => {
            return Ok(Err((
                "neetan/argument".to_owned(),
                "the machine specification has no target and no model".to_owned(),
            )));
        }
    };
    if let (Some(target), Some(_)) = (&target_name, &model_name)
        && target_symbol_name(model.target()) != target
    {
        return Ok(Err((
            "neetan/argument".to_owned(),
            format!("target '{target}' and the selected model disagree"),
        )));
    }
    let mut config = EmulatorConfig::for_model(model);
    let mut explicit = ExplicitSettings::default();
    if let Err(message) = seed_rom_directories(&mut config, common) {
        return Ok(Err(("neetan/argument".to_owned(), message)));
    }

    for (setting, value) in &settings {
        if let Err(message) = explicit.apply(&mut config, setting, value) {
            return Ok(Err(("neetan/argument".to_owned(), message)));
        }
    }
    apply_derived_defaults(&mut config, explicit);

    let media_requests = match collect_media(media) {
        Ok(requests) => requests,
        Err(pair) => return Ok(Err(pair)),
    };

    Ok(Ok((config, media_requests)))
}

/// Registers the result, note, and API-version natives.
pub(super) fn register_result_natives(
    engine: &mut Engine,
    session: &Rc<RefCell<AutomationSession>>,
    internal: &LibraryName,
) -> Result<(), Error> {
    let result_ok = Rc::clone(session);
    engine.register_library_fn(internal, "%result-ok", 0..=0, move |_context, _args| {
        let accepted = result_ok.borrow_mut().record_result(ExecutionResult::Ok);
        Ok(Value::boolean(accepted))
    })?;

    let result_error = Rc::clone(session);
    engine.register_library_fn(internal, "%result-error", 1..=1, move |context, args| {
        let message = context.to_str(args[0])?.to_owned();
        let accepted = result_error
            .borrow_mut()
            .record_result(ExecutionResult::Error { message });
        Ok(Value::boolean(accepted))
    })?;

    let emit_note = Rc::clone(session);
    engine.register_library_fn(internal, "%emit-note", 1..=1, move |context, args| {
        let text = context.to_str(args[0])?.to_owned();
        emit_note.borrow().emit_output(format!("{text}\n"));
        Ok(Value::unspecified())
    })?;

    let emit_test_case = Rc::clone(session);
    engine.register_library_fn(
        internal,
        "%emit-test-case-result",
        5..=5,
        move |context, args| {
            let suite = context.to_str(args[0])?.to_owned();
            let test_case = context.to_str(args[1])?.to_owned();
            let status = context.to_symbol_name(args[2])?;
            let kind = context.to_symbol_name(args[3])?.to_owned();
            let message = context.to_str(args[4])?.to_owned();
            let outcome = if status == "success" {
                TestCaseOutcome::Success
            } else {
                TestCaseOutcome::Failure { kind, message }
            };
            emit_test_case
                .borrow()
                .emit_test_case_result(suite, test_case, outcome);
            Ok(Value::unspecified())
        },
    )?;

    engine.register_library_fn(internal, "%api-version", 0..=0, move |context, _args| {
        let major = context.integer(API_VERSION_MAJOR)?;
        let minor = context.integer(API_VERSION_MINOR)?;
        Ok(NativeValues::many([major, minor]))
    })?;

    Ok(())
}

/// Registers the machine construction, configuration, and inspection natives.
pub(super) fn register_query_natives(
    engine: &mut Engine,
    session: &Rc<RefCell<AutomationSession>>,
    internal: &LibraryName,
) -> Result<(), Error> {
    let host_config = Rc::clone(session);
    engine.register_library_fn(internal, "%host-config", 0..=0, move |context, _args| {
        host_config_value(context, &host_config)
    })?;

    let open_machine = Rc::clone(session);
    engine.register_library_fn(internal, "%open-machine", 1..=1, move |context, args| {
        let common = open_machine.borrow().common_config().clone();
        let (config, media) = match parse_specification(context, args[0], &common)? {
            Ok(pair) => pair,
            Err((symbol, message)) => return error_value(context, &symbol, &message),
        };
        match open_machine.borrow_mut().open_machine(config, media) {
            Ok((handle, _descriptor)) => context.integer(i128::from(handle)),
            Err(error) => op_error_value(context, &error),
        }
    })?;

    let validate_machine = Rc::clone(session);
    engine.register_library_fn(
        internal,
        "%validate-machine",
        1..=1,
        move |context, args| {
            let handle = match u64::try_from(context.to_i128(args[0])?) {
                Ok(handle) => handle,
                Err(_) => {
                    return error_value(
                        context,
                        "neetan/stale-handle",
                        "machine handle is no longer active",
                    );
                }
            };
            match validate_machine.borrow().validate_machine_handle(handle) {
                Ok(()) => Ok(Value::boolean(true)),
                Err(error) => op_error_value(context, &error),
            }
        },
    )?;

    let close_machine = Rc::clone(session);
    engine.register_library_fn(internal, "%close-machine", 1..=1, move |context, args| {
        let handle = match u64::try_from(context.to_i128(args[0])?) {
            Ok(handle) => handle,
            Err(_) => {
                return error_value(
                    context,
                    "neetan/stale-handle",
                    "machine handle is no longer active",
                );
            }
        };
        match close_machine.borrow_mut().close_machine(handle) {
            Ok(()) => Ok(Value::boolean(true)),
            Err(error) => op_error_value(context, &error),
        }
    })?;

    let machine_info = Rc::clone(session);
    engine.register_library_fn(internal, "%machine-info", 1..=1, move |context, args| {
        if let Err(value) = machine_id(context, &machine_info, args[0])? {
            return Ok(value);
        }
        machine_info_value(context, &machine_info)
    })?;

    let machine_capabilities = Rc::clone(session);
    engine.register_library_fn(
        internal,
        "%machine-capabilities",
        1..=1,
        move |context, args| {
            if let Err(value) = machine_id(context, &machine_capabilities, args[0])? {
                return Ok(value);
            }
            capabilities_value(context, &machine_capabilities)
        },
    )?;

    let machine_capability = Rc::clone(session);
    engine.register_library_fn(
        internal,
        "%machine-capability?",
        2..=2,
        move |context, args| {
            if let Err(value) = machine_id(context, &machine_capability, args[0])? {
                return Ok(value);
            }
            let name = context.to_symbol_name(args[1])?;
            Ok(Value::boolean(capability_present(
                &machine_capability,
                name,
            )))
        },
    )?;

    register_timeline_natives(engine, session, internal)
}

/// Registers the timeline counter natives.
fn register_timeline_natives(
    engine: &mut Engine,
    session: &Rc<RefCell<AutomationSession>>,
    internal: &LibraryName,
) -> Result<(), Error> {
    let current_epoch = Rc::clone(session);
    engine.register_library_fn(internal, "%current-epoch", 1..=1, move |context, args| {
        if let Err(value) = machine_id(context, &current_epoch, args[0])? {
            return Ok(value);
        }
        context.integer(i128::from(current_epoch.borrow().timeline().epoch))
    })?;

    let current_tick = Rc::clone(session);
    engine.register_library_fn(internal, "%current-tick", 1..=1, move |context, args| {
        if let Err(value) = machine_id(context, &current_tick, args[0])? {
            return Ok(value);
        }
        context.integer(current_tick.borrow().timeline().session_ticks as i128)
    })?;

    let current_frame = Rc::clone(session);
    engine.register_library_fn(internal, "%current-frame", 1..=1, move |context, args| {
        if let Err(value) = machine_id(context, &current_frame, args[0])? {
            return Ok(value);
        }
        context.integer(current_frame.borrow().timeline().session_frames as i128)
    })?;

    let epoch_tick = Rc::clone(session);
    engine.register_library_fn(internal, "%epoch-tick", 1..=1, move |context, args| {
        if let Err(value) = machine_id(context, &epoch_tick, args[0])? {
            return Ok(value);
        }
        context.integer(epoch_tick.borrow().timeline().epoch_ticks as i128)
    })?;

    let epoch_frame = Rc::clone(session);
    engine.register_library_fn(internal, "%epoch-frame", 1..=1, move |context, args| {
        if let Err(value) = machine_id(context, &epoch_frame, args[0])? {
            return Ok(value);
        }
        context.integer(epoch_frame.borrow().timeline().epoch_frames as i128)
    })?;

    let emulated_time = Rc::clone(session);
    engine.register_library_fn(
        internal,
        "%emulated-time-ns",
        1..=1,
        move |context, args| {
            if let Err(value) = machine_id(context, &emulated_time, args[0])? {
                return Ok(value);
            }
            context.integer(emulated_time.borrow().emulated_time_ns() as i128)
        },
    )?;

    let shutdown_requested = Rc::clone(session);
    engine.register_library_fn(
        internal,
        "%shutdown-requested?",
        1..=1,
        move |context, args| {
            if let Err(value) = machine_id(context, &shutdown_requested, args[0])? {
                return Ok(value);
            }
            Ok(Value::boolean(
                shutdown_requested.borrow().shutdown_requested(),
            ))
        },
    )?;

    Ok(())
}
