//! What input does to a board, as a pure function of the input and what
//! the view shows. No iced type appears here: the widget turns iced's
//! events into [`Input`]s and the view applies the [`Effect`]s, so every
//! rule below runs in a unit test with no window.
//!
//! Phase 1 knows two states: Idle, and Panning while a drag moves the view.

use std::time::{Duration, Instant};

use wren_board::Dir;

use crate::{Effect, Viewport};

/// Logical pixels a wheel line pans by.
pub const LINE: f32 = 40.0;
/// The zoom factor of one wheel line.
pub const ZOOM_PER_LINE: f32 = 1.1;
/// Wheel events closer together than this are one burst, which keeps the
/// pan-or-zoom choice of its first event: iced drops the scroll phase, and
/// macOS momentum outlives the modifier.
pub const BURST: Duration = Duration::from_millis(150);

/// A wheel delta as the toolkit reports it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Scroll {
    /// Wheel lines; X11 reports touchpads this way too.
    Lines { x: f32, y: f32 },
    /// Physical pixels, from a precise device.
    Pixels { x: f32, y: f32 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    Left,
    Middle,
    Right,
    Other,
}

/// The modifiers the machine reads, as the widget resolves them for the
/// platform.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Mods {
    /// Shift: a click toggles, a marquee adds.
    pub shift: bool,
    /// ⌥ on macOS, Ctrl elsewhere: a drag past the threshold moves copies
    /// (§5.8 rule 3).
    pub duplicate: bool,
}

/// A key the board's state reads itself rather than as a command: Escape's
/// ladder (§5.14), and Enter, the arrows and Backspace in keyboard
/// Connecting (§5.1 rule 3).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Key {
    Escape,
    Enter,
    Backspace,
    Arrow(Dir),
}

/// One input to the machine. Points are widget-local logical pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Input {
    Wheel {
        delta: Scroll,
        at: (f32, f32),
        /// The zoom modifier (⌘ on macOS, Ctrl elsewhere) was held.
        zoom: bool,
        /// Shift was held.
        sideways: bool,
        /// The window's scale factor, which turns `Pixels` into logical
        /// pixels.
        scale_factor: f32,
    },
    Press {
        button: Button,
        at: (f32, f32),
    },
    Move {
        at: (f32, f32),
    },
    Release {
        button: Button,
    },
    /// Space went down or came up.
    Space {
        held: bool,
    },
    /// The board lost the keyboard or the window its focus: a drag in
    /// flight ends where it is, and Space counts as up.
    Cancel,
    /// The modifiers changed.
    Modifiers(Mods),
    /// A key from [`Key`]; `repeat` when it is held down.
    Key {
        key: Key,
        repeat: bool,
    },
    /// A typed character outside a label: the connect query's text.
    Char(char),
    /// The widget took (`true`) or lost iced focus.
    Focus(bool),
    /// Time passed with no other input, so a deadline the machine set,
    /// such as the hover handles' (§5.5 rule 8), may be due.
    Tick,
}

/// What the machine reads of the view.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    pub viewport: Viewport,
    pub now: Instant,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
enum State {
    #[default]
    Idle,
    /// A drag moves the view; `from` is where the pointer was last seen.
    Panning { button: Button, from: (f32, f32) },
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Machine {
    state: State,
    space: bool,
    /// When the last wheel event came, and whether its burst zooms.
    burst: Option<(Instant, bool)>,
}

impl Machine {
    pub fn step(&mut self, input: Input, view: &View) -> Vec<Effect> {
        let viewport = view.viewport;
        let to = match (input, self.state) {
            (
                Input::Wheel {
                    delta,
                    at,
                    zoom,
                    sideways,
                    scale_factor,
                },
                _,
            ) => self.wheel(delta, at, zoom, sideways, scale_factor, view),
            // A press during a drag means the drag's release never came
            // (the widget lost it with its state), so the press starts over.
            (Input::Press { button, at }, _) => {
                self.state = if button == Button::Middle || (button == Button::Left && self.space) {
                    State::Panning { button, from: at }
                } else {
                    State::Idle
                };
                None
            }
            (Input::Move { at }, State::Panning { button, from }) => {
                self.state = State::Panning { button, from: at };
                Some(viewport.panned(from.0 - at.0, from.1 - at.1))
            }
            (Input::Release { button }, State::Panning { button: by, .. }) if button == by => {
                self.state = State::Idle;
                None
            }
            (Input::Space { held }, _) => {
                self.space = held;
                None
            }
            (Input::Cancel, _) => {
                self.state = State::Idle;
                self.space = false;
                None
            }
            _ => None,
        };
        to.filter(|to| *to != viewport)
            .map(Effect::Look)
            .into_iter()
            .collect()
    }

    /// Whether a drag is moving the view.
    pub fn panning(&self) -> bool {
        matches!(self.state, State::Panning { .. })
    }

    /// Whether Space is held, so a left drag pans.
    pub fn space(&self) -> bool {
        self.space
    }

    /// Back to Idle with Space up; the burst is the wheel's and stays.
    pub fn reset(&mut self) {
        self.state = State::Idle;
        self.space = false;
    }

    fn wheel(
        &mut self,
        delta: Scroll,
        at: (f32, f32),
        zoom: bool,
        sideways: bool,
        scale_factor: f32,
        view: &View,
    ) -> Option<Viewport> {
        let zoom = match self.burst {
            Some((last, choice)) if view.now.saturating_duration_since(last) < BURST => choice,
            _ => zoom,
        };
        self.burst = Some((view.now, zoom));
        // Everything in lines: `y` is what a zoom reads.
        let (x, y, lines) = match delta {
            Scroll::Lines { x, y } if sideways && x == 0.0 => (y, 0.0, y),
            Scroll::Lines { x, y } => (x, y, y),
            Scroll::Pixels { x, y } => {
                let scale = if scale_factor.is_finite() && scale_factor > 0.0 {
                    scale_factor
                } else {
                    1.0
                };
                let unit = scale * LINE;
                (x / unit, y / unit, y / unit)
            }
        };
        if !(x.is_finite() && y.is_finite()) {
            return None;
        }
        Some(if zoom {
            view.viewport.zoomed(ZOOM_PER_LINE.powf(lines), at)
        } else {
            view.viewport.panned(-x * LINE, -y * LINE)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const AT: (f32, f32) = (317.0, 211.0);

    fn wheel(delta: Scroll, zoom: bool) -> Input {
        Input::Wheel {
            delta,
            at: AT,
            zoom,
            sideways: false,
            scale_factor: 1.0,
        }
    }

    fn lines(y: f32) -> Scroll {
        Scroll::Lines { x: 0.0, y }
    }

    /// Steps `inputs` from `viewport`, `gap` apart, applying each look.
    fn run(
        machine: &mut Machine,
        viewport: &mut Viewport,
        start: Instant,
        gap: Duration,
        inputs: &[Input],
    ) -> Instant {
        let mut now = start;
        for input in inputs {
            for effect in machine.step(
                *input,
                &View {
                    viewport: *viewport,
                    now,
                },
            ) {
                if let Effect::Look(to) = effect {
                    *viewport = to;
                }
            }
            now += gap;
        }
        now
    }

    fn look(machine: &mut Machine, viewport: Viewport, input: Input, now: Instant) -> Viewport {
        match machine.step(input, &View { viewport, now }).as_slice() {
            [Effect::Look(to)] => *to,
            [] => viewport,
            more => panic!("one look at most: {more:?}"),
        }
    }

    #[test]
    fn the_wheel_pans_by_forty_pixels_a_line_at_any_zoom() {
        let now = Instant::now();
        for zoom in [0.1, 0.5, 1.0, 4.0] {
            let viewport = Viewport {
                x: 10.0,
                y: 20.0,
                zoom,
            };
            let mut machine = Machine::default();
            // Wheel down shows what is below.
            let to = look(&mut machine, viewport, wheel(lines(-1.0), false), now);
            assert_eq!(to.zoom, zoom);
            assert_eq!(to.to_screen((10.0, 20.0)), (0.0, -LINE));
            // Sideways lines pan sideways.
            let mut machine = Machine::default();
            let to = look(
                &mut machine,
                viewport,
                wheel(Scroll::Lines { x: 2.0, y: 0.0 }, false),
                now,
            );
            assert_eq!(to.to_screen((10.0, 20.0)), (2.0 * LINE, 0.0));
        }
    }

    #[test]
    fn shift_turns_a_vertical_line_sideways() {
        let now = Instant::now();
        let viewport = Viewport::default();
        let mut machine = Machine::default();
        let input = Input::Wheel {
            delta: lines(-1.0),
            at: AT,
            zoom: false,
            sideways: true,
            scale_factor: 1.0,
        };
        let to = look(&mut machine, viewport, input, now);
        assert_eq!((to.x, to.y), (LINE, 0.0));
    }

    #[test]
    fn pixels_are_divided_by_the_scale_factor() {
        let now = Instant::now();
        let viewport = Viewport::default();
        for scale_factor in [1.0, 1.5, 2.0] {
            let input = Input::Wheel {
                delta: Scroll::Pixels { x: 0.0, y: -30.0 },
                at: AT,
                zoom: false,
                sideways: false,
                scale_factor,
            };
            let to = look(&mut Machine::default(), viewport, input, now);
            assert!(
                (to.y - 30.0 / scale_factor).abs() < 1e-4,
                "{scale_factor}: {to:?}"
            );
            // As a zoom, the same delta is that share of a line.
            let input = Input::Wheel {
                delta: Scroll::Pixels {
                    x: 0.0,
                    y: 80.0 * scale_factor,
                },
                at: AT,
                zoom: true,
                sideways: false,
                scale_factor,
            };
            let to = look(&mut Machine::default(), viewport, input, now);
            assert!((to.zoom - ZOOM_PER_LINE * ZOOM_PER_LINE).abs() < 1e-5);
        }
    }

    #[test]
    fn a_zoom_step_keeps_the_board_point_under_the_pointer() {
        let start = Instant::now();
        let mut viewport = Viewport {
            x: -40.0,
            y: 125.5,
            zoom: 1.0,
        };
        let mut machine = Machine::default();
        let mut now = start;
        for y in [1.0, 1.0, -3.0, 0.5, 2.0, -1.0, 4.0] {
            let under = viewport.to_board(AT);
            // Bursts apart, so each reads its own modifier.
            viewport = look(&mut machine, viewport, wheel(lines(y), true), now);
            now += BURST * 2;
            let after = viewport.to_screen(under);
            assert!(
                (after.0 - AT.0).abs() < 0.01 && (after.1 - AT.1).abs() < 0.01,
                "{y}: {after:?}"
            );
        }
        assert!((viewport.zoom - ZOOM_PER_LINE.powf(4.5)).abs() < 1e-4);
    }

    #[test]
    fn wheel_zoom_is_clamped_to_ten_and_four_hundred_percent() {
        let start = Instant::now();
        let mut viewport = Viewport::default();
        let mut machine = Machine::default();
        let up = [wheel(lines(10.0), true); 10];
        run(&mut machine, &mut viewport, start, BURST, &up);
        assert_eq!(viewport.zoom, Viewport::MAX_ZOOM);
        // At the clamp a further step neither zooms nor pans.
        let at_max = viewport;
        assert!(machine
            .step(
                wheel(lines(1.0), true),
                &View {
                    viewport,
                    now: start + BURST * 30,
                }
            )
            .is_empty());
        assert_eq!(viewport, at_max);
        let down = [wheel(lines(-10.0), true); 10];
        run(
            &mut machine,
            &mut viewport,
            start + BURST * 40,
            BURST,
            &down,
        );
        assert_eq!(viewport.zoom, Viewport::MIN_ZOOM);
    }

    #[test]
    fn a_burst_keeps_the_choice_of_its_first_event() {
        let start = Instant::now();
        let gap = Duration::from_millis(20);
        let mut viewport = Viewport::default();
        let mut machine = Machine::default();
        // A pan, then the zoom modifier goes down mid-burst: still a pan.
        let end = run(
            &mut machine,
            &mut viewport,
            start,
            gap,
            &[
                wheel(lines(-1.0), false),
                wheel(lines(-1.0), true),
                wheel(lines(-1.0), true),
            ],
        );
        assert_eq!((viewport.y, viewport.zoom), (3.0 * LINE, 1.0));
        // A zoom burst whose modifier comes up, as macOS momentum does:
        // still a zoom.
        let mut viewport = Viewport::default();
        let end = run(
            &mut machine,
            &mut viewport,
            end + BURST,
            gap,
            &[wheel(lines(1.0), true), wheel(lines(1.0), false)],
        );
        assert!((viewport.zoom - ZOOM_PER_LINE * ZOOM_PER_LINE).abs() < 1e-5);
        // Each event extends the burst, so a long run at a short gap is one.
        let mut viewport = Viewport::default();
        let steps = [wheel(lines(-1.0), true); 20];
        let mut now = end + BURST;
        now = run(
            &mut machine,
            &mut viewport,
            now,
            gap,
            &[wheel(lines(-1.0), false)],
        );
        run(&mut machine, &mut viewport, now, gap, &steps);
        assert_eq!((viewport.y, viewport.zoom), (21.0 * LINE, 1.0));
    }

    #[test]
    fn a_new_burst_reads_the_modifier_again() {
        let start = Instant::now();
        let mut machine = Machine::default();
        let viewport = Viewport::default();
        let panned = look(&mut machine, viewport, wheel(lines(-1.0), false), start);
        assert_eq!(panned.zoom, 1.0);
        // Exactly the burst apart is a new burst.
        let zoomed = look(
            &mut machine,
            viewport,
            wheel(lines(1.0), true),
            start + BURST,
        );
        assert!((zoomed.zoom - ZOOM_PER_LINE).abs() < 1e-6);
        // A clock that reads earlier than the last event is still a burst.
        let early = look(&mut machine, viewport, wheel(lines(-1.0), false), start);
        assert!(early.zoom < 1.0);
    }

    #[test]
    fn a_middle_drag_pans_by_the_pointer_motion() {
        let now = Instant::now();
        let mut machine = Machine::default();
        let mut viewport = Viewport {
            x: 0.0,
            y: 0.0,
            zoom: 2.0,
        };
        let under = viewport.to_board((100.0, 100.0));
        run(
            &mut machine,
            &mut viewport,
            now,
            Duration::ZERO,
            &[
                Input::Press {
                    button: Button::Middle,
                    at: (100.0, 100.0),
                },
                Input::Move { at: (130.0, 90.0) },
                Input::Move { at: (160.0, 115.0) },
            ],
        );
        assert!(machine.panning());
        // The board point that was under the pointer still is.
        assert_eq!(viewport.to_screen(under), (160.0, 115.0));
        // A release of another button does not end the drag.
        run(
            &mut machine,
            &mut viewport,
            now,
            Duration::ZERO,
            &[Input::Release {
                button: Button::Left,
            }],
        );
        assert!(machine.panning());
        run(
            &mut machine,
            &mut viewport,
            now,
            Duration::ZERO,
            &[
                Input::Release {
                    button: Button::Middle,
                },
                Input::Move { at: (0.0, 0.0) },
            ],
        );
        assert!(!machine.panning());
        assert_eq!(viewport.to_screen(under), (160.0, 115.0));
    }

    #[test]
    fn a_left_drag_pans_only_with_space_held() {
        let now = Instant::now();
        let mut machine = Machine::default();
        let mut viewport = Viewport::default();
        let drag = [
            Input::Press {
                button: Button::Left,
                at: (10.0, 10.0),
            },
            Input::Move { at: (50.0, 30.0) },
            Input::Release {
                button: Button::Left,
            },
        ];
        run(&mut machine, &mut viewport, now, Duration::ZERO, &drag);
        assert_eq!(viewport, Viewport::default());

        run(
            &mut machine,
            &mut viewport,
            now,
            Duration::ZERO,
            &[Input::Space { held: true }],
        );
        assert!(machine.space());
        run(&mut machine, &mut viewport, now, Duration::ZERO, &drag);
        assert_eq!((viewport.x, viewport.y), (-40.0, -20.0));

        // Space let go mid-drag: the drag goes on to its release.
        let mut viewport = Viewport::default();
        run(
            &mut machine,
            &mut viewport,
            now,
            Duration::ZERO,
            &[drag[0], Input::Space { held: false }, drag[1], drag[2]],
        );
        assert_eq!((viewport.x, viewport.y), (-40.0, -20.0));
        assert!(!machine.panning() && !machine.space());
        // Neither the right button nor another one pans.
        for button in [Button::Right, Button::Other] {
            machine.step(Input::Space { held: true }, &View { viewport, now });
            machine.step(
                Input::Press {
                    button,
                    at: (0.0, 0.0),
                },
                &View { viewport, now },
            );
            assert!(!machine.panning());
        }
    }

    #[test]
    fn cancel_ends_a_drag_and_lets_space_go() {
        let now = Instant::now();
        let mut machine = Machine::default();
        let mut viewport = Viewport::default();
        run(
            &mut machine,
            &mut viewport,
            now,
            Duration::ZERO,
            &[
                Input::Space { held: true },
                Input::Press {
                    button: Button::Left,
                    at: (0.0, 0.0),
                },
                Input::Move { at: (8.0, 0.0) },
                Input::Cancel,
                Input::Move { at: (80.0, 0.0) },
            ],
        );
        assert_eq!(viewport.x, -8.0);
        assert!(!machine.panning() && !machine.space());
    }

    #[test]
    fn a_press_after_a_lost_release_starts_over() {
        let now = Instant::now();
        let middle = Input::Press {
            button: Button::Middle,
            at: (100.0, 100.0),
        };
        // A middle drag whose release never came, then a plain left drag.
        let mut machine = Machine::default();
        let mut viewport = Viewport::default();
        run(
            &mut machine,
            &mut viewport,
            now,
            Duration::ZERO,
            &[
                middle,
                Input::Press {
                    button: Button::Left,
                    at: (600.0, 400.0),
                },
                Input::Move { at: (650.0, 400.0) },
                Input::Release {
                    button: Button::Left,
                },
            ],
        );
        assert_eq!(viewport, Viewport::default());
        assert!(!machine.panning());
        // A second middle drag pans from its own press, with no jump.
        let mut machine = Machine::default();
        run(
            &mut machine,
            &mut viewport,
            now,
            Duration::ZERO,
            &[
                middle,
                Input::Press {
                    button: Button::Middle,
                    at: (600.0, 400.0),
                },
                Input::Move { at: (650.0, 400.0) },
            ],
        );
        assert_eq!((viewport.x, viewport.y), (-50.0, 0.0));
        assert!(machine.panning());
    }

    #[test]
    fn a_wheel_during_a_drag_still_works() {
        let now = Instant::now();
        let mut machine = Machine::default();
        let mut viewport = Viewport::default();
        run(
            &mut machine,
            &mut viewport,
            now,
            Duration::ZERO,
            &[
                Input::Press {
                    button: Button::Middle,
                    at: (0.0, 0.0),
                },
                wheel(lines(-1.0), false),
                Input::Move { at: (-10.0, 0.0) },
            ],
        );
        assert_eq!((viewport.x, viewport.y), (10.0, LINE));
        assert!(machine.panning());
    }

    #[test]
    fn nothing_that_is_not_a_number_moves_the_view() {
        let now = Instant::now();
        let viewport = Viewport::default();
        for delta in [
            Scroll::Lines {
                x: f32::NAN,
                y: 0.0,
            },
            Scroll::Pixels {
                x: 0.0,
                y: f32::INFINITY,
            },
        ] {
            for zoom in [false, true] {
                assert!(Machine::default()
                    .step(wheel(delta, zoom), &View { viewport, now })
                    .is_empty());
            }
        }
        // A scale factor that is no scale counts as 1.
        for scale_factor in [0.0, -2.0, f32::NAN] {
            let input = Input::Wheel {
                delta: Scroll::Pixels { x: 0.0, y: -1.0 },
                at: AT,
                zoom: false,
                sideways: false,
                scale_factor,
            };
            let to = look(&mut Machine::default(), viewport, input, now);
            assert_eq!((to.x, to.y), (0.0, 1.0), "{scale_factor}");
        }
    }

    #[test]
    fn a_step_that_moves_nothing_says_nothing() {
        let now = Instant::now();
        let viewport = Viewport::default();
        let mut machine = Machine::default();
        assert!(machine
            .step(
                wheel(Scroll::Lines { x: 0.0, y: 0.0 }, false),
                &View { viewport, now }
            )
            .is_empty());
        machine.step(
            Input::Press {
                button: Button::Middle,
                at: (5.0, 5.0),
            },
            &View { viewport, now },
        );
        assert!(machine
            .step(Input::Move { at: (5.0, 5.0) }, &View { viewport, now })
            .is_empty());
    }
}
