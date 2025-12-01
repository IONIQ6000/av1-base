//! AV1 Dashboard TUI
//!
//! Terminal interface for real-time monitoring of encoding jobs and system metrics.
//! Connects to the daemon metrics endpoint at http://127.0.0.1:7878/metrics

use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    symbols,
    text::{Line, Span},
    widgets::{
        Axis, Block, Borders, Cell, Chart, Dataset, Gauge, Paragraph, Row, Table, Wrap,
    },
    Frame, Terminal,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::VecDeque,
    io::{self, Stdout},
    time::{Duration, Instant},
};

const METRICS_URL: &str = "http://127.0.0.1:7878/metrics";
const POLL_INTERVAL_MS: u64 = 500;
const MAX_THROUGHPUT_POINTS: usize = 60;
const MAX_EVENT_LOG_ENTRIES: usize = 100;

// ============================================================================
// Data Models (mirroring daemon metrics types)
// ============================================================================

/// Per-job metrics tracking encoding progress and statistics
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JobMetrics {
    pub id: String,
    pub input_path: String,
    pub stage: String,
    pub progress: f32,
    pub fps: f32,
    pub bitrate_kbps: f32,
    pub crf: u8,
    pub encoder: String,
    pub workers: u32,
    pub est_remaining_secs: f32,
    pub frames_encoded: u64,
    pub total_frames: u64,
    pub size_in_bytes_before: u64,
    pub size_in_bytes_after: u64,
    pub vmaf: Option<f32>,
    pub psnr: Option<f32>,
    pub ssim: Option<f32>,
}

/// System-level metrics for resource monitoring
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SystemMetrics {
    pub cpu_usage_percent: f32,
    pub mem_usage_percent: f32,
    pub load_avg_1: f32,
    pub load_avg_5: f32,
    pub load_avg_15: f32,
}

/// Complete metrics snapshot including jobs, system, and aggregate stats
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MetricsSnapshot {
    pub timestamp_unix_ms: i64,
    pub jobs: Vec<JobMetrics>,
    pub system: SystemMetrics,
    pub queue_len: usize,
    pub running_jobs: usize,
    pub completed_jobs: u64,
    pub failed_jobs: u64,
    pub total_bytes_encoded: u64,
}

impl Default for SystemMetrics {
    fn default() -> Self {
        Self {
            cpu_usage_percent: 0.0,
            mem_usage_percent: 0.0,
            load_avg_1: 0.0,
            load_avg_5: 0.0,
            load_avg_15: 0.0,
        }
    }
}

impl Default for MetricsSnapshot {
    fn default() -> Self {
        Self {
            timestamp_unix_ms: 0,
            jobs: Vec::new(),
            system: SystemMetrics::default(),
            queue_len: 0,
            running_jobs: 0,
            completed_jobs: 0,
            failed_jobs: 0,
            total_bytes_encoded: 0,
        }
    }
}

// ============================================================================
// App State
// ============================================================================

/// Main application state for the TUI dashboard
pub struct App {
    /// Current metrics snapshot from daemon
    pub metrics: Option<MetricsSnapshot>,
    /// Event log with recent job events
    pub event_log: VecDeque<String>,
    /// Throughput history for chart (timestamp_secs, mb_encoded)
    pub throughput_history: VecDeque<(f64, f64)>,
    /// Last known total bytes for delta calculation
    last_total_bytes: u64,
    /// Connection status
    pub connected: bool,
    /// HTTP client for metrics fetching
    client: reqwest::Client,
    /// Start time for throughput chart x-axis
    start_time: Instant,
}

impl App {
    /// Create a new App instance
    pub fn new() -> Self {
        Self {
            metrics: None,
            event_log: VecDeque::with_capacity(MAX_EVENT_LOG_ENTRIES),
            throughput_history: VecDeque::with_capacity(MAX_THROUGHPUT_POINTS),
            last_total_bytes: 0,
            connected: false,
            client: reqwest::Client::new(),
            start_time: Instant::now(),
        }
    }

    /// Add an event to the log
    pub fn log_event(&mut self, event: String) {
        if self.event_log.len() >= MAX_EVENT_LOG_ENTRIES {
            self.event_log.pop_front();
        }
        self.event_log.push_back(event);
    }

    /// Fetch metrics from the daemon HTTP endpoint
    pub async fn fetch_metrics(&mut self) {
        match self.client.get(METRICS_URL).send().await {
            Ok(response) => {
                if response.status().is_success() {
                    match response.json::<MetricsSnapshot>().await {
                        Ok(snapshot) => {
                            self.update_throughput(&snapshot);
                            self.metrics = Some(snapshot);
                            self.connected = true;
                        }
                        Err(e) => {
                            self.log_event(format!("JSON parse error: {}", e));
                            self.connected = false;
                        }
                    }
                } else {
                    self.log_event(format!("HTTP error: {}", response.status()));
                    self.connected = false;
                }
            }
            Err(e) => {
                if self.connected {
                    self.log_event(format!("Connection lost: {}", e));
                }
                self.connected = false;
            }
        }
    }

    /// Update throughput history with new data point
    fn update_throughput(&mut self, snapshot: &MetricsSnapshot) {
        let elapsed_secs = self.start_time.elapsed().as_secs_f64();
        let total_mb = snapshot.total_bytes_encoded as f64 / (1024.0 * 1024.0);

        if self.throughput_history.len() >= MAX_THROUGHPUT_POINTS {
            self.throughput_history.pop_front();
        }
        self.throughput_history.push_back((elapsed_secs, total_mb));
        self.last_total_bytes = snapshot.total_bytes_encoded;
    }
}

// ============================================================================
// Terminal Setup/Teardown
// ============================================================================

/// Initialize the terminal for TUI rendering
fn setup_terminal() -> io::Result<Terminal<CrosstermBackend<Stdout>>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    Terminal::new(backend)
}

/// Restore terminal to normal state
fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> io::Result<()> {
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;
    Ok(())
}


// ============================================================================
// Widget Rendering
// ============================================================================

/// Render the queue table showing job status
fn render_queue_table(f: &mut Frame, area: Rect, app: &App) {
    let header_cells = ["ID", "Stage", "Progress %", "FPS", "Bitrate", "CRF", "Workers", "ETA"]
        .iter()
        .map(|h| Cell::from(*h).style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)));
    let header = Row::new(header_cells).height(1).bottom_margin(1);

    let rows: Vec<Row> = if let Some(ref metrics) = app.metrics {
        metrics
            .jobs
            .iter()
            .map(|job| {
                let eta = if job.est_remaining_secs > 0.0 {
                    format_duration(job.est_remaining_secs)
                } else {
                    "-".to_string()
                };
                Row::new(vec![
                    Cell::from(job.id.clone()),
                    Cell::from(job.stage.clone()),
                    Cell::from(format!("{:.1}%", job.progress * 100.0)),
                    Cell::from(format!("{:.1}", job.fps)),
                    Cell::from(format!("{:.0} kbps", job.bitrate_kbps)),
                    Cell::from(format!("{}", job.crf)),
                    Cell::from(format!("{}", job.workers)),
                    Cell::from(eta),
                ])
            })
            .collect()
    } else {
        vec![]
    };

    let widths = [
        Constraint::Length(12),
        Constraint::Length(12),
        Constraint::Length(12),
        Constraint::Length(8),
        Constraint::Length(12),
        Constraint::Length(6),
        Constraint::Length(8),
        Constraint::Length(10),
    ];

    let title = if app.connected {
        " Queue "
    } else {
        " Queue (Disconnected) "
    };

    let table = Table::new(rows, widths)
        .header(header)
        .block(Block::default().borders(Borders::ALL).title(title));

    f.render_widget(table, area);
}

/// Render CPU and memory usage gauges
fn render_system_gauges(f: &mut Frame, area: Rect, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Min(0),
        ])
        .split(area);

    let (cpu_percent, mem_percent) = if let Some(ref metrics) = app.metrics {
        (
            metrics.system.cpu_usage_percent as f64 / 100.0,
            metrics.system.mem_usage_percent as f64 / 100.0,
        )
    } else {
        (0.0, 0.0)
    };

    let cpu_gauge = Gauge::default()
        .block(Block::default().borders(Borders::ALL).title(" CPU "))
        .gauge_style(Style::default().fg(Color::Cyan))
        .ratio(cpu_percent.clamp(0.0, 1.0))
        .label(format!("{:.1}%", cpu_percent * 100.0));

    let mem_gauge = Gauge::default()
        .block(Block::default().borders(Borders::ALL).title(" Memory "))
        .gauge_style(Style::default().fg(Color::Magenta))
        .ratio(mem_percent.clamp(0.0, 1.0))
        .label(format!("{:.1}%", mem_percent * 100.0));

    f.render_widget(cpu_gauge, chunks[0]);
    f.render_widget(mem_gauge, chunks[1]);
}

/// Render load averages table
fn render_load_averages(f: &mut Frame, area: Rect, app: &App) {
    let (load_1, load_5, load_15) = if let Some(ref metrics) = app.metrics {
        (
            metrics.system.load_avg_1,
            metrics.system.load_avg_5,
            metrics.system.load_avg_15,
        )
    } else {
        (0.0, 0.0, 0.0)
    };

    let rows = vec![
        Row::new(vec![
            Cell::from("1 min"),
            Cell::from(format!("{:.2}", load_1)),
        ]),
        Row::new(vec![
            Cell::from("5 min"),
            Cell::from(format!("{:.2}", load_5)),
        ]),
        Row::new(vec![
            Cell::from("15 min"),
            Cell::from(format!("{:.2}", load_15)),
        ]),
    ];

    let widths = [Constraint::Length(8), Constraint::Length(10)];

    let table = Table::new(rows, widths)
        .block(Block::default().borders(Borders::ALL).title(" Load Avg "));

    f.render_widget(table, area);
}

/// Render throughput chart showing MB encoded over time
fn render_throughput_chart(f: &mut Frame, area: Rect, app: &App) {
    let data: Vec<(f64, f64)> = app.throughput_history.iter().cloned().collect();

    if data.is_empty() {
        let block = Block::default()
            .borders(Borders::ALL)
            .title(" Throughput (MB) ");
        f.render_widget(block, area);
        return;
    }

    let max_x = data.last().map(|(x, _)| *x).unwrap_or(60.0);
    let max_y = data.iter().map(|(_, y)| *y).fold(0.0f64, f64::max).max(1.0);

    let datasets = vec![Dataset::default()
        .name("MB encoded")
        .marker(symbols::Marker::Braille)
        .style(Style::default().fg(Color::Green))
        .data(&data)];

    let chart = Chart::new(datasets)
        .block(Block::default().borders(Borders::ALL).title(" Throughput (MB) "))
        .x_axis(
            Axis::default()
                .title("Time (s)")
                .style(Style::default().fg(Color::Gray))
                .bounds([0.0, max_x])
                .labels(vec![
                    Span::raw("0"),
                    Span::raw(format!("{:.0}", max_x / 2.0)),
                    Span::raw(format!("{:.0}", max_x)),
                ]),
        )
        .y_axis(
            Axis::default()
                .title("MB")
                .style(Style::default().fg(Color::Gray))
                .bounds([0.0, max_y])
                .labels(vec![
                    Span::raw("0"),
                    Span::raw(format!("{:.0}", max_y / 2.0)),
                    Span::raw(format!("{:.0}", max_y)),
                ]),
        );

    f.render_widget(chart, area);
}

/// Render event log showing recent job events
fn render_event_log(f: &mut Frame, area: Rect, app: &App) {
    let events: Vec<Line> = app
        .event_log
        .iter()
        .rev()
        .take(area.height as usize - 2)
        .map(|e| Line::from(e.as_str()))
        .collect();

    let paragraph = Paragraph::new(events)
        .block(Block::default().borders(Borders::ALL).title(" Event Log "))
        .wrap(Wrap { trim: true });

    f.render_widget(paragraph, area);
}

/// Render status bar with aggregate stats
fn render_status_bar(f: &mut Frame, area: Rect, app: &App) {
    let status = if let Some(ref metrics) = app.metrics {
        format!(
            " Queue: {} | Running: {} | Completed: {} | Failed: {} | Total: {:.2} GB | Press 'q' to quit ",
            metrics.queue_len,
            metrics.running_jobs,
            metrics.completed_jobs,
            metrics.failed_jobs,
            metrics.total_bytes_encoded as f64 / (1024.0 * 1024.0 * 1024.0)
        )
    } else {
        " Connecting to daemon... | Press 'q' to quit ".to_string()
    };

    let paragraph = Paragraph::new(status)
        .style(Style::default().fg(Color::White).bg(Color::DarkGray));

    f.render_widget(paragraph, area);
}

/// Format duration in seconds to human-readable string
fn format_duration(secs: f32) -> String {
    let total_secs = secs as u64;
    let hours = total_secs / 3600;
    let minutes = (total_secs % 3600) / 60;
    let seconds = total_secs % 60;

    if hours > 0 {
        format!("{}h {}m", hours, minutes)
    } else if minutes > 0 {
        format!("{}m {}s", minutes, seconds)
    } else {
        format!("{}s", seconds)
    }
}


// ============================================================================
// Main UI Layout
// ============================================================================

/// Render the complete UI layout
fn ui(f: &mut Frame, app: &App) {
    let size = f.area();

    // Main layout: status bar at bottom, rest for content
    let main_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(1)])
        .split(size);

    // Content area: left panel (queue + events) and right panel (system + chart)
    let content_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(65), Constraint::Percentage(35)])
        .split(main_chunks[0]);

    // Left panel: queue table on top, event log on bottom
    let left_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
        .split(content_chunks[0]);

    // Right panel: gauges, load avg, and throughput chart
    let right_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(6),  // CPU + Memory gauges
            Constraint::Length(5),  // Load averages
            Constraint::Min(0),     // Throughput chart
        ])
        .split(content_chunks[1]);

    // Render all widgets
    render_queue_table(f, left_chunks[0], app);
    render_event_log(f, left_chunks[1], app);
    render_system_gauges(f, right_chunks[0], app);
    render_load_averages(f, right_chunks[1], app);
    render_throughput_chart(f, right_chunks[2], app);
    render_status_bar(f, main_chunks[1], app);
}

// ============================================================================
// Main Entry Point
// ============================================================================

#[tokio::main]
async fn main() -> io::Result<()> {
    // Initialize terminal
    let mut terminal = setup_terminal()?;

    // Create app state
    let mut app = App::new();
    app.log_event("AV1 Dashboard started".to_string());

    // Run the main loop
    let result = run_app(&mut terminal, &mut app).await;

    // Restore terminal
    restore_terminal(&mut terminal)?;

    result
}

/// Main application loop
async fn run_app(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    app: &mut App,
) -> io::Result<()> {
    let poll_interval = Duration::from_millis(POLL_INTERVAL_MS);
    let mut last_fetch = Instant::now() - poll_interval; // Fetch immediately on start

    loop {
        // Fetch metrics if poll interval has elapsed
        if last_fetch.elapsed() >= poll_interval {
            app.fetch_metrics().await;
            last_fetch = Instant::now();
        }

        // Draw UI
        terminal.draw(|f| ui(f, app))?;

        // Handle input with a short timeout to allow frequent redraws
        if event::poll(Duration::from_millis(50))? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    match key.code {
                        KeyCode::Char('q') | KeyCode::Char('Q') => {
                            return Ok(());
                        }
                        KeyCode::Esc => {
                            return Ok(());
                        }
                        _ => {}
                    }
                }
            }
        }
    }
}
