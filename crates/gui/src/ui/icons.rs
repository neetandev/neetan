//! Icons drawn from lines, arcs and paths.
//!
//! The icons are drawn on a grid of 16 x 16 units. At the size of 16 logical
//! units one grid unit spans one logical unit, and the main edges of the
//! filled icons lie on whole or half units.

use std::f32::consts::TAU;

use crate::{Color, Fonts, Path, Scene, SlugShape};

/// The units of the icon grid along each side.
const GRID: f32 = 16.0;

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
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        Instance,
        ui::{Viewport, tests::Harness},
    };

    #[test]
    fn every_icon_draws_inside_its_box() {
        let mut harness = Harness::with_viewport(Viewport::new(200, 200, 2.0));
        for icon in [
            Icon::Close,
            Icon::Check,
            Icon::Eject,
            Icon::Disk,
            Icon::Disc,
            Icon::Folder,
            Icon::Search,
        ] {
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
