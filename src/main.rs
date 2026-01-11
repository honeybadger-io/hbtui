use anyhow::Result;
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
    widgets::{Block, Borders, List, ListItem, Paragraph},
    Frame, Terminal,
};
use std::io;
use tokio::sync::mpsc;

mod dashboard;
mod honeybadger;
mod layout;
mod widgets;

use dashboard::{Dashboard, DashboardState, WidgetState};
use honeybadger::{HoneybadgerClient, ProjectStats};
use layout::{find_adjacent_widget, GridLayout, NavDirection};
use widgets::{render_widget, render_maximized_widget};

/// Message type for async widget updates
#[derive(Debug)]
pub enum AppMessage {
    WidgetLoaded {
        dashboard_index: usize,
        widget_id: String,
        result: Result<dashboard::InsightsResponse, String>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum ViewMode {
    Stats,
    Dashboard,
}

struct App {
    client: HoneybadgerClient,
    stats: Option<ProjectStats>,
    dashboards: Vec<DashboardState>,
    dashboard_names: Vec<String>,
    active_dashboard_index: usize,
    view_mode: ViewMode,
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
            stats: None,
            dashboards: Vec::new(),
            dashboard_names: Vec::new(),
            active_dashboard_index: 0,
            view_mode: ViewMode::Dashboard,
            should_quit: false,
            message_tx: tx,
            message_rx: rx,
            selected_widget_index: Some(0),
            maximized_widget_index: None,
            histogram_series_filter: None,
        }
    }

    async fn load_data(&mut self) -> Result<()> {
        self.stats = Some(self.client.get_project_stats().await?);
        Ok(())
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
                } else {
                    if idx >= series_count - 1 {
                        None // Wrap from last to All
                    } else {
                        Some(idx + 1)
                    }
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
    // Read auth token from environment
    let auth_token = std::env::var("HONEYBADGER_PERSONAL_AUTH_TOKEN")
        .expect("HONEYBADGER_PERSONAL_AUTH_TOKEN environment variable not set");

    // Project ID (default or from env)
    let project_id: u64 = std::env::var("HONEYBADGER_PROJECT_ID")
        .unwrap_or_else(|_| "121229".to_string())
        .parse()
        .expect("Invalid HONEYBADGER_PROJECT_ID");

    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Create app
    let mut app = App::new(auth_token);

    // Try to load dashboards from ./dashboards/ directory first
    let dashboards_dir = "./dashboards";
    if std::path::Path::new(dashboards_dir).is_dir() {
        if let Err(e) = app.load_dashboards_from_dir(dashboards_dir, project_id) {
            eprintln!("Failed to load dashboards: {}", e);
        }
    }

    // Fall back to legacy single dashboard file if no dashboards loaded
    if app.dashboards.is_empty() {
        let dashboard_path = std::env::var("HONEYBADGER_DASHBOARD")
            .unwrap_or_else(|_| "rails_dashboard.yml".to_string());

        if std::path::Path::new(&dashboard_path).exists() {
            if let Err(e) = app.load_dashboard(&dashboard_path, project_id) {
                eprintln!("Failed to load dashboard: {}", e);
            }
        }
    }

    // Start fetching widget data if we have dashboards
    if !app.dashboards.is_empty() {
        app.fetch_all_widgets();
    } else {
        // Fall back to stats view if no dashboards
        app.view_mode = ViewMode::Stats;
        if let Err(e) = app.load_data().await {
            eprintln!("Failed to load stats: {}", e);
        }
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
                        if app.view_mode == ViewMode::Dashboard {
                            app.refresh_dashboard();
                        } else if let Err(e) = app.load_data().await {
                            eprintln!("Failed to reload data: {}", e);
                        }
                    }
                    KeyCode::Char('s') => {
                        if app.view_mode != ViewMode::Stats {
                            app.view_mode = ViewMode::Stats;
                            if app.stats.is_none() {
                                if let Err(e) = app.load_data().await {
                                    eprintln!("Failed to load stats: {}", e);
                                }
                            }
                        }
                    }
                    KeyCode::Char('d') => {
                        if app.view_mode != ViewMode::Dashboard && !app.dashboards.is_empty() {
                            app.view_mode = ViewMode::Dashboard;
                        }
                    }
                    // Dashboard switching with [ and ]
                    KeyCode::Char('[') => {
                        if app.view_mode == ViewMode::Dashboard {
                            app.prev_dashboard();
                        }
                    }
                    KeyCode::Char(']') => {
                        if app.view_mode == ViewMode::Dashboard {
                            app.next_dashboard();
                        }
                    }
                    // Number keys to switch dashboards directly (1-9)
                    KeyCode::Char(c) if c.is_ascii_digit() && c != '0' => {
                        if app.view_mode == ViewMode::Dashboard {
                            let idx = c.to_digit(10).unwrap() as usize - 1;
                            app.switch_to_dashboard(idx);
                        }
                    }
                    // Arrow key navigation
                    KeyCode::Up | KeyCode::Down | KeyCode::Left | KeyCode::Right => {
                        if app.view_mode == ViewMode::Dashboard {
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
                    }
                    // Enter to maximize selected widget
                    KeyCode::Enter => {
                        if app.view_mode == ViewMode::Dashboard && app.maximized_widget_index.is_none() {
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
    match app.view_mode {
        ViewMode::Stats => render_stats_view(f, app),
        ViewMode::Dashboard => render_dashboard_view(f, app),
    }
}

fn render_dashboard_view(f: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Length(3), // Header with tabs
            Constraint::Min(10),   // Dashboard content
            Constraint::Length(3), // Footer
        ])
        .split(f.area());

    // Header with tab bar (or widget title if maximized)
    if let Some(max_idx) = app.maximized_widget_index {
        // Show widget title when maximized
        let title = app
            .active_dashboard()
            .and_then(|s| s.widgets.get(max_idx))
            .map(|w| w.widget.presentation.title.as_str())
            .unwrap_or("Widget");

        let header = Paragraph::new(title)
            .style(
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )
            .block(Block::default().borders(Borders::ALL));
        f.render_widget(header, chunks[0]);
    } else {
        // Show tab bar with dashboard names
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
    }

    // Dashboard content
    if let Some(state) = app.active_dashboard() {
        // Check if we're in maximized view
        if let Some(max_idx) = app.maximized_widget_index {
            // Render only the maximized widget filling the content area
            if let Some(widget) = state.widgets.get(max_idx) {
                let dashboard_name = app.dashboard_names.get(app.active_dashboard_index).map(|s| s.as_str());
                render_maximized_widget(f, widget, chunks[1], app.histogram_series_filter, dashboard_name);
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

fn render_stats_view(f: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(2)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(10),
            Constraint::Length(3),
        ])
        .split(f.area());

    // Header
    let header = Paragraph::new("Honeybadger Dashboard")
        .style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )
        .block(Block::default().borders(Borders::ALL));
    f.render_widget(header, chunks[0]);

    // Main content area
    let main_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(chunks[1]);

    // Project stats
    if let Some(stats) = &app.stats {
        let stats_text = vec![
            Line::from(vec![
                Span::styled("Total Projects: ", Style::default().fg(Color::Cyan)),
                Span::raw(stats.total_projects.to_string()),
            ]),
            Line::from(vec![
                Span::styled("Total Faults: ", Style::default().fg(Color::Red)),
                Span::raw(stats.total_faults.to_string()),
            ]),
            Line::from(vec![
                Span::styled("Unresolved Faults: ", Style::default().fg(Color::Yellow)),
                Span::raw(stats.unresolved_faults.to_string()),
            ]),
        ];

        let stats_widget = Paragraph::new(stats_text)
            .block(Block::default().title("Statistics").borders(Borders::ALL));
        f.render_widget(stats_widget, main_chunks[0]);

        // Recent projects list
        let projects: Vec<ListItem> = stats
            .recent_projects
            .iter()
            .map(|p| {
                let content = vec![Line::from(vec![
                    Span::styled(&p.name, Style::default().fg(Color::Green)),
                    Span::raw(format!(" ({} faults)", p.fault_count)),
                ])];
                ListItem::new(content)
            })
            .collect();

        let projects_list = List::new(projects)
            .block(Block::default().title("Recent Projects").borders(Borders::ALL));
        f.render_widget(projects_list, main_chunks[1]);
    } else {
        let loading = Paragraph::new("Loading data...")
            .block(Block::default().title("Statistics").borders(Borders::ALL));
        f.render_widget(loading, main_chunks[0]);
    }

    // Footer
    let footer = Paragraph::new("'q' quit | 'r' refresh | 's' stats | 'd' dashboard")
        .style(Style::default().fg(Color::Gray))
        .block(Block::default().borders(Borders::ALL));
    f.render_widget(footer, chunks[2]);
}
