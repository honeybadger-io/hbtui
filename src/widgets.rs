use ratatui::{
    layout::{Constraint, Rect},
    style::{Color, Modifier, Style},
    symbols,
    text::Span,
    widgets::{Axis, Bar, BarChart, BarGroup, Block, Borders, Cell, Chart, Dataset, GraphType, Paragraph, Row, Table},
    Frame,
};

use crate::dashboard::{ChartConfig, InsightsResponse, WidgetRuntime, WidgetState};

/// Render a single widget based on its current state
pub fn render_widget(f: &mut Frame, widget: &WidgetRuntime, area: Rect) {
    let title = &widget.widget.presentation.title;
    let block = Block::default()
        .title(title.as_str())
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray));

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

    // Collect data points, grouped by z_field if present
    let mut series: std::collections::HashMap<String, Vec<(f64, f64)>> = std::collections::HashMap::new();

    for (i, row) in response.results.iter().enumerate() {
        let x = i as f64; // Use index as x for simplicity
        let y = row
            .get(y_field)
            .and_then(|v| v.as_f64().or_else(|| v.as_str().and_then(|s| s.parse().ok())))
            .unwrap_or(0.0);

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
    let y_min = 0.0; // Start from 0 for charts
    let y_max = all_points.iter().map(|(_, y)| *y).fold(f64::NEG_INFINITY, f64::max) * 1.1;

    // Colors for different series
    let colors = [Color::Cyan, Color::Yellow, Color::Green, Color::Magenta, Color::Red];

    // Sort series by name for stable ordering (prevents color flickering)
    let mut sorted_series: Vec<_> = series.into_iter().collect();
    sorted_series.sort_by(|a, b| a.0.cmp(&b.0));

    // Create datasets
    let datasets: Vec<Dataset> = sorted_series
        .iter()
        .enumerate()
        .map(|(i, (name, points))| {
            Dataset::default()
                .name(name.clone())
                .marker(symbols::Marker::Braille)
                .graph_type(GraphType::Line)
                .style(Style::default().fg(colors[i % colors.len()]))
                .data(points)
        })
        .collect();

    let chart = Chart::new(datasets)
        .block(block)
        .x_axis(
            Axis::default()
                .style(Style::default().fg(Color::Gray))
                .bounds([x_min, x_max.max(x_min + 1.0)]),
        )
        .y_axis(
            Axis::default()
                .style(Style::default().fg(Color::Gray))
                .bounds([y_min, y_max.max(1.0)])
                .labels(vec![
                    Span::raw(format!("{:.0}", y_min)),
                    Span::raw(format!("{:.0}", y_max)),
                ]),
        );

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

    // Build bar data
    let bars: Vec<Bar> = response
        .results
        .iter()
        .filter_map(|row| {
            let x = row.get(x_field).map(|v| format_json_value(v))?;
            let y = row.get(y_field).and_then(|v| v.as_f64()).unwrap_or(0.0) as u64;
            // Truncate label if too long
            let label = if x.len() > 8 { &x[..8] } else { &x };
            Some(Bar::default().label(label.to_string().into()).value(y))
        })
        .take((area.width.saturating_sub(4) / 4) as usize) // Rough fit
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
        .bar_width(3)
        .bar_gap(1)
        .bar_style(Style::default().fg(Color::Cyan));

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
