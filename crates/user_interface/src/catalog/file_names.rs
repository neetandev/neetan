//! The file name of a new or duplicated game, derived from its title, and the free file name of
//! a new file in a folder.

use std::{collections::HashSet, path::Path};

use i18n::Catalog;

use crate::{config::CONFIG_FILE_EXTENSION, texts::editor};

/// The file stem of a title without letters or digits.
pub const FALLBACK_FILE_STEM: &str = "game";
/// The longest file stem that a title derives.
pub const MAXIMUM_FILE_STEM_LENGTH: usize = 80;
/// The file stems that Windows reserves as device names.
const RESERVED_FILE_STEMS: [&str; 24] = [
    "con", "prn", "aux", "nul", "com0", "com1", "com2", "com3", "com4", "com5", "com6", "com7",
    "com8", "com9", "lpt0", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
];

/// Derives a file stem from a title.
///
/// Lowercase ASCII letters and digits stay. Every run of other characters
/// becomes one hyphen. Leading and trailing hyphens go. A stem longer than
/// `MAXIMUM_FILE_STEM_LENGTH` ends at its last hyphen within the limit, or at
/// the limit when the stem has no hyphen there. An empty result becomes `game`.
#[must_use]
pub fn title_file_stem(title: &str) -> String {
    let mut stem = String::with_capacity(title.len());
    let mut pending_hyphen = false;
    for character in title.chars() {
        let character = character.to_ascii_lowercase();
        if character.is_ascii_lowercase() || character.is_ascii_digit() {
            if pending_hyphen && !stem.is_empty() {
                stem.push('-');
            }
            pending_hyphen = false;
            stem.push(character);
        } else {
            pending_hyphen = true;
        }
    }
    if stem.len() > MAXIMUM_FILE_STEM_LENGTH {
        let word_ends_at_limit = stem.as_bytes()[MAXIMUM_FILE_STEM_LENGTH] == b'-';
        let mut cut = &stem[..MAXIMUM_FILE_STEM_LENGTH];
        if !word_ends_at_limit && let Some(hyphen) = cut.rfind('-') {
            cut = &cut[..hyphen];
        }
        stem = cut.trim_end_matches('-').to_owned();
    }
    if stem.is_empty() {
        FALLBACK_FILE_STEM.to_owned()
    } else {
        stem
    }
}

/// Returns whether Windows reserves `stem` as a device name.
#[must_use]
pub fn is_reserved_stem(stem: &str) -> bool {
    RESERVED_FILE_STEMS.contains(&stem)
}

/// Derives a free file name with the extension `extension` from a title.
///
/// The stem comes from `title_file_stem`. A stem for which `is_taken` holds,
/// and a stem that Windows reserves as a device name, takes the suffix `-2`,
/// `-3` and so on.
#[must_use]
pub fn unique_file_name(title: &str, extension: &str, is_taken: impl Fn(&str) -> bool) -> String {
    let stem = title_file_stem(title);
    let mut candidate = stem.clone();
    let mut suffix = 2usize;
    while is_taken(&candidate) || is_reserved_stem(&candidate) {
        candidate = format!("{stem}-{suffix}");
        suffix += 1;
    }
    format!("{candidate}.{extension}")
}

/// Derives a file name with the extension `extension` from a title that no file in `folder`
/// holds.
///
/// The comparison with the existing file names ignores case.
#[must_use]
pub fn free_file_name(folder: &Path, title: &str, extension: &str) -> String {
    let existing: HashSet<String> = std::fs::read_dir(folder)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .map(|entry| entry.file_name().to_string_lossy().to_lowercase())
                .collect()
        })
        .unwrap_or_default();
    let extension = extension.to_lowercase();
    unique_file_name(title, &extension, |stem| {
        existing.contains(&format!("{stem}.{extension}"))
    })
}

/// Derives a game file name from a title that no file in `folder` holds.
///
/// The comparison with the existing file names ignores case.
#[must_use]
pub fn new_game_file_name(folder: &Path, title: &str) -> String {
    free_file_name(folder, title, CONFIG_FILE_EXTENSION)
}

/// Returns the title of a duplicated game of the title `title` with the
/// copy pattern of `catalog`.
#[must_use]
pub fn copy_title(catalog: &Catalog, title: &str) -> String {
    catalog.display(editor::copy_title(&title)).to_string()
}

#[cfg(test)]
mod tests {
    use common::temporary::TemporaryFolder;

    use super::*;
    use crate::texts::english;

    /// Verifies the file names of the examples table of the design document.
    #[test]
    fn file_names_follow_the_design_examples() {
        let examples: [(&str, &[&str], &str); 6] = [
            (
                "Brandish 2: The Planet Buster",
                &[],
                "brandish-2-the-planet-buster.conf",
            ),
            (
                "YU-NO: A Girl Who Chants Love at the Bound of this World",
                &[],
                "yu-no-a-girl-who-chants-love-at-the-bound-of-this-world.conf",
            ),
            (
                "Brandish (copy)",
                &["brandish-copy"],
                "brandish-copy-2.conf",
            ),
            ("エッジ", &[], "game.conf"),
            ("エッジ", &["game"], "game-2.conf"),
            (
                "Ys II: Ancient Ys Vanished The Final Chapter",
                &[
                    "ys-ii-ancient-ys-vanished-the-final-chapter",
                    "ys-ii-ancient-ys-vanished-the-final-chapter-2",
                ],
                "ys-ii-ancient-ys-vanished-the-final-chapter-3.conf",
            ),
        ];
        for (title, existing, expected) in examples {
            assert_eq!(
                unique_file_name(title, CONFIG_FILE_EXTENSION, |stem| existing
                    .contains(&stem)),
                expected,
                "{title}"
            );
        }
        assert_eq!(copy_title(english(), "Brandish"), "Brandish (copy)");
        assert_eq!(title_file_stem("  --Ys--  "), "ys");
        assert_eq!(title_file_stem("A.Train IV"), "a-train-iv");
        assert_eq!(title_file_stem(""), FALLBACK_FILE_STEM);
    }

    /// Verifies that the existing files of a folder take the suffix without regard to case.
    #[test]
    fn new_file_names_avoid_existing_files() {
        let folder = TemporaryFolder::new("file_names_test");
        assert_eq!(
            new_game_file_name(folder.path(), &copy_title(english(), "Brandish")),
            "brandish-copy.conf"
        );
        folder.write_file("Brandish-Copy.conf", b"");
        assert_eq!(
            new_game_file_name(folder.path(), &copy_title(english(), "Brandish")),
            "brandish-copy-2.conf"
        );
        assert_eq!(
            new_game_file_name(&folder.path().join("absent"), "Edge"),
            "edge.conf"
        );
    }

    /// Verifies the length limit of a file stem and the names that Windows reserves.
    #[test]
    fn file_stems_stay_short_and_avoid_device_names() {
        let long_title = "The Legend of an Extraordinarily Long Title That Keeps Going Past Every \
                      Reasonable Limit Forever";
        let stem = title_file_stem(long_title);
        assert_eq!(
            stem,
            "the-legend-of-an-extraordinarily-long-title-that-keeps-going-past-every"
        );
        assert!(stem.len() <= MAXIMUM_FILE_STEM_LENGTH);

        let exact = format!("{} {}", "a".repeat(40), "b".repeat(39));
        assert_eq!(title_file_stem(&format!("{exact} tail")).len(), 80);
        assert_eq!(
            title_file_stem(&format!("{exact} tail")),
            title_file_stem(&exact)
        );

        let single_word = "x".repeat(100);
        assert_eq!(title_file_stem(&single_word), "x".repeat(80));

        for (title, expected) in [
            ("CON", "con-2.conf"),
            ("Aux", "aux-2.conf"),
            ("LPT1", "lpt1-2.conf"),
            ("COM9", "com9-2.conf"),
            ("com0", "com0-2.conf"),
            ("LPT0", "lpt0-2.conf"),
            ("Conan", "conan.conf"),
            ("COM10", "com10.conf"),
        ] {
            assert_eq!(
                unique_file_name(title, CONFIG_FILE_EXTENSION, |_| false),
                expected,
                "{title}"
            );
        }
    }
}
