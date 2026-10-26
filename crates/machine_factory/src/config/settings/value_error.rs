//! The faults of a setting value: the data of each fault and its English text.

use std::{error::Error, fmt};

use super::{Choice, Families, NumberRange};
use crate::config::phrases::expected_names;

/// A value that a setting rejects, with the data of the fault.
#[derive(Debug, Clone, PartialEq)]
pub enum ValueError {
    /// A value of an on/off setting other than `on` and `off`.
    OnOff {
        /// The value as the file writes it.
        value: String,
    },
    /// An empty value of a folder setting that needs a folder.
    EmptyFolder,
    /// An empty value of a file setting that needs a file.
    EmptyFile,
    /// A value of a number setting that is no number or lies outside the range.
    OutOfRange {
        /// The value as the file writes it.
        value: String,
        /// The range of the setting.
        range: NumberRange,
    },
    /// A value of a choice setting that matches no choice.
    UnknownChoice {
        /// The value as the file writes it.
        value: String,
        /// The choices of the setting.
        choices: &'static [Choice],
    },
    /// A language value that is neither `auto` nor a language tag.
    Language {
        /// The value as the file writes it.
        value: String,
    },
    /// A release year that is no year or lies outside the years of the metadata.
    Year {
        /// The value as the file writes it.
        value: String,
        /// The first accepted year.
        first: u16,
        /// The last accepted year.
        last: u16,
    },
    /// No system is compiled into the build to check the value against.
    NoSystem,
    /// The system to check the value against is not compiled into the build.
    NotCompiled {
        /// The name of the system.
        system: &'static str,
    },
}

/// The ranges of the number settings hold no `NaN`. The equality of two errors is total.
impl Eq for ValueError {}

impl ValueError {
    /// Returns the error of `value`, which matches none of `choices`.
    #[must_use]
    pub fn unknown_choice(value: &str, choices: &'static [Choice]) -> Self {
        Self::UnknownChoice {
            value: value.to_owned(),
            choices,
        }
    }
}

/// Returns the values of `choices` that a compiled family accepts, in the order of the table.
pub fn compiled_choice_values(choices: &'static [Choice]) -> impl Iterator<Item = &'static str> {
    let compiled = Families::compiled();
    choices
        .iter()
        .filter(move |choice| choice.families.intersects(compiled))
        .map(|choice| choice.value)
}

/// One end of the range of a number setting: a whole number, or a decimal number with at least
/// one digit after the decimal point.
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct RangeEnd {
    /// The number.
    pub value: f64,
    /// Whether the range holds whole numbers.
    pub integer: bool,
}

impl NumberRange {
    /// Returns the smallest value of the range as an end of the range.
    #[must_use]
    pub fn minimum_end(self) -> RangeEnd {
        RangeEnd {
            value: self.minimum,
            integer: self.integer,
        }
    }

    /// Returns the largest value of the range as an end of the range.
    #[must_use]
    pub fn maximum_end(self) -> RangeEnd {
        RangeEnd {
            value: self.maximum,
            integer: self.integer,
        }
    }
}

impl fmt::Display for RangeEnd {
    /// Writes the number, for example `3` or `1.0`.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.integer {
            write!(formatter, "{:.0}", self.value)
        } else {
            write!(formatter, "{:?}", self.value)
        }
    }
}

impl fmt::Display for ValueError {
    /// Writes the English text of the fault, for example `invalid value 'maybe', expected on or
    /// off`.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OnOff { value } => {
                write!(formatter, "invalid value '{value}', expected on or off")
            }
            Self::EmptyFolder => formatter.write_str("the value is empty, a folder is needed"),
            Self::EmptyFile => formatter.write_str("the value is empty, a file is needed"),
            Self::OutOfRange { value, range } => write!(
                formatter,
                "invalid value '{value}', expected a number from {} to {}",
                range.minimum_end(),
                range.maximum_end()
            ),
            Self::UnknownChoice { value, choices } => {
                let values: Vec<&str> = compiled_choice_values(choices).collect();
                write!(
                    formatter,
                    "unknown value '{value}', expected {}",
                    expected_names(&values)
                )
            }
            Self::Language { value } => write!(
                formatter,
                "invalid GUI language '{value}', expected auto or a language tag of letters, \
                 digits and -"
            ),
            Self::Year { value, first, last } => {
                write!(formatter, "'{value}' is not a year from {first} to {last}")
            }
            Self::NoSystem => formatter.write_str("no system is compiled"),
            Self::NotCompiled { system } => {
                write!(formatter, "the system {system} is not compiled")
            }
        }
    }
}

impl Error for ValueError {}

#[cfg(test)]
mod tests {
    use super::*;

    /// The choices of a test setting.
    const CHOICES: &[Choice] = &[
        Choice {
            value: "low",
            families: Families::ALL,
        },
        Choice {
            value: "high",
            families: Families::ALL,
        },
        Choice {
            value: "none",
            families: Families::NONE,
        },
    ];

    /// Verifies the English text of every fault.
    #[test]
    fn every_fault_writes_its_english_text() {
        let range = NumberRange {
            minimum: 0.0,
            maximum: 3.0,
            step: 1.0,
            integer: true,
        };
        let volume = NumberRange {
            minimum: 0.0,
            maximum: 1.0,
            step: 0.1,
            integer: false,
        };
        let cases = [
            (
                ValueError::OnOff {
                    value: "maybe".to_owned(),
                },
                "invalid value 'maybe', expected on or off",
            ),
            (
                ValueError::EmptyFolder,
                "the value is empty, a folder is needed",
            ),
            (
                ValueError::EmptyFile,
                "the value is empty, a file is needed",
            ),
            (
                ValueError::OutOfRange {
                    value: "4".to_owned(),
                    range,
                },
                "invalid value '4', expected a number from 0 to 3",
            ),
            (
                ValueError::OutOfRange {
                    value: "loud".to_owned(),
                    range: volume,
                },
                "invalid value 'loud', expected a number from 0.0 to 1.0",
            ),
            (
                ValueError::unknown_choice("medium", CHOICES),
                "unknown value 'medium', expected low or high",
            ),
            (
                ValueError::unknown_choice("medium", &CHOICES[2..]),
                "unknown value 'medium', expected (none)",
            ),
            (
                ValueError::Language {
                    value: "pt_BR".to_owned(),
                },
                "invalid GUI language 'pt_BR', expected auto or a language tag of letters, \
                 digits and -",
            ),
            (
                ValueError::Year {
                    value: "93".to_owned(),
                    first: 1970,
                    last: 2100,
                },
                "'93' is not a year from 1970 to 2100",
            ),
            (ValueError::NoSystem, "no system is compiled"),
            (
                ValueError::NotCompiled { system: "pc98" },
                "the system pc98 is not compiled",
            ),
        ];
        for (error, text) in cases {
            assert_eq!(error.to_string(), text);
        }
    }

    /// Verifies that the compiled choice values leave out the choices of no compiled family.
    #[test]
    fn compiled_choice_values_follow_the_families() {
        let values: Vec<&str> = compiled_choice_values(CHOICES).collect();
        assert_eq!(values, ["low", "high"]);
    }
}
