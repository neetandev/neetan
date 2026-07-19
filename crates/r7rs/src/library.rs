//! R7RS library names and engine-local library registration.

use std::{collections::HashMap, fmt};

use crate::{
    CompiledModule, Error, ErrorKind, SourceId,
    expand::{Form, FormKind},
};

/// One component of an R7RS library name.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum LibraryNameComponent {
    /// An identifier component.
    Identifier(String),
    /// A non-negative numeric component.
    Number(u64),
}

impl LibraryNameComponent {
    /// Creates an identifier component.
    #[must_use]
    pub fn identifier(value: impl Into<String>) -> Self {
        Self::Identifier(value.into())
    }

    /// Creates a numeric component.
    #[must_use]
    pub const fn number(value: u64) -> Self {
        Self::Number(value)
    }
}

/// The structured name of an R7RS library.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct LibraryName(Vec<LibraryNameComponent>);

impl LibraryName {
    /// Creates a non-empty library name.
    pub fn new(parts: impl IntoIterator<Item = LibraryNameComponent>) -> Result<Self, Error> {
        let parts: Vec<_> = parts.into_iter().collect();
        if parts.is_empty() {
            return Err(Error::plain(
                ErrorKind::LibraryError,
                "a library name must contain at least one component",
            ));
        }
        if parts
            .iter()
            .any(|part| matches!(part, LibraryNameComponent::Identifier(value) if value.is_empty()))
        {
            return Err(Error::plain(
                ErrorKind::LibraryError,
                "a library-name identifier must not be empty",
            ));
        }
        Ok(Self(parts))
    }

    /// Returns the components in source order.
    #[must_use]
    pub fn components(&self) -> &[LibraryNameComponent] {
        &self.0
    }

    /// Returns the canonical display form used in diagnostics.
    #[must_use]
    pub fn display_name(&self) -> String {
        self.to_string()
    }
}

impl fmt::Display for LibraryName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("(")?;
        for (index, part) in self.0.iter().enumerate() {
            if index != 0 {
                formatter.write_str(" ")?;
            }
            match part {
                LibraryNameComponent::Identifier(value) => formatter.write_str(value)?,
                LibraryNameComponent::Number(value) => write!(formatter, "{value}")?,
            }
        }
        formatter.write_str(")")
    }
}

#[derive(Clone, Debug)]
pub(crate) enum RegisteredLibrary {
    Source {
        declaration: LibraryDeclaration,
        state: LibraryState,
    },
    Native {
        bindings: LibraryBindings,
        sealed: bool,
    },
}

#[derive(Clone, Debug)]
pub(crate) enum LibraryState {
    Declared,
    Expanding,
    #[allow(dead_code)]
    Compiled(CompiledModule, LibraryBindings),
    Initializing,
    Ready(LibraryBindings),
    Failed(Error),
}

#[derive(Clone, Debug, Default)]
pub(crate) struct LibraryBindings {
    pub(crate) values: HashMap<String, String>,
    pub(crate) macros: HashMap<String, crate::expand::Macro>,
}

#[derive(Clone, Debug)]
pub(crate) struct LibraryDeclaration {
    pub(crate) name: LibraryName,
    pub(crate) exports: Vec<Export>,
    pub(crate) imports: Vec<ImportSet>,
    pub(crate) body: Vec<Form>,
}

#[derive(Clone, Debug)]
pub(crate) struct Export {
    pub(crate) internal: String,
    pub(crate) external: String,
}

#[derive(Clone, Debug)]
pub(crate) enum ImportSet {
    Library(LibraryName),
    Only(Box<Self>, Vec<String>),
    Except(Box<Self>, Vec<String>),
    Prefix(Box<Self>, String),
    Rename(Box<Self>, Vec<(String, String)>),
}

pub(crate) fn parse_declaration(forms: &[Form]) -> Result<LibraryDeclaration, Error> {
    if forms.len() != 1 {
        return Err(Error::plain(
            ErrorKind::LibraryError,
            "a registered library source must contain exactly one define-library declaration",
        ));
    }
    let items = forms[0].proper_list().ok_or_else(|| {
        Error::plain(
            ErrorKind::LibraryError,
            "define-library must be a proper list",
        )
    })?;
    if items.len() < 2 || items[0].symbol() != Some("define-library") {
        return Err(Error::plain(
            ErrorKind::LibraryError,
            "a registered library source must begin with define-library",
        ));
    }
    let name = parse_name(&items[1])?;
    let mut exports = Vec::new();
    let mut imports = Vec::new();
    let mut body = Vec::new();
    for declaration in &items[2..] {
        let declaration_items = declaration.proper_list().ok_or_else(|| {
            Error::plain(
                ErrorKind::LibraryError,
                "library declaration must be a proper list",
            )
        })?;
        let Some(keyword) = declaration_items.first().and_then(Form::symbol) else {
            return Err(Error::plain(
                ErrorKind::LibraryError,
                "library declaration needs a keyword",
            ));
        };
        match keyword {
            "export" => exports.extend(parse_exports(&declaration_items[1..])?),
            "import" => {
                for set in &declaration_items[1..] {
                    imports.push(parse_import_set(set)?);
                }
            }
            "begin" => body.extend_from_slice(&declaration_items[1..]),
            "include" | "include-ci" | "cond-expand" => {
                return Err(Error::plain(
                    ErrorKind::UnsupportedSyntax,
                    format!("library declaration '{keyword}' requires source expansion support"),
                ));
            }
            _ => {
                return Err(Error::plain(
                    ErrorKind::LibraryError,
                    format!("unknown library declaration '{keyword}'"),
                ));
            }
        }
    }
    let mut names = std::collections::HashSet::new();
    for export in &exports {
        if !names.insert(export.external.clone()) {
            return Err(Error::plain(
                ErrorKind::LibraryError,
                format!("duplicate export '{}'", export.external),
            ));
        }
    }
    Ok(LibraryDeclaration {
        name,
        exports,
        imports,
        body,
    })
}

/// Replaces library `include` and `include-ci` declarations with a `begin`
/// declaration containing the datums supplied by the injected source loader.
pub(crate) fn inline_includes(
    forms: &[Form],
    features: &crate::FeatureSet,
    root_source: crate::SourceId,
    limits: &crate::Limits,
    mut load: impl FnMut(&str, bool, crate::SourceId) -> Result<(crate::SourceId, Vec<Form>), Error>,
) -> Result<Vec<Form>, Error> {
    if forms.len() != 1 {
        return Ok(forms.to_vec());
    }
    let Some(items) = forms[0].proper_list() else {
        return Ok(forms.to_vec());
    };
    if items.len() < 2 || items[0].symbol() != Some("define-library") {
        return Ok(forms.to_vec());
    }
    let mut declarations = items[..2].to_vec();
    let mut ancestry = vec![root_source];
    expand_library_declarations(
        &items[2..],
        root_source,
        features,
        limits,
        &mut ancestry,
        &mut load,
        &mut declarations,
        0,
    )?;
    Ok(vec![Form {
        kind: FormKind::List(declarations, None),
        span: forms[0].span,
    }])
}

#[allow(clippy::too_many_arguments)]
fn expand_library_declarations(
    input: &[Form],
    including_source: crate::SourceId,
    features: &crate::FeatureSet,
    limits: &crate::Limits,
    ancestry: &mut Vec<crate::SourceId>,
    load: &mut impl FnMut(&str, bool, crate::SourceId) -> Result<(crate::SourceId, Vec<Form>), Error>,
    output: &mut Vec<Form>,
    depth: usize,
) -> Result<(), Error> {
    if depth > limits.max_expansion_depth() {
        return Err(Error::plain(
            ErrorKind::ExpansionLimitExceeded,
            "library declaration inclusion depth exceeded",
        ));
    }
    for declaration in input {
        let Some(parts) = declaration.proper_list() else {
            output.push(declaration.clone());
            continue;
        };
        let Some(keyword) = parts.first().and_then(Form::symbol) else {
            output.push(declaration.clone());
            continue;
        };
        if keyword == "cond-expand" {
            let selected = select_cond_expand(&parts[1..], features)?;
            expand_library_declarations(
                &selected,
                including_source,
                features,
                limits,
                ancestry,
                load,
                output,
                depth + 1,
            )?;
            continue;
        }
        if keyword == "include-library-declarations" {
            if parts.len() < 2 {
                return Err(Error::plain(
                    ErrorKind::LibraryError,
                    "include-library-declarations requires at least one path",
                ));
            }
            for path in &parts[1..] {
                let FormKind::String(path) = &path.kind else {
                    return Err(Error::plain(
                        ErrorKind::LibraryError,
                        "include-library-declarations path must be a string",
                    ));
                };
                let (source, contents) = load(path, false, including_source)?;
                if ancestry.contains(&source) {
                    return Err(Error::plain(
                        ErrorKind::LibraryError,
                        "cyclic include-library-declarations",
                    ));
                }
                ancestry.push(source);
                let result = expand_library_declarations(
                    &contents,
                    source,
                    features,
                    limits,
                    ancestry,
                    load,
                    output,
                    depth + 1,
                );
                ancestry.pop();
                result?;
            }
            continue;
        }
        if !matches!(keyword, "include" | "include-ci") {
            output.push(declaration.clone());
            continue;
        }
        if parts.len() < 2 {
            return Err(Error::plain(
                ErrorKind::LibraryError,
                format!("{keyword} requires at least one path"),
            ));
        }
        let mut contents = Vec::new();
        for path in &parts[1..] {
            let FormKind::String(path) = &path.kind else {
                return Err(Error::plain(
                    ErrorKind::LibraryError,
                    "include path must be a string",
                ));
            };
            let (_, loaded) = load(path, keyword == "include-ci", including_source)?;
            contents.extend(loaded);
        }
        let mut begin = Vec::with_capacity(contents.len() + 1);
        begin.push(Form {
            kind: FormKind::Symbol("begin".to_owned()),
            span: declaration.span,
        });
        begin.extend(contents);
        output.push(Form {
            kind: FormKind::List(begin, None),
            span: declaration.span,
        });
    }
    Ok(())
}

fn select_cond_expand(clauses: &[Form], features: &crate::FeatureSet) -> Result<Vec<Form>, Error> {
    let mut selected = None;
    for (index, clause) in clauses.iter().enumerate() {
        let parts = clause.proper_list().ok_or_else(|| {
            Error::plain(
                ErrorKind::LibraryError,
                "cond-expand clause must be a proper list",
            )
        })?;
        if parts.is_empty() {
            return Err(Error::plain(
                ErrorKind::LibraryError,
                "cond-expand clause must not be empty",
            ));
        }
        let matched = if parts[0].symbol() == Some("else") {
            if index + 1 != clauses.len() {
                return Err(Error::plain(
                    ErrorKind::LibraryError,
                    "cond-expand else clause must be last",
                ));
            }
            true
        } else {
            requirement_matches(&parts[0], features)?
        };
        if matched && selected.is_none() {
            selected = Some(parts[1..].to_vec());
        }
    }
    selected.ok_or_else(|| {
        Error::plain(
            ErrorKind::LibraryError,
            "no library cond-expand clause matched",
        )
    })
}

fn requirement_matches(requirement: &Form, features: &crate::FeatureSet) -> Result<bool, Error> {
    if let Some(identifier) = requirement.symbol() {
        return Ok(features.contains(identifier));
    }
    let parts = requirement
        .proper_list()
        .ok_or_else(|| Error::plain(ErrorKind::LibraryError, "invalid cond-expand requirement"))?;
    let Some(keyword) = parts.first().and_then(Form::symbol) else {
        return Err(Error::plain(
            ErrorKind::LibraryError,
            "invalid cond-expand requirement",
        ));
    };
    match keyword {
        "and" => parts[1..]
            .iter()
            .map(|part| requirement_matches(part, features))
            .collect::<Result<Vec<_>, _>>()
            .map(|parts| parts.into_iter().all(|part| part)),
        "or" => parts[1..]
            .iter()
            .map(|part| requirement_matches(part, features))
            .collect::<Result<Vec<_>, _>>()
            .map(|parts| parts.into_iter().any(|part| part)),
        "not" if parts.len() == 2 => Ok(!requirement_matches(&parts[1], features)?),
        "library" if parts.len() == 2 => {
            let name = parse_name(&parts[1])?;
            Ok(features.contains_library(&name))
        }
        _ => Err(Error::plain(
            ErrorKind::LibraryError,
            "invalid cond-expand feature requirement",
        )),
    }
}

/// Splits the leading import declarations from a program body.
pub(crate) fn program_imports(forms: &[Form]) -> Result<(Vec<ImportSet>, Vec<Form>), Error> {
    let mut imports = Vec::new();
    let mut body_start = 0;
    for form in forms {
        let Some(items) = form.proper_list() else {
            break;
        };
        if items.first().and_then(Form::symbol) != Some("import") {
            break;
        }
        for set in &items[1..] {
            imports.push(parse_import_set(set)?);
        }
        body_start += 1;
    }
    if forms[body_start..].iter().any(|form| {
        form.proper_list()
            .and_then(|items| items.first())
            .and_then(Form::symbol)
            == Some("import")
    }) {
        return Err(Error::plain(
            ErrorKind::LibraryError,
            "import declarations must precede all program expressions",
        ));
    }
    Ok((imports, forms[body_start..].to_vec()))
}

/// Returns names introduced by direct top-level value definitions.
pub(crate) fn definition_names(forms: &[Form]) -> Vec<String> {
    let mut names = Vec::new();
    for form in forms {
        let Some(items) = form.proper_list() else {
            continue;
        };
        match items.first().and_then(Form::symbol) {
            Some("define") if items.len() >= 2 => {
                if let Some(name) = items[1].symbol() {
                    names.push(name.to_owned());
                } else if let Some(signature) = items[1].proper_list()
                    && let Some(name) = signature.first().and_then(Form::symbol)
                {
                    names.push(name.to_owned());
                }
            }
            Some("define-values") if items.len() >= 2 => {
                push_formal_names(&items[1], &mut names);
            }
            Some("define-record-type") if items.len() >= 4 => {
                // The record type name is not a runtime binding, so it is left
                // out. The constructor, predicate, accessors, and mutators are
                // ordinary top-level definitions and must be scoped like any
                // define, otherwise they leak past the export list.
                if let Some(constructor) = items[2].proper_list()
                    && let Some(name) = constructor.first().and_then(Form::symbol)
                {
                    names.push(name.to_owned());
                }
                if let Some(name) = items[3].symbol() {
                    names.push(name.to_owned());
                }
                for field in &items[4..] {
                    let Some(specification) = field.proper_list() else {
                        continue;
                    };
                    if let Some(accessor) = specification.get(1).and_then(|form| form.symbol()) {
                        names.push(accessor.to_owned());
                    }
                    if let Some(mutator) = specification.get(2).and_then(|form| form.symbol()) {
                        names.push(mutator.to_owned());
                    }
                }
            }
            _ => continue,
        }
    }
    names.sort_unstable();
    names.dedup();
    names
}

/// Collects the identifiers bound by a `define-values` formals list. The list
/// can be proper, dotted with a rest name, or a single rest name.
fn push_formal_names(formals: &Form, names: &mut Vec<String>) {
    match &formals.kind {
        FormKind::Symbol(name) => names.push(name.clone()),
        FormKind::List(items, tail) => {
            for item in items {
                if let Some(name) = item.symbol() {
                    names.push(name.to_owned());
                }
            }
            if let Some(tail) = tail
                && let Some(name) = tail.symbol()
            {
                names.push(name.to_owned());
            }
        }
        _ => {}
    }
}

/// Returns syntax names introduced by direct top-level `define-syntax` forms.
pub(crate) fn syntax_definition_names(forms: &[Form]) -> Vec<String> {
    let mut names = Vec::new();
    for form in forms {
        let Some(items) = form.proper_list() else {
            continue;
        };
        if items.len() == 3
            && items.first().and_then(Form::symbol) == Some("define-syntax")
            && let Some(name) = items[1].symbol()
        {
            names.push(name.to_owned());
        }
    }
    names.sort_unstable();
    names.dedup();
    names
}

/// Returns the Appendix A export names for a standard library.
///
/// Strings keep this table compact while the checker below turns each entry
/// into a deterministic set. Syntax identifiers are included deliberately:
/// an import surface is a surface of bindings, not merely procedures.
pub(crate) fn standard_exports(name: &LibraryName) -> Option<Vec<&'static str>> {
    let key = name.to_string();
    let names = match key.as_str() {
        "(scheme base)" => BASE_EXPORTS,
        "(scheme case-lambda)" => "case-lambda",
        "(scheme char)" => {
            "char-alphabetic? char-ci<=? char-ci<? char-ci=? char-ci>=? char-ci>? char-downcase char-foldcase char-lower-case? char-numeric? char-upcase char-upper-case? char-whitespace? digit-value string-ci<=? string-ci<? string-ci=? string-ci>=? string-ci>? string-downcase string-foldcase string-upcase"
        }
        "(scheme complex)" => "angle imag-part magnitude make-polar make-rectangular real-part",
        "(scheme cxr)" => {
            "caaaar caaadr caaar caadar caaddr caadr cadaar cadadr cadar caddar cadddr caddr cdaaar cdaadr cdaar cdadar cdaddr cdadr cddaar cddadr cddar cdddar cddddr cdddr"
        }
        "(scheme eval)" => "environment eval",
        "(scheme file)" => {
            "call-with-input-file call-with-output-file delete-file file-exists? open-binary-input-file open-binary-output-file open-input-file open-output-file with-input-from-file with-output-to-file"
        }
        "(scheme inexact)" => "acos asin atan cos exp finite? infinite? log nan? sin sqrt tan",
        "(scheme lazy)" => "delay delay-force force make-promise promise?",
        "(scheme load)" => "load",
        "(scheme process-context)" => {
            "command-line emergency-exit exit get-environment-variable get-environment-variables"
        }
        "(scheme read)" => "read",
        "(scheme repl)" => "interaction-environment",
        "(scheme time)" => "current-jiffy current-second jiffies-per-second",
        "(scheme write)" => "display write write-shared write-simple",
        // R5RS is intentionally constructed from the R7RS compatibility
        // surface plus the two required historical numeric aliases.
        "(scheme r5rs)" => R5RS_EXPORTS,
        _ => return None,
    };
    Some(names.split_whitespace().collect())
}

const BASE_EXPORTS: &str = "* + - ... / < <= = => > >= abs and append apply assoc assq assv begin binary-port? boolean=? boolean? bytevector bytevector-append bytevector-copy bytevector-copy! bytevector-length bytevector-u8-ref bytevector-u8-set! bytevector? caar cadr call-with-current-continuation call-with-port call-with-values call/cc car case cdar cddr cdr ceiling char->integer char-ready? char<=? char<? char=? char>=? char>? char? close-input-port close-output-port close-port complex? cond cond-expand cons current-error-port current-input-port current-output-port define define-record-type define-syntax define-values denominator do dynamic-wind else eof-object eof-object? eq? equal? eqv? error error-object-irritants error-object-message error-object? even? exact exact-integer-sqrt exact-integer? exact? expt features file-error? floor floor-quotient floor-remainder floor/ flush-output-port for-each gcd get-output-bytevector get-output-string guard if include include-ci inexact inexact? input-port-open? input-port? integer->char integer? lambda lcm length let let* let*-values let-syntax let-values letrec letrec* letrec-syntax list list->string list->vector list-copy list-ref list-set! list-tail list? make-bytevector make-list make-parameter make-string make-vector map max member memq memv min modulo negative? newline not null? number->string number? numerator odd? open-input-bytevector open-input-string open-output-bytevector open-output-string or output-port-open? output-port? pair? parameterize peek-char peek-u8 port? positive? procedure? quasiquote quote quotient raise raise-continuable rational? rationalize read-bytevector read-bytevector! read-char read-error? read-line read-string read-u8 real? remainder reverse round set! set-car! set-cdr! square string string->list string->number string->symbol string->utf8 string->vector string-append string-copy string-copy! string-fill! string-for-each string-length string-map string-ref string-set! string<=? string<? string=? string>=? string>? string? substring symbol->string symbol=? symbol? syntax-error syntax-rules textual-port? truncate truncate-quotient truncate-remainder truncate/ u8-ready? unless unquote unquote-splicing utf8->string values vector vector->list vector->string vector-append vector-copy vector-copy! vector-fill! vector-for-each vector-length vector-map vector-ref vector-set! vector? when with-exception-handler write-bytevector write-char write-string write-u8 zero?";

const R5RS_EXPORTS: &str = "* + - ... / < <= = => > >= abs acos and angle append apply asin assoc assq assv atan begin boolean? caaaar caaadr caaar caadar caaddr caadr cadaar cadadr cadar caddar cadddr caddr call-with-current-continuation call-with-input-file call-with-output-file call-with-values car cdaaar cdaadr cdaar cdadar cdaddr cdadr cddaar cddadr cddar cdddar cddddr cdddr cddr cdr ceiling char->integer char-alphabetic? char-ci<=? char-ci<? char-ci=? char-ci>=? char-ci>? char-downcase char-lower-case? char-numeric? char-upcase char-upper-case? char-whitespace? char<=? char<? char=? char>=? char>? char? close-input-port close-output-port complex? cond cons cos current-input-port current-output-port define define-syntax delay denominator display do dynamic-wind else eof-object? eq? equal? eqv? eval even? exact->inexact exact? exp expt floor for-each force gcd if imag-part inexact->exact inexact? input-port? integer->char integer? interaction-environment lambda lcm length let let* let-syntax letrec letrec-syntax list list->string list->vector list-ref list-tail list? load log magnitude make-polar make-rectangular make-string make-vector map max member memq memv min modulo negative? newline not null? null-environment number->string number? numerator odd? open-input-file open-output-file or output-port? pair? peek-char positive? procedure? quasiquote quote quotient rational? rationalize read read-char real-part real? remainder reverse round scheme-report-environment set! set-car! set-cdr! sin sqrt string string->list string->number string->symbol string-append string-copy string-fill! string-for-each string-length string-map string-ref string-set! string<=? string<? string=? string>=? string>? string? substring symbol->string symbol? syntax-rules tan truncate vector vector->list vector-fill! vector-for-each vector-length vector-map vector-ref vector-set! vector? with-input-from-file with-output-to-file write write-char zero?";

fn parse_exports(forms: &[Form]) -> Result<Vec<Export>, Error> {
    forms
        .iter()
        .map(|form| {
            if let Some(name) = form.symbol() {
                return Ok(Export {
                    internal: name.to_owned(),
                    external: name.to_owned(),
                });
            }
            let items = form.proper_list().ok_or_else(|| {
                Error::plain(
                    ErrorKind::LibraryError,
                    "export specification must be an identifier or rename",
                )
            })?;
            if items.len() == 3 && items[0].symbol() == Some("rename") {
                let internal = identifier(&items[1])?;
                let external = identifier(&items[2])?;
                Ok(Export { internal, external })
            } else {
                Err(Error::plain(
                    ErrorKind::LibraryError,
                    "invalid export specification",
                ))
            }
        })
        .collect()
}

fn parse_import_set(form: &Form) -> Result<ImportSet, Error> {
    let Some(items) = form.proper_list() else {
        return Err(Error::plain(
            ErrorKind::LibraryError,
            "import set must be a list",
        ));
    };
    if items.is_empty() {
        return Err(Error::plain(ErrorKind::LibraryError, "empty import set"));
    }
    if let Some(keyword) = items[0].symbol() {
        match keyword {
            "only" => {
                if items.len() < 3 {
                    return Err(Error::plain(
                        ErrorKind::LibraryError,
                        "only needs an import set and identifiers",
                    ));
                }
                return Ok(ImportSet::Only(
                    Box::new(parse_import_set(&items[1])?),
                    items[2..]
                        .iter()
                        .map(identifier)
                        .collect::<Result<_, _>>()?,
                ));
            }
            "except" => {
                if items.len() < 3 {
                    return Err(Error::plain(
                        ErrorKind::LibraryError,
                        "except needs an import set and identifiers",
                    ));
                }
                return Ok(ImportSet::Except(
                    Box::new(parse_import_set(&items[1])?),
                    items[2..]
                        .iter()
                        .map(identifier)
                        .collect::<Result<_, _>>()?,
                ));
            }
            "prefix" => {
                if items.len() != 3 {
                    return Err(Error::plain(
                        ErrorKind::LibraryError,
                        "prefix needs an import set and prefix",
                    ));
                }
                return Ok(ImportSet::Prefix(
                    Box::new(parse_import_set(&items[1])?),
                    identifier(&items[2])?,
                ));
            }
            "rename" => {
                if items.len() < 3 {
                    return Err(Error::plain(
                        ErrorKind::LibraryError,
                        "rename needs an import set and renames",
                    ));
                }
                let mut renames = Vec::new();
                for item in &items[2..] {
                    let pair = item.proper_list().ok_or_else(|| {
                        Error::plain(ErrorKind::LibraryError, "rename clause must be a pair")
                    })?;
                    if pair.len() != 2 {
                        return Err(Error::plain(
                            ErrorKind::LibraryError,
                            "rename clause must contain two identifiers",
                        ));
                    }
                    renames.push((identifier(&pair[0])?, identifier(&pair[1])?));
                }
                return Ok(ImportSet::Rename(
                    Box::new(parse_import_set(&items[1])?),
                    renames,
                ));
            }
            _ => {}
        }
    }
    Ok(ImportSet::Library(parse_name(form)?))
}

pub(crate) fn parse_name(form: &Form) -> Result<LibraryName, Error> {
    let parts = form.proper_list().ok_or_else(|| {
        Error::plain(
            ErrorKind::LibraryError,
            "library name must be a proper list",
        )
    })?;
    LibraryName::new(
        parts
            .iter()
            .map(|part| match &part.kind {
                FormKind::Symbol(value) => Ok(LibraryNameComponent::Identifier(value.clone())),
                FormKind::Number(crate::Number::Real(crate::Real::ExactInteger(value)))
                    if *value >= 0 =>
                {
                    Ok(LibraryNameComponent::Number(*value as u64))
                }
                _ => Err(Error::plain(
                    ErrorKind::LibraryError,
                    "library-name components must be identifiers or non-negative integers",
                )),
            })
            .collect::<Result<Vec<_>, _>>()?,
    )
}

fn identifier(form: &Form) -> Result<String, Error> {
    form.symbol()
        .map(str::to_owned)
        .ok_or_else(|| Error::plain(ErrorKind::LibraryError, "expected identifier"))
}

/// The registration portion of the library cache.
///
/// Expansion and initialization are deliberately added by the embedding layer
/// so it can keep the VM's global cells engine-local.
#[derive(Default)]
pub(crate) struct LibraryRegistry {
    libraries: HashMap<LibraryName, RegisteredLibrary>,
}

impl LibraryRegistry {
    /// Returns whether a library of this name is already registered, from
    /// either a source declaration or a native binding set.
    pub(crate) fn contains(&self, name: &LibraryName) -> bool {
        self.libraries.contains_key(name)
    }

    pub(crate) fn register_source(
        &mut self,
        name: LibraryName,
        _source: SourceId,
        _source_text: String,
        declaration: LibraryDeclaration,
    ) -> Result<(), Error> {
        if self.libraries.contains_key(&name) {
            return Err(Error::plain(
                ErrorKind::LibraryError,
                format!("library {name} is already declared"),
            ));
        }
        self.libraries.insert(
            name,
            RegisteredLibrary::Source {
                declaration,
                state: LibraryState::Declared,
            },
        );
        Ok(())
    }

    pub(crate) fn validate_native_binding(
        &self,
        name: &LibraryName,
        binding: &str,
    ) -> Result<(), Error> {
        match self.libraries.get(name) {
            None => Ok(()),
            Some(RegisteredLibrary::Source { .. }) => Err(Error::plain(
                ErrorKind::LibraryError,
                format!("library {name} is already declared from source"),
            )),
            Some(RegisteredLibrary::Native { sealed: true, .. }) => Err(Error::plain(
                ErrorKind::LibraryError,
                format!("native library {name} is sealed after its first import"),
            )),
            Some(RegisteredLibrary::Native { bindings, .. })
                if bindings.values.contains_key(binding) =>
            {
                Err(Error::plain(
                    ErrorKind::LibraryError,
                    format!("native library {name} already exports '{binding}'"),
                ))
            }
            Some(RegisteredLibrary::Native { .. }) => Ok(()),
        }
    }

    pub(crate) fn insert_native_binding(
        &mut self,
        name: LibraryName,
        binding: String,
        global: String,
    ) {
        match self.libraries.entry(name) {
            std::collections::hash_map::Entry::Vacant(entry) => {
                entry.insert(RegisteredLibrary::Native {
                    bindings: LibraryBindings {
                        values: HashMap::from([(binding, global)]),
                        ..Default::default()
                    },
                    sealed: false,
                });
            }
            std::collections::hash_map::Entry::Occupied(mut entry) => {
                let RegisteredLibrary::Native {
                    bindings,
                    sealed: false,
                } = entry.get_mut()
                else {
                    unreachable!("native library registration was validated")
                };
                let previous = bindings.values.insert(binding, global);
                debug_assert!(previous.is_none());
            }
        }
    }

    pub(crate) fn resolve_native(&mut self, name: &LibraryName) -> Option<LibraryBindings> {
        let RegisteredLibrary::Native { bindings, sealed } = self.libraries.get_mut(name)? else {
            return None;
        };
        *sealed = true;
        Some(bindings.clone())
    }

    pub(crate) fn get(&self, name: &LibraryName) -> Result<&RegisteredLibrary, Error> {
        self.libraries.get(name).ok_or_else(|| {
            Error::plain(
                ErrorKind::LibraryNotFound,
                format!("library {name} is not registered"),
            )
        })
    }

    pub(crate) fn state(&self, name: &LibraryName) -> Result<LibraryState, Error> {
        match self.get(name)? {
            RegisteredLibrary::Source { state, .. } => Ok(state.clone()),
            RegisteredLibrary::Native { bindings, .. } => Ok(LibraryState::Ready(bindings.clone())),
        }
    }

    pub(crate) fn declaration(&self, name: &LibraryName) -> Result<LibraryDeclaration, Error> {
        match self.get(name)? {
            RegisteredLibrary::Source { declaration, .. } => Ok(declaration.clone()),
            RegisteredLibrary::Native { .. } => Err(Error::plain(
                ErrorKind::LibraryError,
                format!("native library {name} has no source declaration"),
            )),
        }
    }

    pub(crate) fn set_state(
        &mut self,
        name: &LibraryName,
        state: LibraryState,
    ) -> Result<(), Error> {
        match self.libraries.get_mut(name) {
            Some(RegisteredLibrary::Source {
                state: library_state,
                ..
            }) => {
                *library_state = state;
                Ok(())
            }
            Some(RegisteredLibrary::Native { .. }) => Err(Error::plain(
                ErrorKind::LibraryError,
                format!("cannot change initialization state for native library {name}"),
            )),
            None => Err(Error::plain(
                ErrorKind::LibraryNotFound,
                format!("library {name} is not registered"),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::{LibraryName, LibraryNameComponent, standard_exports};

    #[test]
    fn appendix_a_manifests_are_nonempty_and_duplicate_free() {
        for parts in [
            ["scheme", "base"].as_slice(),
            ["scheme", "case-lambda"].as_slice(),
            ["scheme", "char"].as_slice(),
            ["scheme", "complex"].as_slice(),
            ["scheme", "cxr"].as_slice(),
            ["scheme", "eval"].as_slice(),
            ["scheme", "file"].as_slice(),
            ["scheme", "inexact"].as_slice(),
            ["scheme", "lazy"].as_slice(),
            ["scheme", "load"].as_slice(),
            ["scheme", "process-context"].as_slice(),
            ["scheme", "read"].as_slice(),
            ["scheme", "repl"].as_slice(),
            ["scheme", "time"].as_slice(),
            ["scheme", "write"].as_slice(),
            ["scheme", "r5rs"].as_slice(),
        ] {
            let name = LibraryName::new(
                parts
                    .iter()
                    .map(|part| LibraryNameComponent::identifier(*part)),
            )
            .unwrap();
            let exports = standard_exports(&name).unwrap();
            assert!(!exports.is_empty(), "{name}");
            let unique: HashSet<_> = exports.iter().copied().collect();
            assert_eq!(unique.len(), exports.len(), "{name}");
        }
    }
}
