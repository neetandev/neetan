//! Parts of language files for the unit tests of the crate.

/// The header of an English language file. It spans 3 lines.
pub(crate) const ENGLISH_HEADER: &str = "[language]\nname = English\nplural = one-other\n";

/// The section `[list]` of an English language file. It spans 7 lines.
pub(crate) const ENGLISH_LIST: &str = "[list]\nand-pair = {first} and {second}\n\
                                       and-middle = {first}, {second}\n\
                                       and-end = {first}, and {second}\n\
                                       or-pair = {first} or {second}\n\
                                       or-middle = {first}; {second}\n\
                                       or-end = {first}; or {second}\n";

/// The header of a Japanese language file.
pub(crate) const JAPANESE_HEADER: &str = "[language]\nname = 日本語\nplural = none\n";

/// The section `[list]` of a Japanese language file.
pub(crate) const JAPANESE_LIST: &str = "[list]\nand-pair = {first}と{second}\n\
                                        and-middle = {first}、{second}\n\
                                        and-end = {first}、{second}\n\
                                        or-pair = {first}か{second}\n\
                                        or-middle = {first}、{second}\n\
                                        or-end = {first}か{second}\n";

/// Returns an English reference source with the header, the list patterns and a body.
pub(crate) fn english(body: &str) -> String {
    format!("{ENGLISH_HEADER}{ENGLISH_LIST}{body}")
}

/// Returns a Japanese source with the header, a body and the list patterns.
pub(crate) fn japanese(body: &str) -> String {
    format!("{JAPANESE_HEADER}{body}{JAPANESE_LIST}")
}
