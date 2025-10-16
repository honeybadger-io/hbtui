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

mod honeybadger;

use honeybadger::{HoneybadgerClient, ProjectStats};

struct App {
    client: HoneybadgerClient,
    stats: Option<ProjectStats>,
    should_quit: bool,
}

impl App {
    fn new(auth_token: String) -> Self {
        Self {
            client: HoneybadgerClient::new(auth_token),
            stats: None,
            should_quit: false,
        }
    }

    async fn load_data(&mut self) -> Result<()> {
        self.stats = Some(self.client.get_project_stats().await?);
        Ok(())
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // Read auth token from environment
    let auth_token = std::env::var("HONEYBADGER_PERSONAL_AUTH_TOKEN")
        .expect("HONEYBADGER_PERSONAL_AUTH_TOKEN environment variable not set");

    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Create app and load initial data
    let mut app = App::new(auth_token);
    if let Err(e) = app.load_data().await {
        eprintln!("Failed to load data: {}", e);
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
        terminal.draw(|f| ui(f, app))?;

        if event::poll(std::time::Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                match key.code {
                    KeyCode::Char('q') => app.should_quit = true,
                    KeyCode::Char('r') => {
                        if let Err(e) = app.load_data().await {
                            eprintln!("Failed to reload data: {}", e);
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
        .style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))
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
    let footer = Paragraph::new("Press 'q' to quit, 'r' to refresh")
        .style(Style::default().fg(Color::Gray))
        .block(Block::default().borders(Borders::ALL));
    f.render_widget(footer, chunks[2]);
}
