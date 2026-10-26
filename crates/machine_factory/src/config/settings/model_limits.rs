//! The model limits of the settings table: the models of a family that read
//! a setting, why the other models ignore it, the facts that the note of a
//! choice setting adds, and the model lists of the limits.

use common::CpuMode;

use super::Families;
use crate::config::{MachineSelection, phrases::Megahertz};

/// Models of one family that read a setting.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct ModelLimit {
    /// The family the limit applies to.
    pub family: Families,
    /// The model names of the family that read the setting, in the form of the `machine` values.
    pub models: &'static [&'static str],
    /// Why another model of the family ignores the setting.
    pub reason: LimitReason,
}

/// Why a model ignores a setting that other models of its family read.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum LimitReason {
    /// A sentence about the model. The sentence is a key of the section `[limit]` of the
    /// language files, for example `no-cdrom` for `{model} has no CD-ROM drive.`
    Phrase(&'static str),
    /// The model runs its CPU at one clock. The sentence gives the clock of the model, for example
    /// `PC9801RA runs at a fixed 20 MHz.`
    FixedClock,
}

impl LimitReason {
    /// Returns the clock of `model` for a fixed clock reason. A phrase reason and a model without
    /// a known clock have none.
    #[must_use]
    pub fn fixed_clock(self, model: MachineSelection) -> Option<Megahertz> {
        match self {
            Self::Phrase(_) => None,
            Self::FixedClock => model.cpu_clock_hz(CpuMode::High).map(Megahertz),
        }
    }
}

/// The fact about the values of a choice setting that its note adds.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum ChoiceFact {
    /// The note adds no fact.
    None,
    /// The note lists the choices of the family, for example `The choices are auto, 15k and
    /// 24k.`
    Choices,
    /// The note gives the CPU clocks of both CPU modes of a model with two clocks, for example
    /// `Low runs at 4 MHz and high at 8 MHz.`
    CpuClock,
}

/// The data of a fact about the values of a choice setting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FactData {
    /// The choice values that the family of the model accepts, in the order of the table.
    Choices(Vec<&'static str>),
    /// The CPU clocks of the model in both CPU modes.
    CpuClock {
        /// The CPU clock in the low mode.
        low: Megahertz,
        /// The CPU clock in the high mode.
        high: Megahertz,
    },
}

/// PC-98 models with a selectable CPU clock.
pub(super) const PC98_CPU_MODE_MODELS: &[&str] = &["PC9801F", "PC9801VM", "PC9801VX"];
/// PC-98 models with EMS, XMS and the 5 MHz GDC clock.
pub(super) const PC98_VX_AND_LATER_MODELS: &[&str] =
    &["PC9801VX", "PC9801RS", "PC9801RA", "PC9821AS", "PC9821AP"];
/// PC-98 models that boot a real BIOS.
pub(super) const PC98_REAL_BIOS_MODELS: &[&str] =
    &["PC9801F", "PC9801VM", "PC9801VX", "PC9801RS", "PC9801RA"];
/// PC-98 models with a CD-ROM drive.
pub(super) const PC98_CDROM_MODELS: &[&str] = &["PC9821AS", "PC9821AP"];
/// PC-6000 models with built-in floppy drives.
pub(super) const PC60_FDD_MODELS: &[&str] = &["PC6601", "PC6601SR"];
/// MSX models with a floppy drive.
pub(super) const MSX_FDD_MODELS: &[&str] = &["MSX2", "MSX2PLUS"];
/// FM Towns models with a selectable CPU clock.
pub(super) const TOWNS_CPU_MODE_MODELS: &[&str] = &["FMTownsIICX", "FMTownsIIMX"];
/// X68000 models with a selectable CPU clock.
pub(super) const X68K_CPU_MODE_MODELS: &[&str] = &["X68000XVI"];
/// X68000 models with a CD-ROM capable SCSI interface.
pub(super) const X68K_CDROM_MODELS: &[&str] = &["X68000SUPER", "X68000XVI"];
/// X1 models with the turbo monitor and keyboard switches.
pub(super) const X1_TURBO_MODELS: &[&str] = &["X1TURBO"];

#[cfg(test)]
mod tests {
    use super::{super::MODEL_CHOICES, *};

    /// The model lists of the limits with the family of their models.
    const MODEL_LISTS: [(&[&str], Families); 10] = [
        (PC98_CPU_MODE_MODELS, Families::PC98),
        (PC98_VX_AND_LATER_MODELS, Families::PC98),
        (PC98_REAL_BIOS_MODELS, Families::PC98),
        (PC98_CDROM_MODELS, Families::PC98),
        (PC60_FDD_MODELS, Families::PC60),
        (MSX_FDD_MODELS, Families::MSX),
        (TOWNS_CPU_MODE_MODELS, Families::TOWNS),
        (X68K_CPU_MODE_MODELS, Families::X68K),
        (X68K_CDROM_MODELS, Families::X68K),
        (X1_TURBO_MODELS, Families::X1),
    ];

    /// Verifies that every model list holds distinct models of its family in the form of the `machine` values.
    #[test]
    fn every_model_list_holds_models_of_its_family() {
        for (models, family) in MODEL_LISTS {
            assert!(!models.is_empty());
            for (index, model) in models.iter().enumerate() {
                assert!(
                    MODEL_CHOICES
                        .iter()
                        .any(|choice| choice.value == *model && choice.families == family),
                    "{model}"
                );
                assert!(!models[..index].contains(model), "{model}");
            }
        }
    }

    /// Verifies that a phrase reason gives no clock on any model.
    #[test]
    fn a_phrase_reason_gives_no_clock() {
        let reason = LimitReason::Phrase("no-cdrom");
        for choice in MODEL_CHOICES {
            if let Ok(model) = choice.value.parse::<MachineSelection>() {
                assert_eq!(reason.fixed_clock(model), None);
            }
        }
    }

    #[cfg(feature = "pc98")]
    /// Verifies that a fixed clock reason gives the number of megahertz of the model.
    #[test]
    fn a_fixed_clock_reason_gives_the_clock_of_the_model() {
        let model: MachineSelection = "PC9801RA".parse().expect("a PC-98 model");
        let clock = LimitReason::FixedClock.fixed_clock(model);
        assert_eq!(clock.map(|clock| clock.to_string()).as_deref(), Some("20"));
    }
}
