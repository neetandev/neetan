use std::{
    fmt::Write as _,
    panic::{AssertUnwindSafe, catch_unwind, resume_unwind},
};

use crate::{
    DiagnosticLabel, EngineConfig, Error, ErrorKind, InterruptToken, LabelStyle, LoadedSource,
    SourceId, SourceLoader, SourceLocation, SourceRequest, Span, source::SourceMap,
};

mod bootstrap;
mod extensions;
mod imports;

pub use extensions::Extension;

/// An isolated owner of all mutable Scheme implementation state.
///
/// An engine is intentionally not cloneable and has no ambient source or I/O
/// authority. Host capabilities must be installed explicitly.
pub struct Engine {
    identity: std::rc::Rc<()>,
    config: EngineConfig,
    interrupt_token: InterruptToken,
    sources: SourceMap,
    source_loader: Option<Box<dyn SourceLoader>>,
    heap: crate::heap::Heap,
    globals: crate::global::GlobalStore,
    symbols: std::collections::HashMap<String, crate::Value>,
    natives: crate::native::NativeRegistry,
    libraries: crate::library::LibraryRegistry,
    register_stack: crate::vm::RegisterStack,
    /// The feature identifiers reported by the `features` procedure. Shared
    /// with that procedure's closure so extensions installed after
    /// construction are reflected without re-registering the native.
    feature_identifiers: std::rc::Rc<std::cell::RefCell<Vec<String>>>,
    /// The extensions installed on this engine, tracked so a repeated install
    /// is an idempotent no-op.
    installed_extensions: Vec<Extension>,
    /// The persistent interaction environment used by [`Engine::compile_interactive`].
    /// Imports resolved by interactive compiles accumulate here so a binding
    /// imported by one input stays visible to later ones, the way top-level
    /// `define`s already persist. Left empty by the batch [`Engine::compile`].
    interaction_imports: crate::library::LibraryBindings,
}

impl Engine {
    /// Creates an isolated engine after validating its configuration.
    pub fn new(mut config: EngineConfig) -> Result<Self, Error> {
        config.limits().validate()?;
        let interrupt_token = config.take_interrupt_token();
        let sources = SourceMap::new(config.source_retention());
        let heap = crate::heap::Heap::new(config.limits());
        let mut engine = Self {
            identity: std::rc::Rc::new(()),
            config,
            interrupt_token,
            sources,
            source_loader: None,
            heap,
            globals: crate::global::GlobalStore::default(),
            symbols: std::collections::HashMap::new(),
            natives: crate::native::NativeRegistry::new(),
            libraries: crate::library::LibraryRegistry::default(),
            register_stack: crate::vm::RegisterStack::preallocated(),
            feature_identifiers: std::rc::Rc::new(std::cell::RefCell::new(Vec::new())),
            installed_extensions: Vec::new(),
            interaction_imports: crate::library::LibraryBindings::default(),
        };
        engine
            .natives
            .set_track_panics(!engine.config.trusts_natives());
        #[cfg(feature = "host-capabilities")]
        if engine.config.is_standalone() {
            engine.source_loader = Some(Box::new(crate::StdSourceLoader));
            engine
                .heap
                .set_file_system(Some(Box::new(crate::StdFileSystem)));
            engine
                .heap
                .set_process_context(Some(Box::new(crate::StdProcessContext::snapshot()?)));
            engine
                .heap
                .set_clock(Some(Box::new(crate::StdClock::new())));
        }
        engine.install_default_ports()?;
        #[cfg(feature = "host-capabilities")]
        if engine.config.is_standalone() {
            engine.set_standard_input(Box::new(crate::StdStandardInput))?;
            engine.set_standard_output(Box::new(crate::StdStandardOutput))?;
            engine.set_standard_error(Box::new(crate::StdStandardError))?;
        }
        crate::native::install_base(&mut engine.natives, &mut engine.heap, &mut engine.globals)?;
        engine.install_configured_features()?;
        engine.install_standard_library_bindings()?;
        engine.install_standard_definitions()?;
        engine.install_standard_library_aliases();
        engine.refresh_engine_roots();
        Ok(engine)
    }

    /// Returns this engine's immutable configuration.
    #[must_use]
    pub const fn config(&self) -> &EngineConfig {
        &self.config
    }

    /// Returns a clone of this engine's interruption token.
    #[must_use]
    pub fn interrupt_token(&self) -> InterruptToken {
        self.interrupt_token.clone()
    }

    /// Installs or replaces the capability used to load external sources.
    pub fn set_source_loader(&mut self, loader: Box<dyn SourceLoader>) {
        self.source_loader = Some(loader);
    }

    /// Removes the external source-loading capability.
    pub fn clear_source_loader(&mut self) {
        self.source_loader = None;
    }

    /// Installs or replaces the capability used by Scheme file procedures.
    pub fn set_file_system(&mut self, file_system: Box<dyn crate::FileSystem>) {
        self.heap.set_file_system(Some(file_system));
    }

    /// Removes the capability used by Scheme file procedures.
    pub fn clear_file_system(&mut self) {
        self.heap.set_file_system(None);
    }

    /// Installs or replaces the capability used by Scheme process-context procedures.
    pub fn set_process_context(&mut self, process: Box<dyn crate::ProcessContext>) {
        self.heap.set_process_context(Some(process));
    }

    /// Removes the capability used by Scheme process-context procedures.
    pub fn clear_process_context(&mut self) {
        self.heap.set_process_context(None);
    }

    /// Installs or replaces the capability used by Scheme time procedures.
    pub fn set_clock(&mut self, clock: Box<dyn crate::Clock>) {
        self.heap.set_clock(Some(clock));
    }

    /// Removes the capability used by Scheme time procedures.
    pub fn clear_clock(&mut self) {
        self.heap.set_clock(None);
    }

    /// Installs or replaces the host resource behind `current-input-port`.
    ///
    /// By default the standard ports are engine-local buffers with no host
    /// authority: the input port is at end of file and the output ports
    /// silently accumulate. Installing a resource rebinds the parameter's base
    /// value, so code that has not `parameterize`d the port observes the new
    /// resource immediately.
    ///
    /// # Errors
    ///
    /// Returns an error if the port allocation hits a resource limit.
    pub fn set_standard_input(
        &mut self,
        resource: Box<dyn crate::PortResource>,
    ) -> Result<(), Error> {
        self.rebind_standard_port("current-input-port", resource, true, false)
    }

    /// Installs or replaces the host resource behind `current-output-port`.
    ///
    /// See [`Engine::set_standard_input`] for the rebinding semantics.
    ///
    /// # Errors
    ///
    /// Returns an error if the port allocation hits a resource limit.
    pub fn set_standard_output(
        &mut self,
        resource: Box<dyn crate::PortResource>,
    ) -> Result<(), Error> {
        self.rebind_standard_port("current-output-port", resource, false, true)
    }

    /// Installs or replaces the host resource behind `current-error-port`.
    ///
    /// See [`Engine::set_standard_input`] for the rebinding semantics.
    ///
    /// # Errors
    ///
    /// Returns an error if the port allocation hits a resource limit.
    pub fn set_standard_error(
        &mut self,
        resource: Box<dyn crate::PortResource>,
    ) -> Result<(), Error> {
        self.rebind_standard_port("current-error-port", resource, false, true)
    }

    /// Wraps `resource` as a textual heap port and makes it the base value of
    /// the named standard-port parameter.
    fn rebind_standard_port(
        &mut self,
        name: &str,
        resource: Box<dyn crate::PortResource>,
        input: bool,
        output: bool,
    ) -> Result<(), Error> {
        let id = self.heap.ports_mut().host(resource, input, output, false)?;
        let port = match self
            .heap
            .alloc(crate::heap::Object::Port(crate::port::PortObject { id }))
        {
            Ok(port) => port,
            Err(error) => {
                self.heap.ports_mut().finalize(id);
                return Err(error);
            }
        };
        let parameter = self.globals.get(name).copied().ok_or_else(|| {
            Error::plain(
                ErrorKind::RuntimeError,
                format!("standard port parameter '{name}' is not installed"),
            )
        })?;
        if !self.heap.set_parameter(parameter, port) {
            return Err(Error::plain(
                ErrorKind::RuntimeError,
                format!("global '{name}' is not a parameter"),
            ));
        }
        Ok(())
    }

    /// Registers host-provided UTF-8 source text under a diagnostic name.
    pub fn add_source(
        &mut self,
        name: impl Into<String>,
        text: impl Into<String>,
    ) -> Result<SourceId, Error> {
        let text = text.into();
        self.enforce_source_size(text.len())?;
        self.sources.add(name.into(), None, text)
    }

    /// Creates a streaming datum reader for UTF-8 source text.
    pub fn reader_from_str(
        &mut self,
        name: impl Into<String>,
        text: impl Into<String>,
    ) -> Result<crate::Reader, Error> {
        let text = text.into();
        let source = self.add_source(name, text.clone())?;
        Ok(crate::Reader::new(source, text, &self.config))
    }

    /// Creates a streaming datum reader after validating UTF-8 source bytes.
    pub fn reader_from_bytes(
        &mut self,
        name: impl Into<String>,
        bytes: impl AsRef<[u8]>,
    ) -> Result<crate::Reader, Error> {
        let name = name.into();
        let text = std::str::from_utf8(bytes.as_ref()).map_err(|_| {
            Error::plain(
                ErrorKind::InvalidUtf8,
                format!("source '{name}' is not valid UTF-8"),
            )
        })?;
        self.reader_from_str(name, text)
    }

    /// Compiles a hand-constructed expanded core expression to verified bytecode.
    pub fn compile_core(
        &self,
        expression: &crate::CoreExpr,
    ) -> Result<crate::CompiledModule, Error> {
        crate::compile::compile(expression, self.config.limits())
            .map(|module| module.with_owner(&self.identity))
    }

    /// Reads, hygienically expands, and compiles one source program.
    ///
    /// All datums in `source` form one compilation unit. Syntax bindings made
    /// by `define-syntax` are intentionally local to that unit. Value
    /// definitions take effect when the returned module is evaluated.
    pub fn compile(
        &mut self,
        name: impl Into<String>,
        source: impl Into<String>,
    ) -> Result<crate::CompiledModule, Error> {
        let mut reader = self.reader_from_str(name, source)?;
        let forms = crate::frontend::read_forms(&mut reader)?;
        self.compile_forms(forms)
    }

    /// Compiles one input against a persistent interaction environment.
    ///
    /// This is [`Engine::compile`] for read-eval-print use. A batch `compile`
    /// treats each call as an independent program, so bindings brought in by an
    /// `import` are scoped to that one call and vanish from the next. An
    /// interactive front-end instead wants the environment to accumulate the way
    /// a shared global table does: names imported by an earlier input stay
    /// visible to later ones, just as top-level `define`s already persist on the
    /// engine.
    ///
    /// Each call still resolves the import declarations it names (so a library is
    /// instantiated and its ambiguities are caught), then merges the resolved
    /// bindings into the engine's interaction environment with last-import-wins
    /// semantics, so re-importing a library is a harmless no-op. The input's body
    /// is expanded against the whole accumulated environment. Value definitions
    /// still take effect only when the returned module is evaluated.
    pub fn compile_interactive(
        &mut self,
        name: impl Into<String>,
        source: impl Into<String>,
    ) -> Result<crate::CompiledModule, Error> {
        let mut reader = self.reader_from_str(name, source)?;
        let forms = crate::frontend::read_forms(&mut reader)?;
        let (imports, body) = crate::library::program_imports(&forms)?;
        let resolved = self.resolve_imports(&imports)?;
        for (name, binding) in resolved.values {
            self.interaction_imports.macros.remove(&name);
            self.interaction_imports.values.insert(name, binding);
        }
        for (name, transformer) in resolved.macros {
            self.interaction_imports.values.remove(&name);
            self.interaction_imports.macros.insert(name, transformer);
        }
        let environment = self.interaction_imports.clone();
        let expression = crate::expand::expand_forms_with_imports(
            &body,
            self.config.limits(),
            environment.values,
            environment.macros,
            self.config.features(),
        )?
        .expression;
        crate::compile::compile(&expression, self.config.limits())
            .map(|module| module.with_owner(&self.identity))
    }

    fn compile_forms(
        &mut self,
        forms: Vec<crate::expand::Form>,
    ) -> Result<crate::CompiledModule, Error> {
        let (imports, body) = crate::library::program_imports(&forms)?;
        let imports = self.resolve_imports(&imports)?;
        let expression = crate::expand::expand_forms_with_imports(
            &body,
            self.config.limits(),
            imports.values,
            imports.macros,
            self.config.features(),
        )?
        .expression;
        crate::compile::compile(&expression, self.config.limits())
            .map(|module| module.with_owner(&self.identity))
    }

    /// Compiles an R7RS program that begins with one or more explicit import
    /// declarations.
    ///
    /// Use [`Engine::compile`] for expression-oriented embedding with the
    /// engine's conventional base bindings.
    pub fn compile_program(
        &mut self,
        name: impl Into<String>,
        source: impl Into<String>,
    ) -> Result<crate::CompiledModule, Error> {
        let mut reader = self.reader_from_str(name, source)?;
        let forms = crate::frontend::read_forms(&mut reader)?;
        let (imports, _) = crate::library::program_imports(&forms)?;
        if imports.is_empty() {
            return Err(Error::plain(
                ErrorKind::LibraryError,
                "R7RS program requires at least one import declaration",
            ));
        }
        self.compile_forms(forms)
    }

    /// Registers one `define-library` source declaration under `name`.
    ///
    /// Registration is intentionally separate from initialization: imports
    /// trigger dependency-ordered expansion and execution only when needed.
    pub fn register_library_source(
        &mut self,
        name: crate::LibraryName,
        display_name: impl Into<String>,
        source: impl Into<String>,
    ) -> Result<(), Error> {
        let source = source.into();
        let mut reader = self.reader_from_str(display_name, source.clone())?;
        let forms = crate::frontend::read_forms(&mut reader)?;
        let source_id = forms
            .first()
            .map(|form| form.span.source())
            .ok_or_else(|| {
                Error::plain(ErrorKind::LibraryError, "library source must not be empty")
            })?;
        let features = self.config.features().clone();
        let limits = self.config.limits().clone();
        let forms = crate::library::inline_includes(
            &forms,
            &features,
            source_id,
            &limits,
            |requested, case_insensitive, including_source| {
                let (included_source, mut text) =
                    self.load_source_text(requested, Some(including_source))?;
                if case_insensitive {
                    text.insert_str(0, "#!fold-case\n");
                }
                let mut reader = crate::Reader::new(included_source, text, &self.config);
                Ok((included_source, crate::frontend::read_forms(&mut reader)?))
            },
        )?;
        let declaration = crate::library::parse_declaration(&forms)?;
        if declaration.name != name {
            return Err(Error::plain(
                ErrorKind::LibraryError,
                format!(
                    "registered name {name} does not match declaration {}",
                    declaration.name
                ),
            ));
        }
        self.libraries
            .register_source(name.clone(), source_id, source, declaration)?;
        self.config.add_library(name);
        Ok(())
    }

    /// Executes a verified compiled module and returns a host root for its result.
    pub fn eval(&mut self, module: &crate::CompiledModule) -> Result<crate::EvalOutcome, Error> {
        if !module.belongs_to(&self.identity) {
            return Err(Error::plain(
                ErrorKind::WrongEngine,
                "compiled module belongs to a different engine",
            ));
        }
        self.refresh_engine_roots();
        let mut execute = || {
            crate::vm::execute(
                module,
                &mut self.heap,
                &mut self.register_stack,
                &mut self.globals,
                &mut self.symbols,
                &self.natives,
                self.config.limits(),
                &self.interrupt_token,
                &self.config,
                &mut self.source_loader,
                &mut self.sources,
            )
        };
        if self.config.trusts_natives() {
            return execute();
        }
        match catch_unwind(AssertUnwindSafe(execute)) {
            Ok(result) => result,
            Err(payload) => {
                let Some(name) = self.natives.take_panicked_native_name() else {
                    resume_unwind(payload);
                };
                self.heap.recover_native_unwind();
                Err(Error::plain(
                    ErrorKind::NativePanic,
                    format!("native procedure '{name}' panicked"),
                ))
            }
        }
    }

    /// Registers an engine-local host procedure exported by `library` as `binding`.
    ///
    /// `arity` is the inclusive range of argument counts accepted by the
    /// procedure. For example, `1..=1` requires exactly one argument, while
    /// `1..=3` accepts between one and three arguments.
    ///
    /// Repeated calls may add bindings until the library is first imported.
    /// Importing the library seals it, and later registrations are rejected.
    pub fn register_library_fn<F, R>(
        &mut self,
        library: &crate::LibraryName,
        binding: impl Into<String>,
        arity: std::ops::RangeInclusive<usize>,
        callback: F,
    ) -> Result<(), Error>
    where
        F: for<'a> Fn(&mut crate::NativeContext<'a>, &[crate::Value]) -> Result<R, Error> + 'static,
        R: crate::IntoNativeValues + 'static,
    {
        let binding = binding.into();
        if crate::library::standard_exports(library).is_some() {
            return Err(Error::plain(
                ErrorKind::LibraryError,
                format!("cannot register native bindings in standard library {library}"),
            ));
        }
        if arity.is_empty() {
            return Err(Error::plain(
                ErrorKind::RuntimeError,
                format!("procedure '{binding}' has an invalid arity"),
            ));
        }
        self.libraries.validate_native_binding(library, &binding)?;
        let global = Self::library_global_name(library, &binding);
        let procedure_name = format!("{binding} from library {library}");
        self.refresh_engine_roots();
        self.natives.register_at(
            &mut self.heap,
            &mut self.globals,
            global.clone(),
            procedure_name,
            arity,
            callback,
        )?;
        self.libraries
            .insert_native_binding(library.clone(), binding, global);
        self.config.add_library(library.clone());
        self.refresh_engine_roots();
        Ok(())
    }

    /// Performs a collection using globals, interned symbols, and host roots.
    pub fn collect_now(&mut self) {
        self.refresh_engine_roots();
        self.heap.collect();
    }

    /// Creates a rooted empty list value.
    #[must_use]
    pub fn root_nil(&self) -> crate::Root {
        self.heap.root(crate::Value::nil())
    }

    /// Creates a rooted boolean value.
    #[must_use]
    pub fn root_boolean(&self, value: bool) -> crate::Root {
        self.heap.root(crate::Value::boolean(value))
    }

    /// Creates a rooted exact integer, allocating values outside the inline range.
    #[must_use = "the rooted integer or allocation error must be handled"]
    pub fn root_integer(&mut self, value: i128) -> Result<crate::Root, Error> {
        let value = match i64::try_from(value).ok().map(crate::Value::integer) {
            Some(value) => value,
            None => self.heap.alloc(crate::heap::Object::Number(Box::new(
                crate::number::RuntimeNumber::Real(crate::Real::ExactInteger(value)),
            )))?,
        };
        Ok(self.heap.root(value))
    }

    /// Creates a rooted Unicode character value.
    #[must_use]
    pub fn root_character(&self, value: char) -> crate::Root {
        self.heap.root(crate::Value::character(value))
    }

    /// Returns the dynamic type of a rooted value owned by this engine.
    pub fn value_kind(&self, root: &crate::Root) -> Result<crate::ValueKind, Error> {
        self.require_root(root)?;
        Ok(self.heap.kind(root.value()))
    }

    /// Returns the standard external representation of a rooted value.
    pub fn write_root(&self, root: &crate::Root) -> Result<String, Error> {
        self.require_root(root)?;
        crate::printer::write_value(
            &self.heap,
            root.value(),
            crate::printer::RuntimeWriteMode::Write,
        )
    }

    /// Allocates a pair from two values rooted by this engine.
    pub fn make_pair(
        &mut self,
        car: &crate::Root,
        cdr: &crate::Root,
    ) -> Result<crate::Root, Error> {
        self.require_root(car)?;
        self.require_root(cdr)?;
        self.refresh_engine_roots();
        let value = self
            .heap
            .alloc(crate::heap::Object::Pair(car.value(), cdr.value()))?;
        Ok(self.heap.root(value))
    }

    /// Returns the components of a rooted pair as independent host roots.
    pub fn pair_values(&self, pair: &crate::Root) -> Result<(crate::Root, crate::Root), Error> {
        self.require_root(pair)?;
        let (car, cdr) = self
            .heap
            .pair(pair.value())
            .ok_or_else(|| Error::plain(ErrorKind::TypeError, "expected pair"))?;
        Ok((self.heap.root(car), self.heap.root(cdr)))
    }

    /// Replaces the car of a rooted pair.
    pub fn set_pair_car(&mut self, pair: &crate::Root, value: &crate::Root) -> Result<(), Error> {
        self.require_root(pair)?;
        self.require_root(value)?;
        if self.heap.set_pair_car(pair.value(), value.value()) {
            Ok(())
        } else {
            Err(Error::plain(ErrorKind::TypeError, "expected pair"))
        }
    }

    fn require_root(&self, root: &crate::Root) -> Result<(), Error> {
        if self.heap.owns_root(root) {
            Ok(())
        } else {
            Err(Error::plain(
                ErrorKind::WrongEngine,
                "root belongs to a different engine",
            ))
        }
    }

    /// Refreshes the heap's cached engine roots from the globals and symbol
    /// tables when either has changed since the last refresh. A no-op (two
    /// flag reads) when the tables are clean.
    fn refresh_engine_roots(&mut self) {
        self.heap.sync_engine_roots(&self.globals, &self.symbols);
    }

    fn library_global_name(name: &crate::LibraryName, binding: &str) -> String {
        format!("\u{1f}library:{}:{binding}", name)
    }

    /// Loads and registers a source through the installed host capability.
    ///
    /// The optional including source tells the loader how to interpret a
    /// relative request. Its canonical identity is passed to the loader.
    pub fn load_source(
        &mut self,
        requested: &str,
        including: Option<SourceId>,
    ) -> Result<SourceId, Error> {
        self.load_source_text(requested, including)
            .map(|(source, _)| source)
    }

    fn load_source_text(
        &mut self,
        requested: &str,
        including: Option<SourceId>,
    ) -> Result<(SourceId, String), Error> {
        let including_identity = match including {
            Some(id) => self
                .sources
                .entry(id)?
                .canonical_identity()
                .map(str::to_owned),
            None => None,
        };
        let loader = self.source_loader.as_mut().ok_or_else(|| {
            Error::plain(
                ErrorKind::SourceLoadingDenied,
                "source loading is denied because no loader is installed",
            )
        })?;
        let loaded = loader
            .load(SourceRequest::new(requested, including_identity.as_deref()))
            .map_err(|cause| {
                Error::from_diagnostic(
                    crate::Diagnostic::new(
                        ErrorKind::SourceLoadFailed,
                        format!("failed to load source '{requested}'"),
                    )
                    .with_cause(cause.to_string()),
                )
            })?;
        let text = loaded.text().to_owned();
        let source = self.register_loaded_source(loaded)?;
        Ok((source, text))
    }

    /// Resolves the start of a span to a one-based source location.
    pub fn source_location(&self, span: Span) -> Result<SourceLocation, Error> {
        self.sources.locate(span)
    }

    /// Renders an error deterministically without terminal styling.
    #[must_use]
    pub fn render_error(&self, error: &Error) -> String {
        let diagnostic = error.diagnostic();
        let mut rendered = format!(
            "error[{}]: {}\n",
            diagnostic.kind().code(),
            diagnostic.message()
        );
        for label in diagnostic.labels() {
            self.render_label(&mut rendered, label);
        }
        if let Some(suggestion) = diagnostic.suggestion() {
            let _ = writeln!(rendered, "help: {suggestion}");
        }
        for note in diagnostic.notes() {
            let _ = writeln!(rendered, "note: {note}");
        }
        if let Some(cause) = diagnostic.cause() {
            let _ = writeln!(rendered, "caused by: {cause}");
        }
        rendered
    }

    fn register_loaded_source(&mut self, loaded: LoadedSource) -> Result<SourceId, Error> {
        self.enforce_source_size(loaded.text().len())?;
        self.sources.add(
            loaded.display_name().to_owned(),
            Some(loaded.canonical_identity().to_owned()),
            loaded.text().to_owned(),
        )
    }

    fn enforce_source_size(&self, actual: usize) -> Result<(), Error> {
        let maximum = self.config.limits().max_source_bytes();
        if actual > maximum {
            return Err(Error::plain(
                ErrorKind::SourceTooLarge,
                format!("source contains {actual} bytes, exceeding the {maximum}-byte limit"),
            ));
        }
        Ok(())
    }

    fn render_label(&self, output: &mut String, label: &DiagnosticLabel) {
        let Ok(entry) = self.sources.validate_span(label.span()) else {
            let _ = writeln!(output, " --> <unavailable>: {}", label.message());
            return;
        };
        let location = entry.location(label.span().start());
        let marker = match label.style() {
            LabelStyle::Primary => '^',
            LabelStyle::Secondary => '-',
        };
        let _ = writeln!(
            output,
            " --> {}:{}:{}",
            entry.name(),
            location.line(),
            location.column()
        );
        if let Some(line) = entry.line_text(location.line()) {
            let width = location.line().to_string().len();
            let _ = writeln!(output, "{:>width$} | {line}", location.line());
            let marker_count = self.marker_width(entry, label.span(), location.line());
            let padding = " ".repeat(location.column().saturating_sub(1));
            let markers: String = std::iter::repeat_n(marker, marker_count).collect();
            let _ = writeln!(
                output,
                "{:>width$} | {padding}{markers} {}",
                "",
                label.message()
            );
        } else {
            let _ = writeln!(output, "     = {}", label.message());
        }
    }

    fn marker_width(&self, entry: &crate::source::SourceEntry, span: Span, line: usize) -> usize {
        let end = entry.location(span.end());
        if end.line() == line {
            end.column()
                .saturating_sub(entry.location(span.start()).column())
                .max(1)
        } else {
            1
        }
    }
}

impl std::fmt::Debug for Engine {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Engine")
            .field("config", &self.config)
            .field("interrupt_token", &self.interrupt_token)
            .field("sources", &self.sources)
            .field("has_source_loader", &self.source_loader.is_some())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        Diagnostic, Engine, EngineConfig, Error, ErrorKind, LabelStyle, SourceRetention, Span,
    };

    #[test]
    fn renders_full_and_metadata_diagnostics_deterministically() {
        let mut full = Engine::new(EngineConfig::default()).unwrap();
        let source = full.add_source("unicode.scm", "(λx)\r\nbad").unwrap();
        let span = Span::new(source, 1, 3).unwrap();
        let diagnostic = Diagnostic::new(ErrorKind::InvalidSpan, "example").with_label(
            span,
            LabelStyle::Primary,
            "here",
        );
        let error = Error::from_diagnostic(diagnostic);
        assert_eq!(
            full.render_error(&error),
            "error[source.invalid-span]: example\n --> unicode.scm:1:2\n1 | (λx)\n  |  ^ here\n"
        );

        let mut metadata =
            Engine::new(EngineConfig::default().with_source_retention(SourceRetention::Metadata))
                .unwrap();
        let source = metadata.add_source("unicode.scm", "(λx)").unwrap();
        let span = Span::new(source, 1, 3).unwrap();
        let diagnostic = Diagnostic::new(ErrorKind::InvalidSpan, "example").with_label(
            span,
            LabelStyle::Secondary,
            "context",
        );
        let error = Error::from_diagnostic(diagnostic);
        assert_eq!(
            metadata.render_error(&error),
            "error[source.invalid-span]: example\n --> unicode.scm:1:2\n     = context\n"
        );
    }
}
