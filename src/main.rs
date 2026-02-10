use anyhow::Result;
use clap::Parser;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame, Terminal,
};
use std::io;
use tokio::sync::mpsc;

mod dashboard;
mod honeybadger;
mod layout;
mod widgets;

use dashboard::{Dashboard, DashboardState, WidgetState};
use honeybadger::HoneybadgerClient;
use layout::{find_adjacent_widget, GridLayout, NavDirection};
use widgets::{render_widget, render_maximized_widget};

/// Terminal user interface for Honeybadger.io
#[derive(Parser, Debug)]
#[command(name = "hbtui")]
#[command(version)]
#[command(about = "Terminal dashboard for Honeybadger.io", long_about = None)]
struct Cli {
    /// Honeybadger project ID
    #[arg(short, long, env = "HONEYBADGER_PROJECT_ID")]
    project_id: u64,

    /// Directory containing dashboard YAML files
    #[arg(short = 'd', long, default_value = "./dashboards")]
    dashboard_dir: String,

    /// Honeybadger personal auth token
    #[arg(long, env = "HONEYBADGER_PERSONAL_AUTH_TOKEN")]
    auth_token: String,
}

/// Message type for async widget updates
#[derive(Debug)]
pub enum AppMessage {
    WidgetLoaded {
        dashboard_index: usize,
        widget_id: String,
        result: Result<dashboard::InsightsResponse, String>,
    },
}

struct App {
    client: HoneybadgerClient,
    dashboards: Vec<DashboardState>,
    dashboard_names: Vec<String>,
    active_dashboard_index: usize,
    should_quit: bool,
    message_tx: mpsc::Sender<AppMessage>,
    message_rx: mpsc::Receiver<AppMessage>,
    selected_widget_index: Option<usize>,
    maximized_widget_index: Option<usize>,
    /// When maximized on a histogram, which series to filter to (None = all/stacked)
    histogram_series_filter: Option<usize>,
}

impl App {
    fn new(auth_token: String) -> Self {
        let (tx, rx) = mpsc::channel(100);
        Self {
            client: HoneybadgerClient::new(auth_token),
            dashboards: Vec::new(),
            dashboard_names: Vec::new(),
            active_dashboard_index: 0,
            should_quit: false,
            message_tx: tx,
            message_rx: rx,
            selected_widget_index: Some(0),
            maximized_widget_index: None,
            histogram_series_filter: None,
        }
    }

    /// Load all dashboards from a directory
    fn load_dashboards_from_dir(&mut self, dir: &str, project_id: u64) -> Result<()> {
        let path = std::path::Path::new(dir);
        if !path.is_dir() {
            return Err(anyhow::anyhow!("Dashboard directory not found: {}", dir));
        }

        let mut entries: Vec<_> = std::fs::read_dir(path)?
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.path()
                    .extension()
                    .map(|ext| ext == "yml" || ext == "yaml")
                    .unwrap_or(false)
            })
            .collect();

        // Sort by filename for consistent ordering
        entries.sort_by_key(|e| e.file_name());

        for entry in entries {
            let file_path = entry.path();
            let content = std::fs::read_to_string(&file_path)?;
            let dashboard: Dashboard = serde_yaml::from_str(&content)?;

            // Extract display name from filename (capitalize first letter)
            let name = file_path
                .file_stem()
                .and_then(|s| s.to_str())
                .map(|s| {
                    let mut chars = s.chars();
                    match chars.next() {
                        None => String::new(),
                        Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
                    }
                })
                .unwrap_or_else(|| "Dashboard".to_string());

            self.dashboard_names.push(name);
            self.dashboards
                .push(DashboardState::new(dashboard, project_id));
        }

        Ok(())
    }

    /// Load dashboard from a single YAML file (legacy support)
    fn load_dashboard(&mut self, path: &str, project_id: u64) -> Result<()> {
        let content = std::fs::read_to_string(path)?;
        let dashboard: Dashboard = serde_yaml::from_str(&content)?;

        // Extract name from path
        let name = std::path::Path::new(path)
            .file_stem()
            .and_then(|s| s.to_str())
            .map(|s| {
                let mut chars = s.chars();
                match chars.next() {
                    None => String::new(),
                    Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
                }
            })
            .unwrap_or_else(|| "Dashboard".to_string());

        self.dashboard_names.push(name);
        self.dashboards
            .push(DashboardState::new(dashboard, project_id));
        Ok(())
    }

    /// Get the currently active dashboard state
    fn active_dashboard(&self) -> Option<&DashboardState> {
        self.dashboards.get(self.active_dashboard_index)
    }

    /// Get the currently active dashboard state mutably
    fn active_dashboard_mut(&mut self) -> Option<&mut DashboardState> {
        self.dashboards.get_mut(self.active_dashboard_index)
    }

    /// Spawn background tasks to fetch all widget data for a dashboard
    fn fetch_widgets_for_dashboard(&self, dashboard_index: usize) {
        let Some(state) = self.dashboards.get(dashboard_index) else {
            return;
        };

        for widget_runtime in &state.widgets {
            let client = self.client.clone();
            let project_id = state.project_id;
            let widget_id = widget_runtime.widget.id.clone();
            let query = widget_runtime.widget.config.query.clone();
            let tx = self.message_tx.clone();

            tokio::spawn(async move {
                // Check for empty query (often due to YAML indentation issues)
                let result = if query.trim().is_empty() {
                    Err("Empty query (check YAML indentation)".to_string())
                } else {
                    client
                        .query_insights(project_id, &query)
                        .await
                        .map_err(|e| e.to_string())
                };

                let _ = tx
                    .send(AppMessage::WidgetLoaded {
                        dashboard_index,
                        widget_id,
                        result,
                    })
                    .await;
            });
        }
    }

    /// Spawn background tasks to fetch all widget data for active dashboard
    fn fetch_all_widgets(&self) {
        self.fetch_widgets_for_dashboard(self.active_dashboard_index);
    }

    /// Process any pending messages from background tasks
    fn process_messages(&mut self) {
        while let Ok(msg) = self.message_rx.try_recv() {
            match msg {
                AppMessage::WidgetLoaded {
                    dashboard_index,
                    widget_id,
                    result,
                } => {
                    if let Some(state) = self.dashboards.get_mut(dashboard_index) {
                        let widget_state = match result {
                            Ok(response) => WidgetState::Loaded(response),
                            Err(e) => WidgetState::Error(e),
                        };
                        state.update_widget(&widget_id, widget_state);
                    }
                }
            }
        }
    }

    /// Refresh dashboard - reset all widgets to loading and refetch
    fn refresh_dashboard(&mut self) {
        if let Some(state) = self.active_dashboard_mut() {
            state.reset_all_to_loading();
        }
        self.fetch_all_widgets();
    }

    /// Navigate to an adjacent widget in the given direction
    fn navigate_widget(&mut self, direction: NavDirection) {
        if let (Some(state), Some(current_idx)) =
            (self.active_dashboard(), self.selected_widget_index)
        {
            if let Some(new_idx) = find_adjacent_widget(&state.widgets, current_idx, direction) {
                self.selected_widget_index = Some(new_idx);
            }
        }
    }

    /// Navigate histogram series filter in maximized view
    /// Cycles through: None (all/stacked) -> 0 -> 1 -> ... -> n-1 -> None
    fn navigate_histogram_series(&mut self, up: bool, series_count: usize) {
        self.histogram_series_filter = match self.histogram_series_filter {
            None => {
                if up {
                    Some(series_count - 1) // Wrap from All to last series
                } else {
                    Some(0) // Go from All to first series
                }
            }
            Some(idx) => {
                if up {
                    if idx == 0 {
                        None // Wrap from first to All
                    } else {
                        Some(idx - 1)
                    }
                } else if idx >= series_count - 1 {
                    None // Wrap from last to All
                } else {
                    Some(idx + 1)
                }
            }
        };
    }

    /// Switch to a specific dashboard by index
    fn switch_to_dashboard(&mut self, index: usize) {
        if index < self.dashboards.len() && index != self.active_dashboard_index {
            self.active_dashboard_index = index;
            self.selected_widget_index = Some(0);
            self.maximized_widget_index = None;

            // Fetch data if widgets are still in loading state
            if let Some(state) = self.active_dashboard() {
                let needs_fetch = state.widgets.iter().all(|w| {
                    matches!(w.state, WidgetState::Loading)
                });
                if needs_fetch {
                    self.fetch_all_widgets();
                }
            }
        }
    }

    /// Switch to the next dashboard
    fn next_dashboard(&mut self) {
        if !self.dashboards.is_empty() {
            let next = (self.active_dashboard_index + 1) % self.dashboards.len();
            self.switch_to_dashboard(next);
        }
    }

    /// Switch to the previous dashboard
    fn prev_dashboard(&mut self) {
        if !self.dashboards.is_empty() {
            let prev = if self.active_dashboard_index == 0 {
                self.dashboards.len() - 1
            } else {
                self.active_dashboard_index - 1
            };
            self.switch_to_dashboard(prev);
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // Parse CLI arguments
    let cli = Cli::parse();

    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Create app
    let mut app = App::new(cli.auth_token);

    // Try to load dashboards from configured directory
    if std::path::Path::new(&cli.dashboard_dir).is_dir() {
        if let Err(e) = app.load_dashboards_from_dir(&cli.dashboard_dir, cli.project_id) {
            eprintln!("Failed to load dashboards: {}", e);
        }
    }

    // Fall back to legacy single dashboard file if no dashboards loaded
    if app.dashboards.is_empty() {
        let dashboard_path = std::env::var("HONEYBADGER_DASHBOARD")
            .unwrap_or_else(|_| "rails_dashboard.yml".to_string());

        if std::path::Path::new(&dashboard_path).exists() {
            if let Err(e) = app.load_dashboard(&dashboard_path, cli.project_id) {
                eprintln!("Failed to load dashboard: {}", e);
            }
        }
    }

    // Start fetching widget data if we have dashboards
    if !app.dashboards.is_empty() {
        app.fetch_all_widgets();
    }

    // Run the app
    let res = run_app(&mut terminal, &mut app).await;

    // Restore terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    if let Err(err) = res {
        println!("Error: {:?}", err);
    }

    Ok(())
}

async fn run_app<B: ratatui::backend::Backend>(
    terminal: &mut Terminal<B>,
    app: &mut App,
) -> Result<()> {
    loop {
        // Process any pending background task results
        app.process_messages();

        terminal.draw(|f| ui(f, app))?;

        if event::poll(std::time::Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                match key.code {
                    KeyCode::Char('q') => app.should_quit = true,
                    KeyCode::Char('r') => {
                        app.refresh_dashboard();
                    }
                    // Dashboard switching with [ and ]
                    KeyCode::Char('[') => {
                        app.prev_dashboard();
                    }
                    KeyCode::Char(']') => {
                        app.next_dashboard();
                    }
                    // Number keys to switch dashboards directly (1-9)
                    KeyCode::Char(c) if c.is_ascii_digit() && c != '0' => {
                        let idx = c.to_digit(10).unwrap() as usize - 1;
                        app.switch_to_dashboard(idx);
                    }
                    // Arrow key navigation
                    KeyCode::Up | KeyCode::Down | KeyCode::Left | KeyCode::Right => {
                        if let Some(max_idx) = app.maximized_widget_index {
                            // In maximized view: Up/Down navigate histogram series
                            if matches!(key.code, KeyCode::Up | KeyCode::Down) {
                                if let Some(dashboard) = app.active_dashboard() {
                                    if let Some(widget) = dashboard.widgets.get(max_idx) {
                                        if widget.widget.config.vis.view == "histogram"
                                            && widget.widget.config.vis.chart_config.z_field.is_some()
                                        {
                                            // Get series count from the widget data
                                            if let WidgetState::Loaded(response) = &widget.state {
                                                let z_field = widget.widget.config.vis.chart_config.z_field.as_deref().unwrap();
                                                let series_count = widgets::count_series(response, z_field);
                                                if series_count > 0 {
                                                    app.navigate_histogram_series(key.code == KeyCode::Up, series_count);
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        } else {
                            // Not maximized: regular widget navigation
                            app.navigate_widget(match key.code {
                                KeyCode::Up => NavDirection::Up,
                                KeyCode::Down => NavDirection::Down,
                                KeyCode::Left => NavDirection::Left,
                                KeyCode::Right => NavDirection::Right,
                                _ => unreachable!(),
                            });
                        }
                    }
                    // Enter to maximize selected widget
                    KeyCode::Enter => {
                        if app.maximized_widget_index.is_none() {
                            app.maximized_widget_index = app.selected_widget_index;
                            app.histogram_series_filter = None; // Reset filter when entering maximized view
                        }
                    }
                    // Escape to exit maximized view or deselect
                    KeyCode::Esc => {
                        if app.maximized_widget_index.is_some() {
                            app.maximized_widget_index = None;
                            app.histogram_series_filter = None; // Reset filter when exiting
                        }
                    }
                    _ => {}
                }
            }
        }

        if app.should_quit {
            return Ok(());
        }
    }
}

fn ui(f: &mut Frame, app: &App) {
    render_dashboard_view(f, app);
}

fn render_dashboard_view(f: &mut Frame, app: &App) {
    // If no dashboards found, show helpful message
    if app.dashboards.is_empty() {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .margin(1)
            .constraints([
                Constraint::Length(3), // Header
                Constraint::Min(10),   // Content
                Constraint::Length(3), // Footer
            ])
            .split(f.area());

        let header = Paragraph::new("Honeybadger TUI")
            .style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))
            .block(Block::default().borders(Borders::ALL));
        f.render_widget(header, chunks[0]);

        let message = Paragraph::new("No dashboards found. Add YAML files to ./dashboards/")
            .style(Style::default().fg(Color::Gray))
            .block(Block::default().borders(Borders::ALL));
        f.render_widget(message, chunks[1]);

        let footer = Paragraph::new("'q' quit")
            .style(Style::default().fg(Color::Gray))
            .block(Block::default().borders(Borders::ALL));
        f.render_widget(footer, chunks[2]);

        return;
    }

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Length(3), // Header with tabs
            Constraint::Min(10),   // Dashboard content
            Constraint::Length(3), // Footer
        ])
        .split(f.area());

    // Header with tab bar (always visible, even when maximized)
    let tabs: Vec<Span> = app
        .dashboard_names
        .iter()
        .enumerate()
        .flat_map(|(i, name)| {
            let is_active = i == app.active_dashboard_index;
            let tab_style = if is_active {
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::DarkGray)
            };

            let separator = if i > 0 {
                vec![Span::raw(" ")]
            } else {
                vec![]
            };

            let tab = vec![Span::styled(
                format!("[{}:{}]", i + 1, name),
                tab_style,
            )];

            separator.into_iter().chain(tab)
        })
        .collect();

    let header = Paragraph::new(Line::from(tabs))
        .block(Block::default().borders(Borders::ALL));
    f.render_widget(header, chunks[0]);

    // Dashboard content
    if let Some(state) = app.active_dashboard() {
        // Check if we're in maximized view
        if let Some(max_idx) = app.maximized_widget_index {
            // Render only the maximized widget filling the content area
            if let Some(widget) = state.widgets.get(max_idx) {
                render_maximized_widget(f, widget, chunks[1], app.histogram_series_filter);
            }
        } else {
            // Normal grid view
            let grid = GridLayout::new_scaled(chunks[1], &state.widgets);

            for (idx, (widget, rect)) in grid.layout_widgets(&state.widgets).iter().enumerate() {
                // Find the original index of this widget in the state
                let original_idx = state
                    .widgets
                    .iter()
                    .position(|w| w.widget.id == widget.widget.id)
                    .unwrap_or(idx);
                let is_selected = app.selected_widget_index == Some(original_idx);

                // Only render if widget has meaningful size
                if rect.width >= 4 && rect.height >= 2 {
                    render_widget(f, widget, *rect, is_selected);
                }
            }
        }
    } else {
        let loading = Paragraph::new("No dashboard loaded")
            .block(Block::default().borders(Borders::ALL));
        f.render_widget(loading, chunks[1]);
    }

    // Footer
    let footer_text = if app.maximized_widget_index.is_some() {
        "'q' quit | 'r' refresh | ESC back to grid"
    } else {
        "'q' quit | 'r' refresh | [/] switch | arrows navigate | Enter maximize"
    };
    let footer = Paragraph::new(footer_text)
        .style(Style::default().fg(Color::Gray))
        .block(Block::default().borders(Borders::ALL));
    f.render_widget(footer, chunks[2]);
}
