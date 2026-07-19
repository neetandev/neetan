//! Standard-library installation and the bootstrap Scheme definitions executed
//! when an engine is constructed.

use super::Engine;
use crate::{EngineConfig, Error, InterruptToken};

impl Engine {
    pub(super) fn install_standard_library_aliases(&mut self) {
        const LIBRARIES: &[&[&str]] = &[
            &["scheme", "base"],
            &["scheme", "case-lambda"],
            &["scheme", "char"],
            &["scheme", "complex"],
            &["scheme", "cxr"],
            &["scheme", "eval"],
            &["scheme", "file"],
            &["scheme", "inexact"],
            &["scheme", "lazy"],
            &["scheme", "load"],
            &["scheme", "process-context"],
            &["scheme", "read"],
            &["scheme", "repl"],
            &["scheme", "time"],
            &["scheme", "write"],
            &["scheme", "r5rs"],
        ];
        for parts in LIBRARIES {
            let name = crate::LibraryName::new(
                parts
                    .iter()
                    .map(|part| crate::LibraryNameComponent::identifier(*part)),
            )
            .expect("standard library name");
            for export in crate::library::standard_exports(&name).expect("standard manifest") {
                let binding = match export {
                    "exact->inexact" => "inexact",
                    "inexact->exact" => "exact",
                    other => other,
                };
                if let Some(value) = self.globals.get(binding).copied() {
                    self.globals
                        .insert(format!("\u{1f}library:(scheme base):{binding}"), value);
                }
            }
        }
    }

    pub(super) fn install_standard_library_bindings(&mut self) -> Result<(), Error> {
        const LIBRARIES: &[&[&str]] = &[
            &["scheme", "base"],
            &["scheme", "case-lambda"],
            &["scheme", "char"],
            &["scheme", "complex"],
            &["scheme", "cxr"],
            &["scheme", "eval"],
            &["scheme", "file"],
            &["scheme", "inexact"],
            &["scheme", "lazy"],
            &["scheme", "load"],
            &["scheme", "process-context"],
            &["scheme", "read"],
            &["scheme", "repl"],
            &["scheme", "time"],
            &["scheme", "write"],
            &["scheme", "r5rs"],
        ];
        for name in [
            "call-with-input-file",
            "call-with-output-file",
            "delete-file",
            "file-exists?",
            "open-binary-input-file",
            "open-binary-output-file",
            "open-input-file",
            "open-output-file",
            "with-input-from-file",
            "with-output-to-file",
            "load",
            "command-line",
            "emergency-exit",
            "exit",
            "get-environment-variable",
            "get-environment-variables",
            "current-jiffy",
            "current-second",
            "jiffies-per-second",
        ] {
            self.natives
                .register_capability_denied(&mut self.heap, &mut self.globals, name)?;
        }
        for parts in LIBRARIES {
            let name = crate::LibraryName::new(
                parts
                    .iter()
                    .map(|part| crate::LibraryNameComponent::identifier(*part)),
            )?;
            for export in crate::library::standard_exports(&name).expect("standard manifest") {
                if matches!(export, "exact->inexact" | "inexact->exact") {
                    continue;
                }
                self.natives
                    .register_unsupported(&mut self.heap, &mut self.globals, export)?;
            }
        }
        if let Some(value) = self.globals.get("inexact").copied() {
            self.globals.insert("exact->inexact".to_owned(), value);
        }
        if let Some(value) = self.globals.get("exact").copied() {
            self.globals.insert("inexact->exact".to_owned(), value);
        }
        Ok(())
    }

    pub(super) fn install_standard_definitions(&mut self) -> Result<(), Error> {
        // Bootstrap code is part of the implementation, not guest input. It
        // must not consume a host's deliberately tiny source, compiler, fuel,
        // or interruption allowance before the engine is returned.
        let bootstrap = EngineConfig::default();
        let source = self.sources.add(
            "(r7rs standard procedures)".to_owned(),
            None,
            STANDARD_DEFINITIONS.to_owned(),
        )?;
        let mut reader = crate::Reader::new(source, STANDARD_DEFINITIONS.to_owned(), &bootstrap);
        let forms = crate::frontend::read_forms(&mut reader)?;
        let expression = crate::expand::expand_forms_with_features(
            &forms,
            bootstrap.limits(),
            std::collections::HashMap::new(),
            bootstrap.features(),
        )?;
        let module = crate::compile::compile(&expression, bootstrap.limits())?;
        let interrupt = InterruptToken::new();
        crate::vm::execute(
            &module,
            &mut self.heap,
            &mut self.register_stack,
            &mut self.globals,
            &mut self.symbols,
            &self.natives,
            bootstrap.limits(),
            &interrupt,
            &bootstrap,
            &mut self.source_loader,
            &mut self.sources,
        )?;
        Ok(())
    }

    pub(super) fn install_default_ports(&mut self) -> Result<(), Error> {
        let input = self.heap.ports_mut().text_input(String::new())?;
        let output = self.heap.ports_mut().new_text_output()?;
        let error = self.heap.ports_mut().new_text_output()?;
        for (name, id) in [
            ("current-input-port", input),
            ("current-output-port", output),
            ("current-error-port", error),
        ] {
            let port = self
                .heap
                .alloc(crate::heap::Object::Port(crate::port::PortObject { id }))?;
            let parameter = self.heap.alloc(crate::heap::Object::Parameter(Box::new(
                crate::heap::Parameter {
                    value: port,
                    converter: None,
                },
            )))?;
            self.globals.insert(name.to_owned(), parameter);
        }
        Ok(())
    }

    pub(super) fn install_configured_features(&mut self) -> Result<(), Error> {
        *self.feature_identifiers.borrow_mut() = self
            .config
            .features()
            .identifiers()
            .into_iter()
            .map(str::to_owned)
            .collect();
        let features = std::rc::Rc::clone(&self.feature_identifiers);
        self.natives.register(
            &mut self.heap,
            &mut self.globals,
            "features".to_owned(),
            0..=0,
            move |context: &mut crate::NativeContext<'_>, _arguments: &[crate::Value]| {
                let mut result = crate::Value::nil();
                for feature in features.borrow().iter().rev() {
                    let symbol = context.intern_symbol(feature)?;
                    result = context.pair(symbol, result)?;
                }
                Ok(result)
            },
        )
    }

    /// Refreshes the shared `features` list from the current configuration.
    /// Called after an extension enables its feature identifier.
    pub(super) fn refresh_feature_identifiers(&mut self) {
        *self.feature_identifiers.borrow_mut() = self
            .config
            .features()
            .identifiers()
            .into_iter()
            .map(str::to_owned)
            .collect();
    }
}

const STANDARD_DEFINITIONS: &str = r#"
(define (null-environment version) (environment '(scheme base)))
(define (scheme-report-environment version) (environment '(scheme r5rs)))
(define (member object list . compare)
  (if (null? compare)
      (%member object list)
      (let ((same? (car compare)))
        (let loop ((list list))
          (if (null? list) #f
              (if (same? object (car list)) list (loop (cdr list))))))))
(define (assoc object alist . compare)
  (if (null? compare)
      (%assoc object alist)
      (let ((same? (car compare)))
        (let loop ((alist alist))
          (if (null? alist) #f
              (if (same? object (car (car alist))) (car alist) (loop (cdr alist))))))))
(define (%map-car lists)
  (if (null? lists) '() (cons (car (car lists)) (%map-car (cdr lists)))))
(define (%map-cdr lists)
  (if (null? lists) '() (cons (cdr (car lists)) (%map-cdr (cdr lists)))))
(define (%any-null? lists)
  (if (null? lists) #f
      (if (null? (car lists)) #t (%any-null? (cdr lists)))))
(define (map procedure . lists)
  (if (%any-null? lists) '()
      (cons (apply procedure (%map-car lists))
            (apply map procedure (%map-cdr lists)))))
(define (for-each procedure . lists)
  (if (%any-null? lists) (if #f #f)
      (begin (apply procedure (%map-car lists))
             (apply for-each procedure (%map-cdr lists)))))
(define (string-map procedure first . rest)
  (list->string (apply map procedure (string->list first) (map string->list rest))))
(define (string-for-each procedure first . rest)
  (apply for-each procedure (string->list first) (map string->list rest)))
(define (vector-map procedure first . rest)
  (list->vector (apply map procedure (vector->list first) (map vector->list rest))))
(define (vector-for-each procedure first . rest)
  (apply for-each procedure (vector->list first) (map vector->list rest)))
"#;
