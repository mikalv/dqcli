use clap::Parser;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use futures::StreamExt;
use librdap_storm::{fetch_iana_tlds, Availability, ProbeConfig, Prober};
use ratatui::{
    backend::{Backend, CrosstermBackend},
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
    Frame, Terminal,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    io::{self, Write},
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};

const SPINNER_FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

#[derive(Debug, Clone, Copy, PartialEq)]
enum FilterMode {
    All,
    Available,
    Taken,
}

impl FilterMode {
    fn next(self) -> Self {
        match self {
            FilterMode::All => FilterMode::Available,
            FilterMode::Available => FilterMode::Taken,
            FilterMode::Taken => FilterMode::All,
        }
    }
}

#[derive(Debug, Default, Deserialize, Serialize)]
struct Config {
    #[serde(default)]
    tlds: TldConfig,
}

#[derive(Debug, Default, Deserialize, Serialize)]
struct TldConfig {
    #[serde(default)]
    always: Vec<String>,
    #[serde(default)]
    never: Vec<String>,
}

fn config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|p| p.join("dq").join("config.toml"))
}

fn load_config() -> Config {
    config_path()
        .and_then(|path| std::fs::read_to_string(&path).ok())
        .and_then(|content| toml::from_str(&content).ok())
        .unwrap_or_default()
}

fn apply_config_to_tlds(mut tlds: Vec<String>, config: &Config) -> Vec<String> {
    let never_set: std::collections::HashSet<_> = config.tlds.never.iter()
        .map(|s| s.to_lowercase())
        .collect();
    
    tlds.retain(|tld| !never_set.contains(&tld.to_lowercase()));
    
    for always_tld in config.tlds.always.iter().rev() {
        let lower = always_tld.to_lowercase();
        if !tlds.iter().any(|t| t.to_lowercase() == lower) {
            tlds.insert(0, lower);
        }
    }
    
    tlds
}

fn get_default_config_toml() -> String {
    r#"# Domain Query (dq) Configuration

[tlds]
# TLDs to always include in results, regardless of IANA list
# always = ["com", "net", "org", "io", "dev", "rs", "no", "pm"]
always = []

# TLDs to never include/hide from results
# never = ["adult", "xxx", "reklame"]
never = []
"#.to_string()
}

fn parse_domain_query(query: &str) -> (String, Option<String>) {
    if let Some(dot_pos) = query.rfind('.') {
        let base = &query[..dot_pos];
        let potential_tld = &query[dot_pos + 1..];

        let is_valid_tld = !base.is_empty()
            && !potential_tld.is_empty()
            && potential_tld.len() <= 20
            && potential_tld.chars().all(|c| c.is_ascii_alphabetic());

        if is_valid_tld {
            return (base.to_string(), Some(potential_tld.to_lowercase()));
        }
    }

    (query.to_string(), None)
}

fn prioritize_tld(mut tlds: Vec<String>, priority_tld: &str) -> Vec<String> {
    if let Some(pos) = tlds.iter().position(|t| t.eq_ignore_ascii_case(priority_tld)) {
        let tld = tlds.remove(pos);
        tlds.insert(0, tld);
    }
    tlds
}

#[derive(Parser, Debug)]
#[command(name = "dq")]
#[command(about = "Domain Query - instant availability search across all TLDs", long_about = None)]
struct Args {
    /// Domain query to search (without TLD)
    query: Option<String>,

    /// Output results as NDJSON stream (one JSON object per line)
    #[arg(long, short = 'j')]
    ndjson: bool,

    /// Comma-separated list of specific TLDs to check (e.g., dev,ai,com,net,org,io)
    #[arg(long, value_delimiter = ',')]
    tlds: Option<Vec<String>>,

    /// Print the default config to stdout and exit
    #[arg(long)]
    print_default_config: bool,

    /// Write the default config to the config path and exit
    #[arg(long)]
    write_default_config: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum AvailabilityStatus {
    Available,
    Taken,
    Checking,
    Pending,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DomainCheckResult {
    query: String,
    tld: String,
    domain: String,
    available: Option<bool>,
    status: AvailabilityStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
enum DomainStatus {
    Pending,
    Checking,
    Available,
    Taken,
    Error(String),
}

struct App {
    query: String,
    input_mode: bool,
    results: Arc<Mutex<HashMap<String, DomainStatus>>>,
    tlds: Vec<String>,
    list_state: ListState,
    quit: bool,
    specific_domain: Option<String>,
    specific_domain_status: Arc<Mutex<Option<DomainStatus>>>,
    tick: usize,
    filter_mode: FilterMode,
    toast_message: Option<(String, std::time::Instant)>,
}

impl App {
    fn new(initial_query: Option<String>, specific_tld: Option<String>, tlds: Vec<String>) -> Self {
        let results = Arc::new(Mutex::new(HashMap::new()));

        {
            let mut res = results.lock().unwrap();
            for tld in &tlds {
                res.insert(tld.clone(), DomainStatus::Pending);
            }
        }

        let mut list_state = ListState::default();
        list_state.select(Some(0));

        let specific_domain = match (&initial_query, &specific_tld) {
            (Some(q), Some(tld)) => Some(format!("{}.{}", q, tld)),
            _ => None,
        };

        Self {
            query: initial_query.unwrap_or_default(),
            input_mode: true,
            results,
            tlds,
            list_state,
            quit: false,
            specific_domain,
            specific_domain_status: Arc::new(Mutex::new(None)),
            tick: 0,
            filter_mode: FilterMode::All,
            toast_message: None,
        }
    }

    fn get_selected_domain(&self) -> Option<String> {
        let filtered = self.get_filtered_results();
        self.list_state.selected().and_then(|i| {
            filtered.get(i).map(|(tld, _)| format!("{}.{}", self.query, tld))
        })
    }

    fn copy_selected_to_clipboard(&mut self) {
        if let Some(domain) = self.get_selected_domain() {
            if let Ok(mut clipboard) = arboard::Clipboard::new() {
                if clipboard.set_text(&domain).is_ok() {
                    self.toast_message = Some((format!("Copied: {}", domain), std::time::Instant::now()));
                }
            }
        }
    }

    fn open_selected_in_browser(&mut self) {
        if let Some(domain) = self.get_selected_domain() {
            let url = format!("https://www.namecheap.com/domains/registration/results/?domain={}", domain);
            let _ = open::that(&url);
            self.toast_message = Some((format!("Opening: {}", domain), std::time::Instant::now()));
        }
    }

    fn get_filtered_results(&self) -> Vec<(String, DomainStatus)> {
        self.get_sorted_results()
            .into_iter()
            .filter(|(_, status)| match self.filter_mode {
                FilterMode::All => true,
                FilterMode::Available => matches!(status, DomainStatus::Available),
                FilterMode::Taken => matches!(status, DomainStatus::Taken),
            })
            .collect()
    }

    fn spinner_frame(&self) -> &'static str {
        SPINNER_FRAMES[self.tick % SPINNER_FRAMES.len()]
    }

    fn progress(&self) -> (usize, usize) {
        let results = self.results.lock().unwrap();
        let done = results.values().filter(|s| !matches!(s, DomainStatus::Pending | DomainStatus::Checking)).count();
        (done, self.tlds.len())
    }

    fn scroll_down(&mut self) {
        let i = match self.list_state.selected() {
            Some(i) => {
                if i >= self.tlds.len() - 1 {
                    i
                } else {
                    i + 1
                }
            }
            None => 0,
        };
        self.list_state.select(Some(i));
    }

    fn scroll_up(&mut self) {
        let i = match self.list_state.selected() {
            Some(i) => {
                if i == 0 {
                    0
                } else {
                    i - 1
                }
            }
            None => 0,
        };
        self.list_state.select(Some(i));
    }

    fn scroll_page_down(&mut self) {
        let i = match self.list_state.selected() {
            Some(i) => (i + 20).min(self.tlds.len().saturating_sub(1)),
            None => 0,
        };
        self.list_state.select(Some(i));
    }

    fn scroll_page_up(&mut self) {
        let i = match self.list_state.selected() {
            Some(i) => i.saturating_sub(20),
            None => 0,
        };
        self.list_state.select(Some(i));
    }

    fn scroll_to_top(&mut self) {
        self.list_state.select(Some(0));
    }

    fn scroll_to_bottom(&mut self) {
        self.list_state.select(Some(self.tlds.len().saturating_sub(1)));
    }

    fn start_checking(&self) {
        if self.query.is_empty() {
            return;
        }

        let prober = Prober::with_config(ProbeConfig {
            timeout: Duration::from_secs(5),
            whois_fallback: true,
            max_rate_per_endpoint: 20,
            max_concurrent_per_endpoint: 10,
        });

        if let Some(ref domain) = self.specific_domain {
            let domain = domain.clone();
            let status = Arc::clone(&self.specific_domain_status);
            let prober = prober.clone();
            
            tokio::spawn(async move {
                let result = prober.probe_one(&domain).await;
                
                let new_status = match result.availability {
                    Availability::Available => DomainStatus::Available,
                    Availability::Taken => DomainStatus::Taken,
                    Availability::Unknown { reason } => DomainStatus::Error(reason),
                };
                
                *status.lock().unwrap() = Some(new_status);
            });
        }

        let query = self.query.clone();
        let tlds = self.tlds.clone();
        let results = Arc::clone(&self.results);

        {
            let mut res = results.lock().unwrap();
            for tld in &tlds {
                res.insert(tld.clone(), DomainStatus::Checking);
            }
        }

        tokio::spawn(async move {
            let domains: Vec<String> = tlds.iter()
                .map(|tld| format!("{}.{}", query, tld))
                .collect();

            let mut stream = prober.probe_stream(domains);

            while let Some(result) = stream.next().await {
                let tld = result.domain
                    .rsplit('.')
                    .next()
                    .unwrap_or("")
                    .to_string();
                
                let status = match result.availability {
                    Availability::Available => DomainStatus::Available,
                    Availability::Taken => DomainStatus::Taken,
                    Availability::Unknown { reason } => DomainStatus::Error(reason),
                };
                
                let mut res = results.lock().unwrap();
                res.insert(tld, status);
            }
        });
    }

    fn get_sorted_results(&self) -> Vec<(String, DomainStatus)> {
        let results = self.results.lock().unwrap();
        let mut sorted: Vec<_> = self
            .tlds
            .iter()
            .map(|tld| (tld.clone(), results.get(tld).cloned().unwrap_or(DomainStatus::Pending)))
            .collect();

        sorted.sort_by(|a, b| {
            // First sort by priority (priority TLDs first)
            let a_priority = PRIORITY_TLDS.iter().position(|&t| t == a.0.as_str());
            let b_priority = PRIORITY_TLDS.iter().position(|&t| t == b.0.as_str());

            match (a_priority, b_priority) {
                (Some(a_pos), Some(b_pos)) => {
                    // Both are priority, maintain priority order
                    a_pos.cmp(&b_pos)
                }
                (Some(_), None) => std::cmp::Ordering::Less, // a is priority, comes first
                (None, Some(_)) => std::cmp::Ordering::Greater, // b is priority, comes first
                (None, None) => {
                    // Neither is priority, sort by status then alphabetically
                    let order_a = status_order(&a.1);
                    let order_b = status_order(&b.1);
                    order_a.cmp(&order_b).then_with(|| a.0.cmp(&b.0))
                }
            }
        });

        sorted
    }
}

const PRIORITY_TLDS: &[&str] = &[
    "com", "net", "org", "io", "ai", "dev", "app", "co", "me", "tech",
    "xyz", "online", "site", "store", "shop", "blog", "cloud", "digital",
    "eu", "us", "info", "email", "pro", "live", "zone", "team", "solutions"
];

fn get_builtin_tlds() -> Vec<String> {
    PRIORITY_TLDS.iter().map(|s| s.to_string()).collect()
}

/// Sort TLDs with priority TLDs first, then alphabetically
fn sort_tlds_with_priority(mut tlds: Vec<String>) -> Vec<String> {
    tlds.sort_by(|a, b| {
        let a_priority = PRIORITY_TLDS.iter().position(|&t| t == a.as_str());
        let b_priority = PRIORITY_TLDS.iter().position(|&t| t == b.as_str());

        match (a_priority, b_priority) {
            (Some(a_pos), Some(b_pos)) => a_pos.cmp(&b_pos),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => a.cmp(b),
        }
    });

    tlds
}

fn status_order(status: &DomainStatus) -> u8 {
    match status {
        DomainStatus::Available => 0,
        DomainStatus::Checking => 1,
        DomainStatus::Pending => 2,
        DomainStatus::Taken => 3,
        DomainStatus::Error(_) => 4,
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    if args.print_default_config {
        println!("{}", get_default_config_toml());
        return Ok(());
    }

    if args.write_default_config {
        if let Some(path) = config_path() {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&path, get_default_config_toml())?;
            println!("Default config written to: {}", path.display());
        } else {
            eprintln!("Error: Could not determine config path");
            std::process::exit(1);
        }
        return Ok(());
    }

    let config = load_config();

    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async {
        let user_specified_tlds = args.tlds.is_some();

        let default_tlds = if let Some(custom_tlds) = args.tlds {
            custom_tlds
        } else {
            let client = reqwest::Client::new();
            match fetch_iana_tlds(&client).await {
                Ok(tlds) => tlds,
                Err(e) => {
                    eprintln!("Warning: Failed to fetch from IANA ({}), using built-in list", e);
                    get_builtin_tlds()
                }
            }
        };

        let default_tlds = sort_tlds_with_priority(default_tlds);
        let default_tlds = apply_config_to_tlds(default_tlds, &config);

        let (query, extracted_tld, tlds) = if let Some(q) = args.query {
            let (base_name, extracted_tld) = parse_domain_query(&q);

            let final_tlds = if user_specified_tlds {
                default_tlds
            } else if let Some(ref tld) = extracted_tld {
                prioritize_tld(default_tlds, tld)
            } else {
                default_tlds
            };

            (base_name, extracted_tld, final_tlds)
        } else if args.ndjson {
            eprintln!("Error: Query required in NDJSON mode");
            std::process::exit(1);
        } else {
            return run_tui(None, None, default_tlds).await;
        };

        if args.ndjson {
            run_ndjson(query, tlds).await
        } else {
            run_tui(Some(query), extracted_tld, tlds).await
        }
    })
}

async fn run_ndjson(query: String, tlds: Vec<String>) -> Result<(), Box<dyn std::error::Error>> {
    let prober = Prober::with_config(ProbeConfig {
        timeout: Duration::from_secs(5),
        whois_fallback: true,
        max_rate_per_endpoint: 20,
        max_concurrent_per_endpoint: 10,
    });

    let domains: Vec<String> = tlds.iter()
        .map(|tld| format!("{}.{}", query, tld))
        .collect();

    let mut stream = prober.probe_stream(domains);

    while let Some(result) = stream.next().await {
        let tld = result.domain
            .rsplit('.')
            .next()
            .unwrap_or("")
            .to_string();
        
        let (available, status, error) = match result.availability {
            Availability::Available => (Some(true), AvailabilityStatus::Available, None),
            Availability::Taken => (Some(false), AvailabilityStatus::Taken, None),
            Availability::Unknown { reason } => (None, AvailabilityStatus::Error, Some(reason)),
        };
        
        let check_result = DomainCheckResult {
            query: query.clone(),
            tld,
            domain: result.domain,
            available,
            status,
            error,
        };
        
        if let Ok(json) = serde_json::to_string(&check_result) {
            println!("{}", json);
            io::stdout().flush()?;
        }
    }

    Ok(())
}

async fn run_tui(initial_query: Option<String>, specific_tld: Option<String>, tlds: Vec<String>) -> Result<(), Box<dyn std::error::Error>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new(initial_query, specific_tld, tlds);
    if !app.query.is_empty() {
        app.input_mode = false;
        app.start_checking();
    }

    let res = run_app(&mut terminal, &mut app).await;

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    if let Err(err) = res {
        println!("{:?}", err)
    }

    Ok(())
}

async fn run_app<B: Backend>(terminal: &mut Terminal<B>, app: &mut App) -> io::Result<()> {
    loop {
        app.tick = app.tick.wrapping_add(1);
        
        if let Some((_, created)) = &app.toast_message {
            if created.elapsed() > Duration::from_secs(2) {
                app.toast_message = None;
            }
        }
        
        terminal.draw(|f| ui(f, app))?;

        if app.quit {
            break;
        }

        if event::poll(Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                if app.input_mode {
                    match key.code {
                        KeyCode::Enter => {
                            if !app.query.is_empty() {
                                app.input_mode = false;
                                app.start_checking();
                            }
                        }
                        KeyCode::Char(c) => {
                            app.query.push(c);
                        }
                        KeyCode::Backspace => {
                            app.query.pop();
                        }
                        KeyCode::Esc => {
                            app.quit = true;
                        }
                        _ => {}
                    }
                } else {
                    match key.code {
                        KeyCode::Char('q') | KeyCode::Esc => {
                            app.quit = true;
                        }
                        KeyCode::Down | KeyCode::Char('j') => {
                            app.scroll_down();
                        }
                        KeyCode::Up | KeyCode::Char('k') => {
                            app.scroll_up();
                        }
                        KeyCode::PageDown => {
                            app.scroll_page_down();
                        }
                        KeyCode::PageUp => {
                            app.scroll_page_up();
                        }
                        KeyCode::Home | KeyCode::Char('g') => {
                            app.scroll_to_top();
                        }
                        KeyCode::End | KeyCode::Char('G') => {
                            app.scroll_to_bottom();
                        }
                        KeyCode::Char('i') => {
                            app.input_mode = true;
                        }
                        KeyCode::Enter | KeyCode::Char('y') => {
                            app.copy_selected_to_clipboard();
                        }
                        KeyCode::Char('o') => {
                            app.open_selected_in_browser();
                        }
                        KeyCode::Tab | KeyCode::Char('f') => {
                            app.filter_mode = app.filter_mode.next();
                            app.list_state.select(Some(0));
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    Ok(())
}

fn ui(f: &mut Frame, app: &mut App) {
    let has_specific = app.specific_domain.is_some();
    let has_toast = app.toast_message.is_some();
    
    let mut constraints = vec![Constraint::Length(3)];
    
    if has_specific {
        constraints.push(Constraint::Length(3));
    }
    
    constraints.push(Constraint::Length(1));
    constraints.push(Constraint::Min(1));
    
    if has_toast {
        constraints.push(Constraint::Length(1));
    }
    
    constraints.push(Constraint::Length(3));
    
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(f.area());

    let mut chunk_idx = 0;
    
    let input_text = if app.input_mode {
        format!("Query: {}_", app.query)
    } else {
        format!("Query: {} (press 'i' to edit)", app.query)
    };

    let input = Paragraph::new(input_text)
        .style(if app.input_mode {
            Style::default().fg(Color::Yellow)
        } else {
            Style::default()
        })
        .block(Block::default().borders(Borders::ALL).title("Domain Search"));
    f.render_widget(input, chunks[chunk_idx]);
    chunk_idx += 1;

    if has_specific {
        if let Some(ref domain) = app.specific_domain {
            let status = app.specific_domain_status.lock().unwrap().clone();
            
            let (symbol, color, status_text) = match &status {
                Some(DomainStatus::Available) => ("✓", Color::Green, "AVAILABLE".to_string()),
                Some(DomainStatus::Taken) => ("✗", Color::Red, "TAKEN".to_string()),
                Some(DomainStatus::Checking) => (app.spinner_frame(), Color::Yellow, "Checking...".to_string()),
                Some(DomainStatus::Error(e)) => ("!", Color::Magenta, e.clone()),
                Some(DomainStatus::Pending) | None => (app.spinner_frame(), Color::Yellow, "Checking...".to_string()),
            };
            
            let line = Line::from(vec![
                Span::styled(format!("  {} ", symbol), Style::default().fg(color).add_modifier(Modifier::BOLD)),
                Span::styled(domain.clone(), Style::default().fg(color).add_modifier(Modifier::BOLD)),
                Span::styled(format!("  {}", status_text), Style::default().fg(color)),
            ]);
            
            let specific_widget = Paragraph::new(line)
                .block(Block::default().borders(Borders::ALL).title("Specific Domain"));
            f.render_widget(specific_widget, chunks[chunk_idx]);
        }
        chunk_idx += 1;
    }

    let (done, total) = app.progress();
    let pct = if total > 0 { (done * 100) / total } else { 0 };
    let bar_width = (f.area().width as usize).saturating_sub(20);
    let filled = (bar_width * done) / total.max(1);
    let bar: String = "█".repeat(filled) + &"░".repeat(bar_width - filled);
    
    let progress_line = Line::from(vec![
        Span::styled(format!(" {} ", app.spinner_frame()), Style::default().fg(Color::Cyan)),
        Span::styled(bar, Style::default().fg(Color::Green)),
        Span::styled(format!(" {:>3}% ({}/{})", pct, done, total), Style::default().fg(Color::DarkGray)),
    ]);
    f.render_widget(Paragraph::new(progress_line), chunks[chunk_idx]);
    chunk_idx += 1;

    let results_chunk = chunks[chunk_idx];
    chunk_idx += 1;
    
    let toast_chunk = if has_toast {
        let c = chunks[chunk_idx];
        chunk_idx += 1;
        Some(c)
    } else {
        None
    };
    
    let help_chunk = chunks[chunk_idx];

    let results = app.get_filtered_results();
    let spinner = app.spinner_frame();
    let items: Vec<ListItem> = results
        .iter()
        .map(|(tld, status)| {
            let (symbol, color, text): (&str, Color, String) = match status {
                DomainStatus::Available => ("✓", Color::Green, "Available".to_string()),
                DomainStatus::Taken => ("✗", Color::Red, "Taken".to_string()),
                DomainStatus::Checking => (spinner, Color::Yellow, "Checking...".to_string()),
                DomainStatus::Pending => ("○", Color::DarkGray, "Pending".to_string()),
                DomainStatus::Error(e) => ("!", Color::Magenta, e.clone()),
            };

            let domain = if !app.query.is_empty() {
                format!("{}.{}", app.query, tld)
            } else {
                format!("*.{}", tld)
            };

            let line = Line::from(vec![
                Span::styled(format!("{} ", symbol), Style::default().fg(color).add_modifier(Modifier::BOLD)),
                Span::styled(format!("{:<30}", domain), Style::default().fg(Color::Cyan)),
                Span::styled(text, Style::default().fg(color)),
            ]);

            ListItem::new(line)
        })
        .collect();

    let all_results = app.get_sorted_results();
    let available_count = all_results.iter().filter(|(_, s)| matches!(s, DomainStatus::Available)).count();
    let taken_count = all_results.iter().filter(|(_, s)| matches!(s, DomainStatus::Taken)).count();

    let filter_indicator = match app.filter_mode {
        FilterMode::All => format!("[All:{}]", all_results.len()),
        FilterMode::Available => format!("[Available:{}]", available_count),
        FilterMode::Taken => format!("[Taken:{}]", taken_count),
    };

    let title = format!(
        "Results {} - Tab/f to filter",
        filter_indicator
    );

    let results_list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title(title))
        .style(Style::default().fg(Color::White))
        .highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD)
        )
        .highlight_symbol("» ");

    f.render_stateful_widget(results_list, results_chunk, &mut app.list_state);

    if let Some(chunk) = toast_chunk {
        if let Some((msg, _)) = &app.toast_message {
            let toast = Paragraph::new(Line::from(vec![
                Span::styled(" ✓ ", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
                Span::styled(msg.as_str(), Style::default().fg(Color::White)),
            ]));
            f.render_widget(toast, chunk);
        }
    }

    let help_text = if app.input_mode {
        "Enter: Search | Esc: Quit"
    } else {
        "↑↓/jk: Scroll | Tab/f: Filter | Enter/y: Copy | o: Open | i: Edit | q: Quit"
    };

    let help = Paragraph::new(help_text)
        .style(Style::default().fg(Color::DarkGray))
        .block(Block::default().borders(Borders::ALL).title("Help"));
    f.render_widget(help, help_chunk);
}
