/// The keys of the reference language file in the order of their `Key`.
static KEYS: [::i18n::KeyInfo; 18] = [
    ::i18n::KeyInfo::new("choice.cpu-mode", "high", &[], false),
    ::i18n::KeyInfo::new("choice.cpu-mode", "low", &[], false),
    ::i18n::KeyInfo::new("common", "cancel", &[], false),
    ::i18n::KeyInfo::new("common", "quote", &[], false),
    ::i18n::KeyInfo::new("common", "save", &[], false),
    ::i18n::KeyInfo::new("list", "and-end", &["first", "second"], false),
    ::i18n::KeyInfo::new("list", "and-middle", &["first", "second"], false),
    ::i18n::KeyInfo::new("list", "and-pair", &["first", "second"], false),
    ::i18n::KeyInfo::new("list", "or-end", &["first", "second"], false),
    ::i18n::KeyInfo::new("list", "or-middle", &["first", "second"], false),
    ::i18n::KeyInfo::new("list", "or-pair", &["first", "second"], false),
    ::i18n::KeyInfo::new("manager", "delete-question", &["name"], false),
    ::i18n::KeyInfo::new("manager", "files", &["folder"], true),
    ::i18n::KeyInfo::new("manager", "filter-placeholder", &[], false),
    ::i18n::KeyInfo::new("manager", "full-name", &["first-name", "last-name"], false),
    ::i18n::KeyInfo::new("manager", "images", &[], true),
    ::i18n::KeyInfo::new("setting.cpu-mode", "help", &[], false),
    ::i18n::KeyInfo::new("setting.cpu-mode", "label", &[], false),
];

/// The schema of the reference language file.
pub static SCHEMA: ::i18n::Schema = ::i18n::Schema::new(&KEYS);

/// The texts of the section `[common]`.
pub mod common {
    /// The text `common.cancel`.
    ///
    /// ```text
    /// Cancel
    /// ```
    pub const CANCEL: ::i18n::Key = ::i18n::Key::new(2);

    /// The text `common.quote`.
    ///
    /// ```text
    /// Press `Enter` {{now}}
    /// ```
    pub const QUOTE: ::i18n::Key = ::i18n::Key::new(3);

    /// The text `common.save`.
    ///
    /// ```text
    /// Save
    /// ```
    pub const SAVE: ::i18n::Key = ::i18n::Key::new(4);
}

/// The texts of the section `[manager]`.
pub mod manager {
    /// The text `manager.delete-question`.
    ///
    /// ```text
    /// Delete {name}?
    /// ```
    pub fn delete_question(name: &dyn ::core::fmt::Display) -> ::i18n::Message<'_, 1> {
        ::i18n::Message::new(::i18n::Key::new(11), [name])
    }

    /// The plural text `manager.files`.
    ///
    /// ```text
    /// one = One file in {folder}
    /// other = {count} files in {folder}
    /// ```
    pub fn files(count: u64, folder: &dyn ::core::fmt::Display) -> ::i18n::Message<'_, 1> {
        ::i18n::Message::plural(::i18n::Key::new(12), count, [folder])
    }

    /// The text `manager.filter-placeholder`.
    ///
    /// ```text
    /// Filter games
    /// ```
    pub const FILTER_PLACEHOLDER: ::i18n::Key = ::i18n::Key::new(13);

    /// The text `manager.full-name`.
    ///
    /// ```text
    /// {first-name} {last-name}
    /// ```
    pub fn full_name<'a>(first_name: &'a dyn ::core::fmt::Display, last_name: &'a dyn ::core::fmt::Display) -> ::i18n::Message<'a, 2> {
        ::i18n::Message::new(::i18n::Key::new(14), [first_name, last_name])
    }

    /// The plural text `manager.images`.
    ///
    /// ```text
    /// one = One image
    /// other = {count} images
    /// ```
    pub fn images(count: u64) -> ::i18n::Message<'static, 0> {
        ::i18n::Message::plural(::i18n::Key::new(15), count, [])
    }
}
