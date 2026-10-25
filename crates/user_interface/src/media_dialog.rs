//! The media dialog: a tab for each removable media drive of the machine, the
//! configured images of the drive with a filter, and the insert and eject
//! actions.

use std::path::PathBuf;

use gui::{
    Rect,
    ui::{
        Align, Button, ButtonKind, Icon, ListState, PATH_LINE_HEIGHT, RowContent, RowMark, Tab,
        Truncation, UiContext, UiId, label, list, modal, tab_bar, text_field,
    },
};

/// The size of the dialog panel in logical units.
pub(crate) const PANEL_SIZE: (f32, f32) = (640.0, 540.0);

/// The height of the tab bar in logical units.
pub(crate) const TAB_BAR_HEIGHT: f32 = 40.0;

/// The id of the image list.
const LIST_ID: &str = "images";

/// A removable media drive that the dialog changes the medium of.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Drive {
    /// The floppy drive with this 0-based index.
    Floppy(usize),
    /// The CD-ROM drive.
    CdRom,
}

impl Drive {
    /// The caption of the tab of the drive.
    fn caption(self) -> &'static str {
        match self {
            Drive::Floppy(0) => "FDD 1",
            Drive::Floppy(_) => "FDD 2",
            Drive::CdRom => "CD-ROM",
        }
    }

    /// The icon of the drive and its images.
    fn icon(self) -> Icon {
        match self {
            Drive::Floppy(_) => Icon::Disk,
            Drive::CdRom => Icon::Disc,
        }
    }

    /// The title of the dialog while the tab of the drive is selected.
    fn title(self) -> &'static str {
        match self {
            Drive::Floppy(_) => "Insert disk image",
            Drive::CdRom => "Insert disc image",
        }
    }

    /// The caption of the eject row while the drive holds a medium.
    fn eject_caption(self) -> &'static str {
        match self {
            Drive::Floppy(_) => "Eject disk",
            Drive::CdRom => "Eject disc",
        }
    }

    /// The caption of the eject row while the drive is empty.
    fn empty_caption(self) -> &'static str {
        match self {
            Drive::Floppy(_) => "No disk",
            Drive::CdRom => "No disc",
        }
    }

    /// The name of the drive in log lines.
    pub(crate) fn log_name(self) -> &'static str {
        match self {
            Drive::Floppy(0) => "FDD1",
            Drive::Floppy(_) => "FDD2",
            Drive::CdRom => "CD-ROM",
        }
    }
}

/// The drives of a machine with removable media, in tab order: the floppy
/// drives up to the two the configuration can fill, then the CD-ROM drive.
pub(crate) fn available_drives(floppy_drives: u8, cdrom: bool) -> Vec<Drive> {
    let mut drives: Vec<Drive> = (0..usize::from(floppy_drives.min(2)))
        .map(Drive::Floppy)
        .collect();
    if cdrom {
        drives.push(Drive::CdRom);
    }
    drives
}

/// A disk image that the configuration offers for a drive.
#[derive(Debug, Clone)]
pub(crate) struct ImageEntry {
    /// The path of the image file.
    pub(crate) path: PathBuf,
    /// The file name that the list shows.
    pub(crate) name: String,
    /// The file name in lowercase for the filter.
    folded_name: String,
}

impl ImageEntry {
    /// An entry for the image file at `path`.
    pub(crate) fn new(path: PathBuf) -> Self {
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_string_lossy().into_owned());
        let folded_name = name.to_lowercase();
        Self {
            path,
            name,
            folded_name,
        }
    }
}

/// The images of a drive and its loaded image in a pass.
#[derive(Clone, Copy)]
pub(crate) struct DriveMedia<'a> {
    /// The images the configuration offers for the drive.
    pub(crate) entries: &'a [ImageEntry],
    /// The index of the loaded image in `entries`.
    pub(crate) loaded: Option<usize>,
}

/// The change that a pass of the dialog asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DialogAction {
    /// The dialog stays open.
    None,
    /// The dialog closes without a change.
    Close,
    /// The image `index` goes into `drive`, then the dialog closes.
    Insert {
        /// The drive.
        drive: Drive,
        /// The index of the image in the entries of the drive.
        index: usize,
    },
    /// The medium of `drive` comes out, then the dialog closes.
    Eject {
        /// The drive.
        drive: Drive,
    },
}

/// The action of a media hotkey.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HotkeyAction {
    /// The dialog opens on the tab with this index.
    Open(usize),
    /// The open dialog switches to the tab with this index.
    Switch(usize),
    /// The open dialog closes.
    Close,
    /// The machine has no such drive.
    Unavailable,
}

/// The action of the hotkey of `pressed` on a machine with `drives`, while
/// the dialog shows the tab `open_tab` or is closed.
pub(crate) fn hotkey_action(
    drives: &[Drive],
    open_tab: Option<usize>,
    pressed: Drive,
) -> HotkeyAction {
    let Some(index) = drives.iter().position(|&drive| drive == pressed) else {
        return HotkeyAction::Unavailable;
    };
    match open_tab {
        None => HotkeyAction::Open(index),
        Some(tab) if tab == index => HotkeyAction::Close,
        Some(_) => HotkeyAction::Switch(index),
    }
}

/// The indices of the entries whose file name holds `filter`, ignoring the
/// case.
pub(crate) fn filtered_entries(entries: &[ImageEntry], filter: &str) -> Vec<usize> {
    let folded_filter = filter.trim().to_lowercase();
    entries
        .iter()
        .enumerate()
        .filter(|(_, entry)| entry.folded_name.contains(&folded_filter))
        .map(|(index, _)| index)
        .collect()
}

/// The selection among the visible `rows` for the `preferred` entry: the
/// preferred entry when it is visible, else the first visible entry, else
/// the eject row. While `filtering`, the first visible entry also takes the
/// place of a preferred eject row that the user did not pick with
/// `eject_chosen`. `None` selects the eject row.
fn visible_selection(
    preferred: Option<usize>,
    rows: &[usize],
    filtering: bool,
    eject_chosen: bool,
) -> Option<usize> {
    match preferred {
        Some(entry) if rows.contains(&entry) => Some(entry),
        Some(_) => rows.first().copied(),
        None if filtering && !eject_chosen => rows.first().copied(),
        None => None,
    }
}

/// The list row of `selection` among the visible `rows`. The eject row is
/// row 0.
fn row_of(selection: Option<usize>, rows: &[usize]) -> usize {
    selection
        .and_then(|entry| rows.iter().position(|&row| row == entry))
        .map_or(0, |position| position + 1)
}

/// The state of the open media dialog.
pub(crate) struct MediaDialog {
    /// The drives of the machine, one tab each.
    drives: Vec<Drive>,
    /// The selected tab.
    tab: usize,
    /// The filter text.
    filter: String,
    /// The scroll position and the selected row of the list.
    list: ListState,
    /// The entry of the current drive that the user picked or that the
    /// drive holds. A filter that hides it leaves it in place. `None`
    /// selects the eject row.
    preferred: Option<usize>,
    /// Whether the user picked the eject row under the current filter.
    eject_chosen: bool,
    /// The selected entry among the visible rows. `None` selects the eject
    /// row.
    selection: Option<usize>,
    /// Whether the next pass scrolls the selected row into view.
    reveal: bool,
    /// Whether the tab bar switched the tab in the last pass, so the title
    /// of that pass belongs to the previous tab.
    stale: bool,
}

impl MediaDialog {
    /// A dialog over `drives` on the tab `tab`, with the image `loaded` of
    /// that drive selected.
    pub(crate) fn new(drives: Vec<Drive>, tab: usize, loaded: Option<usize>) -> Self {
        Self {
            drives,
            tab,
            filter: String::new(),
            list: ListState::new(),
            preferred: loaded,
            eject_chosen: false,
            selection: loaded,
            reveal: true,
            stale: false,
        }
    }

    /// The drives of the tabs.
    pub(crate) fn drives(&self) -> &[Drive] {
        &self.drives
    }

    /// The selected tab.
    pub(crate) fn tab(&self) -> usize {
        self.tab
    }

    /// Whether the last pass drew a stale title and the dialog needs another
    /// pass. Clears the flag.
    pub(crate) fn take_stale(&mut self) -> bool {
        std::mem::take(&mut self.stale)
    }

    /// Selects the tab `tab` and the image `loaded` of its drive.
    pub(crate) fn switch_tab(&mut self, tab: usize, loaded: Option<usize>) {
        self.tab = tab.min(self.drives.len().saturating_sub(1));
        self.preferred = loaded;
        self.eject_chosen = false;
        self.selection = loaded;
        self.list = ListState::new();
        self.reveal = true;
    }

    /// Draws one pass of the dialog. `media` holds the images of each drive
    /// in the order of the tabs. Returns the change the pass asks for.
    pub(crate) fn show(&mut self, context: &mut UiContext, media: &[DriveMedia]) -> DialogAction {
        context.layer("media", |context| self.panel(context, media))
    }

    /// Draws the panel of the dialog in its modal layer.
    fn panel(&mut self, context: &mut UiContext, media: &[DriveMedia]) -> DialogAction {
        let Some(&drive) = self.drives.get(self.tab) else {
            return DialogAction::Close;
        };
        let theme = context.theme;
        let gap = theme.gap;
        let response = modal(context, "media", PANEL_SIZE, drive.title());
        let mut action = if response.closed {
            DialogAction::Close
        } else {
            DialogAction::None
        };

        let (tabs_area, rest) = response.content.cut_top(TAB_BAR_HEIGHT);
        let tabs: Vec<Tab> = self
            .drives
            .iter()
            .map(|drive| Tab::new(drive.caption(), Some(drive.icon())))
            .collect();
        let mut tab = self.tab;
        if tab_bar(context, "drives", tabs_area, &tabs, &mut tab) {
            let loaded = media.get(tab).and_then(|media| media.loaded);
            self.switch_tab(tab, loaded);
            self.stale = true;
        }
        let drive = self.drives[self.tab];
        let DriveMedia { entries, loaded } = media.get(self.tab).copied().unwrap_or(DriveMedia {
            entries: &[],
            loaded: None,
        });

        let (filter_area, rest) = rest.cut_top(gap * 1.5).1.cut_top(theme.control_height);
        let filter = text_field(
            context,
            "filter",
            filter_area,
            &mut self.filter,
            "Filter images",
        );
        if filter.changed {
            self.eject_chosen = false;
        }
        let rows = filtered_entries(entries, &self.filter);
        let selection = visible_selection(
            self.preferred,
            &rows,
            !self.filter.trim().is_empty(),
            self.eject_chosen,
        );
        if selection != self.selection || filter.changed {
            self.selection = selection;
            self.reveal = true;
        }
        if self.reveal {
            self.list.select(row_of(self.selection, &rows));
            self.reveal = false;
        }

        let (footer, rest) = rest.cut_bottom(theme.control_height);
        let (path_line, rest) = rest.cut_top(gap).1.cut_top(PATH_LINE_HEIGHT);
        let list_area = rest.cut_top(gap * 0.5).1.cut_bottom(gap * 1.5).1;

        context.default_focus(UiId::from(LIST_ID));
        let list_response = list(
            context,
            LIST_ID,
            list_area,
            rows.len() + 1,
            &mut self.list,
            |context, row| {
                if row.index == 0 {
                    let (caption, marker) = match loaded {
                        Some(_) => (drive.eject_caption(), None),
                        None => (drive.empty_caption(), Some(Icon::Check)),
                    };
                    RowContent::new(caption)
                        .icon(Some(Icon::Eject))
                        .mark(marker.map(RowMark::Marker))
                        .show(context, row);
                } else {
                    let entry = rows[row.index - 1];
                    let marker = (loaded == Some(entry)).then_some(Icon::Check);
                    RowContent::new(&entries[entry].name)
                        .icon(Some(drive.icon()))
                        .mark(marker.map(RowMark::Marker))
                        .show(context, row);
                }
            },
        );
        self.selection = match self.list.selected {
            0 => None,
            row => rows.get(row - 1).copied(),
        };
        if list_response.selection_changed {
            self.preferred = self.selection;
            self.eject_chosen = self.selection.is_none();
        }
        if rows.is_empty() {
            let hint = if entries.is_empty() {
                "No images configured for this drive"
            } else {
                "No images match the filter"
            };
            let hint_area = Rect::new(
                list_area.x,
                list_area.y + gap + theme.row_height,
                list_area.width,
                theme.row_height,
            );
            label(
                context,
                hint_area,
                &theme.small(),
                hint,
                Align::Center,
                Truncation::End,
            );
        }

        if let Some(entry) = self.selection {
            let (icon_area, path_area) = path_line.cut_left(theme.icon_size);
            context.icon(
                icon_area,
                Icon::Folder,
                theme.icon_size,
                theme.colors.text_dim,
            );
            label(
                context,
                path_area.cut_left(gap).1,
                &theme.small(),
                &entries[entry].path.to_string_lossy(),
                Align::Start,
                Truncation::Middle,
            );
        }

        let (caption, icon) = match self.selection {
            Some(_) => ("Insert", Icon::Check),
            None => ("Eject", Icon::Eject),
        };
        let insert = Button::new("insert", caption)
            .icon(icon)
            .kind(ButtonKind::Primary);
        let cancel = Button::new("cancel", "Cancel");
        let (insert_area, footer) = footer.cut_right(insert.width(context));
        let cancel_area = footer.cut_right(gap).1.cut_right(cancel.width(context)).0;
        if cancel.show(context, cancel_area) {
            action = DialogAction::Close;
        }
        let insert = insert.show(context, insert_area);

        let filter_activates = filter.submitted && (self.filter.is_empty() || !rows.is_empty());
        if insert || list_response.activated.is_some() || filter_activates {
            action = match self.selection {
                Some(index) => DialogAction::Insert { drive, index },
                None => DialogAction::Eject { drive },
            };
        }
        action
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use gui::{
        Color,
        ui::{Navigation, ThemeKind, Viewport},
    };

    use super::*;
    use crate::gui_test_support::GuiHarness;

    /// The image files of the test dialog.
    const FILES: [&str; 12] = [
        "System Disk.d88",
        "Ys II - Ancient Ys Vanished The Final Chapter (Disk A).d88",
        "ドラゴンスレイヤー英雄伝説 (ディスク1).hdm",
        "ドラゴンスレイヤー英雄伝説 (ディスク2).hdm",
        "Brandish (Disk 1 of 4).fdi",
        "Touhou Reiiden - Highly Responsive to Prayers (1996)(Amusement Makers)[Disk 1 of 2][Japanese].fdi",
        "信長の野望・全国版.d88",
        "Rusty (1993)(C-Lab).hdm",
        "Sorcerian.d88",
        "幻影都市 (Disk 1).fdi",
        "Lemmings.d88",
        "Xak - The Art of Visual Stage.d88",
    ];

    /// The entries of `names` in a folder.
    fn entries(names: &[&str]) -> Vec<ImageEntry> {
        names
            .iter()
            .map(|name| ImageEntry::new(PathBuf::from("/home/user/Emulation/PC-98").join(name)))
            .collect()
    }

    /// The pass state of a dialog without a window and the dialog.
    struct Harness {
        /// The fonts, the theme and the pass state.
        gui: GuiHarness,
        /// The dialog.
        dialog: MediaDialog,
        /// The images of the floppy drives and the CD-ROM drive.
        media: [Vec<ImageEntry>; 3],
        /// The loaded image of each drive.
        loaded: [Option<usize>; 3],
    }

    impl Harness {
        /// A dialog on the first floppy drive at `viewport` in the theme
        /// `kind`.
        fn new(viewport: Viewport, kind: ThemeKind) -> Self {
            Self {
                gui: GuiHarness::with_viewport(viewport, kind),
                dialog: MediaDialog::new(available_drives(2, true), 0, Some(3)),
                media: [entries(&FILES), entries(&FILES[..4]), Vec::new()],
                loaded: [Some(3), None, None],
            }
        }

        /// Runs one pass with the pending input.
        fn pass(&mut self) -> DialogAction {
            let media: Vec<DriveMedia> = self
                .media
                .iter()
                .zip(self.loaded)
                .map(|(entries, loaded)| DriveMedia { entries, loaded })
                .collect();
            let dialog = &mut self.dialog;
            self.gui.frame(|context| {
                let bounds = context.bounds();
                context.fill(bounds, Color::hex(0x204080));
                dialog.show(context, &media)
            })
        }

        /// Runs one pass with the navigation event `navigation`.
        fn navigate(&mut self, navigation: Navigation) -> DialogAction {
            self.gui.input.navigation = Some(navigation);
            self.pass()
        }

        /// Runs one pass with the committed text `text`.
        fn type_text(&mut self, text: &str) -> DialogAction {
            self.gui.input.text.push_str(text);
            self.pass()
        }

        /// Runs one pass with confirm.
        fn confirm(&mut self) -> DialogAction {
            self.gui.input.confirm = true;
            self.pass()
        }

        /// Runs one pass with cancel.
        fn cancel(&mut self) -> DialogAction {
            self.gui.input.cancel = true;
            self.pass()
        }
    }

    #[test]
    fn drives_follow_the_machine() {
        assert_eq!(available_drives(0, false), Vec::new());
        assert_eq!(available_drives(1, false), vec![Drive::Floppy(0)]);
        assert_eq!(
            available_drives(4, true),
            vec![Drive::Floppy(0), Drive::Floppy(1), Drive::CdRom]
        );
        assert_eq!(available_drives(0, true), vec![Drive::CdRom]);
    }

    #[test]
    fn hotkeys_open_switch_and_close() {
        let drives = available_drives(2, false);
        assert_eq!(
            hotkey_action(&drives, None, Drive::Floppy(1)),
            HotkeyAction::Open(1)
        );
        assert_eq!(
            hotkey_action(&drives, Some(1), Drive::Floppy(1)),
            HotkeyAction::Close
        );
        assert_eq!(
            hotkey_action(&drives, Some(1), Drive::Floppy(0)),
            HotkeyAction::Switch(0)
        );
        assert_eq!(
            hotkey_action(&drives, None, Drive::CdRom),
            HotkeyAction::Unavailable
        );
        assert_eq!(
            hotkey_action(&drives, Some(0), Drive::CdRom),
            HotkeyAction::Unavailable
        );
    }

    #[test]
    fn the_filter_ignores_case_and_keeps_the_order() {
        let entries = entries(&FILES);
        assert_eq!(
            filtered_entries(&entries, ""),
            (0..FILES.len()).collect::<Vec<_>>()
        );
        assert_eq!(filtered_entries(&entries, "ANCIENT ys"), vec![1]);
        assert_eq!(filtered_entries(&entries, "DISK 1"), vec![4, 5, 9]);
        assert_eq!(filtered_entries(&entries, "英雄伝説"), vec![2, 3]);
        assert!(filtered_entries(&entries, "zzz").is_empty());
    }

    #[test]
    fn the_selection_stays_on_its_entry() {
        assert_eq!(visible_selection(Some(4), &[1, 4, 9], true, false), Some(4));
        assert_eq!(row_of(Some(4), &[1, 4, 9]), 2);
        assert_eq!(visible_selection(Some(3), &[1, 4, 9], true, false), Some(1));
        assert_eq!(visible_selection(Some(3), &[], true, false), None);
        assert_eq!(visible_selection(None, &[1, 4], false, false), None);
        assert_eq!(visible_selection(None, &[1, 4], true, false), Some(1));
        assert_eq!(visible_selection(None, &[1, 4], true, true), None);
        assert_eq!(row_of(None, &[1, 4]), 0);
    }

    #[test]
    fn the_dialog_opens_on_the_loaded_image() {
        let mut harness = Harness::new(Viewport::new(1024, 768, 1.0), ThemeKind::Dark);
        assert_eq!(harness.pass(), DialogAction::None);
        assert_eq!(harness.dialog.list.selected, 4);
        assert_eq!(harness.gui.state.focused(), Some(UiId::from(LIST_ID)));
        assert_eq!(
            harness.confirm(),
            DialogAction::Insert {
                drive: Drive::Floppy(0),
                index: 3
            }
        );
    }

    #[test]
    fn the_eject_row_ejects() {
        let mut harness = Harness::new(Viewport::new(1024, 768, 1.0), ThemeKind::Dark);
        harness.pass();
        harness.navigate(Navigation::Home);
        assert_eq!(harness.dialog.selection, None);
        assert_eq!(
            harness.confirm(),
            DialogAction::Eject {
                drive: Drive::Floppy(0)
            }
        );
    }

    #[test]
    fn the_filter_keeps_the_selected_entry() {
        let mut harness = Harness::new(Viewport::new(1024, 768, 1.0), ThemeKind::Dark);
        harness.pass();
        harness.navigate(Navigation::Previous);
        assert_eq!(harness.gui.state.focused(), Some(UiId::from("filter")));
        assert_eq!(harness.dialog.selection, Some(3));
        harness.type_text("英雄");
        assert_eq!(harness.dialog.selection, Some(3));
        assert_eq!(harness.dialog.list.selected, 2);
        harness.type_text("伝説 (ディスク1");
        assert_eq!(harness.dialog.selection, Some(2));
        assert_eq!(harness.dialog.list.selected, 1);
        assert_eq!(
            harness.confirm(),
            DialogAction::Insert {
                drive: Drive::Floppy(0),
                index: 2
            }
        );
    }

    #[test]
    fn confirm_in_a_filter_without_matches_does_nothing() {
        let mut harness = Harness::new(Viewport::new(1024, 768, 1.0), ThemeKind::Dark);
        harness.pass();
        harness.navigate(Navigation::Previous);
        assert_eq!(harness.gui.state.focused(), Some(UiId::from("filter")));
        harness.type_text("zzz");
        assert_eq!(harness.dialog.selection, None);
        assert_eq!(harness.confirm(), DialogAction::None);
        assert_eq!(harness.cancel(), DialogAction::None);
        assert!(harness.dialog.filter.is_empty());
        assert_eq!(harness.cancel(), DialogAction::Close);
    }

    #[test]
    fn a_filter_without_matches_keeps_the_preferred_entry() {
        let mut harness = Harness::new(Viewport::new(1024, 768, 1.0), ThemeKind::Dark);
        harness.pass();
        harness.navigate(Navigation::Previous);
        harness.type_text("zzz");
        assert_eq!(harness.dialog.selection, None);
        for _ in 0..3 {
            harness.gui.input.backspace = true;
            harness.pass();
        }
        assert!(harness.dialog.filter.is_empty());
        assert_eq!(harness.dialog.selection, Some(3));
        assert_eq!(
            harness.confirm(),
            DialogAction::Insert {
                drive: Drive::Floppy(0),
                index: 3
            }
        );
    }

    #[test]
    fn a_filter_on_an_empty_drive_selects_the_first_match() {
        let mut harness = Harness::new(Viewport::new(1024, 768, 1.0), ThemeKind::Dark);
        harness.dialog = MediaDialog::new(available_drives(2, true), 0, None);
        harness.loaded = [None, None, None];
        harness.pass();
        harness.navigate(Navigation::Previous);
        harness.type_text("brandish");
        assert_eq!(harness.dialog.selection, Some(4));
        assert_eq!(
            harness.confirm(),
            DialogAction::Insert {
                drive: Drive::Floppy(0),
                index: 4
            }
        );
    }

    #[test]
    fn a_picked_eject_row_stays_under_the_filter() {
        let mut harness = Harness::new(Viewport::new(1024, 768, 1.0), ThemeKind::Dark);
        harness.pass();
        harness.navigate(Navigation::Previous);
        harness.type_text("disk");
        harness.navigate(Navigation::Next);
        harness.navigate(Navigation::Home);
        assert_eq!(harness.dialog.selection, None);
        harness.pass();
        assert_eq!(harness.dialog.selection, None);
        assert_eq!(
            harness.confirm(),
            DialogAction::Eject {
                drive: Drive::Floppy(0)
            }
        );
    }

    #[test]
    fn a_tab_switch_selects_the_loaded_image() {
        let mut harness = Harness::new(Viewport::new(1024, 768, 1.0), ThemeKind::Dark);
        harness.loaded[1] = Some(2);
        harness.pass();
        harness.navigate(Navigation::NextTab);
        assert!(harness.dialog.take_stale());
        assert!(!harness.dialog.take_stale());
        assert_eq!(harness.dialog.tab(), 1);
        assert_eq!(harness.dialog.selection, Some(2));
        assert_eq!(harness.dialog.list.selected, 3);
        harness.navigate(Navigation::NextTab);
        assert_eq!(harness.dialog.tab(), 2);
        assert_eq!(harness.dialog.selection, None);
        assert_eq!(
            harness.confirm(),
            DialogAction::Eject {
                drive: Drive::CdRom
            }
        );
        harness.dialog.switch_tab(0, Some(5));
        harness.pass();
        assert_eq!(harness.dialog.list.selected, 6);
    }

    #[test]
    fn the_buttons_close_and_insert() {
        let mut harness = Harness::new(Viewport::new(1024, 768, 1.0), ThemeKind::Dark);
        harness.pass();
        harness.navigate(Navigation::End);
        harness.navigate(Navigation::Down);
        assert_eq!(harness.gui.state.focused(), Some(UiId::from("insert")));
        assert_eq!(
            harness.confirm(),
            DialogAction::Insert {
                drive: Drive::Floppy(0),
                index: FILES.len() - 1
            }
        );
        harness.navigate(Navigation::Left);
        assert_eq!(harness.gui.state.focused(), Some(UiId::from("cancel")));
        assert_eq!(harness.confirm(), DialogAction::Close);
    }

    /// Renders the dialog at `scale` in the theme `kind` on each tab and
    /// with a filter.
    fn screenshots(scale: f32, kind: ThemeKind) {
        let width = (960.0 * scale) as u32;
        let height = (680.0 * scale) as u32;
        let mut harness = Harness::new(Viewport::new(width, height, scale), kind);
        harness.loaded[1] = Some(1);
        harness.pass();
        harness.navigate(Navigation::Down);
        let render = |harness: &Harness, name: &str| {
            harness
                .gui
                .write_screenshot(&format!("media_dialog_{name}_{scale}x"));
        };
        render(&harness, "fdd1");
        harness.navigate(Navigation::NextTab);
        harness.pass();
        render(&harness, "fdd2");
        harness.navigate(Navigation::NextTab);
        harness.pass();
        render(&harness, "cdrom");
        harness.navigate(Navigation::NextTab);
        harness.navigate(Navigation::Previous);
        harness.type_text("ディスク");
        harness.gui.input.composition.push_str("いち");
        harness.pass();
        render(&harness, "filter");
    }

    #[test]
    #[ignore = "writes screenshots into the directory of GUI_SHOTS"]
    fn media_dialog_screenshots() {
        for kind in [ThemeKind::Dark, ThemeKind::Light] {
            screenshots(1.0, kind);
            screenshots(2.0, kind);
        }
    }
}
