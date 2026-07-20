//! Machine construction, reconstruction, reset, and runtime save-state handling.

use common::{
    AutomatedMachine, AutomationDescriptor, SaveStateError,
    tracing::{ApplicationTraceSink, TraceHandle, TraceLimits},
};
use machine_factory::{InitError, config::EmulatorConfig};

use super::{
    ActiveMachine, AutomationSession, MachineId, OpError, RuntimeState, StateId, TrackedControls,
};
use crate::{
    media::MediaRequest,
    protocol::{MachineIdentity, MessageProtocol},
};

/// Maps a machine save-state failure to the automation error contract.
///
/// Every capture or restore failure surfaces as `neetan/unsupported`, since the
/// public contract reserves `neetan/argument` for an unknown or stale handle.
fn map_save_state_error(error: SaveStateError) -> OpError {
    match error {
        SaveStateError::Unsupported => {
            OpError::Unsupported("machine does not support runtime save states".to_owned())
        }
        other => OpError::Unsupported(other.to_string()),
    }
}

impl AutomationSession {
    /// Installs a directly constructed machine for Rust-side conformance tests.
    pub fn install_machine(&mut self, machine: Box<dyn AutomatedMachine>) {
        let id = MachineId(
            self.machine_ids
                .allocate()
                .expect("machine identity exhausted"),
        );
        let (_, trace) = ApplicationTraceSink::new(TraceLimits::default());
        trace.set_epoch(0);
        let candidate = ActiveMachine {
            id,
            machine,
            epoch: 0,
            session_ticks_base: 0,
            session_frames_base: 0,
            startup_spec: EmulatorConfig::default(),
            mounts: Default::default(),
            startup_media: Vec::new(),
            tracked: TrackedControls::default(),
            runtime_states: Default::default(),
            trace,
            trace_failure: None,
        };
        self.active = Some(candidate);
    }

    /// Validates that `handle` identifies the active logical machine.
    pub fn validate_machine_handle(&self, handle: u64) -> Result<(), OpError> {
        if self
            .active
            .as_ref()
            .is_some_and(|active| active.id.0 == handle)
        {
            Ok(())
        } else {
            Err(OpError::StaleHandle(
                "machine handle is no longer active".to_owned(),
            ))
        }
    }

    /// Folds the outgoing machine's epoch counters into the session bases and
    /// advances the epoch, then installs a reconstructed machine.
    ///
    /// Session totals stay monotonic while epoch-relative counters reset.
    pub fn reconstruct_machine(&mut self, machine: Box<dyn AutomatedMachine>) {
        let active = self.active.as_mut().expect("machine present");
        let timeline = active.machine.automation_timeline();
        active.session_ticks_base = active
            .session_ticks_base
            .saturating_add(timeline.epoch_ticks);
        active.session_frames_base = active
            .session_frames_base
            .saturating_add(timeline.epoch_frames);
        active.runtime_states.clear();
        active.epoch = active.epoch.saturating_add(1);
        active.machine = machine;
    }

    /// Builds a fresh automated machine from `spec` without installing it.
    ///
    /// Returns the machine together with the external trace handle for its newly
    /// created sink. The two share one queue, so driving the handle controls the
    /// sink embedded in the machine.
    fn build_machine(
        &self,
        spec: &EmulatorConfig,
    ) -> Result<(Box<dyn AutomatedMachine>, TraceHandle), InitError> {
        let (trace_sink, trace_handle) = ApplicationTraceSink::new(TraceLimits::default());
        let machine = machine_factory::machines::initialize_automated_machine(
            spec.clone(),
            self.factory_rtc.clone(),
            self.sample_rate,
            trace_sink,
        )?;
        Ok((machine, trace_handle))
    }

    /// Installs the trace handle for the freshly built machine and stamps it with
    /// the current epoch, clearing any mirrored collector failure.
    fn install_trace_handle(&mut self, handle: TraceHandle) {
        let active = self.active.as_mut().expect("machine present");
        handle.set_epoch(active.epoch);
        active.trace = handle;
        active.trace_failure = None;
    }

    /// Constructs a machine from `spec`, installing or reconstructing it, records
    /// `spec` as the startup specification, and mounts the declared `media`.
    ///
    /// The declared media becomes the startup set replayed by `restore-startup!`.
    pub fn open_machine(
        &mut self,
        spec: EmulatorConfig,
        media: Vec<MediaRequest>,
    ) -> Result<(u64, AutomationDescriptor), OpError> {
        if self.has_machine() {
            return Err(OpError::MachineState(
                "a machine scope is already active".to_owned(),
            ));
        }
        let (machine, trace_handle) = self.build_machine(&spec).map_err(OpError::Construction)?;
        let descriptor = machine.automation_descriptor();
        let id = MachineId(self.machine_ids.allocate()?);
        trace_handle.set_epoch(0);
        let mut candidate = ActiveMachine {
            id,
            machine,
            epoch: 0,
            session_ticks_base: 0,
            session_frames_base: 0,
            startup_spec: spec,
            mounts: Default::default(),
            startup_media: media,
            tracked: TrackedControls::default(),
            runtime_states: Default::default(),
            trace: trace_handle,
            trace_failure: None,
        };
        let requests = candidate.startup_media.clone();
        for request in &requests {
            if let Err(error) = Self::mount_into(
                &mut candidate,
                &self.read_root,
                &self.artifact_root,
                request,
                None,
            ) {
                candidate.trace.stop();
                candidate.machine.flush_floppies();
                candidate.machine.flush_hdds();
                candidate.machine.flush_printer();
                return Err(error);
            }
        }
        self.active = Some(candidate);
        let _ = self.events.send(MessageProtocol::MachineReady {
            identity: MachineIdentity {
                target: descriptor.target.to_owned(),
                model: descriptor.model.to_owned(),
            },
        });
        Ok((id.token(), descriptor))
    }

    /// Closes the active logical machine identified by `handle`.
    pub fn close_machine(&mut self, handle: u64) -> Result<(), OpError> {
        self.validate_machine_handle(handle)?;
        self.close_active_machine();
        Ok(())
    }

    /// Closes any active machine during scope or executor cleanup.
    pub fn close_active_machine(&mut self) {
        if self.active.is_none() {
            return;
        }
        self.release_all_controls();
        self.flush_and_release_media();
        if let Some(active) = self.active.take() {
            active.trace.stop();
        }
    }

    /// Applies a reset. A hard reset reconstructs from the startup specification;
    /// a soft reset asserts the machine reset mechanism when implemented.
    ///
    /// A hard reset retains the current mounted media and its written contents:
    /// the written image bytes are captured from the outgoing machine and
    /// re-injected into the reconstructed one. A soft reset leaves media alone.
    pub fn reset(&mut self, hard: bool) -> Result<(), OpError> {
        if !self.has_machine() {
            return Err(OpError::NoMachine);
        }
        if hard {
            let spec = self
                .active
                .as_ref()
                .map(|active| active.startup_spec.clone())
                .ok_or(OpError::NoMachine)?;
            let requests: Vec<MediaRequest> = self.mount_requests();
            let captured = self.capture_written_media();
            let (machine, trace_handle) =
                self.build_machine(&spec).map_err(OpError::Construction)?;
            self.reconstruct_machine(machine);
            self.install_trace_handle(trace_handle);
            self.active
                .as_mut()
                .expect("machine present")
                .mounts
                .clear();
            self.release_all_controls();
            for request in &requests {
                let bytes = captured
                    .get(&(request.kind, request.slot))
                    .map(Vec::as_slice);
                self.mount(request, bytes)?;
            }
            Ok(())
        } else {
            self.release_all_controls();
            let performed = self
                .active
                .as_mut()
                .expect("machine present")
                .machine
                .soft_reset();
            if performed {
                Ok(())
            } else {
                Err(OpError::Unsupported(
                    "soft reset is not implemented for this machine".to_owned(),
                ))
            }
        }
    }

    /// Reconstructs the machine from the startup specification and remounts the
    /// declared startup media with fresh pristine baselines, discarding all
    /// runtime writes and runtime insert or eject changes.
    pub fn restore_startup(&mut self) -> Result<(), OpError> {
        let active = self.active.as_ref().ok_or(OpError::NoMachine)?;
        let spec = active.startup_spec.clone();
        let requests = active.startup_media.clone();
        let (machine, trace_handle) = self.build_machine(&spec).map_err(OpError::Construction)?;
        self.reconstruct_machine(machine);
        self.install_trace_handle(trace_handle);
        self.active
            .as_mut()
            .expect("machine present")
            .mounts
            .clear();
        self.release_all_controls();
        for request in &requests {
            self.mount(request, None)?;
        }
        Ok(())
    }

    /// Captures the active machine at a safe point and returns its private token.
    pub fn save_state(&mut self, machine_id: u64) -> Result<u64, OpError> {
        self.validate_machine_handle(machine_id)?;
        let state_id = StateId(self.state_ids.allocate()?);
        let active = self.active.as_mut().expect("machine present");
        let blob = active
            .machine
            .capture_state()
            .map_err(map_save_state_error)?;
        active.runtime_states.insert(
            state_id,
            RuntimeState {
                owner: active.id,
                blob,
            },
        );
        Ok(state_id.token())
    }

    /// Transactionally restores `handle` in place and releases held controls.
    ///
    /// Keeps the same machine instance, so the epoch does not advance and the
    /// captured tick and frame counters rewind. A machine restore failure leaves
    /// the machine and the session untouched.
    pub fn restore_state(&mut self, machine_id: u64, state_id: u64) -> Result<(), OpError> {
        self.validate_machine_handle(machine_id)?;
        let active = self.active.as_mut().expect("machine present");
        let state = active
            .runtime_states
            .get(&StateId(state_id))
            .ok_or_else(|| {
                OpError::StaleHandle("save-state handle is no longer active".to_owned())
            })?;
        if state.owner != active.id {
            return Err(OpError::StaleHandle(
                "save-state belongs to another machine".to_owned(),
            ));
        }
        active
            .machine
            .restore_state(&state.blob)
            .map_err(map_save_state_error)?;
        self.release_all_controls();
        Ok(())
    }

    /// Frees `handle`, reporting an unknown or already-freed handle as an error.
    pub fn discard_state(&mut self, machine_id: u64, state_id: u64) -> Result<(), OpError> {
        self.validate_machine_handle(machine_id)?;
        let active = self.active.as_mut().expect("machine present");
        let state_id = StateId(state_id);
        if active
            .runtime_states
            .get(&state_id)
            .is_some_and(|state| state.owner == active.id)
            && active.runtime_states.remove(&state_id).is_some()
        {
            Ok(())
        } else {
            Err(OpError::StaleHandle(
                "save-state handle is no longer active".to_owned(),
            ))
        }
    }
}
