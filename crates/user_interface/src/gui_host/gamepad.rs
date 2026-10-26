//! The navigation of a gamepad for the user interface: the D-pad and the
//! left stick with a synthesized repeat, and the face, shoulder and start
//! buttons.

use std::time::{Duration, Instant};

use gui::ui::{ActionButton, Navigation};
use sdl3::gamepad::{GamepadAxis, GamepadButton};

use super::GuiEvent;

/// The delay before a held gamepad direction repeats.
const REPEAT_DELAY: Duration = Duration::from_millis(400);

/// The interval between the repeats of a held gamepad direction.
const REPEAT_INTERVAL: Duration = Duration::from_millis(80);

/// The stick deflection that engages a direction.
const STICK_PRESS: i16 = 16384;

/// The stick deflection below which an engaged direction releases.
const STICK_RELEASE: i16 = 11000;

/// The control that holds a gamepad direction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DirectionSource {
    /// The D-pad.
    Dpad,
    /// The left stick.
    Stick,
}

/// A held direction that repeats.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Repeat {
    /// The direction.
    navigation: Navigation,
    /// The time of the next repeat.
    next: Instant,
}

/// The navigation of a gamepad: the D-pad and the left stick with a
/// synthesized repeat while a direction is held, and the face, shoulder and
/// start buttons.
#[derive(Debug, Default)]
pub(crate) struct GamepadNavigation {
    /// The held directions of the D-pad and the stick, the most recent last.
    held: Vec<(DirectionSource, Navigation)>,
    /// The horizontal deflection of the left stick.
    stick_x: i16,
    /// The vertical deflection of the left stick.
    stick_y: i16,
    /// The engaged direction of the left stick.
    stick: Option<Navigation>,
    /// The direction that repeats.
    repeat: Option<Repeat>,
}

impl GamepadNavigation {
    /// Forgets the held directions.
    pub(crate) fn clear(&mut self) {
        *self = Self::default();
    }

    /// Holds `navigation` from `source`, starts its repeat at `now` and
    /// returns its event.
    fn press(&mut self, source: DirectionSource, navigation: Navigation, now: Instant) -> GuiEvent {
        self.held.retain(|&entry| entry != (source, navigation));
        self.held.push((source, navigation));
        self.repeat = Some(Repeat {
            navigation,
            next: now + REPEAT_DELAY,
        });
        GuiEvent::GamepadNavigation(navigation)
    }

    /// Releases `navigation` from `source`. The repeat falls back to the most
    /// recent direction still held, which repeats after the initial delay
    /// from `now`, or stops.
    fn release(&mut self, source: DirectionSource, navigation: Navigation, now: Instant) {
        self.held.retain(|&entry| entry != (source, navigation));
        let fallback = self.held.last().map(|&(_, held)| held);
        let current = self.repeat.map(|repeat| repeat.navigation);
        if fallback != current {
            self.repeat = fallback.map(|navigation| Repeat {
                navigation,
                next: now + REPEAT_DELAY,
            });
        }
    }

    /// The event of a button change at `now`. A confirms, B cancels, the
    /// shoulder buttons switch tabs, and X, Y and Start are action buttons.
    pub(crate) fn button(
        &mut self,
        button: GamepadButton,
        pressed: bool,
        now: Instant,
    ) -> Option<GuiEvent> {
        let direction = match button {
            GamepadButton::DpadUp => Some(Navigation::Up),
            GamepadButton::DpadDown => Some(Navigation::Down),
            GamepadButton::DpadLeft => Some(Navigation::Left),
            GamepadButton::DpadRight => Some(Navigation::Right),
            _ => None,
        };
        if let Some(navigation) = direction {
            if pressed {
                return Some(self.press(DirectionSource::Dpad, navigation, now));
            }
            self.release(DirectionSource::Dpad, navigation, now);
            return None;
        }
        if !pressed {
            return None;
        }
        match button {
            GamepadButton::South => Some(GuiEvent::Confirm),
            GamepadButton::East => Some(GuiEvent::Cancel),
            GamepadButton::LeftShoulder => {
                Some(GuiEvent::GamepadNavigation(Navigation::PreviousTab))
            }
            GamepadButton::RightShoulder => Some(GuiEvent::GamepadNavigation(Navigation::NextTab)),
            GamepadButton::West => Some(GuiEvent::Action(ActionButton::X)),
            GamepadButton::North => Some(GuiEvent::Action(ActionButton::Y)),
            GamepadButton::Start => Some(GuiEvent::Action(ActionButton::Start)),
            _ => None,
        }
    }

    /// The event of a stick change at `now`. A direction engages past
    /// [`STICK_PRESS`] on the axis with the larger deflection and releases
    /// below [`STICK_RELEASE`].
    pub(crate) fn axis(&mut self, axis: GamepadAxis, value: i16, now: Instant) -> Option<GuiEvent> {
        match axis {
            GamepadAxis::LeftX => self.stick_x = value,
            GamepadAxis::LeftY => self.stick_y = value,
            _ => return None,
        }
        let horizontal = self.stick_x.unsigned_abs();
        let vertical = self.stick_y.unsigned_abs();
        let still_engaged = match self.stick {
            Some(Navigation::Left | Navigation::Right) => horizontal >= STICK_RELEASE as u16,
            Some(_) => vertical >= STICK_RELEASE as u16,
            None => false,
        };
        let engaged = if still_engaged {
            match self.stick {
                Some(Navigation::Left | Navigation::Right) => Some(if self.stick_x < 0 {
                    Navigation::Left
                } else {
                    Navigation::Right
                }),
                _ => Some(if self.stick_y < 0 {
                    Navigation::Up
                } else {
                    Navigation::Down
                }),
            }
        } else if horizontal.max(vertical) >= STICK_PRESS as u16 {
            Some(if horizontal >= vertical {
                if self.stick_x < 0 {
                    Navigation::Left
                } else {
                    Navigation::Right
                }
            } else if self.stick_y < 0 {
                Navigation::Up
            } else {
                Navigation::Down
            })
        } else {
            None
        };
        if engaged == self.stick {
            return None;
        }
        let previous = std::mem::replace(&mut self.stick, engaged);
        if let Some(previous) = previous {
            self.release(DirectionSource::Stick, previous, now);
        }
        engaged.map(|navigation| self.press(DirectionSource::Stick, navigation, now))
    }

    /// The time of the next repeat of the held direction, or `None` while no
    /// direction is held.
    pub(crate) fn next_repeat(&self) -> Option<Instant> {
        self.repeat.map(|repeat| repeat.next)
    }

    /// The repeat event of the held direction that is due at `now`.
    pub(crate) fn poll(&mut self, now: Instant) -> Option<GuiEvent> {
        let repeat = self.repeat.as_mut()?;
        if now < repeat.next {
            return None;
        }
        let next = repeat.next + REPEAT_INTERVAL;
        repeat.next = if next <= now {
            now + REPEAT_INTERVAL
        } else {
            next
        };
        Some(GuiEvent::GamepadNavigation(repeat.navigation))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verifies that the face, shoulder and start buttons map to their events.
    #[test]
    fn gamepad_buttons_map_to_actions() {
        let mut gamepad = GamepadNavigation::default();
        let now = Instant::now();
        assert_eq!(
            gamepad.button(GamepadButton::South, true, now),
            Some(GuiEvent::Confirm)
        );
        assert_eq!(gamepad.button(GamepadButton::South, false, now), None);
        assert_eq!(
            gamepad.button(GamepadButton::East, true, now),
            Some(GuiEvent::Cancel)
        );
        assert_eq!(
            gamepad.button(GamepadButton::LeftShoulder, true, now),
            Some(GuiEvent::GamepadNavigation(Navigation::PreviousTab))
        );
        assert_eq!(
            gamepad.button(GamepadButton::RightShoulder, true, now),
            Some(GuiEvent::GamepadNavigation(Navigation::NextTab))
        );
        assert_eq!(
            gamepad.button(GamepadButton::West, true, now),
            Some(GuiEvent::Action(ActionButton::X))
        );
        assert_eq!(
            gamepad.button(GamepadButton::North, true, now),
            Some(GuiEvent::Action(ActionButton::Y))
        );
        assert_eq!(
            gamepad.button(GamepadButton::Start, true, now),
            Some(GuiEvent::Action(ActionButton::Start))
        );
        assert_eq!(gamepad.button(GamepadButton::Start, false, now), None);
        assert_eq!(gamepad.button(GamepadButton::Back, true, now), None);
    }

    /// Verifies that a held D-pad direction repeats after the delay at the interval.
    #[test]
    fn a_held_dpad_direction_repeats() {
        let mut gamepad = GamepadNavigation::default();
        let start = Instant::now();
        assert_eq!(
            gamepad.button(GamepadButton::DpadDown, true, start),
            Some(GuiEvent::GamepadNavigation(Navigation::Down))
        );
        assert_eq!(gamepad.poll(start + Duration::from_millis(399)), None);
        let first = start + REPEAT_DELAY;
        assert_eq!(
            gamepad.poll(first),
            Some(GuiEvent::GamepadNavigation(Navigation::Down))
        );
        assert_eq!(gamepad.poll(first + Duration::from_millis(79)), None);
        assert_eq!(
            gamepad.poll(first + REPEAT_INTERVAL),
            Some(GuiEvent::GamepadNavigation(Navigation::Down))
        );
        assert_eq!(gamepad.next_repeat(), Some(first + REPEAT_INTERVAL * 2));
        assert_eq!(gamepad.button(GamepadButton::DpadDown, false, first), None);
        assert_eq!(gamepad.next_repeat(), None);
        assert_eq!(gamepad.poll(first + Duration::from_secs(1)), None);
    }

    /// Verifies that a late poll gives one repeat.
    #[test]
    fn a_late_poll_repeats_once() {
        let mut gamepad = GamepadNavigation::default();
        let start = Instant::now();
        gamepad.button(GamepadButton::DpadUp, true, start);
        let late = start + Duration::from_secs(2);
        assert!(gamepad.poll(late).is_some());
        assert_eq!(gamepad.poll(late), None);
        assert!(gamepad.poll(late + REPEAT_INTERVAL).is_some());
    }

    /// Verifies the dead zone and the hysteresis of the stick.
    #[test]
    fn the_stick_has_a_dead_zone_and_hysteresis() {
        let mut gamepad = GamepadNavigation::default();
        let now = Instant::now();
        assert_eq!(gamepad.axis(GamepadAxis::LeftY, 12000, now), None);
        assert_eq!(
            gamepad.axis(GamepadAxis::LeftY, 20000, now),
            Some(GuiEvent::GamepadNavigation(Navigation::Down))
        );
        assert_eq!(gamepad.axis(GamepadAxis::LeftY, 12000, now), None);
        assert!(gamepad.poll(now + REPEAT_DELAY).is_some());
        assert_eq!(gamepad.axis(GamepadAxis::LeftY, 9000, now), None);
        assert_eq!(gamepad.poll(now + Duration::from_secs(5)), None);
        assert_eq!(
            gamepad.axis(GamepadAxis::LeftX, -30000, now),
            Some(GuiEvent::GamepadNavigation(Navigation::Left))
        );
        assert_eq!(gamepad.axis(GamepadAxis::LeftY, -32768, now), None);
        assert_eq!(
            gamepad.axis(GamepadAxis::LeftX, 0, now),
            Some(GuiEvent::GamepadNavigation(Navigation::Up))
        );
        assert_eq!(gamepad.axis(GamepadAxis::RightX, 32767, now), None);
    }

    /// Verifies that a release hands the repeat to the most recent held direction.
    #[test]
    fn a_release_falls_back_to_the_most_recent_held_direction() {
        let mut gamepad = GamepadNavigation::default();
        let start = Instant::now();
        gamepad.button(GamepadButton::DpadDown, true, start);
        gamepad.axis(GamepadAxis::LeftX, 30000, start);
        gamepad.button(GamepadButton::DpadUp, true, start);
        let release = start + Duration::from_millis(100);
        gamepad.button(GamepadButton::DpadUp, false, release);
        assert_eq!(gamepad.poll(release + Duration::from_millis(399)), None);
        assert_eq!(
            gamepad.poll(release + REPEAT_DELAY),
            Some(GuiEvent::GamepadNavigation(Navigation::Right))
        );
        gamepad.axis(GamepadAxis::LeftX, 0, release);
        assert_eq!(
            gamepad.poll(release + REPEAT_DELAY * 2),
            Some(GuiEvent::GamepadNavigation(Navigation::Down))
        );
        gamepad.button(GamepadButton::DpadDown, false, release);
        assert_eq!(gamepad.poll(release + Duration::from_secs(5)), None);
    }

    /// Verifies that the stick and the D-pad share one repeat.
    #[test]
    fn the_stick_and_the_dpad_share_the_repeat() {
        let mut gamepad = GamepadNavigation::default();
        let now = Instant::now();
        gamepad.button(GamepadButton::DpadRight, true, now);
        gamepad.axis(GamepadAxis::LeftX, 30000, now);
        gamepad.axis(GamepadAxis::LeftX, 0, now);
        assert!(gamepad.poll(now + REPEAT_DELAY).is_some());
        gamepad.button(GamepadButton::DpadRight, false, now);
        assert_eq!(gamepad.poll(now + Duration::from_secs(5)), None);
    }
}
