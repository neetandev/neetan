use std::collections::HashMap;

use crate::{
    Datum, DatumRef, Diagnostic, EngineConfig, Error, ErrorKind, Number, Span,
    datum::{Node, NodeKind},
};

#[derive(Clone, Debug)]
enum TokenKind {
    Atom(String),
    Identifier(String),
    String(String),
    Quote,
    Quasiquote,
    Unquote,
    UnquoteSplicing,
    Open,
    Close,
    VectorOpen,
    BytevectorOpen,
    DatumComment,
    LabelDef(u32),
    LabelRef(u32),
}

#[derive(Clone, Debug)]
struct Token {
    kind: TokenKind,
    span: Span,
}

/// A streaming R7RS external-datum reader created by an [`crate::Engine`].
#[derive(Debug)]
pub struct Reader {
    source: crate::SourceId,
    lexer: Lexer,
    lookahead: Option<Token>,
    max_nesting: usize,
}

impl Reader {
    pub(crate) fn new(source: crate::SourceId, input: String, config: &EngineConfig) -> Self {
        Self {
            source,
            lexer: Lexer::new(
                source,
                input,
                config.limits().max_token_bytes(),
                config.limits().max_nesting_depth(),
            ),
            lookahead: None,
            max_nesting: config.limits().max_nesting_depth(),
        }
    }

    /// Returns the source registered for this reader.
    #[must_use]
    pub const fn source_id(&self) -> crate::SourceId {
        self.source
    }

    /// Reads the next datum, or returns `None` at end of input.
    pub fn read_next(&mut self) -> Result<Option<Datum>, Error> {
        let Some(first) = self.next_token()? else {
            return Ok(None);
        };
        let mut parser = Parser {
            reader: self,
            nodes: Vec::new(),
            labels: HashMap::new(),
            depth: 0,
        };
        let root = parser.parse_datum_from(first)?;
        Ok(Some(Datum::new(parser.nodes, root)))
    }

    /// Returns the number of UTF-8 bytes consumed from this reader's input.
    pub(crate) fn consumed_bytes(&self) -> usize {
        self.lexer.offset
    }

    fn next_token(&mut self) -> Result<Option<Token>, Error> {
        if self.lookahead.is_some() {
            return Ok(self.lookahead.take());
        }
        self.lexer.next()
    }
}

struct Parser<'a> {
    reader: &'a mut Reader,
    nodes: Vec<Node>,
    labels: HashMap<u32, DatumRef>,
    depth: usize,
}

impl Parser<'_> {
    fn add(&mut self, kind: NodeKind, span: Span) -> DatumRef {
        let reference = DatumRef(self.nodes.len() as u32);
        self.nodes.push(Node { kind, span });
        reference
    }

    fn parse_datum_from(&mut self, token: Token) -> Result<DatumRef, Error> {
        match token.kind {
            TokenKind::Atom(atom) => self.atom(atom, token.span),
            TokenKind::Identifier(identifier) => {
                Ok(self.add(NodeKind::Symbol(identifier), token.span))
            }
            TokenKind::String(value) => Ok(self.add(NodeKind::String(value), token.span)),
            TokenKind::Quote => self.abbreviation("quote", token.span),
            TokenKind::Quasiquote => self.abbreviation("quasiquote", token.span),
            TokenKind::Unquote => self.abbreviation("unquote", token.span),
            TokenKind::UnquoteSplicing => self.abbreviation("unquote-splicing", token.span),
            TokenKind::Open => self.list(token.span),
            TokenKind::VectorOpen => self.vector(token.span),
            TokenKind::BytevectorOpen => self.bytevector(token.span),
            TokenKind::DatumComment => {
                let ignored = self.required("a datum after #;", token.span)?;
                self.parse_ignored_datum(ignored)?;
                let next = self.required("a datum after datum comment", token.span)?;
                self.parse_datum_from(next)
            }
            TokenKind::LabelDef(label) => self.label(label, token.span),
            TokenKind::LabelRef(label) => self.labels.get(&label).copied().ok_or_else(|| {
                self.error(
                    ErrorKind::InvalidDatumLabel,
                    format!("datum label #{label}# has not been defined"),
                    token.span,
                )
            }),
            TokenKind::Close => Err(self.error(
                ErrorKind::InvalidDatum,
                "unexpected closing parenthesis",
                token.span,
            )),
        }
    }

    fn parse_ignored_datum(&mut self, token: Token) -> Result<(), Error> {
        let outer_labels = self.labels.clone();
        let result = self.parse_datum_from(token);
        self.labels = outer_labels;
        result.map(|_| ())
    }

    fn required(&mut self, message: &str, span: Span) -> Result<Token, Error> {
        self.reader
            .next_token()?
            .ok_or_else(|| self.error(ErrorKind::UnexpectedEof, message, span))
    }

    fn descend(&mut self, span: Span) -> Result<(), Error> {
        self.depth += 1;
        if self.depth > self.reader.max_nesting {
            return Err(self.error(
                ErrorKind::ReaderLimitExceeded,
                "reader nesting limit exceeded",
                span,
            ));
        }
        Ok(())
    }

    fn ascend(&mut self) {
        self.depth = self.depth.saturating_sub(1);
    }

    fn atom(&mut self, atom: String, span: Span) -> Result<DatumRef, Error> {
        if atom == "." {
            return Err(self.error(
                ErrorKind::InvalidDatum,
                "a dot is valid only inside a list",
                span,
            ));
        }
        if matches!(atom.as_str(), "#t" | "#true") {
            return Ok(self.add(NodeKind::Boolean(true), span));
        }
        if matches!(atom.as_str(), "#f" | "#false") {
            return Ok(self.add(NodeKind::Boolean(false), span));
        }
        if let Some(character) = atom.strip_prefix("#\\") {
            return self.character(character, span);
        }
        let dispatch = atom.to_ascii_lowercase();
        if atom.starts_with('#')
            && !["#b", "#o", "#d", "#x", "#e", "#i"]
                .iter()
                .any(|prefix| dispatch.starts_with(prefix))
        {
            return Err(self.error(ErrorKind::InvalidToken, "invalid # dispatch form", span));
        }
        match crate::number::parse(&atom) {
            Some(Ok(number)) => Ok(self.add(NodeKind::Number(number), span)),
            Some(Err(message)) => Err(self.error(ErrorKind::InvalidNumber, message, span)),
            None if valid_identifier(&atom) => Ok(self.add(NodeKind::Symbol(atom), span)),
            None => Err(self.error(
                ErrorKind::InvalidToken,
                "invalid identifier or number",
                span,
            )),
        }
    }

    fn character(&mut self, text: &str, span: Span) -> Result<DatumRef, Error> {
        let character = match text {
            "alarm" => Ok('\x07'),
            "backspace" => Ok('\x08'),
            "delete" => Ok('\x7f'),
            "escape" => Ok('\x1b'),
            "newline" => Ok('\n'),
            "null" => Ok('\0'),
            "return" => Ok('\r'),
            "space" => Ok(' '),
            "tab" => Ok('\t'),
            _ if text.chars().count() == 1 => Ok(text.chars().next().unwrap()),
            _ if text.starts_with('x') => scalar(&text[1..]),
            _ => Err("invalid character literal".into()),
        }
        .map_err(|message: String| self.error(ErrorKind::InvalidToken, message, span))?;
        Ok(self.add(NodeKind::Character(character), span))
    }

    fn abbreviation(&mut self, name: &str, prefix: Span) -> Result<DatumRef, Error> {
        self.descend(prefix)?;
        let token = self.required("a datum after abbreviation", prefix)?;
        let datum = self.parse_datum_from(token)?;
        self.ascend();
        let symbol = self.add(NodeKind::Symbol(name.to_owned()), prefix);
        let nil = self.add(NodeKind::Nil, prefix);
        let tail = self.add(NodeKind::Pair(datum, nil), prefix);
        Ok(self.add(NodeKind::Pair(symbol, tail), prefix))
    }

    fn list(&mut self, opening: Span) -> Result<DatumRef, Error> {
        self.descend(opening)?;
        let mut values = Vec::new();
        let result = loop {
            let token = self.required_skipping_comments("closing parenthesis for list", opening)?;
            match token.kind {
                TokenKind::Close => {
                    let nil = self.add(NodeKind::Nil, token.span);
                    break self.chain(values, nil, opening);
                }
                TokenKind::Atom(ref atom) if atom == "." => {
                    if values.is_empty() {
                        break Err(self.error(
                            ErrorKind::InvalidDatum,
                            "dot requires a preceding list element",
                            token.span,
                        ));
                    }
                    let tail_token =
                        self.required_skipping_comments("a datum after list dot", token.span)?;
                    let tail = self.parse_datum_from(tail_token)?;
                    let closing = self.required_skipping_comments(
                        "closing parenthesis after dotted tail",
                        token.span,
                    )?;
                    if !matches!(closing.kind, TokenKind::Close) {
                        break Err(self.error(
                            ErrorKind::InvalidDatum,
                            "dotted list must end after one tail datum",
                            closing.span,
                        ));
                    }
                    break self.chain(values, tail, opening);
                }
                _ => values.push(self.parse_datum_from(token)?),
            }
        };
        self.ascend();
        result
    }

    fn required_skipping_comments(&mut self, expected: &str, after: Span) -> Result<Token, Error> {
        loop {
            let token = self.required(expected, after)?;
            if matches!(token.kind, TokenKind::DatumComment) {
                let ignored = self.required("a datum after #;", token.span)?;
                self.parse_ignored_datum(ignored)?;
            } else {
                return Ok(token);
            }
        }
    }

    fn chain(
        &mut self,
        values: Vec<DatumRef>,
        mut tail: DatumRef,
        span: Span,
    ) -> Result<DatumRef, Error> {
        for value in values.into_iter().rev() {
            tail = self.add(NodeKind::Pair(value, tail), span);
        }
        Ok(tail)
    }

    fn vector(&mut self, opening: Span) -> Result<DatumRef, Error> {
        self.descend(opening)?;
        let mut values = Vec::new();
        let result = loop {
            let token =
                self.required_skipping_comments("closing parenthesis for vector", opening)?;
            if matches!(token.kind, TokenKind::Close) {
                break Ok(self.add(NodeKind::Vector(values), opening));
            }
            values.push(self.parse_datum_from(token)?);
        };
        self.ascend();
        result
    }

    fn bytevector(&mut self, opening: Span) -> Result<DatumRef, Error> {
        self.descend(opening)?;
        let mut values = Vec::new();
        let result = loop {
            let token =
                self.required_skipping_comments("closing parenthesis for bytevector", opening)?;
            if matches!(token.kind, TokenKind::Close) {
                break Ok(self.add(NodeKind::Bytevector(values), opening));
            }
            let reference = self.parse_datum_from(token.clone())?;
            let value = match self.nodes.get(reference.0 as usize).map(|node| &node.kind) {
                Some(NodeKind::Number(Number::Real(crate::Real::ExactInteger(value))))
                    if (0..=255).contains(value) =>
                {
                    *value as u8
                }
                _ => {
                    break Err(self.error(
                        ErrorKind::InvalidDatum,
                        "bytevector elements must be exact integers from 0 through 255",
                        token.span,
                    ));
                }
            };
            values.push(value);
        };
        self.ascend();
        result
    }

    fn label(&mut self, label: u32, span: Span) -> Result<DatumRef, Error> {
        if self.labels.contains_key(&label) {
            return Err(self.error(
                ErrorKind::InvalidDatumLabel,
                format!("datum label #{label}= is duplicated"),
                span,
            ));
        }
        let placeholder = self.add(NodeKind::Alias(None), span);
        self.labels.insert(label, placeholder);
        let token = self.required("a datum after label definition", span)?;
        let target = self.parse_datum_from(token)?;
        if target == placeholder {
            return Err(self.error(
                ErrorKind::InvalidDatumLabel,
                "a datum label cannot directly refer to itself",
                span,
            ));
        }
        self.nodes[placeholder.0 as usize].kind = NodeKind::Alias(Some(target));
        Ok(target)
    }

    fn error(&self, kind: ErrorKind, message: impl Into<String>, span: Span) -> Error {
        Error::from_diagnostic(Diagnostic::new(kind, message).with_label(
            span,
            crate::LabelStyle::Primary,
            "here",
        ))
    }
}

#[derive(Debug)]
struct Lexer {
    source: crate::SourceId,
    input: String,
    offset: usize,
    max_token: usize,
    max_nesting: usize,
    fold_case: bool,
}

impl Lexer {
    fn new(source: crate::SourceId, input: String, max_token: usize, max_nesting: usize) -> Self {
        Self {
            source,
            input,
            offset: 0,
            max_token,
            max_nesting,
            fold_case: false,
        }
    }
    fn next(&mut self) -> Result<Option<Token>, Error> {
        self.space()?;
        if self.offset == self.input.len() {
            return Ok(None);
        }
        let start = self.offset;
        let tail = &self.input[start..];
        let token = if tail.starts_with("#;") {
            self.offset += 2;
            TokenKind::DatumComment
        } else if tail.starts_with("#(") {
            self.offset += 2;
            TokenKind::VectorOpen
        } else if tail
            .get(..4)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("#u8("))
        {
            self.offset += 4;
            TokenKind::BytevectorOpen
        } else if tail.starts_with("'") {
            self.offset += 1;
            TokenKind::Quote
        } else if tail.starts_with('`') {
            self.offset += 1;
            TokenKind::Quasiquote
        } else if tail.starts_with(",@") {
            self.offset += 2;
            TokenKind::UnquoteSplicing
        } else if tail.starts_with(',') {
            self.offset += 1;
            TokenKind::Unquote
        } else if tail.starts_with('(') {
            self.offset += 1;
            TokenKind::Open
        } else if tail.starts_with(')') {
            self.offset += 1;
            TokenKind::Close
        } else if tail.starts_with('"') {
            TokenKind::String(self.string(start)?)
        } else if tail.starts_with('|') {
            TokenKind::Identifier(self.escaped_identifier(start)?)
        } else if tail.starts_with("#\\") {
            self.character_token(start)?
        } else {
            self.atom(start)?
        };
        let span = self.span(start, self.offset)?;
        Ok(Some(Token { kind: token, span }))
    }

    fn space(&mut self) -> Result<(), Error> {
        loop {
            while self.current().is_some_and(char::is_whitespace) {
                self.bump();
            }
            if self.remaining().starts_with(';') {
                while self.current().is_some_and(|c| c != '\n' && c != '\r') {
                    self.bump();
                }
                continue;
            }
            if self.remaining().starts_with("#|") {
                self.block_comment()?;
                continue;
            }
            if self.remaining().starts_with("#!fold-case") && self.delimited(self.offset + 11) {
                self.offset += 11;
                self.fold_case = true;
                continue;
            }
            if self.remaining().starts_with("#!no-fold-case") && self.delimited(self.offset + 14) {
                self.offset += 14;
                self.fold_case = false;
                continue;
            }
            return Ok(());
        }
    }

    fn block_comment(&mut self) -> Result<(), Error> {
        let start = self.offset;
        self.offset += 2;
        let mut depth = 1usize;
        while self.offset < self.input.len() {
            if self.remaining().starts_with("#|") {
                depth += 1;
                if depth > self.max_nesting {
                    return Err(self.error(
                        ErrorKind::ReaderLimitExceeded,
                        "nested comment exceeds configured nesting limit",
                        start,
                        self.offset + 2,
                    ));
                }
                self.offset += 2;
            } else if self.remaining().starts_with("|#") {
                depth -= 1;
                self.offset += 2;
                if depth == 0 {
                    return Ok(());
                }
            } else {
                self.bump();
            }
        }
        Err(self.error(
            ErrorKind::UnexpectedEof,
            "unterminated block comment",
            start,
            self.offset,
        ))
    }

    fn character_token(&mut self, start: usize) -> Result<TokenKind, Error> {
        // Consume the "#\" prefix.
        self.bump();
        self.bump();
        // The character right after "#\" is always part of the literal, even
        // when it is a delimiter such as a bracket, brace, or parenthesis. A
        // named or hex literal (space, newline, x41, and so on) then continues
        // with the usual run of non-delimiter characters. Taking the leading
        // character unconditionally is what keeps a bare "#\(" from ending the
        // token before the delimiter is read.
        if let Some(first) = self.current() {
            self.bump();
            if first.is_alphabetic() {
                while self.current().is_some_and(|c| !is_delimiter(c)) {
                    self.bump();
                }
            }
        }
        self.check_token(start)?;
        let mut value = self.input[start..self.offset].to_owned();
        if self.fold_case {
            value = fold_case(&value);
        }
        Ok(TokenKind::Atom(value))
    }

    fn atom(&mut self, start: usize) -> Result<TokenKind, Error> {
        while self.current().is_some_and(|c| !is_delimiter(c)) {
            self.bump();
        }
        if start == self.offset {
            self.bump();
            return Err(self.error(
                ErrorKind::InvalidToken,
                "reserved delimiter",
                start,
                self.offset,
            ));
        }
        self.check_token(start)?;
        let mut value = self.input[start..self.offset].to_owned();
        if self.fold_case {
            value = fold_case(&value);
        }
        if let Some(label) = value.strip_prefix('#').and_then(|v| v.strip_suffix('=')) {
            return label.parse().map(TokenKind::LabelDef).map_err(|_| {
                self.error(
                    ErrorKind::InvalidDatumLabel,
                    "invalid datum label",
                    start,
                    self.offset,
                )
            });
        }
        if let Some(label) = value.strip_prefix('#').and_then(|v| v.strip_suffix('#')) {
            return label.parse().map(TokenKind::LabelRef).map_err(|_| {
                self.error(
                    ErrorKind::InvalidDatumLabel,
                    "invalid datum label",
                    start,
                    self.offset,
                )
            });
        }
        Ok(TokenKind::Atom(value))
    }

    fn escaped_identifier(&mut self, start: usize) -> Result<String, Error> {
        self.bump();
        let mut output = String::new();
        loop {
            let Some(character) = self.current() else {
                return Err(self.error(
                    ErrorKind::UnexpectedEof,
                    "unterminated escaped identifier",
                    start,
                    self.offset,
                ));
            };
            self.bump();
            match character {
                '|' => break,
                '\\' => output.push(self.escape(start)?),
                _ => output.push(character),
            }
        }
        self.check_token(start)?;
        if self.fold_case {
            Ok(fold_case(&output))
        } else {
            Ok(output)
        }
    }

    fn string(&mut self, start: usize) -> Result<String, Error> {
        self.bump();
        let mut output = String::new();
        loop {
            let Some(character) = self.current() else {
                return Err(self.error(
                    ErrorKind::UnexpectedEof,
                    "unterminated string",
                    start,
                    self.offset,
                ));
            };
            self.bump();
            match character {
                '"' => break,
                '\\' if self.consume_string_continuation() => {}
                '\\' => output.push(self.escape(start)?),
                _ => output.push(character),
            }
        }
        self.check_token(start)?;
        Ok(output)
    }

    fn escape(&mut self, start: usize) -> Result<char, Error> {
        let Some(character) = self.current() else {
            return Err(self.error(
                ErrorKind::UnexpectedEof,
                "unterminated escape",
                start,
                self.offset,
            ));
        };
        self.bump();
        match character {
            'a' => Ok('\x07'),
            'b' => Ok('\x08'),
            't' => Ok('\t'),
            'n' => Ok('\n'),
            'r' => Ok('\r'),
            '"' => Ok('"'),
            '\\' => Ok('\\'),
            '|' => Ok('|'),
            'x' => {
                let begin = self.offset;
                while self.current().is_some_and(|c| c.is_ascii_hexdigit()) {
                    self.bump();
                }
                if !self.remaining().starts_with(';') {
                    return Err(self.error(
                        ErrorKind::InvalidToken,
                        "hex escape must end in ';'",
                        begin,
                        self.offset,
                    ));
                }
                let value = scalar(&self.input[begin..self.offset]).map_err(|message| {
                    self.error(ErrorKind::InvalidToken, message, begin, self.offset)
                })?;
                self.bump();
                Ok(value)
            }
            _ => Err(self.error(
                ErrorKind::InvalidToken,
                "unknown escape",
                start,
                self.offset,
            )),
        }
    }

    fn consume_string_continuation(&mut self) -> bool {
        let saved = self.offset;
        while self.current().is_some_and(|c| c == ' ' || c == '\t') {
            self.bump();
        }
        match self.current() {
            Some('\n') => self.bump(),
            Some('\r') => {
                self.bump();
                if self.current() == Some('\n') {
                    self.bump();
                }
            }
            _ => {
                self.offset = saved;
                return false;
            }
        }
        while self.current().is_some_and(|c| c == ' ' || c == '\t') {
            self.bump();
        }
        true
    }

    fn current(&self) -> Option<char> {
        self.input[self.offset..].chars().next()
    }
    fn bump(&mut self) {
        if let Some(character) = self.current() {
            self.offset += character.len_utf8();
        }
    }
    fn remaining(&self) -> &str {
        &self.input[self.offset..]
    }
    fn delimited(&self, offset: usize) -> bool {
        offset == self.input.len()
            || self.input[offset..]
                .chars()
                .next()
                .is_some_and(is_delimiter)
    }
    fn check_token(&self, start: usize) -> Result<(), Error> {
        if self.offset - start > self.max_token {
            Err(self.error(
                ErrorKind::ReaderLimitExceeded,
                "token exceeds configured limit",
                start,
                self.offset,
            ))
        } else {
            Ok(())
        }
    }
    fn span(&self, start: usize, end: usize) -> Result<Span, Error> {
        Span::new(self.source, start as u32, end as u32)
            .ok_or_else(|| self.error(ErrorKind::InvalidToken, "invalid token span", start, end))
    }
    fn error(
        &self,
        kind: ErrorKind,
        message: impl Into<String>,
        start: usize,
        end: usize,
    ) -> Error {
        Error::from_diagnostic(Diagnostic::new(kind, message).with_label(
            Span::new(self.source, start as u32, end as u32).unwrap(),
            crate::LabelStyle::Primary,
            "here",
        ))
    }
}

fn scalar(text: &str) -> Result<char, String> {
    let value = u32::from_str_radix(text, 16).map_err(|_| "invalid hex scalar value".to_owned())?;
    char::from_u32(value).ok_or_else(|| "hex escape is not a Unicode scalar value".to_owned())
}

fn is_delimiter(c: char) -> bool {
    c.is_whitespace()
        || matches!(
            c,
            '(' | ')' | '"' | ';' | '|' | '[' | ']' | '{' | '}' | '\'' | '`' | ','
        )
}

pub(crate) fn valid_identifier(value: &str) -> bool {
    if value.is_empty() {
        return false;
    }
    if value == "+" || value == "-" || value == "..." {
        return true;
    }
    let mut chars = value.chars();
    let first = chars.next().unwrap();
    let initial = first.is_alphabetic()
        || matches!(
            first,
            '!' | '$' | '%' | '&' | '*' | '/' | ':' | '<' | '=' | '>' | '?' | '@' | '^' | '_' | '~'
        )
        || (!first.is_ascii() && !first.is_whitespace() && !first.is_control());
    initial
        && chars.all(|c| {
            c.is_alphanumeric()
                || matches!(
                    c,
                    '!' | '$'
                        | '%'
                        | '&'
                        | '*'
                        | '+'
                        | '-'
                        | '.'
                        | '/'
                        | ':'
                        | '<'
                        | '='
                        | '>'
                        | '?'
                        | '@'
                        | '^'
                        | '_'
                        | '~'
                )
                || (!c.is_ascii() && !c.is_whitespace() && !c.is_control())
        })
}

fn fold_case(value: &str) -> String {
    value
        .chars()
        .flat_map(|character| match character {
            'ß' | 'ẞ' => "ss".chars().collect::<Vec<_>>(),
            'ς' => vec!['σ'],
            'ſ' => vec!['s'],
            character => character.to_lowercase().collect(),
        })
        .collect()
}
