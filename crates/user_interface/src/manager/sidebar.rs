//! The system sidebar of the library: the systems with their game count and
//! ROM status, the line with the ROM status of the selected system and the
//! button that opens its system defaults.

use gui::{
    Rect,
    ui::{
        Align, Button, HEADER_HEIGHT, Icon, List, ListResponse, ListState, PATH_LINE_HEIGHT,
        RowContent, RowMark, TextBuffer, TextValue, Truncation, UiContext, UiId,
    },
};
use i18n::Catalog;
use machine_factory::{
    config::{MachineSelection, Target, systems::System},
    rom_status::RomStatus,
};

use super::rows::{GameRow, shown_path};
#[cfg(test)]
use crate::texts::english_text;
use crate::{
    APPLICATION_TITLE,
    config::HostLayers,
    rom_folders::{RomFolderStatus, RomSeverity, rom_severity},
    texts::{NumberText, common, library},
};

/// The id of the system list.
pub(crate) const SYSTEM_LIST_ID: &str = "systems";

/// The id of the button that opens the system defaults of the selected system.
pub(crate) const SYSTEM_DEFAULTS_ID: &str = "system defaults";

/// The ROM status of a system as the line below the system list tells it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RomLine {
    /// Every ROM set is in the folder.
    Found,
    /// Some ROM sets are missing.
    Partial,
    /// The ROM sets are missing.
    Missing,
    /// The folder holds no ROM set that the system runs without.
    NotFound,
    /// This build cannot check the folder.
    Unsupported,
}

impl TextValue for RomLine {}

impl RomLine {
    /// The line of the ROM status `status`.
    fn of(status: RomStatus) -> Self {
        match status {
            RomStatus::Found => Self::Found,
            RomStatus::Partial => Self::Partial,
            RomStatus::Missing => Self::Missing,
            RomStatus::NotFound => Self::NotFound,
            RomStatus::Unsupported => Self::Unsupported,
        }
    }

    /// Writes the sentence of the line for the ROM folder `folder` in `catalog`.
    fn write(self, catalog: &Catalog, folder: &str, out: &mut String) {
        let _ = match self {
            Self::Found => catalog.write(out, library::roms_found(&folder)),
            Self::Partial => catalog.write(out, library::roms_partial(&folder)),
            Self::Missing => catalog.write(out, library::roms_missing(&folder)),
            Self::NotFound => catalog.write(out, library::roms_not_found(&folder)),
            Self::Unsupported => {
                out.push_str(catalog.text(library::ROMS_UNSUPPORTED));
                Ok(())
            }
        };
    }
}

/// The texts of the sidebar that refill only when their data or the
/// catalog changes.
#[derive(Debug, Clone, Default)]
pub(crate) struct SidebarTexts {
    /// The ROM status of the selected system for its line and its folder.
    rom_line: TextBuffer<(RomLine, String)>,
}

/// The response of the sidebar in a pass.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct SidebarResponse {
    /// The response of the system list.
    pub(crate) list: ListResponse,
    /// The system defaults button was activated.
    pub(crate) system_defaults: bool,
}

/// One system of the sidebar with its games and its ROM status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SystemEntry {
    /// The system.
    pub(crate) system: &'static System,
    /// The icon of the system in the system list.
    pub(crate) icon: Icon,
    /// The games and the files with errors of the system.
    pub(crate) rows: Vec<GameRow>,
    /// The folder of the game files to show, for example `config/pc98`.
    pub(crate) folder: String,
    /// The ROM status of the ROM folder of the system.
    pub(crate) rom_status: RomStatus,
    /// The ROM folder to show, for example `roms/pc98`.
    pub(crate) rom_folder: String,
}

impl SystemEntry {
    /// The entry of `system` without games in `host`, with its ROM status
    /// from the ROM check `statuses`.
    pub(crate) fn new(
        system: &'static System,
        host: &HostLayers,
        statuses: &[RomFolderStatus],
    ) -> Self {
        let mut entry = Self {
            system,
            icon: system_icon(system),
            rows: Vec::new(),
            folder: String::new(),
            rom_status: RomStatus::Unsupported,
            rom_folder: String::new(),
        };
        entry.take_host(host, statuses);
        entry
    }

    /// Takes the folder of the game files of the system in `host` and the
    /// ROM status of its ROM folder from the ROM check `statuses`.
    pub(crate) fn take_host(&mut self, host: &HostLayers, statuses: &[RomFolderStatus]) {
        self.folder = shown_path(&host.config_folder().join(self.system.name), host.root());
        if let Some(rom) = statuses
            .iter()
            .find(|status| status.folder.key == self.system.rom_key)
        {
            self.rom_status = rom.check.status;
            self.rom_folder = shown_path(&rom.location.path, host.root());
        }
    }

    /// The number of games that can start.
    pub(crate) fn game_count(&self) -> usize {
        self.rows.iter().filter(|row| row.launchable()).count()
    }

    /// The number of files with errors.
    pub(crate) fn error_count(&self) -> usize {
        self.rows.len() - self.game_count()
    }

    /// The mark of the ROM status in the system list: a check for found
    /// ROMs, an alert for partial or missing ROMs that the system needs and
    /// no mark for ROMs that the system runs without.
    pub(crate) fn rom_mark(&self) -> RowMark {
        match rom_severity(self.rom_status, self.system.roms_required) {
            RomSeverity::Found => RowMark::Marker(Icon::Check),
            RomSeverity::Warning => RowMark::Alert,
            RomSeverity::Unmarked => RowMark::Blank,
        }
    }

    /// The sentence of the ROM status below the system list in English.
    #[cfg(test)]
    pub(crate) fn rom_sentence(&self) -> String {
        english_text(|catalog, out| {
            RomLine::of(self.rom_status).write(catalog, &self.rom_folder, out);
        })
    }
}

/// The icon of `system` in the system list.
pub(crate) fn system_icon(system: &System) -> Icon {
    let Some(selection) = MachineSelection::of_system(system) else {
        return Icon::SystemDesktop;
    };
    match selection.target() {
        #[cfg(feature = "pc98")]
        Target::Pc98 => Icon::SystemDesktop,
        #[cfg(feature = "pc88")]
        Target::Pc88 => Icon::SystemDesktop,
        #[cfg(feature = "pc88va")]
        Target::Pc88Va => Icon::SystemDesktop,
        #[cfg(feature = "pc60")]
        Target::Pc60 => Icon::SystemKeyboard,
        #[cfg(feature = "msx")]
        Target::Msx => Icon::SystemKeyboard,
        #[cfg(feature = "towns")]
        Target::Towns => Icon::SystemTower,
        #[cfg(feature = "x1")]
        Target::X1 => Icon::SystemDesktop,
        #[cfg(feature = "fm7")]
        Target::Fm7 => Icon::SystemKeyboard,
        #[cfg(feature = "x68k")]
        Target::X68k => Icon::SystemTower,
        #[cfg(feature = "at")]
        Target::At => Icon::SystemTower,
    }
}

/// Draws the sidebar into `area` with the texts `texts`: the name of the
/// application, the system list of `systems` with the selection in `state`,
/// the ROM status of the selected system and the system defaults button.
/// Returns the response of the system list and whether the button was
/// activated.
pub(crate) fn show_sidebar(
    context: &mut UiContext,
    area: Rect,
    systems: &[SystemEntry],
    state: &mut ListState,
    texts: &mut SidebarTexts,
) -> SidebarResponse {
    let theme = context.theme;
    context.fill(
        Rect::new(
            area.right() - theme.stroke,
            area.y,
            theme.stroke,
            area.height,
        ),
        theme.colors.panel_edge,
    );
    let inner = Rect::new(
        area.x + theme.padding,
        area.y,
        (area.width - theme.padding * 2.0 - theme.stroke).max(0.0),
        (area.height - theme.padding).max(0.0),
    );
    let (header, rest) = inner.cut_top(HEADER_HEIGHT);
    let title_style = theme.title();
    let baseline = context.centered_baseline(header, &title_style);
    context.text(header.x, baseline, &title_style, APPLICATION_TITLE);

    let (button_area, rest) = rest.cut_bottom(theme.control_height);
    let (rom_line, list_area) = rest.cut_bottom(PATH_LINE_HEIGHT + theme.gap * 2.0);
    let list = List::new(UiId::from(SYSTEM_LIST_ID), systems.len()).show(
        context,
        list_area,
        state,
        |context, row| {
            let entry = &systems[row.index];
            let count = NumberText::new(entry.game_count());
            RowContent::new(entry.system.label)
                .icon(Some(entry.icon))
                .count(count.as_str())
                .mark(Some(entry.rom_mark()))
                .show(context, row);
        },
    );
    if let Some(entry) = systems.get(state.selected) {
        let line = Rect::new(
            rom_line.x,
            rom_line.y + theme.gap,
            rom_line.width,
            PATH_LINE_HEIGHT,
        );
        show_rom_line(context, line, entry, texts);
    }
    let caption = context.catalog.text(common::SYSTEM_DEFAULTS);
    let system_defaults = Button::new(SYSTEM_DEFAULTS_ID, caption)
        .icon(Icon::Settings)
        .enabled(systems.get(state.selected).is_some())
        .show(context, button_area);
    SidebarResponse {
        list,
        system_defaults,
    }
}

/// Draws the ROM status of `entry` into `area` with the texts `texts`: the
/// mark of the status and its sentence.
fn show_rom_line(
    context: &mut UiContext,
    area: Rect,
    entry: &SystemEntry,
    texts: &mut SidebarTexts,
) {
    let theme = context.theme;
    let (icon, icon_color, text_color) = match entry.rom_mark() {
        RowMark::Marker(icon) => (Some(icon), theme.colors.marker, theme.colors.text_dim),
        RowMark::Alert => (
            Some(Icon::Alert),
            theme.colors.danger_text,
            theme.colors.danger_text,
        ),
        RowMark::Blank => (None, theme.colors.text_dim, theme.colors.text_dim),
    };
    let (slot, text_area) = area.cut_left(theme.icon_size);
    if let Some(icon) = icon {
        context.icon(slot, icon, theme.icon_size, icon_color);
    }
    let sentence = texts.rom_line.text(
        context,
        (RomLine::of(entry.rom_status), entry.rom_folder.as_str()),
        |catalog, (line, folder), out| line.write(catalog, folder, out),
    );
    context.text_in(
        text_area.cut_left(theme.gap).1,
        &theme.small().with_color(text_color),
        sentence,
        Align::Start,
        Truncation::Middle,
    );
}

#[cfg(test)]
mod tests {
    use ::common::temporary::TemporaryFolder;
    use gui::{Color, ui::LIST_PADDING};
    use machine_factory::config::systems::{SYSTEMS, compiled_systems};

    use super::*;
    use crate::{
        config::{LaunchRequest, Locations, SETTINGS_FILE_NAME},
        gui_test_support::{GuiHarness, MINIMUM_SCREEN},
        manager::rows::tests::{broken, game},
        rom_folders::check_all_folders,
        texts::{LanguageCatalog, test_catalog},
    };

    /// The icon of each system by its folder name.
    const SYSTEM_ICONS: [(&str, Icon); 10] = [
        ("pc98", Icon::SystemDesktop),
        ("pc88", Icon::SystemDesktop),
        ("pc88va", Icon::SystemDesktop),
        ("pc6000", Icon::SystemKeyboard),
        ("msx", Icon::SystemKeyboard),
        ("towns", Icon::SystemTower),
        ("x1", Icon::SystemDesktop),
        ("fm7", Icon::SystemKeyboard),
        ("x68k", Icon::SystemTower),
        ("at", Icon::SystemTower),
    ];

    /// Returns the entry of `system` with the ROM status `status`.
    fn entry(system: &'static System, status: RomStatus) -> SystemEntry {
        SystemEntry {
            system,
            icon: system_icon(system),
            rows: Vec::new(),
            folder: format!("config/{}", system.name),
            rom_status: status,
            rom_folder: format!("roms/{}", system.name),
        }
    }

    /// A system of the table that needs its ROM set and one that runs without it.
    fn required_and_optional() -> (&'static System, &'static System) {
        let required = SYSTEMS
            .iter()
            .find(|system| system.roms_required)
            .expect("a system needs its ROMs");
        let optional = SYSTEMS
            .iter()
            .find(|system| !system.roms_required)
            .expect("a system runs without its ROMs");
        (required, optional)
    }

    /// Verifies the mark and the sentence of every ROM status for a system that needs its ROM set.
    #[test]
    fn required_roms_alert_when_they_are_missing() {
        let (required, _) = required_and_optional();
        let cases = [
            (
                RomStatus::Found,
                RowMark::Marker(Icon::Check),
                "ROMs found in roms/",
            ),
            (
                RomStatus::Partial,
                RowMark::Alert,
                "Some ROMs missing in roms/",
            ),
            (RomStatus::Missing, RowMark::Alert, "ROMs missing in roms/"),
            (
                RomStatus::NotFound,
                RowMark::Blank,
                "ROMs not found in roms/",
            ),
            (
                RomStatus::Unsupported,
                RowMark::Blank,
                "No ROM check in this build",
            ),
        ];
        for (status, mark, sentence) in cases {
            let entry = entry(required, status);
            assert_eq!(entry.rom_mark(), mark, "{status:?}");
            assert!(entry.rom_sentence().starts_with(sentence), "{status:?}");
        }
    }

    /// Verifies that a system that runs without its ROM set shows no alert for partial or missing ROMs.
    #[test]
    fn optional_roms_never_alert() {
        let (_, optional) = required_and_optional();
        for (status, mark) in [
            (RomStatus::Found, RowMark::Marker(Icon::Check)),
            (RomStatus::Partial, RowMark::Blank),
            (RomStatus::Missing, RowMark::Blank),
            (RomStatus::NotFound, RowMark::Blank),
            (RomStatus::Unsupported, RowMark::Blank),
        ] {
            assert_eq!(entry(optional, status).rom_mark(), mark, "{status:?}");
        }
        assert!(
            entry(optional, RomStatus::Partial)
                .rom_sentence()
                .starts_with("Some ROMs missing in roms/")
        );
    }

    /// Verifies that the entry of a system takes the ROM status of its folder below the root from the ROM check and shows its folders relative to it.
    #[test]
    fn a_missing_rom_folder_reads_as_missing_or_not_found() {
        let root = TemporaryFolder::new("manager_sidebar_test");
        root.write_file(SETTINGS_FILE_NAME, b"");
        let locations = Locations {
            current_directory: root.path().to_path_buf(),
            executable_directory: None,
            pref_path: None,
        };
        let host = HostLayers::read(&LaunchRequest::default(), &locations).expect("host layers");
        let statuses = check_all_folders(&host);
        for system in compiled_systems() {
            let mut entry = SystemEntry::new(system, &host, &statuses);
            assert_eq!(entry.folder, format!("config/{}", system.name));
            assert_eq!(
                std::path::PathBuf::from(&entry.rom_folder),
                std::path::PathBuf::from("roms").join(system.name)
            );
            let expected = if system.roms_required {
                RomStatus::Missing
            } else {
                RomStatus::NotFound
            };
            assert_eq!(entry.rom_status, expected, "{}", system.name);
            entry.rom_status = RomStatus::Found;
            entry.take_host(&host, &statuses);
            assert_eq!(entry.rom_status, expected, "{}", system.name);
        }
    }

    /// Whether a pixel of the area from `left` to `right` and `top` to
    /// `bottom` lies near the color `color`.
    fn has_color(
        harness: &GuiHarness,
        pixels: &[u8],
        (left, right): (f32, f32),
        (top, bottom): (f32, f32),
        color: Color,
    ) -> bool {
        let width = harness.viewport.width as usize;
        let target = [color.r, color.g, color.b];
        (top as usize..bottom as usize).any(|y| {
            (left as usize..right as usize).any(|x| {
                let index = (y * width + x) * 4;
                target
                    .iter()
                    .zip(&pixels[index..index + 3])
                    .all(|(want, have)| want.abs_diff(*have) < 16)
            })
        })
    }

    /// Verifies that every system row shows its label, its game count and its ROM mark, and that the label and the count draw where the row puts them.
    #[test]
    fn every_system_row_shows_its_label_count_and_mark() {
        let statuses = [
            RomStatus::Found,
            RomStatus::Missing,
            RomStatus::Partial,
            RomStatus::NotFound,
        ];
        let systems: Vec<SystemEntry> = compiled_systems()
            .enumerate()
            .map(|(index, system)| {
                let mut entry = entry(system, statuses[index % statuses.len()]);
                entry.rows = (0..index)
                    .map(|game_index| game(&format!("game-{game_index}"), "", "", "", ""))
                    .collect();
                entry.rows.push(broken("bad"));
                entry
            })
            .collect();
        for (index, entry) in systems.iter().enumerate() {
            assert_eq!(entry.game_count(), index);
            assert_eq!(entry.error_count(), 1);
        }
        let mut harness = GuiHarness::minimum();
        let mut state = ListState::new();
        state.select(0);
        harness.frame(|context| {
            show_sidebar(
                context,
                Rect::new(0.0, 0.0, 240.0, MINIMUM_SCREEN.height),
                &systems,
                &mut state,
                &mut SidebarTexts::default(),
            )
        });
        let pixels = harness.render();
        let theme = &harness.theme;
        let list_left = theme.padding + theme.stroke + LIST_PADDING;
        let list_right = 240.0 - theme.padding - theme.stroke - theme.stroke - LIST_PADDING;
        let mark_left = list_right - theme.gap - theme.icon_size;
        for (index, entry) in systems.iter().enumerate() {
            let top = HEADER_HEIGHT + theme.stroke + LIST_PADDING + index as f32 * theme.row_height;
            let rows = (top + 4.0, top + theme.row_height - 4.0);
            let mark = (mark_left, list_right - theme.gap);
            let marker = has_color(&harness, &pixels, mark, rows, theme.colors.marker);
            let alert = has_color(&harness, &pixels, mark, rows, theme.colors.danger_text);
            let expected = match entry.rom_mark() {
                RowMark::Marker(_) => (true, false),
                RowMark::Alert => (false, true),
                RowMark::Blank => (false, false),
            };
            assert_eq!((marker, alert), expected, "{}", entry.system.label);
            let label_end = list_left + theme.gap + theme.icon_size + theme.gap + 60.0;
            assert!(
                has_color(
                    &harness,
                    &pixels,
                    (
                        list_left + theme.gap + theme.icon_size + theme.gap,
                        label_end
                    ),
                    rows,
                    theme.colors.text
                ),
                "the label of {}",
                entry.system.label
            );
            let count_color = if index == state.selected {
                theme.colors.text
            } else {
                theme.colors.text_dim
            };
            assert!(
                has_color(
                    &harness,
                    &pixels,
                    (mark_left - theme.gap - 16.0, mark_left - theme.gap),
                    rows,
                    count_color
                ),
                "the count of {}",
                entry.system.label
            );
        }
        for system in compiled_systems() {
            let expected = SYSTEM_ICONS
                .iter()
                .find(|(name, _)| *name == system.name)
                .map(|(_, icon)| *icon);
            assert_eq!(Some(system_icon(system)), expected, "{}", system.name);
        }
    }

    /// The German sentence of found ROMs.
    const GERMAN_ROMS: &str = "[library]\n\
                               roms-found = ROMs gefunden in {folder}\n";

    /// Verifies that a switch of the catalog gives the drawn ROM line of the selected system the new language in the next pass.
    #[test]
    fn the_rom_line_follows_a_catalog_switch() {
        let system = compiled_systems().next().expect("one system is compiled");
        let systems = [entry(system, RomStatus::Found)];
        let mut harness = GuiHarness::minimum();
        let mut state = ListState::new();
        state.select(0);
        let mut texts = SidebarTexts::default();
        let mut draw = |harness: &mut GuiHarness, texts: &mut SidebarTexts| {
            harness.frame(|context| {
                show_sidebar(
                    context,
                    Rect::new(0.0, 0.0, 240.0, MINIMUM_SCREEN.height),
                    &systems,
                    &mut state,
                    texts,
                )
            });
        };
        draw(&mut harness, &mut texts);
        let english = format!("ROMs found in roms/{}", system.name);
        assert_eq!(texts.rom_line.last_text(), Some(english.as_str()));
        harness.set_catalog(LanguageCatalog::Loaded(Box::new(test_catalog(GERMAN_ROMS))));
        draw(&mut harness, &mut texts);
        let german = format!("ROMs gefunden in roms/{}", system.name);
        assert_eq!(texts.rom_line.last_text(), Some(german.as_str()));
    }
}
