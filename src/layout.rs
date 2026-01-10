use ratatui::layout::Rect;

use crate::dashboard::{GridPosition, WidgetRuntime};

/// Grid layout system for positioning dashboard widgets.
/// Uses a 12-column grid similar to Bootstrap/web dashboards.
pub struct GridLayout {
    /// Terminal area available for the dashboard
    area: Rect,
    /// Width of one grid column in terminal characters
    col_width: u16,
    /// Height of one grid row in terminal lines
    row_height: u16,
}

impl GridLayout {
    pub fn new(area: Rect) -> Self {
        // 12-column grid
        let col_width = area.width / 12;
        // Each grid row = 3 terminal lines
        let row_height = 3;

        Self {
            area,
            col_width,
            row_height,
        }
    }

    /// Convert a grid position to a terminal Rect
    pub fn grid_to_rect(&self, grid: &GridPosition) -> Rect {
        let x = self.area.x + (grid.x * self.col_width);
        let y = self.area.y + (grid.y * self.row_height);
        let width = grid.w * self.col_width;
        let height = grid.h * self.row_height;

        // Clamp to area bounds
        Rect {
            x: x.min(self.area.right()),
            y: y.min(self.area.bottom()),
            width: width.min(self.area.right().saturating_sub(x)),
            height: height.min(self.area.bottom().saturating_sub(y)),
        }
    }

    /// Get all widget positions sorted by y then x for rendering order
    pub fn layout_widgets<'a>(
        &self,
        widgets: &'a [WidgetRuntime],
    ) -> Vec<(&'a WidgetRuntime, Rect)> {
        let mut positioned: Vec<_> = widgets
            .iter()
            .map(|w| (w, self.grid_to_rect(&w.widget.grid)))
            .collect();

        // Sort by y position, then x (for consistent rendering)
        positioned.sort_by(|a, b| {
            let a_pos = &a.0.widget.grid;
            let b_pos = &b.0.widget.grid;
            (a_pos.y, a_pos.x).cmp(&(b_pos.y, b_pos.x))
        });

        positioned
    }
}
