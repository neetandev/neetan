//! The phrases of the configuration texts: lists of names and clock
//! frequencies.

/// The number of hertz in one megahertz.
const HERTZ_PER_MEGAHERTZ: f64 = 1_000_000.0;

/// The text of an error message in place of an empty list of accepted names.
const NO_NAMES: &str = "(none)";

/// Joins `names` into one phrase with `conjunction` before the last name:
/// `a`, `a and b` or `a, b and c` for the conjunction `and`. An empty list
/// gives an empty phrase.
#[must_use]
pub fn join_names(names: &[&str], conjunction: &str) -> String {
    match names {
        [] => String::new(),
        [only] => (*only).to_owned(),
        [rest @ .., last] => format!("{} {conjunction} {last}", rest.join(", ")),
    }
}

/// Joins the accepted `names` of an error message with `or`. An empty list
/// gives `(none)`.
#[must_use]
pub(crate) fn expected_names(names: &[&str]) -> String {
    if names.is_empty() {
        NO_NAMES.to_owned()
    } else {
        join_names(names, "or")
    }
}

/// Formats `hertz` in megahertz with at most one decimal, for example `16.7 MHz`.
#[must_use]
pub(crate) fn format_megahertz(hertz: u32) -> String {
    let tenths = (f64::from(hertz) / HERTZ_PER_MEGAHERTZ * 10.0).round() / 10.0;
    format!("{tenths} MHz")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verifies the phrase of a list of names with each conjunction.
    #[test]
    fn names_join_with_commas_and_the_conjunction() {
        assert_eq!(join_names(&[], "and"), "");
        assert_eq!(join_names(&["PC6601"], "and"), "PC6601");
        assert_eq!(join_names(&["A", "B"], "and"), "A and B");
        assert_eq!(join_names(&["A", "B", "C"], "and"), "A, B and C");
        assert_eq!(join_names(&["A", "B", "C"], "or"), "A, B or C");
    }

    /// Verifies that the accepted names join with `or` and that an empty list gives `(none)`.
    #[test]
    fn expected_names_join_with_or_or_give_none() {
        assert_eq!(expected_names(&[]), "(none)");
        assert_eq!(expected_names(&["pc98"]), "pc98");
        assert_eq!(expected_names(&["pc98", "pc88", "x1"]), "pc98, pc88 or x1");
    }

    /// Verifies the megahertz of the clock facts.
    #[test]
    fn clocks_format_in_megahertz() {
        assert_eq!(format_megahertz(20_000_000), "20 MHz");
        assert_eq!(format_megahertz(3_993_600), "4 MHz");
        assert_eq!(format_megahertz(16_666_667), "16.7 MHz");
    }
}
