//! The look of the widgets: sizes in logical units, colors, text styles and
//! icons.

use super::{context::TextStyle, icons::Icons};
use crate::{Color, FontId, Fonts, srgb_to_linear};

/// The share of the perceived brightness of the emulator frame that the veil
/// of a modal panel leaves. The scene blends in linear light, so the veil
/// takes the opacity that leaves this brightness after the sRGB encoding.
const VEIL_BRIGHTNESS: f32 = 0.45;

/// The look of every widget.
///
/// The default look is dark and calm for an overlay over the emulator frame,
/// with a blue accent and a light blue focus indicator. The user interface
/// has one font face. Text takes its emphasis from the size and the color.
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
    /// The veil over the emulator frame behind a modal panel.
    pub veil: Color,
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
    /// The shadow under panels.
    pub shadow: Color,
    /// The icons.
    pub icons: Icons,
}

impl Theme {
    /// The default look with every text in `font`. The icon paths go into
    /// `fonts`.
    pub fn new(fonts: &mut Fonts, font: FontId) -> Self {
        let accent = Color::hex(0x3272E0);
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
            veil: Color::BLACK.with_alpha(1.0 - srgb_to_linear(VEIL_BRIGHTNESS)),
            panel: Color::hex(0x1E2128),
            panel_edge: Color::hex(0x353A45),
            well: Color::hex(0x16181D),
            well_edge: Color::hex(0x2B2F38),
            field: Color::hex(0x131519),
            control: Color::hex(0x2B3039),
            control_hover: Color::hex(0x353B46),
            control_pressed: Color::hex(0x23272F),
            control_edge: Color::hex(0x3D434F),
            accent,
            accent_hover: Color::hex(0x3F80EE),
            accent_pressed: Color::hex(0x2A62C2),
            accent_text: Color::WHITE,
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
            shadow: Color::BLACK.with_alpha(0.55),
            icons: Icons::new(fonts),
        }
    }

    /// Normal text.
    pub fn body(&self) -> TextStyle {
        TextStyle {
            font: self.font,
            size: self.text_size,
            color: self.text,
        }
    }

    /// Secondary text at the normal size.
    pub fn dim(&self) -> TextStyle {
        self.body().with_color(self.text_dim)
    }

    /// Small secondary text for hints and states.
    pub fn small(&self) -> TextStyle {
        TextStyle {
            size: self.small_text_size,
            color: self.text_dim,
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
