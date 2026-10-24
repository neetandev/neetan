//! The save of the game editor: the document with the changed fields, its
//! atomic write over a game file, the write of a new file that never
//! replaces a file, the read of a game file together with its stamp, and the
//! stamp that tells a change by another program.

use std::{
    fs::{File, Metadata, OpenOptions},
    io::{Read, Seek, SeekFrom},
    path::Path,
    time::SystemTime,
};

use super::fields::Field;
use crate::config::{ConfigDocument, DocumentError};

/// The most reads of a game file that another program writes during the read.
const MAXIMUM_READ_ATTEMPTS: usize = 4;

/// The modification time and the size of a file. A change of either tells
/// that a program wrote the file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FileStamp {
    /// The time of the last write, where the file system keeps it.
    modified: Option<SystemTime>,
    /// The size in bytes.
    length: u64,
}

impl FileStamp {
    /// The stamp of the file at `path`. `None` when no file reads there.
    pub(crate) fn of(path: &Path) -> Option<Self> {
        Self::from_metadata(&std::fs::metadata(path).ok()?)
    }

    /// The stamp of the file of `metadata`. `None` for a folder or another
    /// entry that is no file.
    pub(crate) fn from_metadata(metadata: &Metadata) -> Option<Self> {
        metadata.is_file().then(|| Self {
            modified: metadata.modified().ok(),
            length: metadata.len(),
        })
    }
}

/// Reads the document of the game file at `path` and the stamp of the
/// bytes that it holds.
///
/// The read and the stamps go through one open file. The stamp before and
/// the stamp after the read of the bytes must agree. A file that another
/// program writes during the read is read again, at most
/// `MAXIMUM_READ_ATTEMPTS` times. The error holds the path and the reason.
pub(crate) fn read_document(path: &Path) -> Result<(ConfigDocument, FileStamp), String> {
    read_document_with(path, |file, bytes| file.read_to_end(bytes).map(drop))
}

/// Reads the document of the file at `path` as [`read_document`] does, with
/// `read_bytes` for each read of the bytes from the start of the open file.
fn read_document_with(
    path: &Path,
    mut read_bytes: impl FnMut(&mut File, &mut Vec<u8>) -> std::io::Result<()>,
) -> Result<(ConfigDocument, FileStamp), String> {
    let failed = |error: DocumentError| format!("{}: {error}", path.display());
    let read_error = |error: std::io::Error| failed(DocumentError::Read(error));
    let mut file = File::open(path).map_err(read_error)?;
    let stamp = |file: &File| {
        let metadata = file.metadata().map_err(read_error)?;
        FileStamp::from_metadata(&metadata).ok_or_else(|| {
            read_error(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "the path holds no file",
            ))
        })
    };
    for _ in 0..MAXIMUM_READ_ATTEMPTS {
        let before = stamp(&file)?;
        let mut bytes = Vec::new();
        file.seek(SeekFrom::Start(0)).map_err(read_error)?;
        read_bytes(&mut file, &mut bytes).map_err(read_error)?;
        if stamp(&file)? == before {
            let document = ConfigDocument::from_bytes(&bytes).map_err(failed)?;
            return Ok((document, before));
        }
    }
    Err(format!(
        "{}: another program kept writing the file while the editor read it",
        path.display()
    ))
}

/// The error of a save over the game file at the shown path `shown` that
/// changed or went away since the editor read it.
pub(crate) fn changed_outside_message(shown: &str) -> String {
    format!("{shown} changed outside the manager since the editor read it.")
}

/// Returns `document` with the changed fields of `fields` written into it.
///
/// Every other line stays: comments, blank lines, key bindings, unknown keys
/// and the lines of fields without a change.
pub(crate) fn edited_document(document: &ConfigDocument, fields: &[Field]) -> ConfigDocument {
    let mut edited = document.clone();
    for field in fields {
        field.write(&mut edited);
    }
    edited
}

/// Returns the first error of a field that the file cannot hold.
pub(crate) fn validation_error(fields: &[Field]) -> Option<String> {
    fields.iter().find_map(Field::error)
}

/// Writes the normalized text of `document` to `path` in one atomic step.
/// Returns the stamp of the written file. The error holds the path and the
/// reason.
pub(crate) fn save_document(
    document: &ConfigDocument,
    path: &Path,
) -> Result<Option<FileStamp>, String> {
    document
        .save(path)
        .map(|metadata| FileStamp::from_metadata(&metadata))
        .map_err(|error| format!("{}: {error}", path.display()))
}

/// Writes the normalized text of `document` into a new file at `path`.
///
/// The write never replaces a file. It first reserves the name with an
/// empty file that only this write creates. A file at `path` stays as it
/// is, and the write fails with [`std::io::ErrorKind::AlreadyExists`]. The
/// atomic save of the document then replaces the reservation: a temporary
/// file, a sync, a rename and a sync of the folder. A failed save removes
/// the reservation. Returns the stamp of the written file.
pub(crate) fn create_document(
    document: &ConfigDocument,
    path: &Path,
) -> std::io::Result<Option<FileStamp>> {
    reserve_and_fill(path, |path| document.save(path))
        .map(|metadata| FileStamp::from_metadata(&metadata))
}

/// Reserves the new file `path` and fills it with `fill`. A failed `fill`
/// removes the reservation. Returns what `fill` returns.
fn reserve_and_fill<T>(
    path: &Path,
    fill: impl FnOnce(&Path) -> std::io::Result<T>,
) -> std::io::Result<T> {
    if let Some(folder) = path.parent() {
        std::fs::create_dir_all(folder)?;
    }
    drop(OpenOptions::new().write(true).create_new(true).open(path)?);
    let filled = fill(path);
    if filled.is_err() {
        let _ = std::fs::remove_file(path);
    }
    filled
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use machine_factory::config::MachineSelection;

    use super::*;
    use crate::{
        config::{LineContent, parse_line},
        editor::{
            fields::{ChoiceValue, FieldValue},
            form::form_fields,
            test_support::EditorRoot,
        },
    };

    /// The fields of a game of the family of `root` read from `document`.
    fn fields(root: &EditorRoot, document: &ConfigDocument) -> Vec<Field> {
        form_fields(root.target())
            .into_iter()
            .map(|(section, source)| Field::read(section, source, root.target(), document, false))
            .collect()
    }

    /// The field of `key` among `fields`.
    fn field<'a>(fields: &'a mut [Field], key: &str) -> &'a mut Field {
        fields
            .iter_mut()
            .find(|field| field.key() == key)
            .expect("the form has the key")
    }

    /// A game file with comments, an unknown key, a key binding, a compact separator and Japanese text.
    const GAME: &str = "; ゲーム設定\nmeta.title=Edge\nmachine = {model}\nfdd3 = old.d88\n\
                        key.F1 = Stop\n\n  crt=on\n# end\n";

    /// Verifies that a save without a change writes the normalized form and that every edit keeps the other lines.
    #[test]
    fn a_save_normalizes_and_keeps_the_other_lines() {
        let root = EditorRoot::new("save_lines");
        let model = root.system().default_model;
        let text = GAME.replace("{model}", model);
        let document = ConfigDocument::parse(&text);
        let mut fields = fields(&root, &document);
        let unchanged = edited_document(&document, &fields);
        assert_eq!(unchanged.to_text(), document.to_text());
        assert_eq!(
            unchanged.to_text(),
            format!(
                "; ゲーム設定\nmeta.title = Edge\nmachine = {model}\nfdd3 = old.d88\n\
                 key.F1 = Stop\n\ncrt = on\n# end\n"
            )
        );

        field(&mut fields, "crt").unset();
        field(&mut fields, "meta.developer").value = FieldValue::Text("TGL".to_owned());
        let scaling = field(&mut fields, "scaling");
        let index = scaling
            .choices
            .iter()
            .position(|choice| *choice == "bilinear")
            .expect("bilinear is a choice");
        scaling.value = FieldValue::Choice(ChoiceValue {
            selected: Some(index),
            invalid: None,
        });
        let edited = edited_document(&document, &fields);
        assert_eq!(
            edited.to_text(),
            format!(
                "; ゲーム設定\nmeta.title = Edge\nmeta.developer = TGL\nmachine = {model}\n\
                 fdd3 = old.d88\nkey.F1 = Stop\n\n# end\nscaling = bilinear\n"
            )
        );
        let path = root.game("edge", &text);
        save_document(&edited, &path).expect("the file saves");
        assert_eq!(
            std::fs::read_to_string(&path).expect("file reads"),
            edited.to_text()
        );
    }

    /// Verifies that a save into a folder that cannot exist reports the path and the reason.
    #[test]
    fn a_save_error_holds_the_path() {
        let root = EditorRoot::new("save_error");
        root.write("blocked", "a file where a folder belongs");
        let path = root.path("blocked/game.conf");
        let error = save_document(&ConfigDocument::parse("crt = on\n"), &path)
            .expect_err("the folder is a file");
        assert!(error.starts_with(&path.display().to_string()), "{error}");
        assert!(!path.exists());
    }

    /// Verifies that a read gives the document and the stamp of the file, and that a folder, a missing file and bytes that are not UTF-8 fail with the path.
    #[test]
    fn a_read_gives_the_document_and_its_stamp() {
        let root = EditorRoot::new("save_read");
        let path = root.game("edge", "meta.title = Edge\n");
        let (document, stamp) = read_document(&path).expect("the file reads");
        assert_eq!(document.get("meta.title"), Some("Edge"));
        assert_eq!(Some(stamp), FileStamp::of(&path));
        for failing in [root.path("config"), root.path("config/absent.conf")] {
            let error = read_document(&failing).expect_err("no file reads there");
            assert!(error.starts_with(&failing.display().to_string()), "{error}");
        }
        let broken = root.write("config/broken.conf", "");
        std::fs::write(&broken, b"crt = \xFF\n").expect("file is written");
        let error = read_document(&broken).expect_err("the bytes are not UTF-8");
        assert!(error.contains("UTF-8"), "{error}");
    }

    /// Verifies that a file that another program writes during the read is read again, and that the stamp belongs to the bytes of the document.
    #[test]
    fn a_write_during_the_read_reads_the_file_again() {
        let root = EditorRoot::new("save_read_again");
        let path = root.game("edge", "meta.title = Edge\n");
        let mut reads = 0;
        let (document, stamp) = read_document_with(&path, |file, bytes| {
            reads += 1;
            file.read_to_end(bytes)?;
            if reads == 1 {
                let mut writer = OpenOptions::new().append(true).open(&path)?;
                std::io::Write::write_all(&mut writer, b"crt = on\n")?;
            }
            Ok(())
        })
        .expect("the second read is stable");
        assert_eq!(reads, 2);
        assert_eq!(document.get("crt"), Some("on"));
        assert_eq!(Some(stamp), FileStamp::of(&path));
    }

    /// Verifies that a file that changes during every read fails after the last attempt.
    #[test]
    fn a_file_that_keeps_changing_fails_the_read() {
        let root = EditorRoot::new("save_read_changing");
        let path = root.game("edge", "meta.title = Edge\n");
        let mut reads = 0;
        let error = read_document_with(&path, |file, bytes| {
            reads += 1;
            file.read_to_end(bytes)?;
            let mut writer = OpenOptions::new().append(true).open(&path)?;
            std::io::Write::write_all(&mut writer, b"; more\n")
        })
        .expect_err("the file never holds still");
        assert_eq!(reads, MAXIMUM_READ_ATTEMPTS);
        assert!(error.starts_with(&path.display().to_string()), "{error}");
    }

    /// Verifies that the write of a new file never replaces a file and that the stamp follows a write.
    #[test]
    fn a_new_file_never_replaces_a_file() {
        let root = EditorRoot::new("save_new");
        let path = root.path("config/pc98/edge.conf");
        let written =
            create_document(&ConfigDocument::parse("crt=on\n"), &path).expect("the file is new");
        assert_eq!(
            std::fs::read_to_string(&path).expect("file reads"),
            "crt = on\n"
        );
        let stamp = FileStamp::of(&path);
        assert!(stamp.is_some());
        assert_eq!(written, stamp);
        let error = create_document(&ConfigDocument::parse("crt = off\n"), &path)
            .expect_err("a file exists");
        assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);
        assert_eq!(
            std::fs::read_to_string(&path).expect("file reads"),
            "crt = on\n"
        );
        assert_eq!(FileStamp::of(&path), stamp);
        std::fs::write(&path, "crt = off\nscaling = nearest\n").expect("file is written");
        assert_ne!(FileStamp::of(&path), stamp);
        assert_eq!(FileStamp::of(&root.path("config/pc98")), None);
        assert_eq!(FileStamp::of(&root.path("config/pc98/absent.conf")), None);
    }

    /// Verifies that a failed fill of a new file removes its reservation and that a later save takes the name.
    #[test]
    fn a_failed_write_removes_the_reservation() {
        let root = EditorRoot::new("save_reservation");
        let path = root.path("config/pc98/edge.conf");
        let error = reserve_and_fill(&path, |reserved| {
            assert!(reserved.is_file());
            assert_eq!(std::fs::read(reserved).expect("reservation reads"), b"");
            Err::<(), _>(std::io::Error::other("disk full"))
        })
        .expect_err("the fill fails");
        assert_eq!(error.to_string(), "disk full");
        assert!(!path.exists());
        create_document(&ConfigDocument::parse("crt=on\n"), &path).expect("the name is free");
        assert_eq!(
            std::fs::read_to_string(&path).expect("file reads"),
            "crt = on\n"
        );
        let leftovers: Vec<_> = std::fs::read_dir(root.path("config/pc98"))
            .expect("folder reads")
            .filter_map(Result::ok)
            .map(|entry| entry.file_name())
            .collect();
        assert_eq!(leftovers, ["edge.conf"]);
    }

    /// Verifies that the first field with a value outside its range is the validation error.
    #[test]
    fn a_bad_year_is_a_validation_error() {
        let root = EditorRoot::new("save_validation");
        let document = ConfigDocument::parse("meta.year = 1993\n");
        let mut fields = fields(&root, &document);
        assert_eq!(validation_error(&fields), None);
        field(&mut fields, "meta.year").value = FieldValue::Text("3000".to_owned());
        assert_eq!(
            validation_error(&fields).as_deref(),
            Some("Year: '3000' is not a year from 1970 to 2100")
        );
    }

    /// The active `key = value` lines of `text` grouped by key, in file order.
    fn entries(text: &str) -> BTreeMap<String, Vec<String>> {
        let mut entries: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for line in text.lines() {
            if let LineContent::Entry { key, value } = parse_line(line) {
                entries
                    .entry(key.to_owned())
                    .or_default()
                    .push(value.to_owned());
            }
        }
        entries
    }

    /// Verifies the round trip of every game file of the corpus of the
    /// repository: the editor reads it with the fields of its family and
    /// saves it into a scratch copy without edits. The copy holds the
    /// normalized form of the file and every key, and a second round trip
    /// gives the same bytes. The corpus files are only read.
    #[test]
    fn every_corpus_file_round_trips_through_the_editor() {
        let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config");
        let Ok(folders) = std::fs::read_dir(&corpus) else {
            return;
        };
        let scratch = EditorRoot::new("corpus");
        let mut files = 0usize;
        for folder in folders.filter_map(Result::ok) {
            let name = folder.file_name().to_string_lossy().into_owned();
            let Some(system) = machine_factory::config::systems::compiled_systems()
                .find(|system| system.name == name)
            else {
                continue;
            };
            let target = MachineSelection::of_system(system)
                .expect("compiled system")
                .target();
            let Ok(games) = std::fs::read_dir(folder.path()) else {
                continue;
            };
            for game in games.filter_map(Result::ok) {
                let path = game.path();
                if path.extension().is_none_or(|extension| extension != "conf") {
                    continue;
                }
                let original = std::fs::read(&path).expect("corpus file reads");
                let document = ConfigDocument::from_bytes(&original).expect("UTF-8");
                let fields: Vec<Field> = form_fields(target)
                    .into_iter()
                    .map(|(section, source)| Field::read(section, source, target, &document, false))
                    .collect();
                assert!(
                    fields.iter().all(|field| !field.changed()),
                    "{}",
                    path.display()
                );
                let copy = scratch.path(&format!("{name}/{}", game.file_name().display()));
                save_document(&edited_document(&document, &fields), &copy).expect("copy saves");
                let saved = std::fs::read(&copy).expect("copy reads");
                assert_eq!(
                    saved,
                    document.to_text().into_bytes(),
                    "{} saves its normalized form",
                    path.display()
                );
                let text = String::from_utf8(saved.clone()).expect("UTF-8");
                assert_eq!(
                    entries(&text),
                    entries(&String::from_utf8_lossy(&original)),
                    "{} keeps every key",
                    path.display()
                );
                let reread = ConfigDocument::parse(&text);
                let fields: Vec<Field> = form_fields(target)
                    .into_iter()
                    .map(|(section, source)| Field::read(section, source, target, &reread, false))
                    .collect();
                save_document(&edited_document(&reread, &fields), &copy).expect("copy saves");
                assert_eq!(
                    std::fs::read(&copy).expect("copy reads"),
                    saved,
                    "{} saves the same bytes twice",
                    path.display()
                );
                files += 1;
            }
        }
        assert!(files > 0, "the corpus holds game files");
        eprintln!("corpus round trip: {files} game files");
    }
}
