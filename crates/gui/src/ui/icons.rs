//! Icons drawn from lines, arcs and paths.
//!
//! The icons are drawn on a grid of 16 x 16 units. At the size of 16 logical
//! units one grid unit spans one logical unit, and the main edges of the
//! filled icons lie on whole or half units.

use std::f32::consts::TAU;

use crate::{Color, Fonts, Path, Scene, SlugShape};

/// The units of the icon grid along each side.
const GRID: f32 = 16.0;

/// The angle where the arc of [`Icon::Refresh`] starts, above the right of
/// its center.
const REFRESH_ARC_START: f32 = -std::f32::consts::FRAC_PI_3;

/// The clockwise sweep of the arc of [`Icon::Refresh`]. It ends at the
/// arrowhead above the left of its center.
const REFRESH_ARC_SWEEP: f32 = 1.6 * std::f32::consts::PI;

/// An icon of the user interface.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Icon {
    /// A cross for closing a panel or clearing a field.
    Close,
    /// A check mark for the item in use.
    Check,
    /// A triangle over a bar for ejecting a medium.
    Eject,
    /// A 3.5 inch floppy disk.
    Disk,
    /// An optical disc.
    Disc,
    /// A folder.
    Folder,
    /// A magnifying glass for a search field.
    Search,
    /// A triangle that points right for starting a game.
    Play,
    /// A plus for adding an item.
    Add,
    /// A pencil for editing an item.
    Edit,
    /// Two overlapping squares for duplicating an item.
    Duplicate,
    /// A waste bin for deleting an item.
    Delete,
    /// Two sliders for settings.
    Settings,
    /// A circular arrow for reading the files again.
    Refresh,
    /// A triangle with an exclamation mark for errors and warnings.
    Alert,
    /// A chevron that points up, for ascending order and for shifting an
    /// item up.
    Up,
    /// A chevron that points down, for descending order, for shifting an
    /// item down and for opening a dropdown.
    Down,
    /// A desktop computer with a monitor on a flat case.
    SystemDesktop,
    /// A tower case.
    SystemTower,
    /// A computer built into its keyboard.
    SystemKeyboard,
}

impl Icon {
    /// Every icon.
    pub const ALL: [Icon; 20] = [
        Icon::Close,
        Icon::Check,
        Icon::Eject,
        Icon::Disk,
        Icon::Disc,
        Icon::Folder,
        Icon::Search,
        Icon::Play,
        Icon::Add,
        Icon::Edit,
        Icon::Duplicate,
        Icon::Delete,
        Icon::Settings,
        Icon::Refresh,
        Icon::Alert,
        Icon::Up,
        Icon::Down,
        Icon::SystemDesktop,
        Icon::SystemTower,
        Icon::SystemKeyboard,
    ];
}

/// The paths of the filled icons.
#[derive(Clone, Copy, Debug)]
pub struct Icons {
    /// The path of [`Icon::Eject`].
    eject: Option<SlugShape>,
    /// The path of [`Icon::Disk`].
    disk: Option<SlugShape>,
    /// The path of [`Icon::Folder`].
    folder: Option<SlugShape>,
    /// The path of [`Icon::Play`].
    play: Option<SlugShape>,
    /// The path of [`Icon::Edit`].
    edit: Option<SlugShape>,
    /// The filled body of [`Icon::Delete`].
    delete: Option<SlugShape>,
    /// The path of [`Icon::Alert`].
    alert: Option<SlugShape>,
    /// The path of [`Icon::SystemDesktop`].
    system_desktop: Option<SlugShape>,
    /// The path of [`Icon::SystemTower`].
    system_tower: Option<SlugShape>,
    /// The path of [`Icon::SystemKeyboard`].
    system_keyboard: Option<SlugShape>,
}

/// The winding of a contour of a filled icon. The nonzero fill rule fills
/// a body and leaves a hole of the opposite winding inside it empty.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Winding {
    /// A filled area, clockwise on the screen.
    Body,
    /// A hole in a body, counterclockwise on the screen.
    Hole,
}

/// A closed polygon through `points` on the icon grid with the winding
/// `winding`, added to `path`.
fn wound_polygon(path: Path, points: &[[f32; 2]], winding: Winding) -> Path {
    let doubled_area: f32 = points
        .iter()
        .zip(points.iter().cycle().skip(1))
        .map(|(&[x0, y0], &[x1, y1])| x0 * y1 - x1 * y0)
        .sum();
    let clockwise = doubled_area > 0.0;
    let mut ordered = points.to_vec();
    if clockwise != (winding == Winding::Body) {
        ordered.reverse();
    }
    grid_polygon(path, &ordered)
}

/// The rectangle from `(left, top)` to `(right, bottom)` on the icon grid
/// with the winding `winding`, added to `path`.
fn grid_rectangle(
    path: Path,
    (left, top): (f32, f32),
    (right, bottom): (f32, f32),
    winding: Winding,
) -> Path {
    wound_polygon(
        path,
        &[[left, top], [right, top], [right, bottom], [left, bottom]],
        winding,
    )
}

/// The play triangle.
fn play_path() -> Path {
    wound_polygon(
        Path::new(),
        &[[4.5, 2.5], [13.5, 8.0], [4.5, 13.5]],
        Winding::Body,
    )
}

/// The pencil with the slit between its tip and its body.
fn edit_path() -> Path {
    let body = wound_polygon(
        Path::new(),
        &[
            [11.0, 2.0],
            [14.0, 5.0],
            [6.0, 13.0],
            [2.5, 13.5],
            [3.0, 10.0],
        ],
        Winding::Body,
    );
    wound_polygon(
        body,
        &[[9.0, 4.0], [8.25, 4.75], [11.25, 7.75], [12.0, 7.0]],
        Winding::Hole,
    )
}

/// The bin of the delete icon with its two slots. The lid is stroked.
fn delete_path() -> Path {
    let bin = wound_polygon(
        Path::new(),
        &[[3.75, 5.75], [12.25, 5.75], [11.5, 14.5], [4.5, 14.5]],
        Winding::Body,
    );
    let bin = grid_rectangle(bin, (6.0, 7.5), (7.25, 12.75), Winding::Hole);
    grid_rectangle(bin, (8.75, 7.5), (10.0, 12.75), Winding::Hole)
}

/// The warning triangle with the exclamation mark as holes.
fn alert_path() -> Path {
    let triangle = wound_polygon(
        Path::new(),
        &[[8.0, 1.5], [15.0, 14.0], [1.0, 14.0]],
        Winding::Body,
    );
    let triangle = grid_rectangle(triangle, (7.25, 5.5), (8.75, 9.75), Winding::Hole);
    grid_rectangle(triangle, (7.25, 11.0), (8.75, 12.5), Winding::Hole)
}

/// The monitor with its screen and the flat case with its drive slot.
fn system_desktop_path() -> Path {
    let monitor = grid_rectangle(Path::new(), (2.5, 1.5), (13.5, 9.5), Winding::Body);
    let monitor = grid_rectangle(monitor, (4.0, 3.0), (12.0, 8.0), Winding::Hole);
    let case = grid_rectangle(monitor, (1.5, 11.0), (14.5, 14.5), Winding::Body);
    grid_rectangle(case, (9.0, 12.25), (13.0, 13.25), Winding::Hole)
}

/// The tower case with two drive bays and the power button.
fn system_tower_path() -> Path {
    let case = grid_rectangle(Path::new(), (4.5, 1.5), (11.5, 14.5), Winding::Body);
    let case = grid_rectangle(case, (6.0, 3.25), (10.0, 4.5), Winding::Hole);
    let case = grid_rectangle(case, (6.0, 6.0), (10.0, 7.25), Winding::Hole);
    grid_rectangle(case, (7.25, 11.25), (8.75, 12.75), Winding::Hole)
}

/// The keyboard case with a row of keys and the space bar.
fn system_keyboard_path() -> Path {
    let mut keyboard = grid_rectangle(Path::new(), (1.0, 4.5), (15.0, 12.5), Winding::Body);
    for (left, right) in [
        (2.5, 4.0),
        (5.0, 6.5),
        (7.5, 9.0),
        (10.0, 11.5),
        (12.5, 13.5),
    ] {
        keyboard = grid_rectangle(keyboard, (left, 6.0), (right, 7.5), Winding::Hole);
    }
    grid_rectangle(keyboard, (4.5, 9.0), (11.5, 10.75), Winding::Hole)
}

/// A closed polygon through `points` on the icon grid, added to `path`.
fn grid_polygon(path: Path, points: &[[f32; 2]]) -> Path {
    let points: Vec<[f32; 2]> = points.iter().map(|&[x, y]| [x / GRID, y / GRID]).collect();
    path.polygon(&points)
}

impl Icons {
    /// Converts the paths of the filled icons into Slug data of `fonts`.
    pub fn new(fonts: &mut Fonts) -> Self {
        let eject = grid_polygon(Path::new(), &[[8.0, 2.5], [14.0, 9.0], [2.0, 9.0]]);
        let eject = grid_polygon(
            eject,
            &[[2.0, 11.0], [14.0, 11.0], [14.0, 13.5], [2.0, 13.5]],
        );
        // The body with the shutter notch, the slot of the shutter and the
        // label as a hole of the opposite winding.
        let disk = grid_polygon(
            Path::new(),
            &[
                [2.0, 2.0],
                [5.0, 2.0],
                [5.0, 6.0],
                [10.5, 6.0],
                [10.5, 2.0],
                [11.5, 2.0],
                [14.0, 4.5],
                [14.0, 14.0],
                [2.0, 14.0],
            ],
        );
        let disk = grid_polygon(disk, &[[8.25, 2.75], [9.5, 2.75], [9.5, 5.0], [8.25, 5.0]]);
        let disk = grid_polygon(disk, &[[4.5, 8.5], [4.5, 12.5], [11.5, 12.5], [11.5, 8.5]]);
        let folder = grid_polygon(
            Path::new(),
            &[
                [1.5, 3.0],
                [6.0, 3.0],
                [7.5, 4.5],
                [14.5, 4.5],
                [14.5, 13.0],
                [1.5, 13.0],
            ],
        );
        // The seam between the tab and the front of the folder.
        let folder = grid_polygon(
            folder,
            &[[2.75, 6.25], [2.75, 7.25], [13.25, 7.25], [13.25, 6.25]],
        );
        Self {
            eject: fonts.add_path(&eject),
            disk: fonts.add_path(&disk),
            folder: fonts.add_path(&folder),
            play: fonts.add_path(&play_path()),
            edit: fonts.add_path(&edit_path()),
            delete: fonts.add_path(&delete_path()),
            alert: fonts.add_path(&alert_path()),
            system_desktop: fonts.add_path(&system_desktop_path()),
            system_tower: fonts.add_path(&system_tower_path()),
            system_keyboard: fonts.add_path(&system_keyboard_path()),
        }
    }

    /// Draws `icon` into the box of `size` logical units whose top left
    /// corner lies at `origin`.
    pub fn draw(&self, scene: &mut Scene, origin: (f32, f32), size: f32, icon: Icon, color: Color) {
        let unit = size / GRID;
        let at = |x: f32, y: f32| (origin.0 + x * unit, origin.1 + y * unit);
        let stroke = (1.75 * unit).max(1.25);
        let path = |scene: &mut Scene, shape: Option<SlugShape>| {
            if let Some(shape) = shape {
                scene.path(origin, size, shape, color);
            }
        };
        match icon {
            Icon::Close => {
                scene.line(at(4.0, 4.0), at(12.0, 12.0), stroke, color);
                scene.line(at(12.0, 4.0), at(4.0, 12.0), stroke, color);
            }
            Icon::Check => {
                scene.line(at(3.0, 8.5), at(6.5, 12.0), stroke, color);
                scene.line(at(6.5, 12.0), at(13.0, 4.5), stroke, color);
            }
            Icon::Eject => path(scene, self.eject),
            Icon::Disk => path(scene, self.disk),
            Icon::Folder => path(scene, self.folder),
            Icon::Disc => {
                scene.arc(at(8.0, 8.0), 4.25 * unit, 4.5 * unit, (0.0, TAU), color);
            }
            Icon::Search => {
                scene.arc(at(7.0, 7.0), 4.0 * unit, stroke, (0.0, TAU), color);
                scene.line(at(10.25, 10.25), at(13.5, 13.5), stroke, color);
            }
            Icon::Play => path(scene, self.play),
            Icon::Add => {
                scene.line(at(8.0, 3.0), at(8.0, 13.0), stroke, color);
                scene.line(at(3.0, 8.0), at(13.0, 8.0), stroke, color);
            }
            Icon::Edit => path(scene, self.edit),
            Icon::Duplicate => {
                let back = [at(2.5, 10.5), at(2.5, 2.5), at(10.5, 2.5)];
                let front = [
                    at(5.5, 5.5),
                    at(13.5, 5.5),
                    at(13.5, 13.5),
                    at(5.5, 13.5),
                    at(5.5, 5.5),
                ];
                for pair in back.windows(2).chain(front.windows(2)) {
                    scene.line(pair[0], pair[1], stroke, color);
                }
            }
            Icon::Delete => {
                scene.line(at(2.5, 4.0), at(13.5, 4.0), stroke, color);
                scene.line(at(6.25, 2.0), at(9.75, 2.0), stroke, color);
                path(scene, self.delete);
            }
            Icon::Refresh => {
                scene.arc(
                    at(8.0, 8.0),
                    5.0 * unit,
                    stroke,
                    (REFRESH_ARC_START, REFRESH_ARC_SWEEP),
                    color,
                );
                scene.line(at(1.65, 4.39), at(4.65, 4.28), stroke, color);
                scene.line(at(4.65, 4.28), at(4.23, 7.25), stroke, color);
            }
            Icon::Settings => {
                scene.line(at(2.5, 4.5), at(7.5, 4.5), stroke, color);
                scene.line(at(12.5, 4.5), at(13.5, 4.5), stroke, color);
                scene.line(at(2.5, 11.5), at(3.5, 11.5), stroke, color);
                scene.line(at(8.5, 11.5), at(13.5, 11.5), stroke, color);
                scene.arc(at(10.0, 4.5), 1.75 * unit, stroke, (0.0, TAU), color);
                scene.arc(at(6.0, 11.5), 1.75 * unit, stroke, (0.0, TAU), color);
            }
            Icon::Alert => path(scene, self.alert),
            Icon::Up => {
                scene.line(at(4.0, 10.0), at(8.0, 6.0), stroke, color);
                scene.line(at(8.0, 6.0), at(12.0, 10.0), stroke, color);
            }
            Icon::Down => {
                scene.line(at(4.0, 6.0), at(8.0, 10.0), stroke, color);
                scene.line(at(8.0, 10.0), at(12.0, 6.0), stroke, color);
            }
            Icon::SystemDesktop => path(scene, self.system_desktop),
            Icon::SystemTower => path(scene, self.system_tower),
            Icon::SystemKeyboard => path(scene, self.system_keyboard),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        Instance,
        ui::{
            Viewport,
            tests::{Harness, pixel},
        },
    };

    /// Points on the icon grid.
    type Points = &'static [(f32, f32)];

    /// Draws `icon` at 64 units on a 64 x 64 target and returns the
    /// coverage of the grid points `points` from 0 to 255.
    fn coverage(icon: Icon, points: &[(f32, f32)]) -> Vec<u8> {
        let mut harness = Harness::with_viewport(Viewport::new(64, 64, 1.0));
        harness.frame(|context| {
            context.icon(
                crate::Rect::new(0.0, 0.0, 64.0, 64.0),
                icon,
                64.0,
                Color::WHITE,
            );
        });
        let pixels = harness.render();
        points
            .iter()
            .map(|&(x, y)| pixel(&pixels, 64, x * 4.0, y * 4.0)[3])
            .collect()
    }

    #[test]
    fn filled_icons_leave_their_holes_empty() {
        let cases: [(Icon, Points, Points); 7] = [
            (Icon::Play, &[(7.0, 8.0)], &[(12.0, 3.0)]),
            (Icon::Edit, &[(12.5, 4.5), (4.0, 11.5)], &[(10.1, 5.9)]),
            (
                Icon::Delete,
                &[(5.0, 10.0), (8.0, 10.0), (8.0, 4.0)],
                &[(6.6, 10.0), (9.4, 10.0)],
            ),
            (
                Icon::Alert,
                &[(5.0, 12.5), (8.0, 4.0)],
                &[(8.0, 7.5), (8.0, 11.75)],
            ),
            (
                Icon::SystemDesktop,
                &[(3.2, 5.0), (3.0, 13.0)],
                &[(8.0, 5.5), (11.0, 12.75)],
            ),
            (
                Icon::SystemTower,
                &[(5.2, 8.0), (8.0, 9.5)],
                &[(8.0, 3.9), (8.0, 6.6), (8.0, 12.0)],
            ),
            (
                Icon::SystemKeyboard,
                &[(2.0, 11.5), (8.0, 8.25)],
                &[(3.25, 6.75), (8.0, 9.9)],
            ),
        ];
        for (icon, filled, holes) in cases {
            for (point, alpha) in filled.iter().zip(coverage(icon, filled)) {
                assert_eq!(alpha, 255, "{icon:?} fills {point:?}");
            }
            for (point, alpha) in holes.iter().zip(coverage(icon, holes)) {
                assert_eq!(alpha, 0, "{icon:?} leaves {point:?} empty");
            }
        }
    }

    #[test]
    fn stroked_icons_cover_their_strokes() {
        let cases: [(Icon, Points, Points); 6] = [
            (Icon::Add, &[(8.0, 4.0), (4.0, 8.0)], &[(4.0, 4.0)]),
            (
                Icon::Duplicate,
                &[(2.5, 6.0), (13.5, 9.0), (9.0, 13.5)],
                &[(9.0, 9.0)],
            ),
            (
                Icon::Settings,
                &[(4.0, 4.5), (10.0, 2.75), (11.0, 11.5)],
                &[(10.0, 4.5)],
            ),
            (Icon::Up, &[(8.0, 6.2), (5.0, 9.0)], &[(8.0, 9.0)]),
            (
                Icon::Refresh,
                &[(13.0, 8.0), (8.0, 13.0), (3.0, 8.0), (2.5, 4.4), (4.4, 6.0)],
                &[(8.0, 8.0), (8.0, 3.0), (13.5, 13.5)],
            ),
            (Icon::Down, &[(8.0, 9.8), (5.0, 7.0)], &[(8.0, 7.0)]),
        ];
        for (icon, filled, empty) in cases {
            for (point, alpha) in filled.iter().zip(coverage(icon, filled)) {
                assert!(alpha > 200, "{icon:?} covers {point:?}: {alpha}");
            }
            for (point, alpha) in empty.iter().zip(coverage(icon, empty)) {
                assert_eq!(alpha, 0, "{icon:?} leaves {point:?} empty");
            }
        }
    }

    #[test]
    fn every_icon_draws_inside_its_box() {
        let mut harness = Harness::with_viewport(Viewport::new(200, 200, 2.0));
        for icon in Icon::ALL {
            harness.frame(|context| {
                context.icon(
                    crate::Rect::new(10.0, 10.0, 16.0, 16.0),
                    icon,
                    16.0,
                    Color::WHITE,
                );
            });
            assert!(!harness.scene.is_empty(), "{icon:?}");
            for instance in harness.scene.instances() {
                let [x, y, width, height] = instance.rect;
                assert!(x >= 19.0 && y >= 19.0, "{icon:?} {instance:?}");
                assert!(
                    x + width <= 53.0 && y + height <= 53.0,
                    "{icon:?} {instance:?}"
                );
                if instance.kind[0] == Instance::KIND_GLYPH {
                    assert!(instance.kind[1] < harness.fonts.slug().glyphs().len() as u32);
                }
            }
        }
    }
}
