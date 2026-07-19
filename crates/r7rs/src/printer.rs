use std::collections::{HashMap, HashSet};

use crate::{
    Datum, DatumKind, DatumRef, Error, ErrorKind, Number, Real, Value,
    datum::NodeKind,
    heap::Heap,
    value::{GcRef, ValueRepr},
};

/// Runtime writer policy used by Scheme output procedures.
#[derive(Clone, Copy)]
pub(crate) enum RuntimeWriteMode {
    Write,
    Shared,
    Simple,
    Display,
}

/// Produces an external representation of a runtime value.
pub(crate) fn write_value(
    heap: &Heap,
    value: Value,
    mode: RuntimeWriteMode,
) -> Result<String, Error> {
    let labels = RuntimeLabels::new(heap, value, mode)?;
    let mut output = String::new();
    let mut emitted = HashSet::new();
    runtime_value(heap, value, mode, &labels, &mut emitted, &mut output)?;
    Ok(output)
}

/// Labels are assigned in first-discovery order. The writer operates only on
/// pairs and vectors, the datum types that can form shared/cyclic graphs.
struct RuntimeLabels {
    labels: HashMap<GcRef, usize>,
}

impl RuntimeLabels {
    fn new(heap: &Heap, root: Value, mode: RuntimeWriteMode) -> Result<Self, Error> {
        let mut counts = HashMap::<GcRef, usize>::new();
        let mut order = Vec::new();
        let mut expanded = HashSet::new();
        let mut stack = vec![root];
        while let Some(value) = stack.pop() {
            let mut children = Vec::new();
            if !graph_children(heap, value, &mut children) {
                continue;
            }
            let key = graph_key(value).expect("pair/vector is heap-backed");
            let count = counts.entry(key).or_insert_with(|| {
                order.push(key);
                0
            });
            *count += 1;
            if !expanded.insert(key) {
                continue;
            }
            stack.extend(children);
        }
        let cyclic = cyclic_nodes(heap, root);
        if matches!(mode, RuntimeWriteMode::Simple) && !cyclic.is_empty() {
            return Err(Error::plain(
                ErrorKind::RuntimeError,
                "write-simple cannot print cyclic data",
            ));
        }
        let mut labels = HashMap::new();
        if !matches!(mode, RuntimeWriteMode::Simple) {
            for value in order {
                let required = match mode {
                    RuntimeWriteMode::Shared => counts[&value] > 1,
                    RuntimeWriteMode::Write | RuntimeWriteMode::Display => cyclic.contains(&value),
                    RuntimeWriteMode::Simple => false,
                };
                if required {
                    let index = labels.len();
                    labels.insert(value, index);
                }
            }
        }
        Ok(Self { labels })
    }
}

fn graph_children(heap: &Heap, value: Value, output: &mut Vec<Value>) -> bool {
    if let Some((car, cdr)) = heap.pair(value) {
        output.extend([car, cdr]);
        true
    } else if let Some(values) = heap.vector(value) {
        output.extend(values);
        true
    } else {
        false
    }
}

fn graph_key(value: Value) -> Option<GcRef> {
    value.heap_ref()
}

fn cyclic_nodes(heap: &Heap, root: Value) -> HashSet<GcRef> {
    fn visit(
        heap: &Heap,
        value: Value,
        states: &mut HashMap<GcRef, u8>,
        path: &mut Vec<GcRef>,
        cyclic: &mut HashSet<GcRef>,
    ) {
        if !graph_children(heap, value, &mut Vec::new()) {
            return;
        }
        let key = graph_key(value).expect("pair/vector is heap-backed");
        match states.get(&key).copied() {
            Some(1) => {
                if let Some(start) = path.iter().position(|candidate| *candidate == key) {
                    for candidate in &path[start..] {
                        cyclic.insert(*candidate);
                    }
                }
                return;
            }
            Some(2) => return,
            _ => {}
        }
        states.insert(key, 1);
        path.push(key);
        let mut children = Vec::new();
        let _ = graph_children(heap, value, &mut children);
        for child in children {
            visit(heap, child, states, path, cyclic);
        }
        path.pop();
        states.insert(key, 2);
    }
    let mut cyclic = HashSet::new();
    visit(
        heap,
        root,
        &mut HashMap::new(),
        &mut Vec::new(),
        &mut cyclic,
    );
    cyclic
}

fn runtime_value(
    heap: &Heap,
    value: Value,
    mode: RuntimeWriteMode,
    labels: &RuntimeLabels,
    emitted: &mut HashSet<GcRef>,
    output: &mut String,
) -> Result<(), Error> {
    match value.decode() {
        ValueRepr::Nil => output.push_str("()"),
        ValueRepr::Boolean(value) => output.push_str(if value { "#t" } else { "#f" }),
        ValueRepr::Character(value) => match mode {
            RuntimeWriteMode::Display => output.push(value),
            _ => runtime_character(value, output),
        },
        ValueRepr::Fixnum(value) => output.push_str(&value.to_string()),
        ValueRepr::Float(value) => runtime_real(&Real::Inexact(value), output),
        ValueRepr::Eof => output.push_str("#<eof>"),
        ValueRepr::Unspecified => output.push_str("#<unspecified>"),
        ValueRepr::Undefined => output.push_str("#<undefined>"),
        ValueRepr::Heap(_) => {
            if let Some(value) = heap.string_slice(value) {
                if matches!(mode, RuntimeWriteMode::Display) {
                    output.push_str(value);
                } else {
                    runtime_string(value, output);
                }
            } else if let Some(value) = heap.symbol(value) {
                if matches!(mode, RuntimeWriteMode::Display) {
                    output.push_str(&value);
                } else {
                    runtime_symbol(&value, output);
                }
            } else if let Some(value) = heap.number(value) {
                let (real, imaginary) = value.components();
                if matches!(value, crate::number::RuntimeNumber::Real(_)) {
                    runtime_real(&real, output);
                } else {
                    runtime_real(&real, output);
                    if matches!(imaginary, Real::ExactInteger(value) if value >= 0)
                        || matches!(imaginary, Real::ExactRational(value) if value.numerator() >= 0)
                        || matches!(imaginary, Real::Inexact(value) if value.is_finite() && !value.is_sign_negative())
                    {
                        output.push('+');
                    }
                    runtime_real(&imaginary, output);
                    output.push('i');
                }
            } else if let Some(values) = heap.vector(value) {
                runtime_graph(
                    heap,
                    value,
                    mode,
                    labels,
                    emitted,
                    output,
                    |out, emitted| {
                        out.push_str("#(");
                        for (index, child) in values.iter().enumerate() {
                            if index != 0 {
                                out.push(' ');
                            }
                            runtime_value(heap, *child, mode, labels, emitted, out)?;
                        }
                        out.push(')');
                        Ok(())
                    },
                )?;
            } else if heap.pair(value).is_some() {
                runtime_graph(
                    heap,
                    value,
                    mode,
                    labels,
                    emitted,
                    output,
                    |out, emitted| runtime_pair(heap, value, mode, labels, emitted, out),
                )?;
            } else if let Some(value) = heap.bytevector(value) {
                out_bytevector(&value, output);
            } else if heap.port(value).is_some() {
                output.push_str("#<port>");
            } else if heap.random_source(value).is_some() {
                output.push_str("#<random-source>");
            } else {
                output.push_str("#<object>");
            }
        }
    }
    Ok(())
}

fn runtime_graph<F>(
    _heap: &Heap,
    value: Value,
    mode: RuntimeWriteMode,
    labels: &RuntimeLabels,
    emitted: &mut HashSet<GcRef>,
    output: &mut String,
    render: F,
) -> Result<(), Error>
where
    F: FnOnce(&mut String, &mut HashSet<GcRef>) -> Result<(), Error>,
{
    let key = graph_key(value).expect("runtime graph value is heap-backed");
    if let Some(label) = labels.labels.get(&key) {
        if !emitted.insert(key) {
            output.push('#');
            output.push_str(&label.to_string());
            output.push('#');
            return Ok(());
        }
        output.push('#');
        output.push_str(&label.to_string());
        output.push('=');
    }
    let _ = mode;
    render(output, emitted)
}

fn runtime_pair(
    heap: &Heap,
    value: Value,
    mode: RuntimeWriteMode,
    labels: &RuntimeLabels,
    emitted: &mut HashSet<GcRef>,
    output: &mut String,
) -> Result<(), Error> {
    output.push('(');
    let mut pair = value;
    let mut first = true;
    loop {
        if !first {
            output.push(' ');
        }
        first = false;
        let (car, cdr) = heap
            .pair(pair)
            .ok_or_else(|| Error::plain(ErrorKind::RuntimeError, "invalid pair"))?;
        runtime_value(heap, car, mode, labels, emitted, output)?;
        if cdr == Value::nil() {
            break;
        }
        if heap.pair(cdr).is_some()
            && !labels
                .labels
                .contains_key(&graph_key(cdr).expect("pair is heap-backed"))
        {
            pair = cdr;
            continue;
        }
        output.push_str(" . ");
        runtime_value(heap, cdr, mode, labels, emitted, output)?;
        break;
    }
    output.push(')');
    Ok(())
}

fn runtime_string(value: &str, output: &mut String) {
    output.push('"');
    for character in value.chars() {
        match character {
            '"' => output.push_str("\\\""),
            '\\' => output.push_str("\\\\"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            value if value.is_control() => {
                output.push_str("\\x");
                output.push_str(&format!("{:x}", value as u32));
                output.push(';');
            }
            value => output.push(value),
        }
    }
    output.push('"');
}
fn runtime_character(value: char, output: &mut String) {
    output.push_str("#\\");
    match value {
        ' ' => output.push_str("space"),
        '\n' => output.push_str("newline"),
        '\t' => output.push_str("tab"),
        value => output.push(value),
    }
}
fn runtime_symbol(value: &str, output: &mut String) {
    if valid_bare_symbol(value) {
        output.push_str(value);
    } else {
        output.push('|');
        for character in value.chars() {
            if matches!(character, '|' | '\\') {
                output.push('\\');
            }
            output.push(character);
        }
        output.push('|');
    }
}
fn out_bytevector(values: &[u8], output: &mut String) {
    output.push_str("#u8(");
    for (index, value) in values.iter().enumerate() {
        if index != 0 {
            output.push(' ');
        }
        output.push_str(&value.to_string());
    }
    output.push(')');
}
fn runtime_real(value: &Real, output: &mut String) {
    match value {
        // Every NaN prints canonically: runtime arithmetic skips per-result
        // NaN canonicalization, so sign/payload bits carry no meaning here.
        Real::Inexact(value) if value.is_nan() => output.push_str("+nan.0"),
        Real::Inexact(value) if *value == f64::INFINITY => output.push_str("+inf.0"),
        Real::Inexact(value) if *value == f64::NEG_INFINITY => output.push_str("-inf.0"),
        Real::Inexact(value) => {
            output.push_str(&format_finite_inexact(*value));
        }
        Real::ExactInteger(value) => output.push_str(&value.to_string()),
        Real::ExactRational(value) => {
            output.push_str(&format!("{}/{}", value.numerator(), value.denominator()));
        }
    }
}

fn format_finite_inexact(value: f64) -> String {
    if value == 0.0 && value.is_sign_negative() {
        return "-0.0".into();
    }
    let mut text = format!("{value:?}");
    if let Some(index) = text.find('e') {
        let mut exponent_index = index;
        if !text[..index].contains('.') {
            text.insert_str(index, ".0");
            exponent_index += 2;
        }
        if !matches!(
            text.as_bytes().get(exponent_index + 1),
            Some(b'+') | Some(b'-')
        ) {
            text.insert(exponent_index + 1, '+');
        }
    } else if !text.contains('.') {
        text.push_str(".0");
    }
    text
}

pub(crate) fn print(datum: &Datum) -> String {
    let mut counts = HashMap::new();
    let mut seen = HashSet::new();
    count(datum, datum.root(), &mut counts, &mut seen);
    let mut labels = HashMap::new();
    let mut order = Vec::new();
    order_nodes(
        datum,
        datum.root(),
        &counts,
        &mut HashSet::new(),
        &mut order,
    );
    for node in order {
        if counts.get(&node).copied().unwrap_or(0) > 1 {
            let index = labels.len();
            labels.insert(node, index);
        }
    }
    let mut writer = Writer {
        datum,
        labels,
        emitted: HashSet::new(),
        output: String::new(),
    };
    writer.node(datum.root());
    writer.output
}

fn count(
    datum: &Datum,
    reference: DatumRef,
    counts: &mut HashMap<DatumRef, usize>,
    seen: &mut HashSet<DatumRef>,
) {
    let Some(reference) = datum.resolve(reference) else {
        return;
    };
    *counts.entry(reference).or_default() += 1;
    if !seen.insert(reference) {
        return;
    }
    for child in children(datum, reference) {
        count(datum, child, counts, seen);
    }
}

fn order_nodes(
    datum: &Datum,
    reference: DatumRef,
    counts: &HashMap<DatumRef, usize>,
    seen: &mut HashSet<DatumRef>,
    output: &mut Vec<DatumRef>,
) {
    let Some(reference) = datum.resolve(reference) else {
        return;
    };
    if !seen.insert(reference) {
        return;
    }
    if counts.get(&reference).copied().unwrap_or(0) > 1 {
        output.push(reference);
    }
    for child in children(datum, reference) {
        order_nodes(datum, child, counts, seen, output);
    }
}

fn children(datum: &Datum, reference: DatumRef) -> Vec<DatumRef> {
    match datum.nodes.get(reference.0 as usize).map(|node| &node.kind) {
        Some(NodeKind::Pair(car, cdr)) => vec![*car, *cdr],
        Some(NodeKind::Vector(values)) => values.clone(),
        _ => Vec::new(),
    }
}

struct Writer<'a> {
    datum: &'a Datum,
    labels: HashMap<DatumRef, usize>,
    emitted: HashSet<DatumRef>,
    output: String,
}

impl Writer<'_> {
    fn node(&mut self, original: DatumRef) {
        let Some(reference) = self.datum.resolve(original) else {
            self.output.push_str("#<invalid-datum>");
            return;
        };
        if let Some(label) = self.labels.get(&reference) {
            if !self.emitted.insert(reference) {
                self.output.push('#');
                self.output.push_str(&label.to_string());
                self.output.push('#');
                return;
            }
            self.output.push('#');
            self.output.push_str(&label.to_string());
            self.output.push('=');
        }
        match self.datum.kind(reference) {
            Some(DatumKind::Nil) => self.output.push_str("()"),
            Some(DatumKind::Boolean(value)) => {
                self.output.push_str(if value { "#t" } else { "#f" })
            }
            Some(DatumKind::Character(value)) => self.character(value),
            Some(DatumKind::String(value)) => self.string(value),
            Some(DatumKind::Symbol(value)) => self.symbol(value),
            Some(DatumKind::Number(value)) => self.number(value),
            Some(DatumKind::Pair { car, cdr }) => self.pair(car, cdr),
            Some(DatumKind::Vector(values)) => {
                self.output.push_str("#(");
                for (index, value) in values.iter().enumerate() {
                    if index != 0 {
                        self.output.push(' ');
                    }
                    self.node(*value);
                }
                self.output.push(')');
            }
            Some(DatumKind::Bytevector(values)) => {
                self.output.push_str("#u8(");
                for (index, value) in values.iter().enumerate() {
                    if index != 0 {
                        self.output.push(' ');
                    }
                    self.output.push_str(&value.to_string());
                }
                self.output.push(')');
            }
            None => self.output.push_str("#<invalid-datum>"),
        }
    }

    fn pair(&mut self, car: DatumRef, mut cdr: DatumRef) {
        self.output.push('(');
        self.node(car);
        loop {
            let Some(resolved) = self.datum.resolve(cdr) else {
                self.output.push_str(" . #<invalid-datum>");
                break;
            };
            if self.labels.contains_key(&resolved) {
                self.output.push_str(" . ");
                self.node(cdr);
                break;
            }
            match self.datum.kind(resolved) {
                Some(DatumKind::Nil) => break,
                Some(DatumKind::Pair {
                    car: next_car,
                    cdr: next_cdr,
                }) => {
                    self.output.push(' ');
                    self.node(next_car);
                    cdr = next_cdr;
                }
                _ => {
                    self.output.push_str(" . ");
                    self.node(cdr);
                    break;
                }
            }
        }
        self.output.push(')');
    }

    fn string(&mut self, value: &str) {
        self.output.push('"');
        for character in value.chars() {
            match character {
                '"' => self.output.push_str("\\\""),
                '\\' => self.output.push_str("\\\\"),
                '\n' => self.output.push_str("\\n"),
                '\r' => self.output.push_str("\\r"),
                '\t' => self.output.push_str("\\t"),
                c if c.is_control() => {
                    self.output.push_str("\\x");
                    self.output.push_str(&format!("{:x}", c as u32));
                    self.output.push(';');
                }
                c => self.output.push(c),
            }
        }
        self.output.push('"');
    }
    fn character(&mut self, value: char) {
        self.output.push_str("#\\");
        match value {
            ' ' => self.output.push_str("space"),
            '\n' => self.output.push_str("newline"),
            '\t' => self.output.push_str("tab"),
            '\r' => self.output.push_str("return"),
            '\0' => self.output.push_str("null"),
            c if c.is_control() => {
                self.output.push('x');
                self.output.push_str(&format!("{:x}", c as u32));
            }
            c => self.output.push(c),
        }
    }
    fn symbol(&mut self, value: &str) {
        if valid_bare_symbol(value) {
            self.output.push_str(value);
        } else {
            self.output.push('|');
            for character in value.chars() {
                match character {
                    '|' => self.output.push_str("\\|"),
                    '\\' => self.output.push_str("\\\\"),
                    c if c.is_control() => {
                        self.output.push_str("\\x");
                        self.output.push_str(&format!("{:x}", c as u32));
                        self.output.push(';');
                    }
                    c => self.output.push(c),
                }
            }
            self.output.push('|');
        }
    }
    fn number(&mut self, value: &Number) {
        match value {
            Number::Real(real) => self.real(real),
            Number::Rectangular { real, imaginary } => {
                self.real(real);
                match imaginary {
                    Real::ExactInteger(value) if *value >= 0 => self.output.push('+'),
                    Real::ExactRational(value) if value.numerator() >= 0 => self.output.push('+'),
                    Real::Inexact(value) if value.is_finite() && !value.is_sign_negative() => {
                        self.output.push('+')
                    }
                    _ => {}
                }
                self.real(imaginary);
                self.output.push('i');
            }
            Number::Polar { magnitude, angle } => {
                self.real(magnitude);
                self.output.push('@');
                self.real(angle);
            }
        }
    }
    fn real(&mut self, value: &Real) {
        match value {
            Real::ExactInteger(value) => self.output.push_str(&value.to_string()),
            Real::ExactRational(value) => {
                self.output.push_str(&value.numerator().to_string());
                self.output.push('/');
                self.output.push_str(&value.denominator().to_string());
            }
            // Canonical for any NaN bit pattern; see `runtime_real`.
            Real::Inexact(value) if value.is_nan() => self.output.push_str("+nan.0"),
            Real::Inexact(value) if *value == f64::INFINITY => self.output.push_str("+inf.0"),
            Real::Inexact(value) if *value == f64::NEG_INFINITY => self.output.push_str("-inf.0"),
            Real::Inexact(value) => {
                self.output.push_str(&format_finite_inexact(*value));
            }
        }
    }
}

fn valid_bare_symbol(value: &str) -> bool {
    crate::reader::valid_identifier(value) && crate::number::parse(value).is_none()
}
