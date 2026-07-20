use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use crate::{Error, ErrorKind};

/// Feature identifiers reported by `cond-expand` and `features`.
///
/// The core R7RS feature is always present. Platform identifiers are opt-in so
/// embedders do not accidentally expose host details to guest programs.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct FeatureSet {
    additional: Vec<String>,
    libraries: Vec<crate::LibraryName>,
}

impl FeatureSet {
    /// Returns the feature identifiers in deterministic order.
    #[must_use]
    pub fn identifiers(&self) -> Vec<&str> {
        // These describe the fixed numeric representation used by this
        // profile. `ratios` and `exact-closed` are intentionally absent:
        // checked i128 exact arithmetic can reject an out-of-range result.
        let mut values = vec!["r7rs", "ieee-float", "exact-complex"];
        values.extend(self.additional.iter().map(String::as_str));
        values.sort_unstable();
        values.dedup();
        values
    }

    /// Returns whether `identifier` is reported by this engine.
    #[must_use]
    pub fn contains(&self, identifier: &str) -> bool {
        matches!(identifier, "r7rs" | "ieee-float" | "exact-complex")
            || self.additional.iter().any(|value| value == identifier)
    }

    /// Returns this set with one explicitly enabled implementation feature.
    ///
    /// This is intended for host/platform descriptors such as `x86-64` or
    /// `gnu-linux`. Language features must only be enabled when implemented.
    #[must_use]
    pub fn with_identifier(mut self, identifier: impl Into<String>) -> Self {
        let identifier = identifier.into();
        if !matches!(identifier.as_str(), "r7rs" | "ieee-float" | "exact-complex")
            && !self.additional.contains(&identifier)
        {
            self.additional.push(identifier);
        }
        self
    }

    /// Adds one feature identifier in place, skipping the always-present
    /// built-ins and any duplicate. Used when an extension enables its feature
    /// after engine construction.
    pub(crate) fn add_identifier(&mut self, identifier: &str) {
        if !matches!(identifier, "r7rs" | "ieee-float" | "exact-complex")
            && !self.additional.iter().any(|value| value == identifier)
        {
            self.additional.push(identifier.to_owned());
        }
    }

    pub(crate) fn add_library(&mut self, library: crate::LibraryName) {
        if !self.libraries.contains(&library) {
            self.libraries.push(library);
        }
    }

    pub(crate) fn contains_library(&self, library: &crate::LibraryName) -> bool {
        crate::library::standard_exports(library).is_some() || self.libraries.contains(library)
    }
}

/// Controls whether source text remains available after registration.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SourceRetention {
    /// Retain text so rendered diagnostics can include source snippets.
    #[default]
    Full,
    /// Retain names and location metadata, but discard the original text.
    Metadata,
}

/// Controls how execution-time resource exhaustion is surfaced.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum LimitBehavior {
    /// Return a structured host-visible limit error.
    #[default]
    HostError,
    /// Raise a Scheme condition when a handler is active, otherwise return a
    /// structured limit error.
    CatchableCondition,
}

/// A cheap, thread-safe signal used to request that an engine stop.
#[derive(Clone, Debug, Default)]
pub struct InterruptToken(Arc<AtomicBool>);

impl InterruptToken {
    /// Creates a token in the non-interrupted state.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Requests interruption.
    pub fn interrupt(&self) {
        self.0.store(true, Ordering::Release);
    }

    /// Clears a previous interruption request.
    pub fn reset(&self) {
        self.0.store(false, Ordering::Release);
    }

    /// Returns whether interruption has been requested.
    #[must_use]
    pub fn is_interrupted(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

/// Resource limits used by compilation and execution.
///
/// Every limit is enforced by the engine, so hosts can establish their full
/// resource policy before creating one.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Limits {
    max_source_bytes: usize,
    max_token_bytes: usize,
    max_nesting_depth: usize,
    max_expansion_steps: usize,
    max_expansion_depth: usize,
    initial_gc_threshold: usize,
    max_heap_slots: usize,
    max_heap_bytes: usize,
    fuel: Option<u64>,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_source_bytes: 16 * 1024 * 1024,
            max_token_bytes: 1024 * 1024,
            max_nesting_depth: 1024,
            max_expansion_steps: 1_000_000,
            max_expansion_depth: 1024,
            // Also the floor for the adaptive headroom after each collection.
            // Raising it delays the first collection and keeps small heaps from
            // collecting on a tiny threshold. Measured with the 4x-6x adaptive
            // policy this cut instructions on allocation-heavy benchmarks while
            // allocation-light hot loops stayed flat. Earlier measurements with
            // the old 2x-3x policy saw cache-locality regressions from a large
            // floor, so re-measure both sides when changing it.
            initial_gc_threshold: 8192,
            max_heap_slots: 1_048_576,
            max_heap_bytes: 256 * 1024 * 1024,
            fuel: None,
        }
    }
}

macro_rules! limit_accessors {
    ($(($get:ident, $with:ident, $field:ident, $ty:ty, $doc:literal)),* $(,)?) => {$ (
        #[doc = $doc]
        #[must_use]
        pub fn $get(&self) -> $ty { self.$field }

        #[doc = concat!("Returns these limits with a new value for `", stringify!($field), "`.")]
        #[must_use]
        pub fn $with(mut self, value: $ty) -> Self {
            self.$field = value;
            self
        }
    )*};
}

impl Limits {
    limit_accessors! {
        (max_source_bytes, with_max_source_bytes, max_source_bytes, usize, "Returns the maximum UTF-8 byte length of one source."),
        (max_token_bytes, with_max_token_bytes, max_token_bytes, usize, "Returns the maximum UTF-8 byte length of one token."),
        (max_nesting_depth, with_max_nesting_depth, max_nesting_depth, usize, "Returns the maximum reader nesting depth."),
        (max_expansion_steps, with_max_expansion_steps, max_expansion_steps, usize, "Returns the maximum number of macro expansion steps."),
        (max_expansion_depth, with_max_expansion_depth, max_expansion_depth, usize, "Returns the maximum macro expansion depth."),
        (initial_gc_threshold, with_initial_gc_threshold, initial_gc_threshold, usize, "Returns the initial allocation threshold for garbage collection."),
        (max_heap_slots, with_max_heap_slots, max_heap_slots, usize, "Returns the maximum number of heap slots."),
        (max_heap_bytes, with_max_heap_bytes, max_heap_bytes, usize, "Returns the approximate maximum heap size in bytes."),
        (fuel, with_fuel, fuel, Option<u64>, "Returns the optional instruction fuel allowance."),
    }

    pub(crate) fn validate(&self) -> Result<(), Error> {
        let positive = [
            ("max_source_bytes", self.max_source_bytes),
            ("max_token_bytes", self.max_token_bytes),
            ("max_nesting_depth", self.max_nesting_depth),
            ("max_expansion_steps", self.max_expansion_steps),
            ("max_expansion_depth", self.max_expansion_depth),
            ("initial_gc_threshold", self.initial_gc_threshold),
            ("max_heap_slots", self.max_heap_slots),
            ("max_heap_bytes", self.max_heap_bytes),
        ];
        if let Some((name, _)) = positive.into_iter().find(|(_, value)| *value == 0) {
            return Err(Error::plain(
                ErrorKind::InvalidConfiguration,
                format!("configuration limit `{name}` must be greater than zero"),
            ));
        }
        if self.max_source_bytes > u32::MAX as usize {
            return Err(Error::plain(
                ErrorKind::InvalidConfiguration,
                "`max_source_bytes` cannot exceed the span offset range",
            ));
        }
        if self.max_token_bytes > self.max_source_bytes {
            return Err(Error::plain(
                ErrorKind::InvalidConfiguration,
                "`max_token_bytes` cannot exceed `max_source_bytes`",
            ));
        }
        if self.initial_gc_threshold > self.max_heap_slots {
            return Err(Error::plain(
                ErrorKind::InvalidConfiguration,
                "`initial_gc_threshold` cannot exceed `max_heap_slots`",
            ));
        }
        if self.fuel == Some(0) {
            return Err(Error::plain(
                ErrorKind::InvalidConfiguration,
                "instruction fuel must be greater than zero when configured",
            ));
        }
        Ok(())
    }
}

/// Configuration used to construct an [`crate::Engine`].
#[derive(Clone, Debug, Default)]
pub struct EngineConfig {
    limits: Limits,
    source_retention: SourceRetention,
    interrupt_token: Option<InterruptToken>,
    features: FeatureSet,
    limit_behavior: LimitBehavior,
    trust_natives: bool,
    #[cfg(feature = "host-capabilities")]
    standalone: bool,
}

impl EngineConfig {
    /// Creates a conventional standalone configuration with standard-library
    /// backed filesystem, source-loading, process-context, and clock access.
    ///
    /// This constructor is available only with the `host-capabilities` Cargo
    /// feature. Merely enabling that feature does not grant authority to
    /// engines created with [`EngineConfig::default`].
    #[cfg(feature = "host-capabilities")]
    #[must_use]
    pub fn standalone() -> Self {
        Self {
            standalone: true,
            ..Self::default()
        }
    }

    /// Returns the configured resource limits.
    #[must_use]
    pub fn limits(&self) -> &Limits {
        &self.limits
    }

    /// Returns the source-retention policy.
    #[must_use]
    pub fn source_retention(&self) -> SourceRetention {
        self.source_retention
    }

    /// Returns this configuration with the supplied resource limits.
    #[must_use]
    pub fn with_limits(mut self, limits: Limits) -> Self {
        self.limits = limits;
        self
    }

    /// Returns this configuration with the supplied source-retention policy.
    #[must_use]
    pub fn with_source_retention(mut self, retention: SourceRetention) -> Self {
        self.source_retention = retention;
        self
    }

    /// Returns this configuration using a host-provided interruption token.
    #[must_use]
    pub fn with_interrupt_token(mut self, token: InterruptToken) -> Self {
        self.interrupt_token = Some(token);
        self
    }

    /// Returns the feature identifiers exposed to Scheme programs.
    #[must_use]
    pub fn features(&self) -> &FeatureSet {
        &self.features
    }

    /// Returns how execution-time limits are surfaced.
    #[must_use]
    pub const fn limit_behavior(&self) -> LimitBehavior {
        self.limit_behavior
    }

    /// Returns this configuration with the selected limit behavior.
    #[must_use]
    pub fn with_limit_behavior(mut self, behavior: LimitBehavior) -> Self {
        self.limit_behavior = behavior;
        self
    }

    /// Returns this configuration with an explicit feature set.
    #[must_use]
    pub fn with_features(mut self, features: FeatureSet) -> Self {
        self.features = features;
        self
    }

    /// Enables one feature identifier in place. Used by extension installation
    /// so `cond-expand` and `features` report the extension after it loads.
    pub(crate) fn add_feature(&mut self, identifier: &str) {
        self.features.add_identifier(identifier);
    }

    pub(crate) fn add_library(&mut self, library: crate::LibraryName) {
        self.features.add_library(library);
    }

    /// Returns whether native (host) procedures are trusted not to panic.
    ///
    /// Defaults to `false`: a panic in a host callback is caught and surfaced as
    /// an [`crate::ErrorKind::NativePanic`] error at the evaluation boundary,
    /// isolating the VM. Enabling this skips that boundary and the active-native
    /// marker. A panicking native then unwinds through the interpreter as an
    /// ordinary Rust panic, so only enable it when every registered native is
    /// known not to panic.
    #[must_use]
    pub const fn trusts_natives(&self) -> bool {
        self.trust_natives
    }

    /// Returns this configuration with native panic-catching disabled.
    ///
    /// See [`EngineConfig::trusts_natives`] for the safety trade-off.
    #[must_use]
    pub fn with_trusted_natives(mut self, trusted: bool) -> Self {
        self.trust_natives = trusted;
        self
    }

    pub(crate) fn take_interrupt_token(&mut self) -> InterruptToken {
        self.interrupt_token.take().unwrap_or_default()
    }

    #[cfg(feature = "host-capabilities")]
    pub(crate) const fn is_standalone(&self) -> bool {
        self.standalone
    }
}
