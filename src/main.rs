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
use widgets::render_widget;

/// Message type for async widget updates
#[derive(Debug)]
pub enum AppMessage {
    WidgetLoaded {
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
    dashboard_state: Option<DashboardState>,
    view_mode: ViewMode,
    should_quit: bool,
    message_tx: mpsc::Sender<AppMessage>,
    message_rx: mpsc::Receiver<AppMessage>,
    selected_widget_index: Option<usize>,
    maximized_widget_index: Option<usize>,
}

impl App {
    fn new(auth_token: String) -> Self {
        let (tx, rx) = mpsc::channel(100);
        Self {
            client: HoneybadgerClient::new(auth_token),
            stats: None,
            dashboard_state: None,
            view_mode: ViewMode::Dashboard,
            should_quit: false,
            message_tx: tx,
            message_rx: rx,
            selected_widget_index: Some(0),
            maximized_widget_index: None,
        }
    }

    async fn load_data(&mut self) -> Result<()> {
        self.stats = Some(self.client.get_project_stats().await?);
        Ok(())
    }

    /// Load dashboard from a YAML file
    fn load_dashboard(&mut self, path: &str, project_id: u64) -> Result<()> {
        let content = std::fs::read_to_string(path)?;
        let dashboard: Dashboard = serde_yaml::from_str(&content)?;
        self.dashboard_state = Some(DashboardState::new(dashboard, project_id));
        Ok(())
    }

    /// Spawn background tasks to fetch all widget data in parallel
    fn fetch_all_widgets(&self) {
        let Some(state) = &self.dashboard_state else {
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
                    .send(AppMessage::WidgetLoaded { widget_id, result })
                    .await;
            });
        }
    }

    /// Process any pending messages from background tasks
    fn process_messages(&mut self) {
        while let Ok(msg) = self.message_rx.try_recv() {
            match msg {
                AppMessage::WidgetLoaded { widget_id, result } => {
                    if let Some(state) = &mut self.dashboard_state {
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
        if let Some(state) = &mut self.dashboard_state {
            state.reset_all_to_loading();
        }
        self.fetch_all_widgets();
    }

    /// Navigate to an adjacent widget in the given direction
    fn navigate_widget(&mut self, direction: NavDirection) {
        if let (Some(state), Some(current_idx)) =
            (&self.dashboard_state, self.selected_widget_index)
        {
            if let Some(new_idx) = find_adjacent_widget(&state.widgets, current_idx, direction) {
                self.selected_widget_index = Some(new_idx);
            }
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // Read auth token from environment
    let auth_token = std::env::var("HONEYBADGER_PERSONAL_AUTH_TOKEN")
        .expect("HONEYBADGER_PERSONAL_AUTH_TOKEN environment variable not set");

    // Dashboard file path (default or from env)
    let dashboard_path = std::env::var("HONEYBADGER_DASHBOARD")
        .unwrap_or_else(|_| "rails_dashboard.yml".to_string());

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

    // Load dashboard if file exists
    if std::path::Path::new(&dashboard_path).exists() {
        if let Err(e) = app.load_dashboard(&dashboard_path, project_id) {
            eprintln!("Failed to load dashboard: {}", e);
        } else {
            // Start fetching widget data
            app.fetch_all_widgets();
        }
    } else {
        // Fall back to stats view if no dashboard
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
                        if app.view_mode != ViewMode::Dashboard && app.dashboard_state.is_some() {
                            app.view_mode = ViewMode::Dashboard;
                        }
                    }
                    // Arrow key navigation (only in dashboard view, not maximized)
                    KeyCode::Up | KeyCode::Down | KeyCode::Left | KeyCode::Right => {
                        if app.view_mode == ViewMode::Dashboard && app.maximized_widget_index.is_none() {
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
                        if app.view_mode == ViewMode::Dashboard && app.maximized_widget_index.is_none() {
                            app.maximized_widget_index = app.selected_widget_index;
                        }
                    }
                    // Escape to exit maximized view or deselect
                    KeyCode::Esc => {
                        if app.maximized_widget_index.is_some() {
                            app.maximized_widget_index = None;
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
            Constraint::Length(3), // Header
            Constraint::Min(10),   // Dashboard content
            Constraint::Length(3), // Footer
        ])
        .split(f.area());

    // Header with dashboard title (or widget title if maximized)
    let title = if let Some(max_idx) = app.maximized_widget_index {
        app.dashboard_state
            .as_ref()
            .and_then(|s| s.widgets.get(max_idx))
            .map(|w| w.widget.presentation.title.as_str())
            .unwrap_or("Widget")
    } else {
        app.dashboard_state
            .as_ref()
            .map(|s| s.dashboard.title.as_str())
            .unwrap_or("Dashboard")
    };

    let header = Paragraph::new(title)
        .style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )
        .block(Block::default().borders(Borders::ALL));
    f.render_widget(header, chunks[0]);

    // Dashboard content
    if let Some(state) = &app.dashboard_state {
        // Check if we're in maximized view
        if let Some(max_idx) = app.maximized_widget_index {
            // Render only the maximized widget filling the content area
            if let Some(widget) = state.widgets.get(max_idx) {
                render_widget(f, widget, chunks[1], true);
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
        "'q' quit | 'r' refresh | arrows navigate | Enter maximize"
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
