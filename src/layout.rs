use ratatui::layout::Rect;

use crate::dashboard::{GridPosition, WidgetRuntime};

/// Grid layout system for positioning dashboard widgets.
/// Uses a 12-column grid similar to Bootstrap/web dashboards.
pub struct GridLayout {
    /// Terminal area available for the dashboard
    area: Rect,
    /// Total grid columns (always 12)
    grid_cols: u16,
    /// Total grid rows needed
    grid_rows: u16,
}

impl GridLayout {
    /// Create a new grid layout that scales to fill the available area
    pub fn new_scaled(area: Rect, widgets: &[WidgetRuntime]) -> Self {
        // Find the max grid row needed (y + h)
        let grid_rows = widgets
            .iter()
            .map(|w| w.widget.grid.y + w.widget.grid.h)
            .max()
            .unwrap_or(1);

        Self {
            area,
            grid_cols: 12,
            grid_rows,
        }
    }

    /// Convert a grid position to a terminal Rect using proportional positioning
    pub fn grid_to_rect(&self, grid: &GridPosition) -> Rect {
        // Calculate proportional positions to fill the entire area
        let x = self.area.x + (grid.x as u32 * self.area.width as u32 / self.grid_cols as u32) as u16;
        let y = self.area.y + (grid.y as u32 * self.area.height as u32 / self.grid_rows as u32) as u16;

        let x_end = self.area.x + ((grid.x + grid.w) as u32 * self.area.width as u32 / self.grid_cols as u32) as u16;
        let y_end = self.area.y + ((grid.y + grid.h) as u32 * self.area.height as u32 / self.grid_rows as u32) as u16;

        Rect {
            x,
            y,
            width: x_end.saturating_sub(x),
            height: y_end.saturating_sub(y),
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
