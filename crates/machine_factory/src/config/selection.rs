//! The machine selection: one model of one family, and the parser of the
//! `machine` values.

#[cfg(feature = "pc98")]
use common::MachineModel;
#[cfg(feature = "pc60")]
use machine_60::Pc6000Model;
#[cfg(feature = "pc88")]
use machine_88::Pc8801Model;
#[cfg(feature = "pc88va")]
use machine_88va::Pc88VaModel;
#[cfg(feature = "at")]
use machine_at::AtModel;
#[cfg(feature = "fm7")]
use machine_fm7::Fm7Model;
#[cfg(feature = "msx")]
use machine_msx::MsxModel;
#[cfg(feature = "towns")]
use machine_towns::TownsModel;
#[cfg(feature = "x1")]
use machine_x1::X1Model;
#[cfg(feature = "x68k")]
use machine_x68k::X68kModel;

use super::{
    Target, join_alternatives,
    settings::compiled_model_names,
    systems::{System, system_of},
};

/// One machine model of one family compiled into this build.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum MachineSelection {
    /// A PC-9800 series model.
    #[cfg(feature = "pc98")]
    Pc98(MachineModel),
    /// A PC-8801 series model.
    #[cfg(feature = "pc88")]
    Pc88(Pc8801Model),
    /// A PC-88VA series model.
    #[cfg(feature = "pc88va")]
    Pc88Va(Pc88VaModel),
    /// A PC-6000 / PC-6600 series model.
    #[cfg(feature = "pc60")]
    Pc60(Pc6000Model),
    /// An MSX series model.
    #[cfg(feature = "msx")]
    Msx(MsxModel),
    /// An FM Towns series model.
    #[cfg(feature = "towns")]
    Towns(TownsModel),
    /// A Sharp X1 series model.
    #[cfg(feature = "x1")]
    X1(X1Model),
    /// A Fujitsu FM-7 series model.
    #[cfg(feature = "fm7")]
    Fm7(Fm7Model),
    /// A Sharp X68000 series model.
    #[cfg(feature = "x68k")]
    X68k(X68kModel),
    /// An IBM PC/AT series model.
    #[cfg(feature = "at")]
    At(AtModel),
}

impl MachineSelection {
    /// Returns the family of the model.
    #[must_use]
    pub fn target(self) -> Target {
        match self {
            #[cfg(feature = "pc98")]
            Self::Pc98(_) => Target::Pc98,
            #[cfg(feature = "pc88")]
            Self::Pc88(_) => Target::Pc88,
            #[cfg(feature = "pc88va")]
            Self::Pc88Va(_) => Target::Pc88Va,
            #[cfg(feature = "pc60")]
            Self::Pc60(_) => Target::Pc60,
            #[cfg(feature = "msx")]
            Self::Msx(_) => Target::Msx,
            #[cfg(feature = "towns")]
            Self::Towns(_) => Target::Towns,
            #[cfg(feature = "x1")]
            Self::X1(_) => Target::X1,
            #[cfg(feature = "fm7")]
            Self::Fm7(_) => Target::Fm7,
            #[cfg(feature = "x68k")]
            Self::X68k(_) => Target::X68k,
            #[cfg(feature = "at")]
            Self::At(_) => Target::At,
        }
    }

    /// Returns the model name in the form of the `machine` values.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            #[cfg(feature = "pc98")]
            Self::Pc98(model) => match model {
                MachineModel::PC9801F => "PC9801F",
                MachineModel::PC9801VM => "PC9801VM",
                MachineModel::PC9801VX => "PC9801VX",
                MachineModel::PC9801RS => "PC9801RS",
                MachineModel::PC9801RA => "PC9801RA",
                MachineModel::PC9821AS => "PC9821AS",
                MachineModel::PC9821AP => "PC9821AP",
            },
            #[cfg(feature = "pc88")]
            Self::Pc88(model) => match model {
                Pc8801Model::PC8801MC => "PC8801MC",
            },
            #[cfg(feature = "pc88va")]
            Self::Pc88Va(model) => match model {
                Pc88VaModel::PC88VA2 => "PC88VA2",
            },
            #[cfg(feature = "pc60")]
            Self::Pc60(model) => match model {
                Pc6000Model::Pc6001 => "PC6001",
                Pc6000Model::Pc6001Mk2 => "PC6001MK2",
                Pc6000Model::Pc6601 => "PC6601",
                Pc6000Model::Pc6001Mk2Sr => "PC6001MK2SR",
                Pc6000Model::Pc6601Sr => "PC6601SR",
            },
            #[cfg(feature = "msx")]
            Self::Msx(model) => match model {
                MsxModel::Msx => "MSX",
                MsxModel::Msx2 => "MSX2",
                MsxModel::Msx2Plus => "MSX2PLUS",
            },
            #[cfg(feature = "towns")]
            Self::Towns(model) => match model {
                TownsModel::FmTowns => "FMTowns",
                TownsModel::FmTownsIICx => "FMTownsIICX",
                TownsModel::FmTownsIIMx => "FMTownsIIMX",
            },
            #[cfg(feature = "x1")]
            Self::X1(model) => match model {
                X1Model::X1 => "X1",
                X1Model::X1Turbo => "X1TURBO",
            },
            #[cfg(feature = "fm7")]
            Self::Fm7(model) => match model {
                Fm7Model::Fm7 => "FM7",
                Fm7Model::Fm77Av => "FM77AV",
            },
            #[cfg(feature = "x68k")]
            Self::X68k(model) => match model {
                X68kModel::X68000 => "X68000",
                X68kModel::X68000Super => "X68000SUPER",
                X68kModel::X68000Xvi => "X68000XVI",
            },
            #[cfg(feature = "at")]
            Self::At(model) => match model {
                AtModel::At486Dx50 => "AT486DX50",
                AtModel::At486Dx66 => "AT486DX66",
            },
        }
    }

    /// Returns the default model of `system`. `None` for a system outside this build.
    #[must_use]
    pub fn of_system(system: &System) -> Option<Self> {
        system.default_model.parse().ok()
    }

    /// Returns the default model of the family `target` from the systems table.
    #[must_use]
    pub fn default_of(target: Target) -> Self {
        Self::of_system(system_of(target))
            .expect("the systems table holds a model of each compiled family")
    }
}

impl std::str::FromStr for MachineSelection {
    type Err = String;

    /// Parses a `machine` value. Each compiled family is offered the value in
    /// turn and the PC-98 models are tried last. The case of the value does
    /// not matter.
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        #[cfg(feature = "pc88")]
        if let Ok(model) = value.parse::<Pc8801Model>() {
            return Ok(Self::Pc88(model));
        }
        #[cfg(feature = "pc88va")]
        if let Ok(model) = value.parse::<Pc88VaModel>() {
            return Ok(Self::Pc88Va(model));
        }
        #[cfg(feature = "pc60")]
        if let Ok(model) = value.parse::<Pc6000Model>() {
            return Ok(Self::Pc60(model));
        }
        #[cfg(feature = "msx")]
        if let Ok(model) = value.parse::<MsxModel>() {
            return Ok(Self::Msx(model));
        }
        #[cfg(feature = "x1")]
        if let Ok(model) = value.parse::<X1Model>() {
            return Ok(Self::X1(model));
        }
        #[cfg(feature = "fm7")]
        if let Ok(model) = value.parse::<Fm7Model>() {
            return Ok(Self::Fm7(model));
        }
        #[cfg(feature = "towns")]
        if let Ok(model) = value.parse::<TownsModel>() {
            return Ok(Self::Towns(model));
        }
        #[cfg(feature = "x68k")]
        if let Ok(model) = value.parse::<X68kModel>() {
            return Ok(Self::X68k(model));
        }
        #[cfg(feature = "at")]
        if let Ok(model) = value.parse::<AtModel>() {
            return Ok(Self::At(model));
        }
        #[cfg(feature = "pc98")]
        if let Ok(model) = value.parse::<MachineModel>() {
            return Ok(Self::Pc98(model));
        }
        Err(format!(
            "unknown machine type '{value}', expected {}",
            join_alternatives(&compiled_model_names())
        ))
    }
}

impl std::fmt::Display for MachineSelection {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.name())
    }
}

#[cfg(test)]
mod tests {
    use super::{
        super::systems::{SYSTEMS, compiled_systems},
        *,
    };

    /// Verifies that every compiled model name parses to a selection with the same name and family.
    #[test]
    fn every_compiled_model_name_round_trips() {
        for name in compiled_model_names() {
            let selection: MachineSelection = name.parse().expect("model name parses");
            assert_eq!(selection.name(), name);
            let lowercase: MachineSelection = name
                .to_ascii_lowercase()
                .parse()
                .expect("model names ignore case");
            assert_eq!(lowercase, selection);
        }
        assert!("FOOBAR".parse::<MachineSelection>().is_err());
    }

    /// Verifies that the default model of each compiled system belongs to the family of the system.
    #[test]
    fn the_default_of_a_system_belongs_to_its_family() {
        for system in compiled_systems() {
            let selection = MachineSelection::of_system(system).expect("compiled system");
            assert_eq!(system_of(selection.target()), system);
            assert_eq!(MachineSelection::default_of(selection.target()), selection);
            assert_eq!(selection.name(), system.default_model);
        }
        let compiled = compiled_systems().count();
        let parsed = SYSTEMS
            .iter()
            .filter_map(MachineSelection::of_system)
            .count();
        assert_eq!(parsed, compiled);
    }
}
