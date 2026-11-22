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
pub fn required_states(action: Action) -> &'static [ElementState] {
    use ElementState::*;
    match action {
        Action::Click | Action::DblClick | Action::Tap | Action::Check => {
            &[Visible, Enabled, Stable]
        }
        Action::Hover | Action::ScrollIntoView => &[Visible, Stable],
        Action::Fill => &[Visible, Enabled, Editable],
        Action::SelectOption => &[Visible, Enabled],
    }
}

/// Delay before attempt `retry` (1-based), Playwright's `[0, 20, 100, 100, 500]`.
pub fn retry_wait_ms(retry: usize) -> u64 {
    const WAITS: [u64; 5] = [0, 20, 100, 100, 500];
    WAITS[retry.saturating_sub(1).min(WAITS.len() - 1)]
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
    quads: &[[Point; 4]],
    viewport_width: f64,
    viewport_height: f64,
) -> Result<Point, PointError> {
    if quads.is_empty() {
        return Err(PointError::NotVisible);
    }
    let clip = |q: &[Point; 4]| {
        q.map(|p| Point {
            x: p.x.clamp(0.0, viewport_width),
            y: p.y.clamp(0.0, viewport_height),
        })
    };
    let area = |q: &[Point; 4]| {
        let mut a = 0.0;
        for i in 0..4 {
            let (p1, p2) = (q[i], q[(i + 1) % 4]);
            a += (p1.x * p2.y - p2.x * p1.y) / 2.0;
        }
        f64::abs(a)
    };
    let quad = quads
        .iter()
        .map(clip)
        .find(|q| area(q) > 0.99)
        .ok_or(PointError::NotInViewport)?;
    let x = quad.iter().map(|p| p.x).sum::<f64>() / 4.0;
    let y = quad.iter().map(|p| p.y).sum::<f64>() / 4.0;
    Ok(Point {
        x: round(x),
        y: round(y),
    })
}

/// Playwright's `roundPoint` component (JS `Math.round(v * 100) / 100`).
fn round(v: f64) -> f64 {
    (v * 100.0 + 0.5).floor() / 100.0
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
        // Playwright 1.65: actions that wait for "enabled" also wait for "non-inert".
        let pointer = &[Visible, Enabled, Stable, NonInert];
        assert_eq!(required_states(Action::Click), pointer);
        assert_eq!(required_states(Action::DblClick), pointer);
        assert_eq!(required_states(Action::Tap), pointer);
        assert_eq!(required_states(Action::Check), pointer);
        assert_eq!(required_states(Action::Hover), &[Visible, Stable]);
        assert_eq!(
            required_states(Action::Fill),
            &[Visible, Enabled, Editable, NonInert]
        );
        assert_eq!(
            required_states(Action::SelectOption),
            &[Visible, Enabled, NonInert]
        );
        assert_eq!(NonInert.name(), "non-inert");
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
