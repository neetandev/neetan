//! The scan of the system folders: the classification of each game file and
//! the model that a launch of it takes.

use std::path::{Path, PathBuf};

use machine_factory::config::{
    MachineSelection, Target,
    systems::{System, system_of},
};

use super::{CatalogEntry, ModelSource, SystemCatalog};
use crate::config::{
    CONFIG_FILE_EXTENSION, ConfigDocument, Diagnostic, GameMetadata, HostLayers, Severity,
    keymap::parse_key_binding,
    layers::{launch_model, launch_target, read_system_defaults, system_defaults_file},
    reading::{
        FileReading, Layer, LayerKind, LayerLine, invalid_key_binding_message, read_game_document,
    },
};

/// The separator of the identity of a game.
const IDENTITY_SEPARATOR: char = '/';

/// Adds a warning for each key binding of `reading` that the family `target` does not know.
fn check_key_bindings(reading: &FileReading, target: Target, diagnostics: &mut Vec<Diagnostic>) {
    for (line, content) in &reading.lines {
        if let LayerLine::KeyBinding(host_name, native_name) = content
            && parse_key_binding(target, host_name, native_name).is_none()
        {
            diagnostics.push(Diagnostic::warning(
                *line,
                invalid_key_binding_message(host_name, native_name),
            ));
        }
    }
}

/// Orders diagnostics by line. A diagnostic without a line comes first.
fn sort_diagnostics(diagnostics: &mut [Diagnostic]) {
    diagnostics.sort_by_key(|diagnostic| diagnostic.line);
}

/// The system defaults of one system as the catalog reads them.
struct SystemDefaults {
    /// The layer of the file, when it exists and reads.
    layer: Option<Layer>,
    /// The findings about the file.
    diagnostics: Vec<Diagnostic>,
    /// The error of a file that fails every launch of the system.
    blocking: Option<Diagnostic>,
}

/// Reads the system defaults file `file` of the family `target`.
fn system_defaults(file: &Path, target: Target, root: &Path) -> SystemDefaults {
    let mut defaults = SystemDefaults {
        layer: None,
        diagnostics: Vec::new(),
        blocking: None,
    };
    match read_system_defaults(file, target, root) {
        Ok(None) => {}
        Ok(Some(file)) => {
            defaults.diagnostics = file.reading.diagnostics.clone();
            check_key_bindings(&file.reading, target, &mut defaults.diagnostics);
            defaults.layer = Some(file.layer);
        }
        Err(error) => {
            let diagnostic = Diagnostic {
                line: error.line(),
                severity: Severity::Error,
                message: error.message(),
            };
            defaults.blocking = Some(Diagnostic {
                line: None,
                severity: Severity::Error,
                message: format!(
                    "system defaults {}",
                    diagnostic.located(&file.display().to_string())
                ),
            });
            defaults.diagnostics.push(diagnostic);
        }
    }
    sort_diagnostics(&mut defaults.diagnostics);
    defaults
}

/// Scans the system defaults file and the game files of `system`.
pub(super) fn scan_system(
    system: &'static System,
    config_folder: &Path,
    host: &HostLayers,
) -> Option<SystemCatalog> {
    let target = MachineSelection::of_system(system)?.target();
    let root = host.root();
    let folder = config_folder.join(system.name);
    let defaults_file = system_defaults_file(config_folder, target);
    let defaults = system_defaults(&defaults_file, target, root);
    let (default_value, default_model_source) = launch_model(
        target,
        host.settings_layer(),
        defaults.layer.as_ref(),
        &[],
        host.command_line_layer(),
    );
    let default_model = default_value
        .parse::<MachineSelection>()
        .map_or(system.default_model, MachineSelection::name);

    let mut games = Vec::new();
    let mut files_with_errors = Vec::new();
    for (path, file_name) in game_files(&folder) {
        let entry = read_entry(system, path, file_name, host, &defaults);
        if entry.has_errors() {
            files_with_errors.push(entry);
        } else {
            games.push(entry);
        }
    }
    sort_entries(&mut games);
    sort_entries(&mut files_with_errors);
    Some(SystemCatalog {
        system,
        folder,
        defaults_file,
        defaults_diagnostics: defaults.diagnostics,
        default_model,
        default_model_source,
        games,
        files_with_errors,
    })
}

/// Reads the one game file `path` of `system` with the system defaults below `config_folder`,
/// as the scan of the system reads it.
pub(super) fn read_game(
    system: &'static System,
    path: &Path,
    config_folder: &Path,
    host: &HostLayers,
) -> Option<CatalogEntry> {
    let target = MachineSelection::of_system(system)?.target();
    let defaults_file = system_defaults_file(config_folder, target);
    let defaults = system_defaults(&defaults_file, target, host.root());
    let file_name = path.file_name()?.to_string_lossy().into_owned();
    Some(read_entry(
        system,
        path.to_path_buf(),
        file_name,
        host,
        &defaults,
    ))
}

/// Returns the `.conf` files directly inside `folder` with their file names.
///
/// A file whose name starts with `.` is hidden and stays out.
fn game_files(folder: &Path) -> Vec<(PathBuf, String)> {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return Vec::new();
    };
    entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case(CONFIG_FILE_EXTENSION))
                && path.is_file()
        })
        .filter_map(|path| {
            let file_name = path.file_name()?.to_string_lossy().into_owned();
            (!file_name.starts_with('.')).then_some((path, file_name))
        })
        .collect()
}

/// Sorts entries by title without regard to case, then by file name.
fn sort_entries(entries: &mut [CatalogEntry]) {
    entries.sort_by_cached_key(|entry| (entry.title.to_lowercase(), entry.file_name.clone()));
}

/// Returns the file name of `file_name` without its extension.
fn file_stem(file_name: &str) -> &str {
    Path::new(file_name)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or(file_name)
}

/// Reads one game file of `system` with the model that a launch of it takes.
fn read_entry(
    system: &'static System,
    path: PathBuf,
    file_name: String,
    host: &HostLayers,
    defaults: &SystemDefaults,
) -> CatalogEntry {
    let identity = format!("{}{IDENTITY_SEPARATOR}{file_name}", system.name);
    let mut diagnostics: Vec<Diagnostic> = defaults.blocking.iter().cloned().collect();
    let mut model = system.default_model;
    let mut model_source = ModelSource::BuiltIn;
    let metadata = match ConfigDocument::load(&path) {
        Ok(document) => {
            let reading = read_game_document(&document, host.root(), Some(system));
            diagnostics.extend(reading.diagnostics.iter().cloned());
            let game = [Layer::from_reading(
                LayerKind::GameFile,
                path.display().to_string(),
                &reading,
            )];
            if let Ok(target) = launch_target(
                host.command_line_layer(),
                &game,
                Some(system),
                host.settings_layer(),
            ) {
                let system_defaults = defaults
                    .layer
                    .as_ref()
                    .filter(|_| system_of(target) == system);
                let (value, source) = launch_model(
                    target,
                    host.settings_layer(),
                    system_defaults,
                    &game,
                    host.command_line_layer(),
                );
                if let Ok(resolved) = value.parse::<MachineSelection>() {
                    model = resolved.name();
                    model_source = source;
                }
                check_key_bindings(&reading, target, &mut diagnostics);
            }
            GameMetadata::read(&document, &mut diagnostics)
        }
        Err(error) => {
            diagnostics.push(Diagnostic {
                line: error.line(),
                severity: Severity::Error,
                message: error.message(),
            });
            GameMetadata::default()
        }
    };
    sort_diagnostics(&mut diagnostics);
    let title = match metadata.title.trim() {
        "" => file_stem(&file_name).to_owned(),
        title => title.to_owned(),
    };
    CatalogEntry {
        identity,
        path,
        file_name,
        title,
        metadata,
        model,
        model_source,
        diagnostics,
    }
}

#[cfg(test)]
mod tests {
    use common::temporary::TemporaryFolder;
    use machine_factory::config::systems::compiled_systems;
    #[cfg(feature = "pc98")]
    use machine_factory::config::{EmulatorConfig, model_name};

    use super::*;
    use crate::{
        catalog::Catalog,
        config::{LaunchRequest, LineContent, Locations, SETTINGS_FILE_NAME, parse_line},
    };

    /// A game file with Japanese comments, compact and spaced separators, a
    /// repeated `fdd1`, commented-out keys and no final newline.
    const JAPANESE_COMMENTS: &str = "machine=PC9801VM\n\
    ; 26K ボードで BGM が鳴る。86 は効果音のみ。\n\
    soundboard=26k\n\
    force-gdc-clock = 2.5\n\
    ;fdd1=games/98/os/MS-DOS 6.20 (System disk #1).hdm\n\
    fdd1=games/98/games/Dragon Knight (Disk A).hdm\n\
    fdd1=games/98/games/Dragon Knight (Disk B).hdm\n\
    # bios=on\n\
    meta.title-original=ドラゴンナイト\n\
    ;bios=on";

    /// A game file with CRLF line endings and a byte order mark.
    const CRLF_WITH_BYTE_ORDER_MARK: &str = "\u{FEFF}; Written on Windows\r\n\
    meta.title = Ys II: Ancient Ys Vanished The Final Chapter\r\n\
    meta.year = 1988\r\n\
    machine = PC9801VX\r\n\
    \r\n\
    fdd1 = games/98/Ys II (Disk A).d88\r\n\
    fdd1 = games/98/Ys II (Disk B).d88\r\n";

    /// A game file whose lines are kept with warnings: an unknown key, a host
    /// key, an invalid value, a repeated single key, a bad year and free text.
    const WARNINGS: &str = "meta.title = Policenauts\n\
    machine = PC9801RA\n\
    fdd3 = disk.d88\n\
    sc55-roms = roms/sc55\n\
    soundboard = 99\n\
    machine = PC9821AP\n\
    meta.year = nineteen\n\
    cargo run --release -- --machine PC9821AP\n";

    /// A temporary config folder. Dropping the value removes the folder.
    struct ConfigFolder(TemporaryFolder);

    impl ConfigFolder {
        /// Creates an empty and unique temporary folder.
        fn new(name: &str) -> Self {
            Self(TemporaryFolder::new(&format!("catalog_test_{name}")))
        }

        /// Writes `bytes` into the file `name` below the folder.
        fn write(&self, name: &str, bytes: &[u8]) -> PathBuf {
            self.0.write_file(name, bytes)
        }

        /// Returns the host layers of a settings file in the folder that holds `settings`.
        fn host(&self, settings: &str) -> HostLayers {
            let file = self.write(SETTINGS_FILE_NAME, settings.as_bytes());
            let locations = Locations {
                current_directory: self.0.path().to_path_buf(),
                executable_directory: None,
                pref_path: None,
            };
            let request = LaunchRequest {
                settings: Some(file),
                ..LaunchRequest::default()
            };
            HostLayers::read(&request, &locations).expect("settings read")
        }

        /// Scans the folder as the root and the config folder.
        fn scan(&self) -> Catalog {
            self.scan_with("")
        }

        /// Scans the folder as the root and the config folder with more settings lines.
        fn scan_with(&self, settings: &str) -> Catalog {
            Catalog::scan(&self.host(&format!("config-folder = .\n{settings}")))
        }
    }

    /// Returns the messages of the diagnostics with their lines.
    fn messages(diagnostics: &[Diagnostic]) -> Vec<String> {
        diagnostics
            .iter()
            .map(|diagnostic| format!("{:?} {diagnostic}", diagnostic.severity))
            .collect()
    }

    /// Returns the active lines and the other lines of a document without trailing whitespace.
    fn content(document: &ConfigDocument) -> (Vec<(String, String)>, Vec<String>) {
        let entries = document
            .entries()
            .map(|entry| (entry.key.to_owned(), entry.value.to_owned()))
            .collect();
        let others = document
            .lines()
            .filter(|(_, text)| !matches!(parse_line(text), LineContent::Entry { .. }))
            .map(|(_, text)| text.trim_end().to_owned())
            .collect();
        (entries, others)
    }

    /// Verifies that a save normalizes each fixture, keeps its content and reproduces a normalized file.
    #[test]
    fn fixtures_save_normalized_and_idempotent() {
        let folder = ConfigFolder::new("round_trip");
        for (index, text) in [JAPANESE_COMMENTS, CRLF_WITH_BYTE_ORDER_MARK, WARNINGS]
            .into_iter()
            .enumerate()
        {
            let source = folder.write(&format!("source_{index}.conf"), text.as_bytes());
            let original = ConfigDocument::load(&source).expect("fixture loads");
            original.save(&source).expect("fixture saves");
            let saved = std::fs::read(&source).expect("copy reads");
            assert_eq!(saved, original.to_text().as_bytes(), "fixture {index}");
            let reloaded = ConfigDocument::load(&source).expect("copy loads");
            assert_eq!(content(&reloaded), content(&original), "fixture {index}");
            reloaded.save(&source).expect("copy saves");
            assert_eq!(
                std::fs::read(&source).expect("copy reads"),
                saved,
                "fixture {index}"
            );
        }
        assert!(
            ConfigDocument::parse(CRLF_WITH_BYTE_ORDER_MARK)
                .to_text()
                .starts_with("\u{FEFF}; Written on Windows\r\n")
        );
        assert!(
            !ConfigDocument::parse(JAPANESE_COMMENTS)
                .to_text()
                .ends_with('\n')
        );
    }

    /// Verifies that edits of a fixture keep the other lines in their place.
    #[test]
    fn edits_of_a_fixture_keep_the_other_lines() {
        let mut document = ConfigDocument::parse(JAPANESE_COMMENTS);
        assert_eq!(
            document.get_all("fdd1"),
            vec![
                "games/98/games/Dragon Knight (Disk A).hdm",
                "games/98/games/Dragon Knight (Disk B).hdm"
            ]
        );
        document.set("force-gdc-clock", "5");
        document.set_all("fdd1", &["b.hdm", "a.hdm"]);
        document.set("bios", "on");
        assert_eq!(
            document.to_text(),
            "machine = PC9801VM\n\
         ; 26K ボードで BGM が鳴る。86 は効果音のみ。\n\
         soundboard = 26k\n\
         force-gdc-clock = 5\n\
         ;fdd1=games/98/os/MS-DOS 6.20 (System disk #1).hdm\n\
         fdd1 = b.hdm\n\
         fdd1 = a.hdm\n\
         # bios=on\n\
         meta.title-original = ドラゴンナイト\n\
         ;bios=on\n\
         bios = on"
        );

        let mut document = ConfigDocument::parse(CRLF_WITH_BYTE_ORDER_MARK);
        document.set("meta.genre", "Action RPG");
        assert_eq!(
            document.to_text(),
            format!("{CRLF_WITH_BYTE_ORDER_MARK}meta.genre = Action RPG\r\n")
        );
    }

    /// Verifies that a missing config folder gives every system without games.
    #[test]
    fn missing_folders_give_empty_systems() {
        let folder = ConfigFolder::new("missing");
        let catalog = Catalog::scan(&folder.host("config-folder = absent\n"));
        assert_eq!(catalog.systems.len(), compiled_systems().count());
        for system in &catalog.systems {
            assert_eq!(system.game_count(), 0);
            assert!(system.files_with_errors.is_empty());
            assert_eq!(system.default_model, system.system.default_model);
            assert_eq!(system.default_model_source, ModelSource::BuiltIn);
            assert_eq!(
                system.defaults_file,
                folder
                    .0
                    .path()
                    .join("absent")
                    .join(format!("{}.conf", system.system.name))
            );
        }
    }

    /// Verifies the classification of game files into games and files with errors.
    #[cfg(feature = "pc98")]
    #[test]
    fn catalog_separates_games_from_files_with_errors() {
        let folder = ConfigFolder::new("classification");
        folder.write("pc98.conf", b"machine = PC9821AP\nfdd1 = system.d88\n");
        folder.write("pc98/dragon_knight.conf", JAPANESE_COMMENTS.as_bytes());
        folder.write("pc98/ys2.conf", CRLF_WITH_BYTE_ORDER_MARK.as_bytes());
        folder.write("pc98/policenauts.conf", WARNINGS.as_bytes());
        folder.write(
            "pc98/no_machine.conf",
            b"; nothing but a comment\nbios = on\n",
        );
        folder.write(
            "pc98/nemesis.conf",
            b"meta.title = Nemesis '90\n\n\nmachine = X68030\n",
        );
        folder.write("pc98/broken.conf", b"machine = PC9801RA\nfdd1 = \xff.d88\n");
        folder.write("pc98/notes.txt", b"machine = FOO\n");
        folder.write("pc98/nested/inner.conf", b"machine = PC9801RA\n");

        let catalog = folder.scan();
        let pc98 = catalog.system("pc98").expect("pc98 is compiled");
        assert_eq!(pc98.default_model, "PC9821AP");
        assert_eq!(pc98.default_model_source, ModelSource::SystemDefaults);
        assert_eq!(
            messages(&pc98.defaults_diagnostics),
            vec![
                "Warning line 2: media key fdd1 is ignored. Media keys belong to the game files."
                    .to_owned()
            ]
        );

        let games: Vec<(&str, &str, &str, ModelSource)> = pc98
            .games
            .iter()
            .map(|entry| {
                (
                    entry.identity.as_str(),
                    entry.title.as_str(),
                    entry.model,
                    entry.model_source,
                )
            })
            .collect();
        assert_eq!(
            games,
            vec![
                (
                    "pc98/dragon_knight.conf",
                    "dragon_knight",
                    "PC9801VM",
                    ModelSource::GameFile
                ),
                (
                    "pc98/no_machine.conf",
                    "no_machine",
                    "PC9821AP",
                    ModelSource::SystemDefaults
                ),
                (
                    "pc98/policenauts.conf",
                    "Policenauts",
                    "PC9821AP",
                    ModelSource::GameFile
                ),
                (
                    "pc98/ys2.conf",
                    "Ys II: Ancient Ys Vanished The Final Chapter",
                    "PC9801VX",
                    ModelSource::GameFile
                ),
            ]
        );
        assert_eq!(pc98.game_count(), 4);

        let dragon_knight = pc98
            .find("pc98/dragon_knight.conf")
            .expect("dragon knight is listed");
        assert!(dragon_knight.diagnostics.is_empty());
        assert_eq!(dragon_knight.metadata.title_original, "ドラゴンナイト");
        assert_eq!(
            dragon_knight.path,
            folder.0.path().join("pc98/dragon_knight.conf")
        );
        assert_eq!(dragon_knight.file_name, "dragon_knight.conf");

        let ys2 = pc98.find("pc98/ys2.conf").expect("ys2 is listed");
        assert!(ys2.diagnostics.is_empty());
        assert_eq!(ys2.metadata.year(), Some(1988));

        let policenauts = pc98
            .find("pc98/policenauts.conf")
            .expect("policenauts is listed");
        assert_eq!(
            messages(&policenauts.diagnostics),
            vec![
                "Warning line 3: unknown key fdd3",
                "Warning line 4: host key sc55-roms is ignored. Only the settings file and the \
             command line set host keys.",
                "Warning line 5: invalid soundboard: unknown soundboard type '99', expected none, \
             14, 26k, 86, 86+26k, sb16 or sb16+26k",
                "Warning line 6: duplicate key machine. It overrides line 2.",
                "Warning line 7: invalid meta.year: 'nineteen' is not a year from 1970 to 2100",
                "Warning line 8: text without a key",
            ]
        );

        let errors: Vec<(&str, Vec<String>)> = pc98
            .files_with_errors
            .iter()
            .map(|entry| (entry.identity.as_str(), messages(&entry.diagnostics)))
            .collect();
        assert_eq!(
            errors,
            vec![
                (
                    "pc98/broken.conf",
                    vec!["Error line 2: the file is not valid UTF-8".to_owned()]
                ),
                (
                    "pc98/nemesis.conf",
                    vec!["Error line 4: unknown machine X68030".to_owned()]
                ),
            ]
        );
        let nemesis = pc98.find("pc98/nemesis.conf").expect("nemesis is listed");
        assert_eq!(nemesis.title, "Nemesis '90");
        assert_eq!(nemesis.model_source, ModelSource::SystemDefaults);
    }

    /// Verifies that a machine of another family is an error of the file.
    #[cfg(all(feature = "pc98", feature = "x68k"))]
    #[test]
    fn machine_of_another_family_is_an_error() {
        let folder = ConfigFolder::new("foreign");
        folder.write("pc98/gradius.conf", b"machine=X68000\nfdd1=gradius.dim\n");
        folder.write("x68k.conf", b"machine = PC9801RA\n");
        let catalog = folder.scan();
        let pc98 = catalog.system("pc98").expect("pc98 is compiled");
        assert_eq!(pc98.game_count(), 0);
        let gradius = &pc98.files_with_errors[0];
        assert_eq!(gradius.model, "X68000");
        assert_eq!(
            gradius
                .first_error()
                .map(|diagnostic| diagnostic.to_string()),
            Some("line 1: machine X68000 belongs to another system".to_owned())
        );

        let x68k = catalog.system("x68k").expect("x68k is compiled");
        assert_eq!(x68k.default_model, "X68000");
        assert_eq!(x68k.default_model_source, ModelSource::BuiltIn);
        assert_eq!(
            messages(&x68k.defaults_diagnostics),
            vec![
                "Warning line 1: machine PC9801RA belongs to another system. The line is ignored."
            ]
        );
    }

    /// Verifies the order of the games: title without regard to case, then file name.
    #[cfg(feature = "pc98")]
    #[test]
    fn games_sort_by_title_then_file_name() {
        let folder = ConfigFolder::new("sort");
        folder.write("pc98/b.conf", b"meta.title = alpha\n");
        folder.write("pc98/a.conf", b"meta.title = Alpha\n");
        folder.write("pc98/zeta.conf", b"machine = PC9801RA\n");
        folder.write("pc98/c.conf", b"meta.title = beta\n");
        folder.write("pc98/d.conf", b"meta.title = Beta 2\n");
        let catalog = folder.scan();
        let pc98 = catalog.system("pc98").expect("pc98 is compiled");
        let order: Vec<&str> = pc98
            .games
            .iter()
            .map(|entry| entry.file_name.as_str())
            .collect();
        assert_eq!(
            order,
            vec!["a.conf", "b.conf", "c.conf", "d.conf", "zeta.conf"]
        );
        assert_eq!(pc98.games[4].title, "zeta");
    }

    /// Verifies that the metadata of a catalog entry round-trips through its file.
    #[cfg(feature = "pc98")]
    #[test]
    fn catalog_metadata_round_trips_through_the_file() {
        let folder = ConfigFolder::new("metadata");
        let path = folder.write("pc98/ys2.conf", CRLF_WITH_BYTE_ORDER_MARK.as_bytes());
        let catalog = folder.scan();
        let entry = catalog
            .system("pc98")
            .and_then(|system| system.find("pc98/ys2.conf"))
            .expect("ys2 is listed");
        let mut metadata = entry.metadata.clone();
        metadata.developer = "Nihon Falcom".to_owned();
        let mut document = ConfigDocument::load(&path).expect("file loads");
        metadata.write(&mut document);
        document.save(&path).expect("file saves");

        let catalog = folder.scan();
        let entry = catalog
            .system("pc98")
            .and_then(|system| system.find("pc98/ys2.conf"))
            .expect("ys2 is listed");
        assert_eq!(entry.metadata, metadata);
        assert_eq!(
            std::fs::read_to_string(&path).expect("file reads"),
            "\u{FEFF}; Written on Windows\r\n\
         meta.title = Ys II: Ancient Ys Vanished The Final Chapter\r\n\
         meta.year = 1988\r\n\
         meta.developer = Nihon Falcom\r\n\
         machine = PC9801VX\r\n\
         \r\n\
         fdd1 = games/98/Ys II (Disk A).d88\r\n\
         fdd1 = games/98/Ys II (Disk B).d88\r\n"
        );
    }

    /// Verifies that the read of one game file gives the entry of the scan and follows a change of the file.
    #[cfg(feature = "pc98")]
    #[test]
    fn one_game_reads_as_the_scan_reads_it() {
        let folder = ConfigFolder::new("read_game");
        let path = folder.write("pc98/ys2.conf", CRLF_WITH_BYTE_ORDER_MARK.as_bytes());
        let host = folder.host("config-folder = .\n");
        let system = compiled_systems()
            .find(|system| system.name == "pc98")
            .expect("pc98 is compiled");
        let scanned = folder
            .scan()
            .system("pc98")
            .and_then(|catalog| catalog.find("pc98/ys2.conf"))
            .cloned()
            .expect("ys2 is listed");
        let read = crate::catalog::read_game(&host, system, &path).expect("the game reads");
        assert_eq!(read, scanned);
        std::fs::write(&path, "meta.title = Ys II Eternal\nmachine = PC9801RA\n")
            .expect("file is written");
        let read = crate::catalog::read_game(&host, system, &path).expect("the game reads");
        assert_eq!(read.title, "Ys II Eternal");
        assert_eq!(read.model, "PC9801RA");
        assert_eq!(read.identity, "pc98/ys2.conf");
    }

    /// Returns the result of a `--config` launch of `game` with the settings file of `folder`.
    #[cfg(feature = "pc98")]
    fn launch(folder: &ConfigFolder, game: &Path) -> crate::Result<EmulatorConfig> {
        let locations = Locations {
            current_directory: folder.0.path().to_path_buf(),
            executable_directory: None,
            pref_path: None,
        };
        let request = LaunchRequest {
            settings: Some(folder.0.path().join(SETTINGS_FILE_NAME)),
            game_files: vec![game.to_path_buf()],
            ..LaunchRequest::default()
        };
        crate::config::build_configuration(&request, &locations).map(|(config, _)| config)
    }

    /// Verifies that the catalog shows the model of the settings file that a launch takes.
    #[cfg(feature = "pc98")]
    #[test]
    fn settings_machine_reaches_the_catalog_and_the_launch() {
        let folder = ConfigFolder::new("settings_machine");
        let game = folder.write("pc98/plain.conf", b"bios = on\n");
        let catalog = folder.scan_with("machine = PC9821AP\n");
        let pc98 = catalog.system("pc98").expect("pc98 is compiled");
        assert_eq!(pc98.default_model, "PC9821AP");
        assert_eq!(pc98.default_model_source, ModelSource::SettingsFile);
        let entry = pc98.find("pc98/plain.conf").expect("plain is listed");
        assert_eq!(
            (entry.model, entry.model_source),
            ("PC9821AP", ModelSource::SettingsFile)
        );
        let config = launch(&folder, &game).expect("game launches");
        assert_eq!(model_name(&config), entry.model);

        folder.write("pc98.conf", b"machine = PC9801VX\n");
        let catalog = folder.scan_with("machine = PC9821AP\n");
        let entry = catalog
            .system("pc98")
            .and_then(|system| system.find("pc98/plain.conf"))
            .expect("plain is listed");
        assert_eq!(
            (entry.model, entry.model_source),
            ("PC9801VX", ModelSource::SystemDefaults)
        );
        assert_eq!(
            model_name(&launch(&folder, &game).expect("game launches")),
            "PC9801VX"
        );
    }

    /// Verifies that the catalog and a `--config` launch agree on the files that cannot start.
    #[cfg(feature = "pc98")]
    #[test]
    fn files_with_errors_fail_their_launch() {
        let folder = ConfigFolder::new("agreement");
        folder.write("pc98/good.conf", b"machine = PC9801RA\nfdd3 = x.d88\n");
        folder.write("pc98/unknown.conf", b"machine = X68030\n");
        folder.write("pc98/bytes.conf", b"machine = PC9801RA\n; \xfe\n");
        #[cfg(feature = "x68k")]
        folder.write("pc98/foreign.conf", b"machine = X68000\n");
        let catalog = folder.scan();
        let pc98 = catalog.system("pc98").expect("pc98 is compiled");
        for entry in &pc98.games {
            assert!(launch(&folder, &entry.path).is_ok(), "{}", entry.identity);
        }
        for entry in &pc98.files_with_errors {
            let error = launch(&folder, &entry.path)
                .expect_err("launch fails")
                .to_string();
            let diagnostic = entry.first_error().expect("entry has an error");
            assert!(
                error.contains(&diagnostic.message),
                "{}: {error}",
                entry.identity
            );
        }
        assert_eq!(pc98.game_count(), 1);
    }

    /// Verifies that a system defaults file that fails to read marks every game of its system.
    #[cfg(feature = "pc98")]
    #[test]
    fn broken_system_defaults_mark_every_game() {
        let folder = ConfigFolder::new("broken_defaults");
        let game = folder.write("pc98/good.conf", b"machine = PC9801RA\n");
        folder.write("pc98.conf", b"machine = PC9801RA\nbios = \xff\n");
        let catalog = folder.scan();
        let pc98 = catalog.system("pc98").expect("pc98 is compiled");
        assert_eq!(pc98.game_count(), 0);
        let entry = &pc98.files_with_errors[0];
        let defaults_file = folder.0.path().join("pc98.conf");
        assert_eq!(
            entry.first_error().map(ToString::to_string),
            Some(format!(
                "system defaults {} line 2: the file is not valid UTF-8",
                defaults_file.display()
            ))
        );
        assert_eq!(
            messages(&pc98.defaults_diagnostics),
            vec!["Error line 2: the file is not valid UTF-8"]
        );
        assert!(launch(&folder, &game).is_err());

        folder.write("pc98.conf", b"machine = X68030\nbios = on\n");
        let catalog = folder.scan();
        let pc98 = catalog.system("pc98").expect("pc98 is compiled");
        assert_eq!(pc98.game_count(), 1);
        assert_eq!(
            messages(&pc98.defaults_diagnostics),
            vec!["Warning line 1: unknown machine X68030"]
        );
        assert!(launch(&folder, &game).is_ok());
    }

    /// Verifies that hidden files stay out of the catalog.
    #[cfg(feature = "pc98")]
    #[test]
    fn hidden_files_stay_out() {
        let folder = ConfigFolder::new("hidden");
        folder.write("pc98/.edge.conf", b"machine = PC9801RA\n");
        folder.write("pc98/.edge.conf.12.3.tmp", b"machine = PC9801RA\n");
        folder.write("pc98/edge.conf", b"machine = PC9801RA\n");
        let catalog = folder.scan();
        let pc98 = catalog.system("pc98").expect("pc98 is compiled");
        assert_eq!(pc98.game_count(), 1);
        assert!(pc98.files_with_errors.is_empty());
    }

    /// Verifies the classification on the first system of the build.
    #[test]
    fn first_system_classifies_its_files() {
        let system = compiled_systems().next().expect("one system is compiled");
        let folder = ConfigFolder::new("first_system");
        folder.write(
            &format!("{}/good.conf", system.name),
            format!("machine = {}\nfdd9 = x.d88\n", system.default_model).as_bytes(),
        );
        folder.write(&format!("{}/bad.conf", system.name), b"machine = NOPE\n");
        let catalog = folder.scan();
        let systems = catalog.system(system.name).expect("system is scanned");
        assert_eq!(systems.game_count(), 1);
        assert_eq!(
            messages(&systems.games[0].diagnostics),
            vec!["Warning line 2: unknown key fdd9"]
        );
        assert_eq!(systems.games[0].model, system.default_model);
        assert_eq!(
            messages(&systems.files_with_errors[0].diagnostics),
            vec!["Error line 1: unknown machine NOPE"]
        );
    }
}
