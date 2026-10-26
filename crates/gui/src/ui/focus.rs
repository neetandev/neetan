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

/// The most scroll areas around a widget, the innermost area included. A
/// scroll area nests at most this deep. A debug build asserts the limit, and
/// a release build keeps the innermost areas of a deeper nesting.
const SCROLL_NESTING: usize = 4;

/// A scroll area of the chain of areas around a focusable widget.
#[derive(Clone, Copy, Debug, PartialEq)]
struct ScrollLevel {
    /// The id of the scroll area.
    id: super::UiId,
    /// The area of the scroll area in logical units.
    rect: Rect,
}

/// The scroll area that holds a focusable widget.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ScrollAnchor {
    /// The id of the scroll area.
    pub(crate) id: super::UiId,
    /// The top of the content of the area in logical units, in the pass of
    /// the widget.
    pub(crate) origin: f32,
    /// The area of the scroll area in logical units.
    rect: Rect,
    /// The visible part of the area in logical units: the area inside the
    /// visible parts of the scroll areas around it.
    view: Rect,
    /// The scroll areas around the area, the innermost first.
    outer: [Option<ScrollLevel>; SCROLL_NESTING - 1],
}

impl ScrollAnchor {
    /// The scroll area `id` at `rect` with the top of its content at
    /// `origin`, inside the scroll area `outer`.
    pub(crate) fn inside(
        id: super::UiId,
        origin: f32,
        rect: Rect,
        outer: Option<ScrollAnchor>,
    ) -> Self {
        debug_assert!(
            outer.is_none_or(|outer| outer.outer.iter().any(Option::is_none)),
            "scroll areas nest at most {SCROLL_NESTING} deep"
        );
        let mut chain = [None; SCROLL_NESTING - 1];
        if let Some(outer) = outer {
            for (slot, level) in chain.iter_mut().zip(outer.levels()) {
                *slot = Some(level);
            }
        }
        Self {
            id,
            origin,
            rect,
            view: outer.map_or(rect, |outer| clamp_into(rect, outer.view)),
            outer: chain,
        }
    }

    /// The area and the scroll areas around it, the innermost first.
    fn levels(&self) -> impl Iterator<Item = ScrollLevel> + '_ {
        std::iter::once(ScrollLevel {
            id: self.id,
            rect: self.rect,
        })
        .chain(self.outer.iter().flatten().copied())
    }

    /// Whether `area` is the area or one of the scroll areas around it.
    fn contains(&self, area: super::UiId) -> bool {
        self.levels().any(|level| level.id == area)
    }

    /// The intersection of the areas of the chain for which `shared` gives
    /// `false`, or `None` when `shared` gives `true` for every area of the
    /// chain.
    fn region_outside(&self, shared: impl Fn(super::UiId) -> bool) -> Option<Rect> {
        self.levels()
            .filter(|level| !shared(level.id))
            .map(|level| level.rect)
            .reduce(clamp_into)
    }
}

/// The area that a directional move starts from and the scroll area that
/// holds it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct MoveStart {
    /// The area in logical units.
    rect: Rect,
    /// The innermost scroll area around the area.
    scroll: Option<ScrollAnchor>,
}

impl MoveStart {
    /// The start of a move from the widget `entry`.
    pub(crate) fn of(entry: &Focusable) -> Self {
        Self {
            rect: entry.rect,
            scroll: entry.scroll,
        }
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
    /// The layer the widget draws in. The base of the pass is layer 0, and
    /// every modal or popup above it adds one.
    pub(crate) layer: u32,
    /// The innermost scroll area around the widget.
    pub(crate) scroll: Option<ScrollAnchor>,
    /// Whether a modal that opens may give the widget the focus.
    pub(crate) default_choice: bool,
}

impl Focusable {
    /// A widget `id` at `rect` in the base layer that captures `captures`
    /// and takes neither text nor cancel.
    pub(crate) fn new(id: super::UiId, rect: Rect, captures: NavigationSet) -> Self {
        Self {
            id,
            rect,
            captures,
            text: false,
            takes_cancel: false,
            preferred: false,
            layer: 0,
            scroll: None,
            default_choice: true,
        }
    }
}

/// The weight of the distance across the direction of a move against the
/// distance along it.
const CROSS_WEIGHT: f32 = 2.0;

/// The index of the widget of `ring` that takes the focus on `navigation`.
/// `current` is the focused widget in the ring and `from` the start of the
/// move: the focused widget, or the last area of a focused widget that left
/// the ring. Without a start the first widget of the ring takes the focus,
/// or the last one on a move backwards. `None` keeps the focus where it is.
///
/// A scroll area groups the arrows, see [`directional_focus`].
pub(crate) fn next_focus(
    ring: &[Focusable],
    current: Option<usize>,
    from: Option<MoveStart>,
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
        (_, Some(from)) if directional => directional_focus(ring, current, from, navigation),
        (Some(_), _) => None,
        (None, _) => match navigation {
            Navigation::Up | Navigation::Left | Navigation::Previous | Navigation::End => {
                Some(count - 1)
            }
            Navigation::PreviousTab | Navigation::NextTab => None,
            Navigation::Down
            | Navigation::Right
            | Navigation::Next
            | Navigation::Home
            | Navigation::PageUp
            | Navigation::PageDown => Some(0),
        },
    }
}

/// The index of the widget of `ring` that takes the focus on the arrow
/// `navigation` from `from`, other than `current`.
///
/// A scroll area is a group of the arrows. A widget belongs to the group of
/// every scroll area around it, the outer areas of a nested area included.
/// Up or Down from a widget of a scroll area takes the nearest widget of the
/// innermost area of the widget first, also one outside its visible part,
/// then the nearest widget of each area further out. Left and Right take the
/// nearest widget of the outermost area of the widget. Inside a group the
/// start counts by its whole area, and a widget of a nested area counts by
/// its part inside the nested area. The arrow enters a nested area at its
/// nearest widget with a part inside the nested area. Only an arrow without
/// a widget of the group leaves the outermost area.
///
/// An arrow that leaves an area starts from the visible part of the widget
/// and measures every widget outside the group by its visible part. A widget
/// outside the visible part of its area counts as a line on the nearest edge
/// of that part, and of two such widgets at the same cost the one nearer to
/// the visible part wins. An arrow that reaches a widget of a scroll area
/// outside the chain of the start enters that area at its nearest widget
/// with a visible part in the direction, or else at the reached widget.
fn directional_focus(
    ring: &[Focusable],
    current: Option<usize>,
    from: MoveStart,
    navigation: Navigation,
) -> Option<usize> {
    let shared = |area: super::UiId| from.scroll.is_some_and(|anchor| anchor.contains(area));
    let vertical = matches!(navigation, Navigation::Up | Navigation::Down);
    let outermost = from.scroll.and_then(|anchor| anchor.levels().last());
    let groups = from
        .scroll
        .iter()
        .flat_map(ScrollAnchor::levels)
        .filter(|level| vertical || outermost.is_some_and(|outermost| outermost.id == level.id));
    for group in groups {
        let inside = nearest(ring, current, from.rect, navigation, |entry| {
            let anchor = entry.scroll.filter(|anchor| anchor.contains(group.id))?;
            Some(
                anchor
                    .region_outside(shared)
                    .map_or(Placed::shown(entry.rect), |region| {
                        Placed::within(entry.rect, region)
                    }),
            )
        });
        if let Some(inside) = inside {
            return Some(enter(
                ring,
                current,
                from.rect,
                navigation,
                inside,
                shared,
                |anchor| anchor.region_outside(shared),
            ));
        }
    }
    let start = from
        .scroll
        .map_or(from.rect, |anchor| clamp_into(from.rect, anchor.view));
    let reached = nearest(ring, current, start, navigation, |entry| {
        let grouped = outermost
            .is_some_and(|level| entry.scroll.is_some_and(|anchor| anchor.contains(level.id)));
        (!grouped).then(|| Placed::of(entry))
    })?;
    Some(enter(
        ring,
        current,
        start,
        navigation,
        reached,
        shared,
        |anchor| Some(anchor.view),
    ))
}

/// The widget of `ring` that takes the focus when a move from `start` in the
/// direction `navigation` reaches the widget `reached`. A move that reaches
/// a widget inside a scroll area for which `shared` gives `false` enters the
/// outermost such area at its nearest widget with a visible part in the
/// direction, or else at `reached`. `region` maps the scroll area of a
/// widget to the rectangle inside which a part of the widget counts as
/// shown: the nested areas of the widget inside a group, or the visible part
/// of its area.
fn enter(
    ring: &[Focusable],
    current: Option<usize>,
    start: Rect,
    navigation: Navigation,
    reached: usize,
    shared: impl Fn(super::UiId) -> bool,
    region: impl Fn(&ScrollAnchor) -> Option<Rect>,
) -> usize {
    let Some(entered) = ring[reached]
        .scroll
        .and_then(|anchor| anchor.levels().filter(|level| !shared(level.id)).last())
    else {
        return reached;
    };
    nearest(ring, current, start, navigation, |entry| {
        let anchor = entry.scroll.filter(|anchor| anchor.contains(entered.id))?;
        visible_part(entry.rect, region(&anchor)?).map(Placed::shown)
    })
    .unwrap_or(reached)
}

/// The area of a widget in a directional move and how far the widget lies
/// outside the visible part of its scroll area. Inside a group the distance
/// is measured to the region of the nested areas of the widget, see
/// [`ScrollAnchor::region_outside`].
#[derive(Clone, Copy, Debug, PartialEq)]
struct Placed {
    /// The area the move measures.
    rect: Rect,
    /// The distance from the widget to the visible part of its scroll area,
    /// or inside a group to the region of its nested areas. A widget with a
    /// part inside has none.
    hidden: f32,
}

impl Placed {
    /// The area `rect` of a widget that shows.
    fn shown(rect: Rect) -> Self {
        Self { rect, hidden: 0.0 }
    }

    /// The visible part of the widget `entry`, or the nearest edge of the
    /// visible part of its scroll area for a hidden widget.
    fn of(entry: &Focusable) -> Self {
        entry.scroll.map_or(Self::shown(entry.rect), |anchor| {
            Self::within(entry.rect, anchor.view)
        })
    }

    /// The part of the area `rect` inside `view`, or the nearest edge of
    /// `view` for an area outside it.
    fn within(rect: Rect, view: Rect) -> Self {
        let across = (view.x - rect.right()).max(rect.x - view.right()).max(0.0);
        let along = (view.y - rect.bottom())
            .max(rect.y - view.bottom())
            .max(0.0);
        Self {
            rect: clamp_into(rect, view),
            hidden: across.max(along),
        }
    }
}

/// The index of the widget of `ring` with the lowest cost of a move from
/// `from` in the direction `navigation`, other than `current`. `place`
/// places a widget that takes part in the move, or gives `None` for a widget
/// that takes no part.
fn nearest(
    ring: &[Focusable],
    current: Option<usize>,
    from: Rect,
    navigation: Navigation,
    place: impl Fn(&Focusable) -> Option<Placed>,
) -> Option<usize> {
    ring.iter()
        .enumerate()
        .filter(|&(index, _)| Some(index) != current)
        .filter_map(|(index, entry)| {
            let placed = place(entry)?;
            let cost = directional_cost(from, placed.rect, navigation)?;
            Some((index, entry, cost, placed.hidden))
        })
        .min_by(
            |(_, first, first_cost, first_hidden), (_, second, second_cost, second_hidden)| {
                first_cost
                    .gap
                    .total_cmp(&second_cost.gap)
                    .then(first_hidden.total_cmp(second_hidden))
                    .then(second.preferred.cmp(&first.preferred))
                    .then(
                        first_cost
                            .center_offset
                            .total_cmp(&second_cost.center_offset),
                    )
            },
        )
        .map(|(index, _, _, _)| index)
}

/// `rect` with each edge clamped into `view`: the part of `rect` inside
/// `view`, or a rectangle without width or height on the edge of `view`
/// nearest to `rect` when they do not overlap.
fn clamp_into(rect: Rect, view: Rect) -> Rect {
    let left = rect.x.clamp(view.x, view.right().max(view.x));
    let right = rect.right().clamp(view.x, view.right().max(view.x));
    let top = rect.y.clamp(view.y, view.bottom().max(view.y));
    let bottom = rect.bottom().clamp(view.y, view.bottom().max(view.y));
    Rect::new(left, top, right - left, bottom - top)
}

/// The part of `rect` inside `view`, or `None` when nothing of `rect` lies
/// inside `view`.
fn visible_part(rect: Rect, view: Rect) -> Option<Rect> {
    let part = clamp_into(rect, view);
    (part.width > 0.0 && part.height > 0.0).then_some(part)
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
/// direction cost only their gap. Left and Right take only a widget that
/// overlaps `from` across the direction, a widget of the same row.
fn directional_cost(from: Rect, to: Rect, navigation: Navigation) -> Option<Cost> {
    let flip = |rect: Rect| Rect::new(-rect.right(), -rect.bottom(), rect.width, rect.height);
    let transpose = |rect: Rect| Rect::new(rect.y, rect.x, rect.height, rect.width);
    // Every direction maps onto a move downwards.
    let (from, to) = match navigation {
        Navigation::Down => (from, to),
        Navigation::Up => (flip(from), flip(to)),
        Navigation::Right => (transpose(from), transpose(to)),
        Navigation::Left => (flip(transpose(from)), flip(transpose(to))),
        Navigation::PageUp
        | Navigation::PageDown
        | Navigation::Home
        | Navigation::End
        | Navigation::Next
        | Navigation::Previous
        | Navigation::PreviousTab
        | Navigation::NextTab => return None,
    };
    let (_, from_center) = from.center();
    let (_, to_center) = to.center();
    if to_center <= from_center + 0.5 || to.bottom() <= from.bottom() + 0.5 {
        return None;
    }
    let sideways = matches!(navigation, Navigation::Left | Navigation::Right);
    let overlap = to.right().min(from.right()) - to.x.max(from.x);
    if sideways && overlap <= 0.5 {
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
    use crate::ui::{ui_id, ui_id_indexed};

    /// A ring of the rectangles `rects` in their order.
    fn ring(rects: &[Rect]) -> Vec<Focusable> {
        rects
            .iter()
            .enumerate()
            .map(|(index, &rect)| {
                Focusable::new(ui_id_indexed("widget", index), rect, NavigationSet::EMPTY)
            })
            .collect()
    }

    /// The widget that takes the focus from the focused widget `current`.
    fn step(ring: &[Focusable], current: Option<usize>, navigation: Navigation) -> Option<usize> {
        next_focus(
            ring,
            current,
            current.map(|index| MoveStart::of(&ring[index])),
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
        let removed = MoveStart {
            rect: Rect::new(10.0, 82.0, 300.0, 6.0),
            scroll: None,
        };
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

    /// The page of a form in a short scroll area: a header button above the area, three rows of
    /// the form and a footer button below the area. The view of the area shows the first row
    /// and the top of the second row. The second row is tall, and its bottom lies between the
    /// top and the bottom of the footer button. The third row lies below the view, farther
    /// down than the top of the footer button.
    fn short_page() -> Vec<Focusable> {
        let view = Rect::new(10.0, 50.0, 400.0, 100.0);
        let anchor = ScrollAnchor::inside(ui_id("form"), view.y, view, None);
        let mut ring = ring(&[
            Rect::new(10.0, 10.0, 300.0, 30.0),
            Rect::new(100.0, 54.0, 300.0, 30.0),
            Rect::new(100.0, 94.0, 300.0, 140.0),
            Rect::new(100.0, 244.0, 300.0, 30.0),
            Rect::new(300.0, 210.0, 100.0, 30.0),
        ]);
        for entry in &mut ring[1..4] {
            entry.scroll = Some(anchor);
        }
        ring
    }

    /// Verifies that an arrow from a widget of a scroll area takes the nearest widget of the
    /// area, also one below the view, that the focus leaves the area only from its last and
    /// its first widget, and that it enters the area at a visible widget.
    #[test]
    fn a_scroll_area_keeps_the_arrows_inside_until_its_edge() {
        let ring = short_page();
        assert_eq!(step(&ring, Some(1), Navigation::Down), Some(2));
        assert_eq!(step(&ring, Some(2), Navigation::Down), Some(3));
        assert_eq!(step(&ring, Some(3), Navigation::Down), Some(4));
        assert_eq!(step(&ring, Some(3), Navigation::Up), Some(2));
        assert_eq!(step(&ring, Some(2), Navigation::Up), Some(1));
        assert_eq!(step(&ring, Some(1), Navigation::Up), Some(0));
        assert_eq!(step(&ring, Some(0), Navigation::Down), Some(1));
        assert_eq!(step(&ring, Some(4), Navigation::Up), Some(2));
    }

    /// Verifies that a move from outside a scroll area enters at a widget of the area that
    /// shows in the direction, and at the hidden widget nearest to the view when none shows.
    #[test]
    fn a_move_enters_a_scroll_area_at_a_visible_widget() {
        let mut ring = short_page();
        ring[2].rect = Rect::new(100.0, 160.0, 300.0, 30.0);
        assert_eq!(step(&ring, Some(4), Navigation::Up), Some(1));
        ring[1].rect = Rect::new(100.0, 150.0, 300.0, 30.0);
        assert_eq!(step(&ring, Some(0), Navigation::Down), Some(1));
        assert_eq!(step(&ring, Some(4), Navigation::Up), Some(1));
    }

    /// Verifies that Left and Right take neither a wide widget of another row that spans the
    /// start nor a widget of the next row past the end of the row, and that they take a widget
    /// beside the start outside a scroll area over a hidden widget of the area.
    #[test]
    fn the_sideways_arrows_stay_in_their_row() {
        let rows = ring(&[
            Rect::new(10.0, 10.0, 400.0, 30.0),
            Rect::new(10.0, 60.0, 150.0, 30.0),
            Rect::new(170.0, 60.0, 60.0, 30.0),
            Rect::new(300.0, 100.0, 50.0, 30.0),
        ]);
        assert_eq!(step(&rows, Some(2), Navigation::Right), None);
        assert_eq!(step(&rows, Some(1), Navigation::Right), Some(2));
        assert_eq!(step(&rows, Some(2), Navigation::Left), Some(1));

        let view = Rect::new(10.0, 50.0, 200.0, 100.0);
        let anchor = ScrollAnchor::inside(ui_id("form"), view.y, view, None);
        let mut ring = ring(&[
            Rect::new(10.0, 60.0, 100.0, 30.0),
            Rect::new(120.0, 300.0, 80.0, 30.0),
            Rect::new(220.0, 60.0, 50.0, 30.0),
        ]);
        ring[0].scroll = Some(anchor);
        ring[1].scroll = Some(anchor);
        assert_eq!(step(&ring, Some(0), Navigation::Right), Some(2));
        assert_eq!(step(&ring, Some(0), Navigation::Down), Some(1));
    }

    /// Verifies that Left and Right walk the widgets of a row of a scroll area above its view,
    /// that they take a hidden widget beside the visible part of a tall widget, and that they
    /// take neither a widget of another row of the area nor a widget outside the area beside
    /// the hidden row.
    #[test]
    fn the_sideways_arrows_walk_a_row_outside_the_view() {
        let view = Rect::new(10.0, 100.0, 400.0, 100.0);
        let anchor = ScrollAnchor::inside(ui_id("form"), 0.0, view, None);
        let mut ring = ring(&[
            Rect::new(10.0, 20.0, 200.0, 30.0),
            Rect::new(220.0, 20.0, 80.0, 30.0),
            Rect::new(220.0, 110.0, 80.0, 30.0),
            Rect::new(420.0, 20.0, 80.0, 30.0),
            Rect::new(10.0, 60.0, 200.0, 120.0),
            Rect::new(220.0, 60.0, 80.0, 30.0),
        ]);
        for index in [0, 1, 2, 4, 5] {
            ring[index].scroll = Some(anchor);
        }
        assert_eq!(step(&ring[..4], Some(0), Navigation::Right), Some(1));
        assert_eq!(step(&ring[..4], Some(1), Navigation::Left), Some(0));
        assert_eq!(step(&ring[..4], Some(1), Navigation::Right), None);
        assert_eq!(step(&ring[..4], Some(2), Navigation::Left), None);
        assert_eq!(step(&ring[4..], Some(0), Navigation::Right), Some(1));
    }

    /// Verifies the clamp of a rectangle into a view.
    #[test]
    fn a_rectangle_clamps_into_a_view() {
        let view = Rect::new(0.0, 0.0, 100.0, 50.0);
        assert_eq!(
            clamp_into(Rect::new(-10.0, 40.0, 50.0, 30.0), view),
            Rect::new(0.0, 40.0, 40.0, 10.0)
        );
        assert_eq!(
            clamp_into(Rect::new(20.0, 80.0, 10.0, 10.0), view),
            Rect::new(20.0, 50.0, 10.0, 0.0)
        );
        assert_eq!(visible_part(Rect::new(20.0, 80.0, 10.0, 10.0), view), None);
    }
}
