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
                "billboard" => render_billboard(f, block, response, chart_config, area),
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

    // Sort series by average value descending (highest first), then by name for stability
    let mut sorted_series: Vec<_> = series.into_iter().collect();
    sorted_series.sort_by(|a, b| {
        let avg_a = if a.1.is_empty() { 0.0 } else { a.1.iter().map(|(_, y)| y).sum::<f64>() / a.1.len() as f64 };
        let avg_b = if b.1.is_empty() { 0.0 } else { b.1.iter().map(|(_, y)| y).sum::<f64>() / b.1.len() as f64 };
        avg_b.partial_cmp(&avg_a)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.0.cmp(&b.0))
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
    // Normal view always shows stacked histogram (no filter)
    render_histogram_with_filter(f, block, response, config, area, None);
}

/// Render histogram with optional series filter
/// series_filter: None = stacked (all series), Some(i) = show only series i
pub fn render_histogram_with_filter(
    f: &mut Frame,
    block: Block,
    response: &InsightsResponse,
    config: &ChartConfig,
    area: Rect,
    series_filter: Option<usize>,
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
    let z_field = config.z_field.as_deref();
    let x_unit = format_unit_suffix(config.x_field_unit.as_deref());

    if let Some(z) = z_field {
        match series_filter {
            None => {
                // Stacked histogram - all series in stacked bars
                render_stacked_histogram(f, block, response, x_field, y_field, z, x_unit, area);
            }
            Some(idx) => {
                // Filtered histogram - show only one series
                render_filtered_histogram(f, block, response, x_field, y_field, z, x_unit, idx, area);
            }
        }
    } else {
        // Simple histogram - no grouping
        render_simple_histogram(f, block, response, x_field, y_field, x_unit, area);
    }
}

fn render_simple_histogram(
    f: &mut Frame,
    block: Block,
    response: &InsightsResponse,
    x_field: &str,
    y_field: &str,
    x_unit: &str,
    area: Rect,
) {
    let bars: Vec<Bar> = response
        .results
        .iter()
        .filter_map(|row| {
            let x = row.get(x_field).map(|v| format_json_value(v))?;
            let y = row.get(y_field).and_then(|v| v.as_f64()).unwrap_or(0.0) as u64;
            let label_with_unit = format!("{}{}", x, x_unit);
            let label = if label_with_unit.len() > 8 {
                label_with_unit[..8].to_string()
            } else {
                label_with_unit
            };
            Some(Bar::default().label(label.into()).value(y))
        })
        .take((area.width.saturating_sub(4) / 5) as usize)
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

/// Shared data structure for histogram rendering
struct HistogramData {
    x_values: Vec<String>,
    series_names: Vec<String>,
    data: std::collections::HashMap<(String, String), u64>,
}

/// Collect and organize histogram data from response
fn collect_histogram_data(
    response: &InsightsResponse,
    x_field: &str,
    y_field: &str,
    z_field: &str,
) -> HistogramData {
    let mut x_values: Vec<String> = Vec::new();
    let mut series_stats: std::collections::HashMap<String, (f64, usize)> = std::collections::HashMap::new();
    let mut data: std::collections::HashMap<(String, String), u64> = std::collections::HashMap::new();

    for row in &response.results {
        let x = row.get(x_field).map(|v| format_json_value(v)).unwrap_or_default();
        let y = row.get(y_field).and_then(|v| v.as_f64()).unwrap_or(0.0);
        let z = row.get(z_field).map(|v| format_json_value(v)).unwrap_or_default();

        if !x_values.contains(&x) {
            x_values.push(x.clone());
        }
        let entry = series_stats.entry(z.clone()).or_insert((0.0, 0));
        entry.0 += y;
        entry.1 += 1;
        data.insert((x, z), y as u64);
    }

    // Sort series by average value descending, then by name (matching legend sort order)
    let mut series: Vec<_> = series_stats.into_iter().collect();
    series.sort_by(|a, b| {
        let avg_a = if a.1.1 == 0 { 0.0 } else { a.1.0 / a.1.1 as f64 };
        let avg_b = if b.1.1 == 0 { 0.0 } else { b.1.0 / b.1.1 as f64 };
        avg_b.partial_cmp(&avg_a)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.0.cmp(&b.0))
    });
    let series_names: Vec<String> = series.into_iter().map(|(name, _)| name).collect();

    // Sort x values numerically if possible
    x_values.sort_by(|a, b| {
        let a_num: Result<f64, _> = a.parse();
        let b_num: Result<f64, _> = b.parse();
        match (a_num, b_num) {
            (Ok(a), Ok(b)) => a.partial_cmp(&b).unwrap_or(std::cmp::Ordering::Equal),
            _ => a.cmp(b),
        }
    });

    HistogramData { x_values, series_names, data }
}

/// Render a stacked histogram (all series stacked in each bar)
fn render_stacked_histogram(
    f: &mut Frame,
    block: Block,
    response: &InsightsResponse,
    x_field: &str,
    y_field: &str,
    z_field: &str,
    x_unit: &str,
    area: Rect,
) {
    let hist_data = collect_histogram_data(response, x_field, y_field, z_field);

    if hist_data.x_values.is_empty() || hist_data.series_names.is_empty() {
        let empty = Paragraph::new("No data")
            .style(Style::default().fg(Color::DarkGray))
            .block(block);
        f.render_widget(empty, area);
        return;
    }

    // First render the block to get inner area
    let inner = block.inner(area);
    f.render_widget(block, area);

    if inner.height < 3 || inner.width < 5 {
        return;
    }

    // Reserve space for labels at bottom
    let label_height = 1u16;
    let chart_height = inner.height.saturating_sub(label_height + 1); // +1 for value labels
    let chart_area = Rect {
        x: inner.x,
        y: inner.y,
        width: inner.width,
        height: chart_height,
    };

    // Calculate totals for each x value and find max
    let mut totals: Vec<(String, u64)> = hist_data.x_values
        .iter()
        .map(|x| {
            let total: u64 = hist_data.series_names
                .iter()
                .map(|z| hist_data.data.get(&(x.clone(), z.clone())).copied().unwrap_or(0))
                .sum();
            (x.clone(), total)
        })
        .collect();

    let max_total = totals.iter().map(|(_, t)| *t).max().unwrap_or(1).max(1);

    // Calculate bar dimensions
    let bar_width = 4u16;
    let gap = 1u16;
    let bar_step = bar_width + gap;
    let max_bars = ((inner.width + gap) / bar_step) as usize;
    totals.truncate(max_bars);

    let buf = f.buffer_mut();

    // Draw each stacked bar
    for (bar_idx, (x_val, total)) in totals.iter().enumerate() {
        let bar_x = chart_area.x + (bar_idx as u16 * bar_step);

        // Calculate total bar height
        let bar_height = ((*total as f64 / max_total as f64) * chart_height as f64).ceil() as u16;
        let bar_height = bar_height.min(chart_height);

        // Draw stacked segments from bottom up
        let mut current_y = chart_area.y + chart_height; // Start at bottom

        // Reverse iterate series (draw bottom series first, which is highest-avg first)
        for (series_idx, series_name) in hist_data.series_names.iter().enumerate().rev() {
            let value = hist_data.data.get(&(x_val.clone(), series_name.clone())).copied().unwrap_or(0);
            if value == 0 {
                continue;
            }

            let segment_height = ((value as f64 / max_total as f64) * chart_height as f64).ceil() as u16;
            let segment_height = segment_height.min(current_y.saturating_sub(chart_area.y));

            if segment_height == 0 {
                continue;
            }

            let color = SERIES_COLORS[series_idx % SERIES_COLORS.len()];

            // Draw segment
            for dy in 0..segment_height {
                let y = current_y.saturating_sub(dy + 1);
                if y < chart_area.y {
                    break;
                }
                for dx in 0..bar_width {
                    let x = bar_x + dx;
                    if x < chart_area.x + chart_area.width {
                        buf[(x, y)].set_char('█').set_fg(color);
                    }
                }
            }
            current_y = current_y.saturating_sub(segment_height);
        }

        // Draw value label above bar
        let value_str = total.to_string();
        let value_x = bar_x + (bar_width.saturating_sub(value_str.len() as u16)) / 2;
        let value_y = chart_area.y + chart_height - bar_height;
        if value_y > chart_area.y && bar_height > 0 {
            for (i, ch) in value_str.chars().enumerate() {
                let x = value_x + i as u16;
                if x < chart_area.x + chart_area.width {
                    buf[(x, value_y.saturating_sub(1))].set_char(ch).set_fg(Color::White);
                }
            }
        }

        // Draw x-axis label
        let label = format!("{}{}", x_val, x_unit);
        let label_y = inner.y + inner.height - 1;
        let label_x = bar_x;
        for (i, ch) in label.chars().take(bar_width as usize + gap as usize) .enumerate() {
            let x = label_x + i as u16;
            if x < inner.x + inner.width {
                buf[(x, label_y)].set_char(ch).set_fg(Color::DarkGray);
            }
        }
    }
}

/// Render histogram filtered to a single series
fn render_filtered_histogram(
    f: &mut Frame,
    block: Block,
    response: &InsightsResponse,
    x_field: &str,
    y_field: &str,
    z_field: &str,
    x_unit: &str,
    series_index: usize,
    area: Rect,
) {
    let hist_data = collect_histogram_data(response, x_field, y_field, z_field);

    if hist_data.x_values.is_empty() || series_index >= hist_data.series_names.len() {
        let empty = Paragraph::new("No data")
            .style(Style::default().fg(Color::DarkGray))
            .block(block);
        f.render_widget(empty, area);
        return;
    }

    let series_name = &hist_data.series_names[series_index];
    let color = SERIES_COLORS[series_index % SERIES_COLORS.len()];

    // Build bars for just this series
    let bars: Vec<Bar> = hist_data.x_values
        .iter()
        .filter_map(|x| {
            let value = hist_data.data.get(&(x.clone(), series_name.clone())).copied().unwrap_or(0);
            let label = format!("{}{}", x, x_unit);
            let label = if label.len() > 6 { label[..6].to_string() } else { label };
            Some(Bar::default().label(label.into()).value(value).style(Style::default().fg(color)))
        })
        .take((area.width.saturating_sub(4) / 6) as usize)
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
        .bar_width(5)
        .bar_gap(1)
        .value_style(Style::default().fg(Color::White));

    f.render_widget(bar_chart, area);
}

fn render_billboard(
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

    let title_field = config.title_field.as_deref().unwrap_or("title");
    let value_field = config.value_field.as_deref().unwrap_or("value");

    // Build billboard items - each row becomes a title/value pair
    let items: Vec<(String, String)> = response
        .results
        .iter()
        .map(|row| {
            let title = row
                .get(title_field)
                .map(|v| format_json_value(v))
                .unwrap_or_default();
            let value = row
                .get(value_field)
                .map(|v| format_json_value(v))
                .unwrap_or_default();
            (title, value)
        })
        .collect();

    if items.is_empty() {
        let empty = Paragraph::new("No data")
            .style(Style::default().fg(Color::DarkGray))
            .block(block);
        f.render_widget(empty, area);
        return;
    }

    // Calculate layout - show items side by side if space allows
    let inner = block.inner(area);
    let item_width = (inner.width / items.len() as u16).max(10);

    let lines: Vec<Line> = if items.len() == 1 {
        // Single item - center it vertically
        let (title, value) = &items[0];
        vec![
            Line::from(""),
            Line::from(Span::styled(
                title.clone(),
                Style::default().fg(Color::DarkGray),
            )),
            Line::from(""),
            Line::from(Span::styled(
                value.clone(),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            )),
        ]
    } else {
        // Multiple items - show titles on one line, values on another
        let title_spans: Vec<Span> = items
            .iter()
            .enumerate()
            .flat_map(|(i, (title, _))| {
                let padding = " ".repeat((item_width as usize).saturating_sub(title.len()) / 2);
                let mut spans = vec![];
                if i > 0 {
                    spans.push(Span::raw(" │ "));
                }
                spans.push(Span::styled(
                    format!("{}{}{}", padding, title, padding),
                    Style::default().fg(Color::DarkGray),
                ));
                spans
            })
            .collect();

        let value_spans: Vec<Span> = items
            .iter()
            .enumerate()
            .flat_map(|(i, (_, value))| {
                let padding = " ".repeat((item_width as usize).saturating_sub(value.len()) / 2);
                let mut spans = vec![];
                if i > 0 {
                    spans.push(Span::raw(" │ "));
                }
                spans.push(Span::styled(
                    format!("{}{}{}", padding, value, padding),
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                ));
                spans
            })
            .collect();

        vec![
            Line::from(""),
            Line::from(title_spans),
            Line::from(""),
            Line::from(value_spans),
        ]
    };

    let paragraph = Paragraph::new(lines)
        .block(block)
        .alignment(ratatui::layout::Alignment::Center);
    f.render_widget(paragraph, area);
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

/// Count unique series in the data for a given z_field
pub fn count_series(response: &InsightsResponse, z_field: &str) -> usize {
    let mut unique: std::collections::HashSet<String> = std::collections::HashSet::new();
    for row in &response.results {
        if let Some(v) = row.get(z_field) {
            unique.insert(format_json_value(v));
        }
    }
    unique.len()
}

/// Extract series names from a line chart's data for legend display, sorted by average value (highest first)
pub fn extract_series_names(response: &InsightsResponse, config: &ChartConfig) -> Vec<String> {
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

    // Sort by average value descending, then by name for stability
    let mut sorted: Vec<_> = series_stats.into_iter().collect();
    sorted.sort_by(|a, b| {
        let avg_a = if a.1.1 == 0 { 0.0 } else { a.1.0 / a.1.1 as f64 };
        let avg_b = if b.1.1 == 0 { 0.0 } else { b.1.0 / b.1.1 as f64 };
        avg_b.partial_cmp(&avg_a)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.0.cmp(&b.0))
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

/// Render legend with optional highlight for selected series
fn render_legend_with_highlight(f: &mut Frame, series_names: &[String], area: Rect, highlight: Option<usize>) {
    if series_names.is_empty() || area.height < 1 {
        return;
    }

    // Wrap into lines if needed (simple wrapping based on terminal width)
    let mut lines: Vec<Line> = Vec::new();
    let mut current_spans: Vec<Span> = Vec::new();
    let mut current_width: usize = 0;
    let max_width = area.width.saturating_sub(2) as usize;

    for (i, name) in series_names.iter().enumerate() {
        let color = SERIES_COLORS[i % SERIES_COLORS.len()];
        let is_highlighted = highlight == Some(i);
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

        // Use bold/underline for highlighted series, dim for others when filtering
        let square_style = Style::default().fg(color);
        let name_style = if is_highlighted {
            Style::default().fg(Color::White).add_modifier(Modifier::BOLD)
        } else if highlight.is_some() {
            Style::default().fg(Color::DarkGray)
        } else {
            Style::default()
        };

        current_spans.push(Span::styled("■ ", square_style));
        current_spans.push(Span::styled(name.clone(), name_style));
        current_width += 2 + name.len();
    }

    if !current_spans.is_empty() {
        lines.push(Line::from(current_spans));
    }

    let paragraph = Paragraph::new(lines)
        .block(Block::default().borders(Borders::TOP));
    f.render_widget(paragraph, area);
}

/// Render a maximized widget with legend below (for charts with z-field)
pub fn render_maximized_widget(
    f: &mut Frame,
    widget: &WidgetRuntime,
    area: Rect,
    histogram_series_filter: Option<usize>,
) {
    let widget_title = &widget.widget.presentation.title;
    let view_type = &widget.widget.config.vis.view;

    // Check if this is a chart with series data (line or histogram with z-field)
    let series_names = if view_type == "line" || view_type == "histogram" {
        if let WidgetState::Loaded(response) = &widget.state {
            extract_series_names(response, &widget.widget.config.vis.chart_config)
        } else {
            vec![]
        }
    } else {
        vec![]
    };

    // Build title with optional series filter info for histograms
    let title = if view_type == "histogram" && !series_names.is_empty() {
        match histogram_series_filter {
            None => format!("{} (All - ↑↓ to filter)", widget_title),
            Some(idx) if idx < series_names.len() => {
                format!("{} ({}) - ↑↓ to navigate", widget_title, series_names[idx])
            }
            _ => widget_title.clone(),
        }
    } else {
        widget_title.clone()
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
                "histogram" => render_histogram_with_filter(f, block, response, chart_config, chart_area, histogram_series_filter),
                "billboard" => render_billboard(f, block, response, chart_config, chart_area),
                _ => render_table(f, block, response, chart_area),
            }
        }
    }

    // Render legend if we have series names
    if let Some(legend_rect) = legend_area {
        render_legend_with_highlight(f, &series_names, legend_rect, histogram_series_filter);
    }
}
