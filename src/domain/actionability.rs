//! Actionability rules (Playwright's `dom.ts`): which checks an action needs,
//! how long to wait between attempts, and where to click.

/// Element states the injected script can check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ElementState {
    Visible,
    Hidden,
    Enabled,
    Disabled,
    Editable,
    Checked,
    Unchecked,
    Stable,
}

impl ElementState {
    pub fn name(self) -> &'static str {
        match self {
            ElementState::Visible => "visible",
            ElementState::Hidden => "hidden",
            ElementState::Enabled => "enabled",
            ElementState::Disabled => "disabled",
            ElementState::Editable => "editable",
            ElementState::Checked => "checked",
            ElementState::Unchecked => "unchecked",
            ElementState::Stable => "stable",
        }
    }
}

/// Element actions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Click,
    DblClick,
    Tap,
    Hover,
    Check,
    Fill,
    SelectOption,
    ScrollIntoView,
}

/// The checks Playwright performs before an action.
pub fn required_states(_action: Action) -> &'static [ElementState] {
    unimplemented!()
}

/// Delay before attempt `retry` (1-based), Playwright's `[0, 20, 100, 100, 500]`.
pub fn retry_wait_ms(_retry: usize) -> u64 {
    unimplemented!()
}

/// A point in CSS pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

/// Why no point could be found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointError {
    /// No content quads (no layout).
    NotVisible,
    /// Quads exist but none overlaps the viewport.
    NotInViewport,
}

/// Playwright's `_clickablePoint`: middle of the first content quad that
/// overlaps the viewport by more than ~1 px², rounded to 1/100 px.
pub fn clickable_point(
    _quads: &[[Point; 4]],
    _viewport_width: f64,
    _viewport_height: f64,
) -> Result<Point, PointError> {
    unimplemented!()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ElementState::*;

    fn quad(x: f64, y: f64, w: f64, h: f64) -> [Point; 4] {
        [
            Point { x, y },
            Point { x: x + w, y },
            Point { x: x + w, y: y + h },
            Point { x, y: y + h },
        ]
    }

    #[test]
    fn states_per_action_follow_playwright() {
        assert_eq!(required_states(Action::Click), &[Visible, Enabled, Stable]);
        assert_eq!(
            required_states(Action::DblClick),
            &[Visible, Enabled, Stable]
        );
        assert_eq!(required_states(Action::Tap), &[Visible, Enabled, Stable]);
        assert_eq!(required_states(Action::Check), &[Visible, Enabled, Stable]);
        assert_eq!(required_states(Action::Hover), &[Visible, Stable]);
        assert_eq!(required_states(Action::Fill), &[Visible, Enabled, Editable]);
        assert_eq!(required_states(Action::SelectOption), &[Visible, Enabled]);
        assert_eq!(required_states(Action::ScrollIntoView), &[Visible, Stable]);
    }

    #[test]
    fn retry_waits() {
        let waits: Vec<u64> = (1..=7).map(retry_wait_ms).collect();
        assert_eq!(waits, vec![0, 20, 100, 100, 500, 500, 500]);
    }

    #[test]
    fn clickable_point_is_the_middle_of_the_visible_quad() {
        let p = clickable_point(&[quad(10.0, 20.0, 100.0, 40.0)], 800.0, 600.0).unwrap();
        assert_eq!(p, Point { x: 60.0, y: 40.0 });
        // Off-screen quads are skipped; partially visible ones are clipped first.
        let p = clickable_point(
            &[quad(-500.0, 0.0, 10.0, 10.0), quad(790.0, 0.0, 20.0, 20.0)],
            800.0,
            600.0,
        )
        .unwrap();
        assert_eq!(p, Point { x: 795.0, y: 10.0 });
        assert_eq!(
            clickable_point(&[], 800.0, 600.0),
            Err(PointError::NotVisible)
        );
        assert_eq!(
            clickable_point(&[quad(900.0, 0.0, 10.0, 10.0)], 800.0, 600.0),
            Err(PointError::NotInViewport)
        );
        let p = clickable_point(&[quad(0.0, 0.0, 10.333, 10.0)], 800.0, 600.0).unwrap();
        assert_eq!(p.x, 5.17);
    }
}
