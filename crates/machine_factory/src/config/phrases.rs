//! The phrases of the configuration texts: the English list of the accepted names and the number
//! of a clock frequency in megahertz.

use std::fmt;

/// The number of hertz in one megahertz.
const HERTZ_PER_MEGAHERTZ: f64 = 1_000_000.0;

/// The text of an error message in place of an empty list of accepted names.
const NO_NAMES: &str = "(none)";

/// Joins the accepted `names` of an error message with commas and `or` before the last name:
/// `a`, `a or b` or `a, b or c`. An empty list gives `(none)`.
#[must_use]
pub(crate) fn expected_names(names: &[&str]) -> String {
    match names {
        [] => NO_NAMES.to_owned(),
        [only] => (*only).to_owned(),
        [rest @ .., last] => format!("{} or {last}", rest.join(", ")),
    }
}

/// A clock frequency in hertz. It formats as the number of megahertz with at most one decimal and
/// without the unit, for example `16.7`.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct Megahertz(pub u32);

impl fmt::Display for Megahertz {
    /// Writes the number of megahertz with at most one decimal.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let tenths = (f64::from(self.0) / HERTZ_PER_MEGAHERTZ * 10.0).round() / 10.0;
        write!(formatter, "{tenths}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verifies that the accepted names join with commas and `or` and that an empty list gives
    /// `(none)`.
    #[test]
    fn expected_names_join_with_or_or_give_none() {
        assert_eq!(expected_names(&[]), "(none)");
        assert_eq!(expected_names(&["pc98"]), "pc98");
        assert_eq!(expected_names(&["pc98", "x1"]), "pc98 or x1");
        assert_eq!(expected_names(&["pc98", "pc88", "x1"]), "pc98, pc88 or x1");
    }

    /// Verifies the number of megahertz of the clock facts.
    #[test]
    fn clocks_format_as_the_number_of_megahertz() {
        assert_eq!(Megahertz(20_000_000).to_string(), "20");
        assert_eq!(Megahertz(3_993_600).to_string(), "4");
        assert_eq!(Megahertz(16_666_667).to_string(), "16.7");
    }
}
