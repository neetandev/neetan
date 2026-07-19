//! Import-set resolution and dependency-ordered library initialization.

use super::Engine;
use crate::{Error, ErrorKind};

impl Engine {
    pub(super) fn resolve_imports(
        &mut self,
        imports: &[crate::library::ImportSet],
    ) -> Result<crate::library::LibraryBindings, Error> {
        self.resolve_imports_with_path(imports, &mut Vec::new())
    }

    fn resolve_imports_with_path(
        &mut self,
        imports: &[crate::library::ImportSet],
        path: &mut Vec<crate::LibraryName>,
    ) -> Result<crate::library::LibraryBindings, Error> {
        let mut result = crate::library::LibraryBindings::default();
        for import in imports {
            let imported = self.resolve_import_set_with_path(import, path)?;
            for (name, binding) in imported.values {
                if result.macros.contains_key(&name) {
                    return Err(Error::plain(
                        ErrorKind::LibraryError,
                        format!("duplicate or ambiguous imported binding '{name}'"),
                    ));
                }
                if let Some(existing) = result.values.get(&name) {
                    if existing != &binding {
                        return Err(Error::plain(
                            ErrorKind::LibraryError,
                            format!("duplicate or ambiguous imported binding '{name}'"),
                        ));
                    }
                } else {
                    result.values.insert(name, binding);
                }
            }
            for (name, transformer) in imported.macros {
                if result.macros.insert(name.clone(), transformer).is_some()
                    || result.values.contains_key(&name)
                {
                    return Err(Error::plain(
                        ErrorKind::LibraryError,
                        format!("duplicate or ambiguous imported binding '{name}'"),
                    ));
                }
            }
        }
        Ok(result)
    }

    fn resolve_import_set_with_path(
        &mut self,
        import: &crate::library::ImportSet,
        path: &mut Vec<crate::LibraryName>,
    ) -> Result<crate::library::LibraryBindings, Error> {
        use crate::library::ImportSet;
        let bindings = match import {
            ImportSet::Library(name) if crate::library::standard_exports(name).is_some() => self
                .standard_library_bindings(name)
                .map(|values| crate::library::LibraryBindings {
                    values,
                    ..Default::default()
                })
                .ok_or_else(|| {
                    Error::plain(
                        ErrorKind::LibraryNotFound,
                        format!("standard library {name} is unavailable"),
                    )
                })?,
            ImportSet::Library(name) => {
                if let Some(bindings) = self.libraries.resolve_native(name) {
                    return Ok(bindings);
                }
                self.initialize_library(name, path)?;
                match self.libraries.state(name)? {
                    crate::library::LibraryState::Ready(values) => values,
                    crate::library::LibraryState::Failed(error) => return Err(error),
                    _ => {
                        return Err(Error::plain(
                            ErrorKind::LibraryError,
                            format!("library {name} did not become ready"),
                        ));
                    }
                }
            }
            ImportSet::Only(set, names) => {
                let available = self.resolve_import_set_with_path(set, path)?;
                let mut selected = crate::library::LibraryBindings::default();
                for name in names {
                    if let Some(binding) = available.values.get(name).cloned() {
                        if selected.values.insert(name.clone(), binding).is_some() {
                            return Err(Error::plain(
                                ErrorKind::LibraryError,
                                format!("duplicate only import '{name}'"),
                            ));
                        }
                    } else if let Some(transformer) = available.macros.get(name).cloned() {
                        if selected.macros.insert(name.clone(), transformer).is_some() {
                            return Err(Error::plain(
                                ErrorKind::LibraryError,
                                format!("duplicate only import '{name}'"),
                            ));
                        }
                    } else {
                        return Err(Error::plain(
                            ErrorKind::LibraryError,
                            format!("import set does not export '{name}'"),
                        ));
                    }
                    if selected.values.contains_key(name) && selected.macros.contains_key(name) {
                        return Err(Error::plain(
                            ErrorKind::LibraryError,
                            format!("duplicate only import '{name}'"),
                        ));
                    }
                }
                selected
            }
            ImportSet::Except(set, names) => {
                let mut available = self.resolve_import_set_with_path(set, path)?;
                for name in names {
                    if available.values.remove(name).is_none()
                        && available.macros.remove(name).is_none()
                    {
                        return Err(Error::plain(
                            ErrorKind::LibraryError,
                            format!("import set does not export '{name}'"),
                        ));
                    }
                }
                available
            }
            ImportSet::Prefix(set, prefix) => {
                let available = self.resolve_import_set_with_path(set, path)?;
                crate::library::LibraryBindings {
                    values: available
                        .values
                        .into_iter()
                        .map(|(name, binding)| (format!("{prefix}{name}"), binding))
                        .collect(),
                    macros: available
                        .macros
                        .into_iter()
                        .map(|(name, transformer)| (format!("{prefix}{name}"), transformer))
                        .collect(),
                }
            }
            ImportSet::Rename(set, renames) => {
                let mut available = self.resolve_import_set_with_path(set, path)?;
                for (from, to) in renames {
                    if let Some(binding) = available.values.remove(from) {
                        if available.values.insert(to.clone(), binding).is_some()
                            || available.macros.contains_key(to)
                        {
                            return Err(Error::plain(
                                ErrorKind::LibraryError,
                                format!("rename creates duplicate import '{to}'"),
                            ));
                        }
                    } else if let Some(transformer) = available.macros.remove(from) {
                        if available.macros.insert(to.clone(), transformer).is_some()
                            || available.values.contains_key(to)
                        {
                            return Err(Error::plain(
                                ErrorKind::LibraryError,
                                format!("rename creates duplicate import '{to}'"),
                            ));
                        }
                    } else {
                        return Err(Error::plain(
                            ErrorKind::LibraryError,
                            format!("import set does not export '{from}'"),
                        ));
                    }
                }
                available
            }
        };
        Ok(bindings)
    }

    fn initialize_library(
        &mut self,
        name: &crate::LibraryName,
        path: &mut Vec<crate::LibraryName>,
    ) -> Result<(), Error> {
        use crate::library::LibraryState;
        if self.is_base_library(name) {
            return Ok(());
        }
        match self.libraries.state(name)? {
            LibraryState::Ready(_) => return Ok(()),
            LibraryState::Failed(error) => return Err(error),
            LibraryState::Expanding | LibraryState::Initializing => {
                let start = path.iter().position(|item| item == name).unwrap_or(0);
                let mut cycle: Vec<String> =
                    path[start..].iter().map(ToString::to_string).collect();
                cycle.push(name.to_string());
                let error = Error::plain(
                    ErrorKind::LibraryCycle,
                    format!("library dependency cycle: {}", cycle.join(" -> ")),
                );
                self.libraries
                    .set_state(name, LibraryState::Failed(error.clone()))?;
                return Err(error);
            }
            LibraryState::Declared | LibraryState::Compiled(..) => {}
        }
        let declaration = self.libraries.declaration(name)?;
        self.libraries.set_state(name, LibraryState::Expanding)?;
        path.push(name.clone());
        let compiled = (|| {
            let imported = self.resolve_imports_with_path(&declaration.imports, path)?;
            let mut bindings = imported;
            let mut mutable_values = std::collections::HashSet::new();
            let syntax_names = crate::library::syntax_definition_names(&declaration.body);
            for defined in crate::library::definition_names(&declaration.body) {
                let global = Self::library_global_name(name, &defined);
                mutable_values.insert(global.clone());
                bindings.values.insert(defined.clone(), global);
            }
            for export in &declaration.exports {
                if !syntax_names.contains(&export.internal)
                    && !bindings.macros.contains_key(&export.internal)
                {
                    let global = Self::library_global_name(name, &export.internal);
                    mutable_values.insert(global.clone());
                    bindings
                        .values
                        .entry(export.internal.clone())
                        .or_insert(global);
                }
            }
            let expanded = crate::expand::expand_forms_with_imports_and_mutable(
                &declaration.body,
                self.config.limits(),
                bindings.values.clone(),
                bindings.macros.clone(),
                self.config.features(),
                &mutable_values,
            )?;
            let module = crate::compile::compile(&expanded.expression, self.config.limits())?;
            let mut exports = crate::library::LibraryBindings::default();
            for export in &declaration.exports {
                if let Some(binding) = bindings.values.get(&export.internal).cloned() {
                    exports.values.insert(export.external.clone(), binding);
                }
                if let Some(transformer) = expanded.macros.get(&export.internal).cloned() {
                    exports.macros.insert(export.external.clone(), transformer);
                }
                if !exports.values.contains_key(&export.external)
                    && !exports.macros.contains_key(&export.external)
                {
                    return Err(Error::plain(
                        ErrorKind::LibraryError,
                        format!(
                            "export '{}' is not defined by library {name}",
                            export.internal
                        ),
                    ));
                }
            }
            Ok::<_, Error>((module, exports))
        })();
        path.pop();
        let (module, exports) = match compiled {
            Ok(value) => value,
            Err(error) => {
                self.libraries
                    .set_state(name, LibraryState::Failed(error.clone()))?;
                return Err(error);
            }
        };
        self.libraries.set_state(
            name,
            LibraryState::Compiled(module.clone(), exports.clone()),
        )?;
        self.libraries.set_state(name, LibraryState::Initializing)?;
        let initialized = self.eval(&module).map(|_| ());
        match initialized {
            Ok(()) => self.libraries.set_state(name, LibraryState::Ready(exports)),
            Err(error) => {
                self.libraries
                    .set_state(name, LibraryState::Failed(error.clone()))?;
                Err(error)
            }
        }
    }

    fn is_base_library(&self, name: &crate::LibraryName) -> bool {
        matches!(
            name.components(),
            [crate::LibraryNameComponent::Identifier(first), crate::LibraryNameComponent::Identifier(second)]
                if first == "scheme" && second == "base"
        )
    }

    fn standard_library_bindings(
        &self,
        name: &crate::LibraryName,
    ) -> Option<std::collections::HashMap<String, String>> {
        let mut bindings = std::collections::HashMap::new();
        for export in crate::library::standard_exports(name)? {
            let binding = match export {
                "exact->inexact" => "inexact",
                "inexact->exact" => "exact",
                other => other,
            };
            if self.globals.contains_key(binding) {
                bindings.insert(
                    export.to_owned(),
                    format!("\u{1f}library:(scheme base):{binding}"),
                );
            }
        }
        Some(bindings)
    }
}
