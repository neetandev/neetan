//! The schema of a catalog: the keys of the reference language and their placeholders.

use core::cmp::Ordering;

/// The section of the list patterns.
pub const LIST_SECTION: &str = "list";

/// The keys of the list patterns of the section `[list]`.
pub const LIST_PATTERN_KEYS: [&str; 6] = [
    "and-pair",
    "and-middle",
    "and-end",
    "or-pair",
    "or-middle",
    "or-end",
];

/// The placeholders of every list pattern in their argument order.
pub const LIST_PLACEHOLDERS: [&str; 2] = ["first", "second"];

/// The separator between the group and the name of a section such as `setting.cpu-mode`.
const GROUP_SEPARATOR: &[u8] = b".";

/// The parts of a section of a search. The section is their concatenation.
type SectionParts<'a> = [&'a [u8]; 3];

/// The largest number of keys that a schema holds.
pub const MAXIMUM_KEY_COUNT: usize = u16::MAX as usize + 1;

/// A key of a catalog: the index of the key in its schema.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Key(u16);

impl Key {
    /// Creates the key with the given index in its schema.
    pub const fn new(index: u16) -> Key {
        Key(index)
    }

    /// Returns the index of the key in its schema.
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

/// The description of one key of a schema.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyInfo {
    /// The section name.
    section: &'static str,
    /// The key inside the section. A plural text has the key without its category.
    key: &'static str,
    /// The placeholder names in their argument order, without the `count` of a plural text.
    placeholders: &'static [&'static str],
    /// True for a plural text.
    plural: bool,
}

impl KeyInfo {
    /// Creates the description of a key.
    pub const fn new(
        section: &'static str,
        key: &'static str,
        placeholders: &'static [&'static str],
        plural: bool,
    ) -> KeyInfo {
        KeyInfo {
            section,
            key,
            placeholders,
            plural,
        }
    }

    /// Returns the section name.
    pub const fn section(&self) -> &'static str {
        self.section
    }

    /// Returns the key inside the section.
    pub const fn key(&self) -> &'static str {
        self.key
    }

    /// Returns the placeholder names in their argument order.
    pub const fn placeholders(&self) -> &'static [&'static str] {
        self.placeholders
    }

    /// Returns true for a plural text.
    pub const fn is_plural(&self) -> bool {
        self.plural
    }

    /// Returns the argument index of a placeholder.
    pub fn placeholder_index(&self, name: &str) -> Option<u16> {
        self.placeholders
            .iter()
            .position(|placeholder| *placeholder == name)
            .and_then(|index| u16::try_from(index).ok())
    }
}

/// The schema of a catalog: one [`KeyInfo`] for each key, sorted by section and key.
#[derive(Debug)]
pub struct Schema {
    /// The keys sorted by section and key.
    keys: &'static [KeyInfo],
    /// The keys of the list patterns in the order of [`LIST_PATTERN_KEYS`].
    list: [Key; LIST_PATTERN_KEYS.len()],
}

impl Schema {
    /// Creates a schema from keys sorted by section and key.
    ///
    /// # Panics
    ///
    /// Panics when the keys are out of order, when a key appears twice, when there are more than
    /// [`MAXIMUM_KEY_COUNT`] keys, or when a list pattern of [`LIST_PATTERN_KEYS`] is missing or
    /// has other placeholders than [`LIST_PLACEHOLDERS`]. In the initializer of a `static`, the
    /// panic is a compile error.
    pub const fn new(keys: &'static [KeyInfo]) -> Schema {
        assert!(keys.len() <= MAXIMUM_KEY_COUNT, "too many keys");
        let mut index = 1;
        while index < keys.len() {
            let previous = &keys[index - 1];
            let current = &keys[index];
            if !matches!(
                compare_full_keys(previous.section, previous.key, current.section, current.key),
                Ordering::Less
            ) {
                panic!("the keys of a schema are out of order or appear twice");
            }
            index += 1;
        }
        let mut list = [Key(0); LIST_PATTERN_KEYS.len()];
        let mut pattern = 0;
        while pattern < LIST_PATTERN_KEYS.len() {
            let Some(key) = find_in(
                keys,
                [LIST_SECTION.as_bytes(), b"", b""],
                LIST_PATTERN_KEYS[pattern],
            ) else {
                panic!("a list pattern is missing");
            };
            let info = &keys[key.index()];
            if info.plural || !names_equal(info.placeholders, &LIST_PLACEHOLDERS) {
                panic!("a list pattern has other placeholders than first and second");
            }
            list[pattern] = key;
            pattern += 1;
        }
        Schema { keys, list }
    }

    /// Finds a key by its section and its key inside the section with a binary search.
    pub const fn find(&self, section: &str, key: &str) -> Option<Key> {
        find_in(self.keys, [section.as_bytes(), b"", b""], key)
    }

    /// Finds a key by the two parts of its section and its key inside the section with a binary
    /// search. The section is `<group>.<name>`, for example the group `setting` and the name
    /// `cpu-mode` for the section `setting.cpu-mode`. The search compares the parts in place.
    pub const fn find_in_group(&self, group: &str, name: &str, key: &str) -> Option<Key> {
        find_in(
            self.keys,
            [group.as_bytes(), GROUP_SEPARATOR, name.as_bytes()],
            key,
        )
    }

    /// Returns the number of keys.
    pub const fn len(&self) -> usize {
        self.keys.len()
    }

    /// Returns true when the schema holds no key.
    pub const fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    /// Returns the keys sorted by section and key.
    pub const fn keys(&self) -> &'static [KeyInfo] {
        self.keys
    }

    /// Returns the description of a key.
    pub const fn info(&self, key: Key) -> &'static KeyInfo {
        &self.keys[key.index()]
    }

    /// Returns the key of a list pattern by its index in [`LIST_PATTERN_KEYS`].
    pub(crate) const fn list_key(&self, pattern: usize) -> Key {
        self.list[pattern]
    }
}

/// Finds a key in keys sorted by section and key. The section of the search is the concatenation
/// of its parts.
const fn find_in(keys: &[KeyInfo], section: SectionParts<'_>, key: &str) -> Option<Key> {
    let mut low = 0;
    let mut high = keys.len();
    while low < high {
        let middle = low + (high - low) / 2;
        let info = &keys[middle];
        let ordering = match compare_with_parts(info.section.as_bytes(), section) {
            Ordering::Equal => compare_bytes(info.key.as_bytes(), key.as_bytes()),
            ordering => ordering,
        };
        match ordering {
            Ordering::Less => low = middle + 1,
            Ordering::Greater => high = middle,
            Ordering::Equal => return Some(Key(middle as u16)),
        }
    }
    None
}

/// Compares a byte string with the concatenation of the parts of a section in lexicographic
/// order.
const fn compare_with_parts(first: &[u8], second: SectionParts<'_>) -> Ordering {
    let mut index = 0;
    let mut part = 0;
    let mut part_index = 0;
    loop {
        while part < second.len() && part_index == second[part].len() {
            part += 1;
            part_index = 0;
        }
        let first_ended = index == first.len();
        let second_ended = part == second.len();
        if first_ended && second_ended {
            return Ordering::Equal;
        }
        if first_ended {
            return Ordering::Less;
        }
        if second_ended {
            return Ordering::Greater;
        }
        let second_byte = second[part][part_index];
        if first[index] < second_byte {
            return Ordering::Less;
        }
        if first[index] > second_byte {
            return Ordering::Greater;
        }
        index += 1;
        part_index += 1;
    }
}

/// Compares two full keys by section first and by key second.
const fn compare_full_keys(
    first_section: &str,
    first_key: &str,
    second_section: &str,
    second_key: &str,
) -> Ordering {
    match compare_bytes(first_section.as_bytes(), second_section.as_bytes()) {
        Ordering::Equal => compare_bytes(first_key.as_bytes(), second_key.as_bytes()),
        ordering => ordering,
    }
}

/// Compares two byte strings in lexicographic order.
const fn compare_bytes(first: &[u8], second: &[u8]) -> Ordering {
    let mut index = 0;
    while index < first.len() && index < second.len() {
        if first[index] < second[index] {
            return Ordering::Less;
        }
        if first[index] > second[index] {
            return Ordering::Greater;
        }
        index += 1;
    }
    if first.len() < second.len() {
        Ordering::Less
    } else if first.len() > second.len() {
        Ordering::Greater
    } else {
        Ordering::Equal
    }
}

/// Returns true when two lists of names are equal.
const fn names_equal(first: &[&str], second: &[&str]) -> bool {
    if first.len() != second.len() {
        return false;
    }
    let mut index = 0;
    while index < first.len() {
        if !matches!(
            compare_bytes(first[index].as_bytes(), second[index].as_bytes()),
            Ordering::Equal
        ) {
            return false;
        }
        index += 1;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The list patterns that every schema holds.
    const LIST_KEYS: [KeyInfo; 6] = [
        KeyInfo::new("list", "and-end", &LIST_PLACEHOLDERS, false),
        KeyInfo::new("list", "and-middle", &LIST_PLACEHOLDERS, false),
        KeyInfo::new("list", "and-pair", &LIST_PLACEHOLDERS, false),
        KeyInfo::new("list", "or-end", &LIST_PLACEHOLDERS, false),
        KeyInfo::new("list", "or-middle", &LIST_PLACEHOLDERS, false),
        KeyInfo::new("list", "or-pair", &LIST_PLACEHOLDERS, false),
    ];

    /// The keys of the test schema, sorted by section and key.
    static KEYS: [KeyInfo; 16] = [
        KeyInfo::new("choice.aspect", "4:3", &[], false),
        KeyInfo::new("choice.aspect", "A", &[], false),
        KeyInfo::new("choice.aspect", "one", &[], false),
        KeyInfo::new("choice.sound", "86+26k", &[], false),
        KeyInfo::new("common", "cancel", &[], false),
        KeyInfo::new("common", "save", &[], false),
        LIST_KEYS[0],
        LIST_KEYS[1],
        LIST_KEYS[2],
        LIST_KEYS[3],
        LIST_KEYS[4],
        LIST_KEYS[5],
        KeyInfo::new("manager", "images", &["folder"], true),
        KeyInfo::new("setting.cpu-mode", "label", &[], false),
        KeyInfo::new("setting.cpu-mode-2", "label", &[], false),
        KeyInfo::new("setting.cpu-mode.fast", "label", &[], false),
    ];

    /// The test schema.
    static SCHEMA: Schema = Schema::new(&KEYS);

    #[test]
    fn find_compares_section_and_key_on_their_own() {
        assert_eq!(SCHEMA.find("common", "cancel"), Some(Key::new(4)));
        assert_eq!(SCHEMA.find("common", "save"), Some(Key::new(5)));
        assert_eq!(SCHEMA.find("manager", "images"), Some(Key::new(12)));
        assert_eq!(SCHEMA.find("setting.cpu-mode", "label"), Some(Key::new(13)));
        assert_eq!(SCHEMA.find("setting", "cpu-mode.label"), None);
        assert_eq!(SCHEMA.find("common", "cancel2"), None);
        assert_eq!(SCHEMA.find("", ""), None);
        assert_eq!(SCHEMA.find("zzz", "a"), None);
        assert_eq!(SCHEMA.find("choice.aspect", "4:3"), Some(Key::new(0)));
        assert_eq!(SCHEMA.find("choice.aspect", "A"), Some(Key::new(1)));
        assert_eq!(SCHEMA.find("choice.aspect", "a"), None);
        assert_eq!(SCHEMA.find("choice.aspect", "one"), Some(Key::new(2)));
        assert_eq!(SCHEMA.find("choice.sound", "86+26k"), Some(Key::new(3)));
        assert_eq!(SCHEMA.find("choice", "sound.86+26k"), None);
        for (index, info) in KEYS.iter().enumerate() {
            assert_eq!(
                SCHEMA.find(info.section(), info.key()).map(Key::index),
                Some(index)
            );
        }
    }

    #[test]
    fn find_in_group_compares_the_parts_of_the_section_in_place() {
        assert_eq!(
            SCHEMA.find_in_group("choice", "aspect", "4:3"),
            Some(Key::new(0))
        );
        assert_eq!(
            SCHEMA.find_in_group("choice", "sound", "86+26k"),
            Some(Key::new(3))
        );
        assert_eq!(
            SCHEMA.find_in_group("setting", "cpu-mode", "label"),
            Some(Key::new(13))
        );
        assert_eq!(
            SCHEMA.find_in_group("setting", "cpu-mode-2", "label"),
            Some(Key::new(14))
        );
        assert_eq!(
            SCHEMA.find_in_group("setting", "cpu-mode.fast", "label"),
            Some(Key::new(15))
        );
        assert_eq!(
            SCHEMA.find_in_group("setting.cpu-mode", "fast", "label"),
            Some(Key::new(15))
        );
        assert_eq!(SCHEMA.find_in_group("setting", "cpu", "label"), None);
        assert_eq!(SCHEMA.find_in_group("setting", "cpu-mode-", "label"), None);
        assert_eq!(
            SCHEMA.find_in_group("setting", "cpu-mode-2-", "label"),
            None
        );
        assert_eq!(SCHEMA.find_in_group("setting", "cpu-mode", "missing"), None);
        assert_eq!(SCHEMA.find_in_group("settings", "cpu-mode", "label"), None);
        assert_eq!(SCHEMA.find_in_group("setting", "", "cpu-mode.label"), None);
        assert_eq!(SCHEMA.find_in_group("choice", "aspect", "a"), None);
        assert_eq!(SCHEMA.find_in_group("common", "", "cancel"), None);
        assert_eq!(SCHEMA.find_in_group("", "", ""), None);
        assert_eq!(SCHEMA.find_in_group("a", "a", "a"), None);
        assert_eq!(SCHEMA.find_in_group("zzz", "a", "a"), None);
        for (index, info) in KEYS.iter().enumerate() {
            let Some((group, name)) = info.section().split_once('.') else {
                continue;
            };
            assert_eq!(
                SCHEMA
                    .find_in_group(group, name, info.key())
                    .map(Key::index),
                Some(index)
            );
        }
    }

    #[test]
    fn find_allocates_nothing() {
        let (allocations, found) = crate::allocation_counter::count_allocations(|| {
            [
                SCHEMA.find("setting.cpu-mode", "label"),
                SCHEMA.find("manager", "images"),
                SCHEMA.find("manager", "missing"),
                SCHEMA.find_in_group("setting", "cpu-mode", "label"),
                SCHEMA.find_in_group("setting", "cpu-mode.fast", "label"),
                SCHEMA.find_in_group("setting", "cpu", "label"),
            ]
        });
        assert_eq!(allocations, 0);
        assert_eq!(
            found,
            [
                Some(Key::new(13)),
                Some(Key::new(12)),
                None,
                Some(Key::new(13)),
                Some(Key::new(15)),
                None
            ]
        );
    }

    #[test]
    fn find_works_in_a_constant() {
        /// The key `common.save`, found at compile time.
        const SAVE: Option<Key> = SCHEMA.find("common", "save");
        /// The key `setting.cpu-mode.label`, found at compile time.
        const LABEL: Option<Key> = SCHEMA.find_in_group("setting", "cpu-mode", "label");
        assert_eq!(SAVE, Some(Key::new(5)));
        assert_eq!(LABEL, Some(Key::new(13)));
    }

    #[test]
    fn list_patterns_resolve_in_their_order() {
        for (pattern, name) in LIST_PATTERN_KEYS.iter().enumerate() {
            assert_eq!(SCHEMA.info(SCHEMA.list_key(pattern)).key(), *name);
        }
    }

    #[test]
    fn key_infos_hold_their_placeholders() {
        let info = SCHEMA.info(Key::new(12));
        assert!(info.is_plural());
        assert_eq!(info.placeholder_index("folder"), Some(0));
        assert_eq!(info.placeholder_index("count"), None);
        assert_eq!(SCHEMA.len(), 16);
        assert!(!SCHEMA.is_empty());
    }

    #[test]
    #[should_panic(expected = "out of order")]
    fn unsorted_keys_panic() {
        /// Keys with the last two out of order.
        static UNSORTED: [KeyInfo; 8] = [
            LIST_KEYS[0],
            LIST_KEYS[1],
            LIST_KEYS[2],
            LIST_KEYS[3],
            LIST_KEYS[4],
            LIST_KEYS[5],
            KeyInfo::new("x", "b", &[], false),
            KeyInfo::new("x", "a", &[], false),
        ];
        let _ = Schema::new(&UNSORTED);
    }

    #[test]
    #[should_panic(expected = "list pattern is missing")]
    fn missing_list_patterns_panic() {
        /// Keys without the list patterns.
        static KEYS_WITHOUT_LIST: [KeyInfo; 1] = [KeyInfo::new("x", "a", &[], false)];
        let _ = Schema::new(&KEYS_WITHOUT_LIST);
    }

    #[test]
    fn comparison_is_lexicographic() {
        assert_eq!(compare_bytes(b"a", b"b"), Ordering::Less);
        assert_eq!(compare_bytes(b"ab", b"a"), Ordering::Greater);
        assert_eq!(compare_bytes(b"", b""), Ordering::Equal);
        assert_eq!(compare_full_keys("a", "z", "b", "a"), Ordering::Less);
        assert_eq!(
            compare_with_parts(b"a.b", [b"a", b".", b"b"]),
            Ordering::Equal
        );
        assert_eq!(
            compare_with_parts(b"a.b", [b"", b"", b"a.b"]),
            Ordering::Equal
        );
        assert_eq!(
            compare_with_parts(b"a.b", [b"a", b".", b"bc"]),
            Ordering::Less
        );
        assert_eq!(
            compare_with_parts(b"a.bc", [b"a", b".", b"b"]),
            Ordering::Greater
        );
        assert_eq!(
            compare_with_parts(b"a.a", [b"a", b".", b"b"]),
            Ordering::Less
        );
        assert_eq!(compare_with_parts(b"", [b"", b"", b""]), Ordering::Equal);
        assert_eq!(compare_with_parts(b"", [b"", b".", b""]), Ordering::Less);
    }
}
