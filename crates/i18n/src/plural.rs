//! The plural rules and the plural categories of a language.

/// The plural category of a count. A plural text holds one form for each category of its rule.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PluralCategory {
    /// The singular form: the count 1, and also 0 under the rule `one-below-two`.
    One,
    /// The form for every other count.
    Other,
}

impl PluralCategory {
    /// The number of plural categories.
    pub const COUNT: usize = 2;

    /// Every plural category in the order of [`PluralCategory::index`].
    pub const ALL: [PluralCategory; Self::COUNT] = [PluralCategory::One, PluralCategory::Other];

    /// Returns the index of the category in [`PluralCategory::ALL`].
    pub const fn index(self) -> usize {
        match self {
            PluralCategory::One => 0,
            PluralCategory::Other => 1,
        }
    }

    /// Returns the name of the category, the last part of a plural key.
    pub const fn name(self) -> &'static str {
        match self {
            PluralCategory::One => "one",
            PluralCategory::Other => "other",
        }
    }

    /// Returns the category with the given name.
    pub fn from_name(name: &str) -> Option<PluralCategory> {
        PluralCategory::ALL
            .into_iter()
            .find(|category| category.name() == name)
    }
}

/// The plural rule of a language. It selects the plural category of a count.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PluralRule {
    /// A count of one takes `one`. Every other count takes `other`.
    OneOther,
    /// The counts 0 and 1 take `one`. Every other count takes `other`.
    OneBelowTwo,
    /// Every count takes `other`.
    None,
}

impl PluralRule {
    /// Every plural rule.
    pub const ALL: [PluralRule; 3] = [
        PluralRule::OneOther,
        PluralRule::OneBelowTwo,
        PluralRule::None,
    ];

    /// Returns the name of the rule in the `plural` header of a language file.
    pub const fn name(self) -> &'static str {
        match self {
            PluralRule::OneOther => "one-other",
            PluralRule::OneBelowTwo => "one-below-two",
            PluralRule::None => "none",
        }
    }

    /// Returns the rule with the given name.
    pub fn from_name(name: &str) -> Option<PluralRule> {
        PluralRule::ALL.into_iter().find(|rule| rule.name() == name)
    }

    /// Returns the categories that a plural text of this rule holds.
    pub const fn categories(self) -> &'static [PluralCategory] {
        match self {
            PluralRule::OneOther | PluralRule::OneBelowTwo => {
                &[PluralCategory::One, PluralCategory::Other]
            }
            PluralRule::None => &[PluralCategory::Other],
        }
    }

    /// Returns true when a plural text of this rule holds the category.
    pub const fn has(self, category: PluralCategory) -> bool {
        match self {
            PluralRule::OneOther | PluralRule::OneBelowTwo => true,
            PluralRule::None => matches!(category, PluralCategory::Other),
        }
    }

    /// Returns the category of a count.
    pub const fn category(self, count: u64) -> PluralCategory {
        match self {
            PluralRule::OneOther if count == 1 => PluralCategory::One,
            PluralRule::OneBelowTwo if count < 2 => PluralCategory::One,
            PluralRule::OneOther | PluralRule::OneBelowTwo | PluralRule::None => {
                PluralCategory::Other
            }
        }
    }
}

/// Splits a key into its base key and the plural category of its last part.
///
/// The split applies to the last `.`-part of the key. A key whose last part after a `.` is the
/// name of a category is a plural form: `images.one` gives `images` and `one`. Every other key
/// gives itself and no category. A key without a `.`, such as the key `one`, gives
/// itself.
pub(crate) fn split_plural_key(key: &str) -> (&str, Option<PluralCategory>) {
    if let Some((base, last)) = key.rsplit_once('.')
        && let Some(category) = PluralCategory::from_name(last)
    {
        return (base, Some(category));
    }
    (key, None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_other_selects_one_only_for_a_single_item() {
        assert_eq!(PluralRule::OneOther.category(0), PluralCategory::Other);
        assert_eq!(PluralRule::OneOther.category(1), PluralCategory::One);
        assert_eq!(PluralRule::OneOther.category(2), PluralCategory::Other);
        assert_eq!(
            PluralRule::OneOther.category(u64::MAX),
            PluralCategory::Other
        );
    }

    #[test]
    fn one_below_two_selects_one_for_zero_and_one() {
        assert_eq!(PluralRule::OneBelowTwo.category(0), PluralCategory::One);
        assert_eq!(PluralRule::OneBelowTwo.category(1), PluralCategory::One);
        assert_eq!(PluralRule::OneBelowTwo.category(2), PluralCategory::Other);
        assert_eq!(PluralRule::OneBelowTwo.category(11), PluralCategory::Other);
        assert_eq!(
            PluralRule::OneBelowTwo.category(u64::MAX),
            PluralCategory::Other
        );
    }

    #[test]
    fn none_always_selects_other() {
        for count in [0, 1, 2, 11, u64::MAX] {
            assert_eq!(PluralRule::None.category(count), PluralCategory::Other);
        }
    }

    #[test]
    fn rules_list_their_categories() {
        assert_eq!(
            PluralRule::OneOther.categories(),
            &[PluralCategory::One, PluralCategory::Other]
        );
        assert_eq!(PluralRule::None.categories(), &[PluralCategory::Other]);
        assert_eq!(
            PluralRule::OneBelowTwo.categories(),
            &[PluralCategory::One, PluralCategory::Other]
        );
        assert!(PluralRule::OneOther.has(PluralCategory::One));
        assert!(PluralRule::OneBelowTwo.has(PluralCategory::One));
        assert!(PluralRule::OneBelowTwo.has(PluralCategory::Other));
        assert!(!PluralRule::None.has(PluralCategory::One));
        assert!(PluralRule::None.has(PluralCategory::Other));
    }

    #[test]
    fn names_round_trip() {
        for rule in PluralRule::ALL {
            assert_eq!(PluralRule::from_name(rule.name()), Some(rule));
        }
        for category in PluralCategory::ALL {
            assert_eq!(PluralCategory::from_name(category.name()), Some(category));
            assert_eq!(PluralCategory::ALL[category.index()], category);
        }
        assert_eq!(
            PluralRule::from_name("one-below-two"),
            Some(PluralRule::OneBelowTwo)
        );
        assert_eq!(PluralRule::from_name("one_other"), None);
        assert_eq!(PluralCategory::from_name("few"), None);
    }

    #[test]
    fn plural_keys_split_at_the_last_part() {
        assert_eq!(
            split_plural_key("images.one"),
            ("images", Some(PluralCategory::One))
        );
        assert_eq!(
            split_plural_key("a.b.other"),
            ("a.b", Some(PluralCategory::Other))
        );
        assert_eq!(split_plural_key("other"), ("other", None));
        assert_eq!(split_plural_key("one"), ("one", None));
        assert_eq!(split_plural_key("4:3"), ("4:3", None));
        assert_eq!(split_plural_key("1.5"), ("1.5", None));
        assert_eq!(split_plural_key("images.many"), ("images.many", None));
        assert_eq!(split_plural_key("cancel"), ("cancel", None));
    }
}
