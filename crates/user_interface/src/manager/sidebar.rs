//! The system sidebar of the library: the systems with their game count and
//! ROM status, and the line with the ROM status of the selected system.

use gui::{
    Rect,
    ui::{
        Align, HEADER_HEIGHT, Icon, List, ListResponse, ListState, PATH_LINE_HEIGHT, RowContent,
        RowMark, Truncation, UiContext, UiId,
    },
};
use machine_factory::{
    config::{MachineSelection, Target, systems::System},
    rom_status::RomStatus,
};

use super::rows::{GameRow, shown_path};
use crate::{APPLICATION_TITLE, config::HostLayers, rom_folders::check_folder};

/// The id of the system list.
pub(crate) const SYSTEM_LIST_ID: &str = "systems";

/// The subtitle beside the name of the application.
const SUBTITLE: &str = "Game manager";

/// One system of the sidebar with its games and its ROM status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SystemEntry {
    /// The system.
    pub(crate) system: &'static System,
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
    /// The entry of `system` without games, with the ROM status of its
    /// ROM folder in `host`.
    pub(crate) fn new(system: &'static System, host: &HostLayers) -> Self {
        let mut entry = Self {
            system,
            rows: Vec::new(),
            folder: shown_path(&host.config_folder().join(system.name), host.root()),
            rom_status: RomStatus::Unsupported,
            rom_folder: String::new(),
        };
        entry.check_roms(host);
        entry
    }

    /// Checks the ROM folder of the system in `host` again. The check reads
    /// and hashes the files of the folder once.
    pub(crate) fn check_roms(&mut self, host: &HostLayers) {
        let rom = check_folder(host, &self.system.rom_folder());
        self.rom_status = rom.check.status;
        self.rom_folder = shown_path(&rom.location.path, host.root());
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
    /// ROMs, an alert for missing ROMs that the system needs and no mark
    /// for ROMs that the system runs without.
    pub(crate) fn rom_mark(&self) -> RowMark {
        let required = self.system.roms_required;
        match self.rom_status {
            RomStatus::Found => RowMark::Marker(Icon::Check),
            RomStatus::Partial | RomStatus::Missing if required => RowMark::Alert,
            RomStatus::Partial
            | RomStatus::Missing
            | RomStatus::NotFound
            | RomStatus::Unsupported => RowMark::Blank,
        }
    }

    /// The sentence of the ROM status below the system list.
    pub(crate) fn rom_sentence(&self) -> String {
        let folder = &self.rom_folder;
        match self.rom_status {
            RomStatus::Found => format!("ROMs found in {folder}"),
            RomStatus::Partial => format!("Some ROMs missing in {folder}"),
            RomStatus::Missing => format!("ROMs missing in {folder}"),
            RomStatus::NotFound => format!("ROMs not found in {folder}"),
            RomStatus::Unsupported => "No ROM check in this build".to_owned(),
        }
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

/// Draws the sidebar into `area`: the name of the application, the system
/// list of `systems` with the selection in `state` and the ROM status of
/// the selected system. Returns what the system list did.
pub(crate) fn show_sidebar(
    context: &mut UiContext,
    area: Rect,
    systems: &[SystemEntry],
    state: &mut ListState,
) -> ListResponse {
    let theme = context.theme;
    context.fill(
        Rect::new(
            area.right() - theme.stroke,
            area.y,
            theme.stroke,
            area.height,
        ),
        theme.panel_edge,
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
    let title_width = context.measure(&title_style, APPLICATION_TITLE);
    context.text(
        header.x + title_width + theme.gap,
        baseline,
        &theme.small(),
        SUBTITLE,
    );

    let (rom_line, list_area) = rest.cut_bottom(PATH_LINE_HEIGHT + theme.gap);
    let response = List::new(UiId::from(SYSTEM_LIST_ID), systems.len()).show(
        context,
        list_area,
        state,
        |context, row| {
            let entry = &systems[row.index];
            let count = entry.game_count().to_string();
            RowContent::new(entry.system.label)
                .icon(Some(system_icon(entry.system)))
                .count(&count)
                .mark(Some(entry.rom_mark()))
                .show(context, row);
        },
    );
    if let Some(entry) = systems.get(state.selected) {
        show_rom_line(context, rom_line.cut_top(theme.gap).1, entry);
    }
    response
}

/// Draws the ROM status of `entry` into `area`: the mark of the status and
/// its sentence.
fn show_rom_line(context: &mut UiContext, area: Rect, entry: &SystemEntry) {
    let theme = context.theme;
    let (icon, icon_color, text_color) = match entry.rom_mark() {
        RowMark::Marker(icon) => (Some(icon), theme.marker, theme.text_dim),
        RowMark::Alert => (Some(Icon::Alert), theme.danger_text, theme.danger_text),
        RowMark::Blank => (None, theme.text_dim, theme.text_dim),
    };
    let (slot, text_area) = area.cut_left(theme.icon_size);
    if let Some(icon) = icon {
        context.icon(slot, icon, theme.icon_size, icon_color);
    }
    context.text_in(
        text_area.cut_left(theme.gap).1,
        &theme.small().with_color(text_color),
        &entry.rom_sentence(),
        Align::Start,
        Truncation::Middle,
    );
}

#[cfg(test)]
mod tests {
    use machine_factory::config::systems::{SYSTEMS, compiled_systems};

    use super::*;
    use crate::{
        config::{LaunchRequest, Locations},
        gui_test_support::GuiHarness,
        manager::rows::tests::{broken, game},
    };

    /// The padding between the edge of a list and its rows.
    const LIST_PADDING: f32 = 4.0;

    /// Returns the entry of `system` with the ROM status `status`.
    fn entry(system: &'static System, status: RomStatus) -> SystemEntry {
        SystemEntry {
            system,
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

    /// Verifies that the entry of a system checks its ROM folder below the root and shows its folders relative to it.
    #[test]
    fn a_missing_rom_folder_reads_as_missing_or_not_found() {
        let root = std::env::temp_dir().join(format!(
            "neetan_manager_sidebar_test_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("root is created");
        std::fs::write(root.join(crate::config::SETTINGS_FILE_NAME), "").expect("settings");
        let locations = Locations {
            current_directory: root.clone(),
            executable_directory: None,
            pref_path: None,
        };
        let host = HostLayers::read(&LaunchRequest::default(), &locations).expect("host layers");
        for system in compiled_systems() {
            let mut entry = SystemEntry::new(system, &host);
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
            entry.check_roms(&host);
            assert_eq!(entry.rom_status, expected, "{}", system.name);
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Whether a pixel of the area from `left` to `right` and `top` to
    /// `bottom` lies near the color `color`.
    fn has_color(
        harness: &GuiHarness,
        pixels: &[u8],
        (left, right): (f32, f32),
        (top, bottom): (f32, f32),
        color: gui::Color,
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
        let mut harness = GuiHarness::new(960, 600);
        let mut state = ListState::new();
        state.select(0);
        harness.frame(|context| {
            show_sidebar(
                context,
                Rect::new(0.0, 0.0, 240.0, 572.0),
                &systems,
                &mut state,
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
            let marker = has_color(&harness, &pixels, mark, rows, theme.marker);
            let alert = has_color(&harness, &pixels, mark, rows, theme.danger_text);
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
                    theme.text
                ),
                "the label of {}",
                entry.system.label
            );
            let count_color = if index == state.selected {
                theme.text
            } else {
                theme.text_dim
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
        assert_eq!(
            system_icon(compiled_systems().next().expect("one system")),
            Icon::SystemDesktop
        );
    }
}
