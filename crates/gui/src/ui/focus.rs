//! The navigation events and the focus ring.

use crate::Rect;

/// A navigation event of the keyboard or a gamepad.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Navigation {
    /// An arrow or a direction pad press upwards.
    Up,
    /// An arrow or a direction pad press downwards.
    Down,
    /// An arrow or a direction pad press to the left.
    Left,
    /// An arrow or a direction pad press to the right.
    Right,
    /// The page up key.
    PageUp,
    /// The page down key.
    PageDown,
    /// The home key.
    Home,
    /// The end key.
    End,
    /// The tab key. The focus goes to the next widget of the ring.
    Next,
    /// The tab key with shift. The focus goes to the previous widget of the
    /// ring.
    Previous,
    /// A left shoulder button. A tab bar selects its previous tab, whatever
    /// widget holds the focus.
    PreviousTab,
    /// A right shoulder button. A tab bar selects its next tab, whatever
    /// widget holds the focus.
    NextTab,
}

impl Navigation {
    /// The bit of the event in a [`NavigationSet`].
    const fn bit(self) -> u16 {
        1 << self as u16
    }
}

/// A set of navigation events, for the events a focused widget handles
/// itself.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct NavigationSet(u16);

impl NavigationSet {
    /// No events.
    pub const EMPTY: Self = Self(0);

    /// The set of `navigations`.
    pub const fn of(navigations: &[Navigation]) -> Self {
        let mut bits = 0;
        let mut index = 0;
        while index < navigations.len() {
            bits |= navigations[index].bit();
            index += 1;
        }
        Self(bits)
    }

    /// The set with `navigation` added when `condition` holds.
    pub const fn with_if(self, condition: bool, navigation: Navigation) -> Self {
        if condition {
            Self(self.0 | navigation.bit())
        } else {
            self
        }
    }

    /// Whether the set holds `navigation`.
    pub const fn contains(self, navigation: Navigation) -> bool {
        self.0 & navigation.bit() != 0
    }
}

/// A focusable widget of a pass.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Focusable {
    /// The id of the widget.
    pub(crate) id: super::UiId,
    /// The area of the widget in logical units.
    pub(crate) rect: Rect,
    /// The events the widget handles itself while it holds the focus.
    pub(crate) captures: NavigationSet,
    /// Whether the widget takes typed text.
    pub(crate) text: bool,
    /// Whether the widget takes cancel while it holds the focus.
    pub(crate) takes_cancel: bool,
    /// Whether a directional move picks the widget over a neighbor at the
    /// same cost, as for the main action of a panel.
    pub(crate) preferred: bool,
}

/// The weight of the distance across the direction of a move against the
/// distance along it.
const CROSS_WEIGHT: f32 = 2.0;

/// The index of the widget of `ring` that takes the focus on `navigation`.
/// `current` is the focused widget in the ring and `from` the area the move
/// starts from: the area of the focused widget, or the last area of a
/// focused widget that left the ring. Without an area the first widget of
/// the ring takes the focus, or the last one on a move backwards. `None`
/// keeps the focus where it is.
pub(crate) fn next_focus(
    ring: &[Focusable],
    current: Option<usize>,
    from: Option<Rect>,
    navigation: Navigation,
) -> Option<usize> {
    let count = ring.len();
    if count == 0 {
        return None;
    }
    let directional = matches!(
        navigation,
        Navigation::Up | Navigation::Down | Navigation::Left | Navigation::Right
    );
    match (current, from) {
        (Some(current), _) if navigation == Navigation::Next => Some((current + 1) % count),
        (Some(current), _) if navigation == Navigation::Previous => {
            Some((current + count - 1) % count)
        }
        (_, Some(from)) if directional => ring
            .iter()
            .enumerate()
            .filter(|&(index, _)| Some(index) != current)
            .filter_map(|(index, entry)| {
                directional_cost(from, entry.rect, navigation).map(|cost| (index, entry, cost))
            })
            .min_by(|(_, first, first_cost), (_, second, second_cost)| {
                first_cost
                    .gap
                    .total_cmp(&second_cost.gap)
                    .then(second.preferred.cmp(&first.preferred))
                    .then(
                        first_cost
                            .center_offset
                            .total_cmp(&second_cost.center_offset),
                    )
            })
            .map(|(index, _, _)| index),
        (Some(_), _) => None,
        (None, _) => match navigation {
            Navigation::Up | Navigation::Left | Navigation::Previous | Navigation::End => {
                Some(count - 1)
            }
            Navigation::PreviousTab | Navigation::NextTab => None,
            _ => Some(0),
        },
    }
}

/// The cost of a directional move.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Cost {
    /// The gap along the direction plus the weighted distance across it.
    gap: f32,
    /// The distance between the centers across the direction.
    center_offset: f32,
}

/// The cost of a move from `from` to `to` in the direction `navigation`, or
/// `None` when `to` lies elsewhere. Widgets that overlap across the
/// direction cost only their gap.
fn directional_cost(from: Rect, to: Rect, navigation: Navigation) -> Option<Cost> {
    let flip = |rect: Rect| Rect::new(-rect.right(), -rect.bottom(), rect.width, rect.height);
    let transpose = |rect: Rect| Rect::new(rect.y, rect.x, rect.height, rect.width);
    // Every direction maps onto a move downwards.
    let (from, to) = match navigation {
        Navigation::Down => (from, to),
        Navigation::Up => (flip(from), flip(to)),
        Navigation::Right => (transpose(from), transpose(to)),
        Navigation::Left => (flip(transpose(from)), flip(transpose(to))),
        _ => return None,
    };
    let (_, from_center) = from.center();
    let (_, to_center) = to.center();
    if to_center <= from_center + 0.5 || to.bottom() <= from.bottom() + 0.5 {
        return None;
    }
    let gap = (to.y - from.bottom()).max(0.0);
    let cross = (to.x - from.right()).max(from.x - to.right()).max(0.0);
    Some(Cost {
        gap: gap + cross * CROSS_WEIGHT,
        center_offset: (to.center().0 - from.center().0).abs(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::ui_id_indexed;

    /// A ring of the rectangles `rects` in their order.
    fn ring(rects: &[Rect]) -> Vec<Focusable> {
        rects
            .iter()
            .enumerate()
            .map(|(index, &rect)| Focusable {
                id: ui_id_indexed("widget", index),
                rect,
                captures: NavigationSet::EMPTY,
                text: false,
                takes_cancel: false,
                preferred: false,
            })
            .collect()
    }

    /// The widget that takes the focus from the focused widget `current`.
    fn step(ring: &[Focusable], current: Option<usize>, navigation: Navigation) -> Option<usize> {
        next_focus(
            ring,
            current,
            current.map(|index| ring[index].rect),
            navigation,
        )
    }

    /// The layout of a dialog: a tab bar, a field, a list and two buttons.
    fn dialog() -> Vec<Focusable> {
        ring(&[
            Rect::new(10.0, 10.0, 300.0, 30.0),
            Rect::new(10.0, 50.0, 300.0, 30.0),
            Rect::new(10.0, 90.0, 300.0, 200.0),
            Rect::new(150.0, 300.0, 70.0, 30.0),
            Rect::new(230.0, 300.0, 80.0, 30.0),
            Rect::new(330.0, 10.0, 30.0, 30.0),
        ])
    }

    #[test]
    fn sets_hold_their_events() {
        let set = NavigationSet::of(&[Navigation::Up, Navigation::End]);
        assert!(set.contains(Navigation::Up));
        assert!(set.contains(Navigation::End));
        assert!(!set.contains(Navigation::Down));
        let set = set
            .with_if(true, Navigation::Down)
            .with_if(false, Navigation::Left);
        assert!(set.contains(Navigation::Down));
        assert!(!set.contains(Navigation::Left));
        assert!(!NavigationSet::EMPTY.contains(Navigation::NextTab));
    }

    #[test]
    fn tab_walks_the_ring_and_wraps() {
        let ring = dialog();
        assert_eq!(step(&ring, Some(0), Navigation::Next), Some(1));
        assert_eq!(step(&ring, Some(5), Navigation::Next), Some(0));
        assert_eq!(step(&ring, Some(0), Navigation::Previous), Some(5));
        assert_eq!(step(&ring, None, Navigation::Next), Some(0));
        assert_eq!(step(&ring, None, Navigation::Previous), Some(5));
        assert_eq!(step(&[], None, Navigation::Next), None);
    }

    #[test]
    fn arrows_take_the_nearest_widget_in_their_direction() {
        let ring = dialog();
        assert_eq!(step(&ring, Some(0), Navigation::Down), Some(1));
        assert_eq!(step(&ring, Some(1), Navigation::Down), Some(2));
        assert_eq!(step(&ring, Some(2), Navigation::Down), Some(3));
        assert_eq!(step(&ring, Some(3), Navigation::Right), Some(4));
        assert_eq!(step(&ring, Some(4), Navigation::Left), Some(3));
        assert_eq!(step(&ring, Some(4), Navigation::Up), Some(2));
        assert_eq!(step(&ring, Some(2), Navigation::Up), Some(1));
        assert_eq!(step(&ring, Some(0), Navigation::Right), Some(5));
        assert_eq!(step(&ring, Some(5), Navigation::Left), Some(0));
        assert_eq!(step(&ring, Some(5), Navigation::Down), Some(1));
    }

    #[test]
    fn a_preferred_widget_wins_a_tie() {
        let mut ring = dialog();
        assert_eq!(step(&ring, Some(2), Navigation::Down), Some(3));
        ring[4].preferred = true;
        assert_eq!(step(&ring, Some(2), Navigation::Down), Some(4));
        assert_eq!(step(&ring, Some(4), Navigation::Left), Some(3));
    }

    #[test]
    fn a_move_starts_from_the_area_of_a_widget_that_left() {
        let ring = dialog();
        let removed = Rect::new(10.0, 82.0, 300.0, 6.0);
        assert_eq!(
            next_focus(&ring, None, Some(removed), Navigation::Down),
            Some(2)
        );
        assert_eq!(
            next_focus(&ring, None, Some(removed), Navigation::Up),
            Some(1)
        );
        assert_eq!(next_focus(&ring, None, None, Navigation::Down), Some(0));
    }

    #[test]
    fn a_move_past_the_edge_keeps_the_focus() {
        let ring = dialog();
        assert_eq!(step(&ring, Some(0), Navigation::Up), None);
        assert_eq!(step(&ring, Some(3), Navigation::Down), None);
        assert_eq!(step(&ring, Some(0), Navigation::Left), None);
        assert_eq!(step(&ring, Some(2), Navigation::PageDown), None);
    }
}
