//! Responsive top-level terminal geometry.

use ratatui_core::layout::Rect;

const NORMAL_WIDTH: u16 = 80;
const SHORT_HEIGHT: u16 = 10;
const NORMAL_HEIGHT: u16 = 16;
const TALL_HEIGHT: u16 = 28;

/// The stable height tiers used by every root-level layout decision.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum HeightTier {
    Tiny,
    Short,
    Normal,
    Tall,
}

/// Terminal dimensions classified once for the current frame.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ViewportProfile {
    width: u16,
    height: u16,
    height_tier: HeightTier,
}

impl ViewportProfile {
    pub(crate) const fn new(area: Rect) -> Self {
        let height_tier = match area.height {
            ..SHORT_HEIGHT => HeightTier::Tiny,
            SHORT_HEIGHT..NORMAL_HEIGHT => HeightTier::Short,
            NORMAL_HEIGHT..TALL_HEIGHT => HeightTier::Normal,
            TALL_HEIGHT.. => HeightTier::Tall,
        };
        Self {
            width: area.width,
            height: area.height,
            height_tier,
        }
    }

    pub(crate) const fn width(self) -> u16 {
        self.width
    }

    pub(crate) const fn height(self) -> u16 {
        self.height
    }

    pub(crate) const fn is_narrow(self) -> bool {
        self.width < NORMAL_WIDTH
    }

    pub(crate) const fn is_short_or_tiny(self) -> bool {
        matches!(self.height_tier, HeightTier::Tiny | HeightTier::Short)
    }

    pub(crate) const fn footer_row_budget(self, library: bool) -> usize {
        match (self.height_tier, library) {
            (HeightTier::Tall, _) => usize::MAX,
            (HeightTier::Normal, true) => 6,
            (HeightTier::Normal, false) => 3,
            (HeightTier::Short, true) => 2,
            _ => 1,
        }
    }

    /// Return the last height in the preceding tier.
    pub(crate) const fn previous_tier_max_height(self) -> Option<u16> {
        match self.height_tier {
            HeightTier::Tiny => None,
            HeightTier::Short => Some(SHORT_HEIGHT - 1),
            HeightTier::Normal => Some(NORMAL_HEIGHT - 1),
            HeightTier::Tall => Some(TALL_HEIGHT - 1),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ViewAreas {
    pub(crate) header: Rect,
    pub(crate) body: Rect,
    pub(crate) footer: Rect,
}

/// One root allocation with its resolved footer presentation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RootLayoutPlan {
    pub(crate) areas: ViewAreas,
    pub(crate) footer_decorated: bool,
}

impl RootLayoutPlan {
    /// Preserve the primary body, allocate the header, and use only spare rows for the footer.
    pub(crate) const fn new(
        area: Rect,
        preferred_header_height: u16,
        preferred_footer_height: u16,
        minimum_body_height: u16,
        decorated_footer_minimum: u16,
    ) -> Self {
        let body_floor = if minimum_body_height < area.height {
            minimum_body_height
        } else {
            area.height
        };
        let header_capacity = area.height.saturating_sub(body_floor);
        let header_height = if preferred_header_height < header_capacity {
            preferred_header_height
        } else {
            header_capacity
        };
        let after_header = area.height.saturating_sub(header_height);
        let footer_capacity = after_header.saturating_sub(body_floor);
        let footer_height = if preferred_footer_height < footer_capacity {
            preferred_footer_height
        } else {
            footer_capacity
        };
        let body_height = after_header.saturating_sub(footer_height);
        let header = Rect::new(area.x, area.y, area.width, header_height);
        let body = Rect::new(
            area.x,
            area.y.saturating_add(header_height),
            area.width,
            body_height,
        );
        let footer = Rect::new(
            area.x,
            body.y.saturating_add(body_height),
            area.width,
            footer_height,
        );
        Self {
            areas: ViewAreas {
                header,
                body,
                footer,
            },
            footer_decorated: decorated_footer_minimum > 0
                && footer_height >= decorated_footer_minimum,
        }
    }
}

/// Report whether the terminal is below the normal-width tier.
pub(crate) const fn is_narrow(width: u16) -> bool {
    ViewportProfile::new(Rect::new(0, 0, width, 0)).is_narrow()
}

/// Report whether the terminal is below the normal-height tier.
pub(crate) const fn is_short(height: u16) -> bool {
    ViewportProfile::new(Rect::new(0, 0, 0, height)).is_short_or_tiny()
}

#[cfg(test)]
mod reviewer_contracts {
    use super::*;

    #[test]
    fn primary_body_has_priority_when_root_chrome_does_not_fit() {
        for height in 0..=6 {
            let plan = RootLayoutPlan::new(Rect::new(0, 0, 80, height), 3, 9, 6, 3);
            assert_eq!(plan.areas.body.height, height);
            assert_eq!(plan.areas.header.height, 0);
            assert_eq!(plan.areas.footer.height, 0);
        }
    }

    #[test]
    fn height_tier_budgets_change_only_at_the_documented_boundaries() {
        let profile = |height| ViewportProfile::new(Rect::new(0, 0, 80, height));
        assert_eq!(profile(9).footer_row_budget(true), 1);
        assert_eq!(profile(9).footer_row_budget(false), 1);
        assert_eq!(profile(10).footer_row_budget(true), 2);
        assert_eq!(profile(10).footer_row_budget(false), 1);
        assert_eq!(profile(15).footer_row_budget(true), 2);
        assert_eq!(profile(15).footer_row_budget(false), 1);
        assert_eq!(profile(16).footer_row_budget(true), 6);
        assert_eq!(profile(16).footer_row_budget(false), 3);
        assert_eq!(profile(27).footer_row_budget(true), 6);
        assert_eq!(profile(27).footer_row_budget(false), 3);
        assert_eq!(profile(28).footer_row_budget(true), usize::MAX);
        assert_eq!(profile(28).footer_row_budget(false), usize::MAX);
    }

    #[test]
    fn each_height_tier_reports_the_exact_preceding_endpoint() {
        let profile = |height| ViewportProfile::new(Rect::new(0, 0, 80, height));
        assert_eq!(profile(0).previous_tier_max_height(), None);
        assert_eq!(profile(9).previous_tier_max_height(), None);
        assert_eq!(profile(10).previous_tier_max_height(), Some(9));
        assert_eq!(profile(15).previous_tier_max_height(), Some(9));
        assert_eq!(profile(16).previous_tier_max_height(), Some(15));
        assert_eq!(profile(27).previous_tier_max_height(), Some(15));
        assert_eq!(profile(28).previous_tier_max_height(), Some(27));
    }

    #[test]
    fn root_plan_tiles_every_zero_through_boundary_dimension() {
        for width in 0..=80 {
            for height in 0..=28 {
                for (header, footer, body) in [(0, 0, 0), (1, 3, 6), (3, 9, 6)] {
                    let area = Rect::new(7, 11, width, height);
                    let plan = RootLayoutPlan::new(area, header, footer, body, 3);
                    let [head, main, foot] =
                        [plan.areas.header, plan.areas.body, plan.areas.footer];
                    assert_eq!(head.x, area.x);
                    assert_eq!(main.x, area.x);
                    assert_eq!(foot.x, area.x);
                    assert_eq!(head.width, area.width);
                    assert_eq!(main.width, area.width);
                    assert_eq!(foot.width, area.width);
                    assert_eq!(head.y, area.y);
                    assert_eq!(main.y, head.bottom());
                    assert_eq!(foot.y, main.bottom());
                    assert_eq!(foot.bottom(), area.bottom());
                    assert!(main.height >= body.min(area.height));
                }
            }
        }
    }
}
