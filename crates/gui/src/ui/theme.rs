//! The look of the widgets: sizes in logical units, colors, text styles and
//! icons.

use super::{context::TextStyle, icons::Icons};
use crate::{Color, FontId, Fonts, srgb_to_linear};

/// The share of the perceived brightness of the emulator frame that the veil
/// of a modal panel leaves. The scene blends in linear light, so the veil
/// takes the opacity that leaves this brightness after the sRGB encoding.
const VEIL_BRIGHTNESS: f32 = 0.45;

/// The share of the fill of a primary or a destructive button that the
/// edge tint replaces at its edge.
pub(super) const BUTTON_EDGE_TINT_SHARE: f32 = 0.12;

/// The look of a theme: dark or light.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ThemeKind {
    /// Dark gray panels with light text. The look is calm as an overlay over
    /// the emulator frame.
    #[default]
    Dark,
    /// Light gray panels with dark text and white fields.
    Light,
}

/// The look of every widget.
///
/// Both kinds share the sizes, the text styles, the icons and the veil over
/// the emulator frame. Their palettes differ. Both palettes are cool grays
/// with a blue accent. The dark palette has a light blue focus indicator,
/// and the light palette has a dark blue one. The user interface has one font
/// face. Text takes its emphasis from the size and the color.
#[derive(Debug)]
pub struct Theme {
    /// The font family of every text.
    pub font: FontId,
    /// The inner padding of panels.
    pub padding: f32,
    /// The gap between neighboring widgets.
    pub gap: f32,
    /// The height of buttons, fields and tab bars.
    pub control_height: f32,
    /// The height of a list row.
    pub row_height: f32,
    /// The corner radius of widgets.
    pub radius: f32,
    /// The corner radius of panels.
    pub panel_radius: f32,
    /// The width of edges.
    pub stroke: f32,
    /// The width of the focus indicator.
    pub focus_width: f32,
    /// The gap between a widget and its focus indicator.
    pub focus_gap: f32,
    /// The blur of the panel shadow.
    pub shadow_blur: f32,
    /// The width of the scroll bar of a list.
    pub scroll_bar_width: f32,
    /// The size of icons beside text.
    pub icon_size: f32,
    /// The size of normal text.
    pub text_size: f32,
    /// The size of small text.
    pub small_text_size: f32,
    /// The size of titles.
    pub title_size: f32,
    /// The smallest room the status line gives the path or the error at its
    /// end. A smaller room leaves the end out.
    pub status_end_min_width: f32,
    /// The veil over the emulator frame behind a modal panel. It is dark in
    /// both kinds and dims the frame.
    pub veil: Color,
    /// The colors of the kind.
    pub colors: Palette,
    /// The icons.
    pub icons: Icons,
}

/// The colors of a theme kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Palette {
    /// The fill of panels.
    pub panel: Color,
    /// The edge of panels.
    pub panel_edge: Color,
    /// The fill of sunken areas such as a list.
    pub well: Color,
    /// The edge of sunken areas.
    pub well_edge: Color,
    /// The fill of text fields.
    pub field: Color,
    /// The fill of buttons.
    pub control: Color,
    /// The fill of hovered buttons.
    pub control_hover: Color,
    /// The fill of pressed buttons.
    pub control_pressed: Color,
    /// The edge of buttons and fields.
    pub control_edge: Color,
    /// The accent of primary buttons, selections and the selected tab.
    pub accent: Color,
    /// The accent of hovered primary buttons.
    pub accent_hover: Color,
    /// The accent of pressed primary buttons.
    pub accent_pressed: Color,
    /// Text on accent fills.
    pub accent_text: Color,
    /// The color that the fill of a primary or a destructive button mixes
    /// with for its edge.
    pub button_edge_tint: Color,
    /// The fill of the selected row of a list without the focus.
    pub selection: Color,
    /// The fill of the selected row of a focused list.
    pub selection_focused: Color,
    /// The fill of a hovered row.
    pub row_hover: Color,
    /// The thumb of a scroll bar.
    pub scroll_thumb: Color,
    /// The thumb of a hovered or dragged scroll bar.
    pub scroll_thumb_hover: Color,
    /// The focus indicator.
    pub focus: Color,
    /// The marker of the item in use, such as the loaded disk image.
    pub marker: Color,
    /// Primary text.
    pub text: Color,
    /// Secondary text.
    pub text_dim: Color,
    /// The text of disabled widgets and placeholders.
    pub text_disabled: Color,
    /// The fill of destructive buttons.
    pub danger: Color,
    /// The fill of hovered destructive buttons.
    pub danger_hover: Color,
    /// The fill of pressed destructive buttons.
    pub danger_pressed: Color,
    /// Error and warning text, alert icons and the error edge of a field.
    pub danger_text: Color,
    /// The shadow under panels.
    pub shadow: Color,
}

impl Palette {
    /// The colors of `kind`.
    pub fn of(kind: ThemeKind) -> Self {
        match kind {
            ThemeKind::Dark => Self::dark(),
            ThemeKind::Light => Self::light(),
        }
    }

    /// Dark gray panels, a darker well and darker fields with light text.
    fn dark() -> Self {
        Self {
            panel: Color::hex(0x1E2128),
            panel_edge: Color::hex(0x353A45),
            well: Color::hex(0x16181D),
            well_edge: Color::hex(0x2B2F38),
            field: Color::hex(0x131519),
            control: Color::hex(0x2B3039),
            control_hover: Color::hex(0x353B46),
            control_pressed: Color::hex(0x23272F),
            control_edge: Color::hex(0x3D434F),
            accent: Color::hex(0x3272E0),
            accent_hover: Color::hex(0x3F80EE),
            accent_pressed: Color::hex(0x2A62C2),
            accent_text: Color::WHITE,
            button_edge_tint: Color::WHITE,
            selection: Color::hex(0x233552),
            selection_focused: Color::hex(0x2858AE),
            row_hover: Color::hex(0x23262E),
            scroll_thumb: Color::hex(0xA0A7B4).with_alpha(0.35),
            scroll_thumb_hover: Color::hex(0xA0A7B4).with_alpha(0.65),
            focus: Color::hex(0x8DB8FF),
            marker: Color::hex(0x5DCB8C),
            text: Color::hex(0xE9ECF1),
            text_dim: Color::hex(0xA0A7B4),
            text_disabled: Color::hex(0x666D7A),
            danger: Color::hex(0xC83A33),
            danger_hover: Color::hex(0xCF423B),
            danger_pressed: Color::hex(0xB3322C),
            danger_text: Color::hex(0xE8605A),
            shadow: Color::BLACK.with_alpha(0.55),
        }
    }

    /// Light gray panels, a slightly darker well and white fields with dark
    /// text.
    fn light() -> Self {
        Self {
            panel: Color::hex(0xF3F5F8),
            panel_edge: Color::hex(0xCDD3DC),
            well: Color::hex(0xE9ECF1),
            well_edge: Color::hex(0xD3D8E0),
            field: Color::WHITE,
            control: Color::hex(0xE2E6EC),
            control_hover: Color::hex(0xD3D8E0),
            control_pressed: Color::hex(0xC8CED8),
            control_edge: Color::hex(0xB9C0CB),
            accent: Color::hex(0x3272E0),
            accent_hover: Color::hex(0x2B66CF),
            accent_pressed: Color::hex(0x2459B8),
            accent_text: Color::WHITE,
            button_edge_tint: Color::BLACK,
            selection: Color::hex(0xC4D4F0),
            selection_focused: Color::hex(0xA3C1F0),
            row_hover: Color::hex(0xDCE0E7),
            scroll_thumb: Color::hex(0x59606C).with_alpha(0.35),
            scroll_thumb_hover: Color::hex(0x59606C).with_alpha(0.6),
            focus: Color::hex(0x1F5FCC),
            marker: Color::hex(0x17773F),
            text: Color::hex(0x1F2329),
            text_dim: Color::hex(0x59606C),
            text_disabled: Color::hex(0x878E9A),
            danger: Color::hex(0xC83A33),
            danger_hover: Color::hex(0xB9352F),
            danger_pressed: Color::hex(0xA52E29),
            danger_text: Color::hex(0xB42D27),
            shadow: Color::BLACK.with_alpha(0.16),
        }
    }
}

impl Theme {
    /// The look of `kind` with every text in `font`. The icon paths go into
    /// `fonts`.
    pub fn new(fonts: &mut Fonts, font: FontId, kind: ThemeKind) -> Self {
        Self {
            font,
            padding: 16.0,
            gap: 8.0,
            control_height: 32.0,
            row_height: 30.0,
            radius: 6.0,
            panel_radius: 10.0,
            stroke: 1.0,
            focus_width: 2.0,
            focus_gap: 2.0,
            shadow_blur: 16.0,
            scroll_bar_width: 10.0,
            icon_size: 16.0,
            text_size: 14.0,
            small_text_size: 12.0,
            title_size: 17.0,
            status_end_min_width: 120.0,
            veil: Color::BLACK.with_alpha(1.0 - srgb_to_linear(VEIL_BRIGHTNESS)),
            colors: Palette::of(kind),
            icons: Icons::new(fonts),
        }
    }

    /// Normal text.
    pub fn body(&self) -> TextStyle {
        TextStyle {
            font: self.font,
            size: self.text_size,
            color: self.colors.text,
        }
    }

    /// Secondary text at the normal size.
    pub fn dim(&self) -> TextStyle {
        self.body().with_color(self.colors.text_dim)
    }

    /// Small secondary text for hints and states.
    pub fn small(&self) -> TextStyle {
        TextStyle {
            size: self.small_text_size,
            color: self.colors.text_dim,
            ..self.body()
        }
    }

    /// Titles.
    pub fn title(&self) -> TextStyle {
        TextStyle {
            size: self.title_size,
            ..self.body()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::luminance;

    /// The smallest contrast of text against its background.
    const TEXT_CONTRAST: f32 = 4.5;

    /// The smallest contrast of the text of disabled widgets against its
    /// background.
    const DISABLED_TEXT_CONTRAST: f32 = 2.5;

    /// The smallest contrast between two fills of a widget in different
    /// states, such as a selected and a hovered row.
    const STATE_CONTRAST: f32 = 1.1;

    /// The WCAG contrast ratio of the opaque colors `first` and `second`.
    fn contrast(first: Color, second: Color) -> f32 {
        let first = luminance(first.to_linear());
        let second = luminance(second.to_linear());
        (first.max(second) + 0.05) / (first.min(second) + 0.05)
    }

    /// Asserts that `foreground` reaches `minimum` against every color of
    /// `backgrounds`.
    fn assert_contrast(
        kind: ThemeKind,
        foreground: (&str, Color),
        backgrounds: &[(&str, Color)],
        minimum: f32,
    ) {
        for &(background_name, background) in backgrounds {
            let ratio = contrast(foreground.1, background);
            assert!(
                ratio >= minimum,
                "{kind:?}: {} on {background_name} has the contrast {ratio:.2}",
                foreground.0
            );
        }
    }

    /// Verifies the WCAG contrast of the text colors on their backgrounds in both kinds.
    #[test]
    fn text_reaches_the_contrast_of_both_kinds() {
        for kind in [ThemeKind::Dark, ThemeKind::Light] {
            let colors = Palette::of(kind);
            let surfaces = [
                ("panel", colors.panel),
                ("well", colors.well),
                ("field", colors.field),
            ];
            for foreground in [
                ("text", colors.text),
                ("text_dim", colors.text_dim),
                ("danger_text", colors.danger_text),
                ("marker", colors.marker),
            ] {
                assert_contrast(kind, foreground, &surfaces, TEXT_CONTRAST);
            }
            assert_contrast(
                kind,
                ("text_disabled", colors.text_disabled),
                &surfaces,
                DISABLED_TEXT_CONTRAST,
            );
            assert_contrast(
                kind,
                ("text", colors.text),
                &[
                    ("selection", colors.selection),
                    ("selection_focused", colors.selection_focused),
                ],
                TEXT_CONTRAST,
            );
            assert_contrast(
                kind,
                ("accent_text", colors.accent_text),
                &[("accent", colors.accent), ("danger", colors.danger)],
                TEXT_CONTRAST,
            );
        }
    }

    /// Verifies that the selection, the focused selection, the row hover and
    /// the button hover differ from each other and from their background in
    /// both kinds.
    #[test]
    fn states_differ_in_both_kinds() {
        for kind in [ThemeKind::Dark, ThemeKind::Light] {
            let colors = Palette::of(kind);
            assert_contrast(
                kind,
                ("selection", colors.selection),
                &[("row_hover", colors.row_hover), ("well", colors.well)],
                STATE_CONTRAST,
            );
            assert_contrast(
                kind,
                ("selection_focused", colors.selection_focused),
                &[("selection", colors.selection)],
                STATE_CONTRAST,
            );
            assert_contrast(
                kind,
                ("row_hover", colors.row_hover),
                &[("well", colors.well)],
                STATE_CONTRAST,
            );
            assert_contrast(
                kind,
                ("control_hover", colors.control_hover),
                &[("control", colors.control)],
                STATE_CONTRAST,
            );
        }
    }

    /// Verifies that the dark kind is the default and keeps its panel and its button edge.
    #[test]
    fn the_dark_kind_is_the_default_and_keeps_its_colors() {
        assert_eq!(ThemeKind::default(), ThemeKind::Dark);
        let colors = Palette::of(ThemeKind::Dark);
        assert_eq!(colors.panel, Color::hex(0x1E2128));
        assert_eq!(
            colors
                .accent
                .mix(colors.button_edge_tint, BUTTON_EDGE_TINT_SHARE),
            Color::hex(0x3272E0).mix(Color::WHITE, 0.12)
        );
    }
}
