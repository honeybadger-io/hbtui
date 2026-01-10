use ratatui::layout::Rect;

use crate::dashboard::{GridPosition, WidgetRuntime};

/// Direction for widget navigation
#[derive(Debug, Clone, Copy)]
pub enum NavDirection {
    Up,
    Down,
    Left,
    Right,
}

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

/// Check if two ranges overlap
fn ranges_overlap(a_start: u16, a_end: u16, b_start: u16, b_end: u16) -> bool {
    a_start < b_end && b_start < a_end
}

/// Find the best widget to navigate to from the current widget in the given direction
pub fn find_adjacent_widget(
    widgets: &[WidgetRuntime],
    current_idx: usize,
    direction: NavDirection,
) -> Option<usize> {
    if widgets.is_empty() || current_idx >= widgets.len() {
        return None;
    }

    let current = &widgets[current_idx].widget.grid;
    let current_center_x = current.x as f32 + current.w as f32 / 2.0;
    let current_center_y = current.y as f32 + current.h as f32 / 2.0;

    let mut best_idx: Option<usize> = None;
    let mut best_score = f32::MAX;
    let mut best_overlaps = false;

    for (idx, widget) in widgets.iter().enumerate() {
        if idx == current_idx {
            continue;
        }

        let grid = &widget.widget.grid;
        let center_x = grid.x as f32 + grid.w as f32 / 2.0;
        let center_y = grid.y as f32 + grid.h as f32 / 2.0;

        // Check if this widget is in the right direction
        let is_valid = match direction {
            NavDirection::Up => center_y < current_center_y,
            NavDirection::Down => center_y > current_center_y,
            NavDirection::Left => center_x < current_center_x,
            NavDirection::Right => center_x > current_center_x,
        };

        if !is_valid {
            continue;
        }

        // Check if widgets overlap in the perpendicular dimension
        // For left/right: check y-range overlap (same row)
        // For up/down: check x-range overlap (same column)
        let overlaps = match direction {
            NavDirection::Left | NavDirection::Right => ranges_overlap(
                current.y,
                current.y + current.h,
                grid.y,
                grid.y + grid.h,
            ),
            NavDirection::Up | NavDirection::Down => ranges_overlap(
                current.x,
                current.x + current.w,
                grid.x,
                grid.x + grid.w,
            ),
        };

        // Score based on distance in primary direction + penalty for perpendicular distance
        let score = match direction {
            NavDirection::Up | NavDirection::Down => {
                let primary = (center_y - current_center_y).abs();
                let perpendicular = (center_x - current_center_x).abs();
                primary + perpendicular * 0.5
            }
            NavDirection::Left | NavDirection::Right => {
                let primary = (center_x - current_center_x).abs();
                let perpendicular = (center_y - current_center_y).abs();
                primary + perpendicular * 0.5
            }
        };

        // Prefer overlapping widgets; only consider non-overlapping if no overlapping found
        if overlaps && !best_overlaps {
            // First overlapping widget found, always take it
            best_overlaps = true;
            best_score = score;
            best_idx = Some(idx);
        } else if overlaps == best_overlaps && score < best_score {
            // Same overlap status, pick better score
            best_score = score;
            best_idx = Some(idx);
        }
    }

    best_idx
}
