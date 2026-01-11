use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    symbols,
    text::{Line, Span},
    widgets::{Axis, Bar, BarChart, BarGroup, Block, Borders, Cell, Chart, Dataset, GraphType, LegendPosition, Paragraph, Row, Table},
    Frame,
};

use crate::dashboard::{ChartConfig, InsightsResponse, WidgetRuntime, WidgetState};

/// Standard colors for chart series
const SERIES_COLORS: [Color; 5] = [Color::Cyan, Color::Yellow, Color::Green, Color::Magenta, Color::Red];

/// Convert unit name to short suffix
fn format_unit_suffix(unit: Option<&str>) -> &str {
    match unit {
        Some("milliseconds") => "ms",
        Some("seconds") => "s",
        Some("bytes") => "B",
        Some("kilobytes") => "KB",
        Some("megabytes") => "MB",
        Some(u) => u,
        None => "",
    }
}

/// Render a single widget based on its current state
pub fn render_widget(f: &mut Frame, widget: &WidgetRuntime, area: Rect, is_selected: bool) {
    let title = &widget.widget.presentation.title;
    let border_style = if is_selected {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    let block = Block::default()
        .title(title.as_str())
        .borders(Borders::ALL)
        .border_style(border_style);

    match &widget.state {
        WidgetState::Loading => {
            render_loading(f, block, area);
        }
        WidgetState::Error(e) => {
            render_error(f, block, e, area);
        }
        WidgetState::Loaded(response) => {
            let view_type = &widget.widget.config.vis.view;
            let chart_config = &widget.widget.config.vis.chart_config;

            match view_type.as_str() {
                "table" => render_table(f, block, response, area),
                "line" => render_line_chart(f, block, response, chart_config, area),
                "histogram" => render_histogram(f, block, response, chart_config, area),
                _ => render_table(f, block, response, area),
            }
        }
    }
}

fn render_loading(f: &mut Frame, block: Block, area: Rect) {
    let loading = Paragraph::new("Loading...")
        .style(Style::default().fg(Color::Yellow))
        .block(block);
    f.render_widget(loading, area);
}

fn render_error(f: &mut Frame, block: Block, error: &str, area: Rect) {
    // Truncate long error messages
    let max_len = (area.width as usize).saturating_sub(4);
    let truncated = if error.len() > max_len && max_len > 3 {
        format!("{}...", &error[..max_len - 3])
    } else {
        error.to_string()
    };

    let error_widget = Paragraph::new(truncated)
        .style(Style::default().fg(Color::Red))
        .block(block);
    f.render_widget(error_widget, area);
}

fn render_table(f: &mut Frame, block: Block, response: &InsightsResponse, area: Rect) {
    if response.results.is_empty() {
        let empty = Paragraph::new("No data")
            .style(Style::default().fg(Color::DarkGray))
            .block(block);
        f.render_widget(empty, area);
        return;
    }

    // Use metadata.fields if available, otherwise extract from first result
    let fields: Vec<String> = if !response.meta.fields.is_empty() {
        response.meta.fields.clone()
    } else if let Some(first_row) = response.results.first() {
        first_row.keys().cloned().collect()
    } else {
        Vec::new()
    };

    if fields.is_empty() {
        let empty = Paragraph::new("No fields")
            .style(Style::default().fg(Color::DarkGray))
            .block(block);
        f.render_widget(empty, area);
        return;
    }

    // Build header row
    let header_cells: Vec<Cell> = fields
        .iter()
        .map(|f| Cell::from(f.as_str()).style(Style::default().add_modifier(Modifier::BOLD)))
        .collect();
    let header = Row::new(header_cells).height(1);

    // Build data rows - leave room for borders and header
    let max_rows = area.height.saturating_sub(4) as usize;
    let rows: Vec<Row> = response
        .results
        .iter()
        .take(max_rows)
        .map(|row| {
            let cells: Vec<Cell> = fields
                .iter()
                .map(|field| {
                    let value = row
                        .get(field)
                        .map(|v| format_json_value(v))
                        .unwrap_or_default();
                    Cell::from(value)
                })
                .collect();
            Row::new(cells)
        })
        .collect();

    // Calculate column widths - distribute evenly
    let col_count = fields.len();
    let available_width = area.width.saturating_sub(2); // borders
    let col_width = (available_width / col_count as u16).max(5);
    let widths: Vec<Constraint> = (0..col_count)
        .map(|_| Constraint::Length(col_width))
        .collect();

    let table = Table::new(rows, widths)
        .header(header)
        .block(block)
        .row_highlight_style(Style::default().bg(Color::DarkGray));

    f.render_widget(table, area);
}

fn render_line_chart(
    f: &mut Frame,
    block: Block,
    response: &InsightsResponse,
    config: &ChartConfig,
    area: Rect,
) {
    if response.results.is_empty() {
        let empty = Paragraph::new("No data")
            .style(Style::default().fg(Color::DarkGray))
            .block(block);
        f.render_widget(empty, area);
        return;
    }

    let x_field = config.x_field.as_deref().unwrap_or("x");
    let y_field = config.y_field.as_deref().unwrap_or("y");
    let z_field = config.z_field.as_deref(); // grouping field
    let y_unit = format_unit_suffix(config.y_field_unit.as_deref());

    // Collect data points and x labels, grouped by z_field if present
    let mut series: std::collections::HashMap<String, Vec<(f64, f64)>> = std::collections::HashMap::new();
    let mut x_labels_raw: Vec<String> = Vec::new();

    for (i, row) in response.results.iter().enumerate() {
        let x = i as f64;
        let y = row
            .get(y_field)
            .and_then(|v| v.as_f64().or_else(|| v.as_str().and_then(|s| s.parse().ok())))
            .unwrap_or(0.0);

        // Collect x labels from the first series we encounter
        if let Some(x_val) = row.get(x_field) {
            let label = format_json_value(x_val);
            // Extract just the time portion if it's a timestamp
            let short_label = if label.contains(' ') {
                label.split(' ').last().unwrap_or(&label).to_string()
            } else {
                label
            };
            if x_labels_raw.len() <= i {
                x_labels_raw.push(short_label);
            }
        }

        let group = z_field
            .and_then(|zf| row.get(zf))
            .map(|v| format_json_value(v))
            .unwrap_or_else(|| y_field.to_string());

        series.entry(group).or_default().push((x, y));
    }

    if series.is_empty() || series.values().all(|v| v.is_empty()) {
        let empty = Paragraph::new("No data points")
            .style(Style::default().fg(Color::DarkGray))
            .block(block);
        f.render_widget(empty, area);
        return;
    }

    // Find bounds
    let all_points: Vec<&(f64, f64)> = series.values().flat_map(|v| v.iter()).collect();
    let x_min = all_points.iter().map(|(x, _)| *x).fold(f64::INFINITY, f64::min);
    let x_max = all_points.iter().map(|(x, _)| *x).fold(f64::NEG_INFINITY, f64::max);
    let y_min = 0.0;
    let y_max = all_points.iter().map(|(_, y)| *y).fold(f64::NEG_INFINITY, f64::max) * 1.1;

    // Generate Y-axis labels (5 values from min to max)
    let y_labels: Vec<Span> = (0..=4)
        .map(|i| {
            let val = y_min + (y_max - y_min) * (i as f64 / 4.0);
            Span::styled(
                format!("{:.0}{}", val, y_unit),
                Style::default().fg(Color::DarkGray),
            )
        })
        .collect();

    // Generate X-axis labels (pick ~5 evenly spaced)
    let x_labels: Vec<Span> = if !x_labels_raw.is_empty() {
        let step = (x_labels_raw.len() / 5).max(1);
        x_labels_raw
            .iter()
            .enumerate()
            .filter(|(i, _)| i % step == 0 || *i == x_labels_raw.len() - 1)
            .map(|(_, label)| Span::styled(label.clone(), Style::default().fg(Color::DarkGray)))
            .take(6)
            .collect()
    } else {
        vec![]
    };

    // Sort series by average value descending (highest first)
    let mut sorted_series: Vec<_> = series.into_iter().collect();
    sorted_series.sort_by(|a, b| {
        let avg_a = if a.1.is_empty() { 0.0 } else { a.1.iter().map(|(_, y)| y).sum::<f64>() / a.1.len() as f64 };
        let avg_b = if b.1.is_empty() { 0.0 } else { b.1.iter().map(|(_, y)| y).sum::<f64>() / b.1.len() as f64 };
        avg_b.partial_cmp(&avg_a).unwrap_or(std::cmp::Ordering::Equal)
    });

    let has_multiple_series = sorted_series.len() > 1;

    // Create datasets
    let datasets: Vec<Dataset> = sorted_series
        .iter()
        .enumerate()
        .map(|(i, (name, points))| {
            Dataset::default()
                .name(name.clone())
                .marker(symbols::Marker::Braille)
                .graph_type(GraphType::Line)
                .style(Style::default().fg(SERIES_COLORS[i % SERIES_COLORS.len()]))
                .data(points)
        })
        .collect();

    let mut chart = Chart::new(datasets)
        .block(block)
        .x_axis(
            Axis::default()
                .style(Style::default().fg(Color::Gray))
                .bounds([x_min, x_max.max(x_min + 1.0)])
                .labels(x_labels),
        )
        .y_axis(
            Axis::default()
                .style(Style::default().fg(Color::Gray))
                .bounds([y_min, y_max.max(1.0)])
                .labels(y_labels),
        );

    // Add legend for multi-series charts
    if has_multiple_series {
        chart = chart.legend_position(Some(LegendPosition::TopRight));
    }

    f.render_widget(chart, area);
}

fn render_histogram(
    f: &mut Frame,
    block: Block,
    response: &InsightsResponse,
    config: &ChartConfig,
    area: Rect,
) {
    if response.results.is_empty() {
        let empty = Paragraph::new("No data")
            .style(Style::default().fg(Color::DarkGray))
            .block(block);
        f.render_widget(empty, area);
        return;
    }

    // Extract x and y fields from config
    let x_field = config.x_field.as_deref().unwrap_or("x");
    let y_field = config.y_field.as_deref().unwrap_or("y");
    let x_unit = format_unit_suffix(config.x_field_unit.as_deref());

    // Build bar data
    let bars: Vec<Bar> = response
        .results
        .iter()
        .filter_map(|row| {
            let x = row.get(x_field).map(|v| format_json_value(v))?;
            let y = row.get(y_field).and_then(|v| v.as_f64()).unwrap_or(0.0) as u64;
            // Add unit suffix and truncate if too long
            let label_with_unit = format!("{}{}", x, x_unit);
            let label = if label_with_unit.len() > 8 {
                label_with_unit[..8].to_string()
            } else {
                label_with_unit
            };
            Some(Bar::default().label(label.into()).value(y))
        })
        .take((area.width.saturating_sub(4) / 5) as usize) // Rough fit
        .collect();

    if bars.is_empty() {
        let empty = Paragraph::new("No data")
            .style(Style::default().fg(Color::DarkGray))
            .block(block);
        f.render_widget(empty, area);
        return;
    }

    let bar_chart = BarChart::default()
        .block(block)
        .data(BarGroup::default().bars(&bars))
        .bar_width(4)
        .bar_gap(1)
        .bar_style(Style::default().fg(Color::Cyan))
        .value_style(Style::default().fg(Color::White));

    f.render_widget(bar_chart, area);
}

/// Format a JSON value for display
fn format_json_value(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Number(n) => {
            if let Some(f) = n.as_f64() {
                if f.fract() == 0.0 {
                    format!("{}", f as i64)
                } else {
                    format!("{:.2}", f)
                }
            } else {
                n.to_string()
            }
        }
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Null => "null".to_string(),
        _ => value.to_string(),
    }
}

/// Extract series names from a line chart's data for legend display, sorted by average value (highest first)
fn extract_series_names(response: &InsightsResponse, config: &ChartConfig) -> Vec<String> {
    let z_field = config.z_field.as_deref();
    let y_field = config.y_field.as_deref().unwrap_or("y");

    if z_field.is_none() {
        return vec![];
    }

    // Collect series with their y-values (sum and count for averaging)
    let mut series_stats: std::collections::HashMap<String, (f64, usize)> = std::collections::HashMap::new();

    for row in &response.results {
        let group = z_field
            .and_then(|zf| row.get(zf))
            .map(|v| format_json_value(v))
            .unwrap_or_else(|| y_field.to_string());

        let y = row
            .get(y_field)
            .and_then(|v| v.as_f64().or_else(|| v.as_str().and_then(|s| s.parse().ok())))
            .unwrap_or(0.0);

        let entry = series_stats.entry(group).or_insert((0.0, 0));
        entry.0 += y;
        entry.1 += 1;
    }

    // Sort by average value descending
    let mut sorted: Vec<_> = series_stats.into_iter().collect();
    sorted.sort_by(|a, b| {
        let avg_a = if a.1.1 == 0 { 0.0 } else { a.1.0 / a.1.1 as f64 };
        let avg_b = if b.1.1 == 0 { 0.0 } else { b.1.0 / b.1.1 as f64 };
        avg_b.partial_cmp(&avg_a).unwrap_or(std::cmp::Ordering::Equal)
    });

    let names: Vec<String> = sorted.into_iter().map(|(name, _)| name).collect();

    // If only one series and it matches the y_field, don't show legend
    if names.len() <= 1 && names.first().map(|s| s.as_str()) == Some(y_field) {
        return vec![];
    }

    names
}

/// Render the legend for a line chart with colored squares
fn render_legend(f: &mut Frame, series_names: &[String], area: Rect) {
    if series_names.is_empty() || area.height < 1 {
        return;
    }

    // Build spans with colored squares and names
    let mut spans: Vec<Span> = Vec::new();
    for (i, name) in series_names.iter().enumerate() {
        let color = SERIES_COLORS[i % SERIES_COLORS.len()];
        // Add spacing between entries
        if i > 0 {
            spans.push(Span::raw("  "));
        }
        // Colored square (using a filled block character)
        spans.push(Span::styled("■ ", Style::default().fg(color)));
        // Series name
        spans.push(Span::raw(name.clone()));
    }

    // Wrap into lines if needed (simple wrapping based on terminal width)
    let mut lines: Vec<Line> = Vec::new();
    let mut current_spans: Vec<Span> = Vec::new();
    let mut current_width: usize = 0;
    let max_width = area.width.saturating_sub(2) as usize;

    for (i, name) in series_names.iter().enumerate() {
        let color = SERIES_COLORS[i % SERIES_COLORS.len()];
        let entry_width = 2 + name.len() + 2; // "■ " + name + "  "

        if current_width + entry_width > max_width && !current_spans.is_empty() {
            lines.push(Line::from(current_spans));
            current_spans = Vec::new();
            current_width = 0;
        }

        if !current_spans.is_empty() {
            current_spans.push(Span::raw("  "));
            current_width += 2;
        }
        current_spans.push(Span::styled("■ ", Style::default().fg(color)));
        current_spans.push(Span::raw(name.clone()));
        current_width += 2 + name.len();
    }

    if !current_spans.is_empty() {
        lines.push(Line::from(current_spans));
    }

    let paragraph = Paragraph::new(lines)
        .block(Block::default().borders(Borders::TOP));
    f.render_widget(paragraph, area);
}

/// Render a maximized widget with legend below (for line charts with z-field)
pub fn render_maximized_widget(f: &mut Frame, widget: &WidgetRuntime, area: Rect) {
    let title = &widget.widget.presentation.title;
    let view_type = &widget.widget.config.vis.view;

    // Check if this is a line chart with series data
    let series_names = if view_type == "line" {
        if let WidgetState::Loaded(response) = &widget.state {
            extract_series_names(response, &widget.widget.config.vis.chart_config)
        } else {
            vec![]
        }
    } else {
        vec![]
    };

    // Calculate legend height needed (lines of legend text)
    let legend_height = if series_names.is_empty() {
        0
    } else {
        // Estimate lines needed based on total text width
        let total_width: usize = series_names.iter().map(|n| n.len() + 4).sum();
        let lines_needed = (total_width / area.width.saturating_sub(4) as usize).max(1) + 1;
        (lines_needed as u16).min(4) + 1 // +1 for border, cap at 5 total
    };

    // Split area for chart and legend
    let (chart_area, legend_area) = if legend_height > 0 {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(10),
                Constraint::Length(legend_height),
            ])
            .split(area);
        (chunks[0], Some(chunks[1]))
    } else {
        (area, None)
    };

    // Render the widget in the main area
    let border_style = Style::default().fg(Color::Cyan);
    let block = Block::default()
        .title(title.as_str())
        .borders(Borders::ALL)
        .border_style(border_style);

    match &widget.state {
        WidgetState::Loading => {
            render_loading(f, block, chart_area);
        }
        WidgetState::Error(e) => {
            render_error(f, block, e, chart_area);
        }
        WidgetState::Loaded(response) => {
            let chart_config = &widget.widget.config.vis.chart_config;
            match view_type.as_str() {
                "table" => render_table(f, block, response, chart_area),
                "line" => render_line_chart(f, block, response, chart_config, chart_area),
                "histogram" => render_histogram(f, block, response, chart_config, chart_area),
                _ => render_table(f, block, response, chart_area),
            }
        }
    }

    // Render legend if we have series names
    if let Some(legend_rect) = legend_area {
        render_legend(f, &series_names, legend_rect);
    }
}
