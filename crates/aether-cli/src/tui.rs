//! Aether TUI — a ratatui-based interactive front-end for the aether coding agent.
//!
//! Launches when `aether.exe` is run with no task and an attached terminal. Provides:
//!   * a one-time Setup screen to enter an OpenAI-compatible API key + base URL,
//!   * a Home screen to type a task and run the agent,
//!   * a Run screen that shows progress and the final result.
//!
//! The CLI engine is reused as-is. The TUI is a thin shell: it builds the same
//! `Agent`, spawns `Agent::run` on the tokio runtime, and displays the outcome.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use crossterm::event::{Event, EventStream, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use futures_util::StreamExt;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph, Wrap};
use ratatui::{DefaultTerminal, Frame};
use tokio::sync::mpsc;

use aether_core::mode::Mode;
use aether_models::ModelProvider;

use crate::Cli;

// ---------------------------------------------------------------------------
// App state
// ---------------------------------------------------------------------------

/// A single text-entry field with cursor position.
#[derive(Default, Clone)]
struct Input {
    value: String,
    cursor: usize,
}
impl Input {
    fn insert(&mut self, c: char) {
        self.value.insert(self.cursor, c);
        self.cursor += c.len_utf8();
    }
    fn backspace(&mut self) {
        if self.cursor > 0 {
            let prev = self.value[..self.cursor].chars().last().unwrap();
            self.cursor -= prev.len_utf8();
            self.value.remove(self.cursor);
        }
    }
    fn delete(&mut self) {
        if self.cursor < self.value.len() {
            self.value.remove(self.cursor);
        }
    }
    fn left(&mut self) {
        if self.cursor > 0 {
            let prev = self.value[..self.cursor].chars().last().unwrap();
            self.cursor -= prev.len_utf8();
        }
    }
    fn right(&mut self) {
        if self.cursor < self.value.len() {
            let n = self.value[self.cursor..].chars().next().unwrap().len_utf8();
            self.cursor += n;
        }
    }
    fn home(&mut self) { self.cursor = 0; }
    fn end(&mut self) { self.cursor = self.value.len(); }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Screen { Setup, Home, Run }

#[derive(Clone, Copy, PartialEq, Eq)]
enum SetupFocus { ApiKey, BaseUrl, Save }

#[derive(Clone, Copy, PartialEq, Eq)]
enum RunPhase { Running, Done, Failed }

pub struct App {
    screen: Screen,
    cfg_path: PathBuf,

    // Setup
    setup_focus: SetupFocus,
    api_key: Input,
    base_url: Input,
    setup_status: Option<String>,

    // Home
    task: Input,
    home_status: Option<String>,
    models: Vec<(String, String)>, // (key, model name) for display

    // Run
    run_phase: RunPhase,
    run_started: Instant,
    run_task: String,
    run_result: String,
    run_error: Option<String>,

    // Wiring
    controller_model: String,
    executor_model: String,
    reviewer_model: Option<String>,
    provider: Option<Arc<dyn ModelProvider>>,
    providers: std::collections::HashMap<String, Arc<dyn ModelProvider>>,
    session_id: String,
    cwd: PathBuf,
    controller_provider: Arc<dyn ModelProvider>,

    // Run result channel
    result_rx: Option<mpsc::UnboundedReceiver<RunResult>>,
}

pub struct RunResult {
    pub ok: bool,
    pub text: String,
    pub error: Option<String>,
}

impl App {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        cfg_path: PathBuf,
        cfg_missing: bool,
        controller_provider: Arc<dyn ModelProvider>,
        controller_model: String,
        executor_model: String,
        reviewer_model: Option<String>,
        providers: std::collections::HashMap<String, Arc<dyn ModelProvider>>,
        session_id: String,
        cwd: PathBuf,
        initial_api_key: Option<String>,
        initial_base_url: String,
        models: Vec<(String, String)>,
    ) -> Self {
        let screen = if std::env::var("OPENAI_API_KEY").ok().is_some() || !cfg_missing {
            Screen::Home
        } else {
            Screen::Setup
        };
        App {
            screen,
            cfg_path,
            setup_focus: SetupFocus::ApiKey,
            api_key: Input { value: initial_api_key.unwrap_or_default(), cursor: 0 },
            base_url: Input { value: initial_base_url.clone(), cursor: initial_base_url.len() },
            setup_status: None,
            task: Input::default(),
            home_status: None,
            models,
            run_phase: RunPhase::Running,
            run_started: Instant::now(),
            run_task: String::new(),
            run_result: String::new(),
            run_error: None,
            controller_model,
            executor_model,
            reviewer_model,
            provider: None,
            providers,
            session_id,
            cwd,
            controller_provider,
            result_rx: None,
        }
    }

    fn save_setup(&mut self) {
        // Persist to config.toml and export OPENAI_API_KEY for the current process.
        let key = self.api_key.value.trim().to_string();
        let url = self.base_url.value.trim().to_string();
        if key.is_empty() {
            self.setup_status = Some("API key cannot be empty.".into());
            return;
        }
        if url.is_empty() {
            self.setup_status = Some("Base URL cannot be empty.".into());
            return;
        }
        std::env::set_var("OPENAI_API_KEY", &key);

        // Append a minimal [models.controller] + [agent] section to config.toml.
        let body = format!(
            "\n# written by `aether` setup on {}\n\
             [agent]\n  controller_model = \"controller\"\n  executor_model = \"executor\"\n\
             \n[models.controller]\n  provider = \"openai_compatible\"\n  base_url = \"{}\"\n  model = \"gpt-4o-mini\"\n  api_key_env = \"OPENAI_API_KEY\"\n\
             \n[models.executor]\n  provider = \"openai_compatible\"\n  base_url = \"{}\"\n  model = \"gpt-4o\"\n  api_key_env = \"OPENAI_API_KEY\"\n",
            chrono::Utc::now().to_rfc3339(),
            url, url,
        );
        match std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.cfg_path)
        {
            Ok(mut f) => {
                if std::io::Write::write_all(&mut f, body.as_bytes()).is_err() {
                    self.setup_status = Some("Failed to write config.toml".into());
                    return;
                }
            }
            Err(e) => {
                self.setup_status = Some(format!("Failed to open config.toml: {e}"));
                return;
            }
        }
        // Rebuild providers so the new env var takes effect.
        match rebuild_providers(&self.cfg_path) {
            Ok((ctrl, provs, review)) => {
                self.controller_provider = ctrl;
                self.providers = provs;
                self.provider = None;
                if let Some(r) = &review {
                    self.provider = Some(r.clone());
                }
                self.setup_status = Some("Saved. Press Enter to continue.".into());
            }
            Err(e) => {
                self.setup_status = Some(format!("Saved but failed to reload: {e}"));
            }
        }
    }

    fn start_run(&mut self) {
        let task = self.task.value.trim().to_string();
        if task.is_empty() {
            self.home_status = Some("Enter a task first.".into());
            return;
        }
        self.run_task = task.clone();
        self.run_phase = RunPhase::Running;
        self.run_started = Instant::now();
        self.run_result.clear();
        self.run_error = None;
        self.screen = Screen::Run;

        let (tx, rx) = mpsc::unbounded_channel();
        self.result_rx = Some(rx);

        let store_path = aether_config::Config::default_dir().join("sessions.db");
        let cwd = self.cwd.clone();
        let session_id = self.session_id.clone();
        let controller = self.controller_provider.clone();
        let providers = self.providers.clone();
        let controller_model = self.controller_model.clone();
        let executor_model = self.executor_model.clone();
        let reviewer_model = self.reviewer_model.clone();
        let frontend = match aether_config::Config::load(Some(self.cfg_path.clone())) {
            Ok(c) => c.frontend,
            Err(_) => aether_config::FrontendConfig::default(),
        };

        // The agent's run() future is `?Send` (visual review holds a
        // `?Send` CorrectionExecutor, plus the SessionStore wraps a
        // non-Send rusqlite::Connection). Run it on a dedicated single-thread
        // tokio runtime in its own OS thread so the TUI's multi-thread
        // runtime doesn't need to drag the future across threads.
        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build();
            let rt = match rt {
                Ok(r) => r,
                Err(e) => {
                    let _ = tx.send(RunResult { ok: false, text: String::new(), error: Some(format!("runtime init failed: {e}")) });
                    return;
                }
            };
            rt.block_on(async move {
                let store = match aether_sessions::SessionStore::open(&store_path) {
                    Ok(s) => s,
                    Err(e) => {
                        let _ = tx.send(RunResult { ok: false, text: String::new(), error: Some(format!("session store open failed: {e}")) });
                        return;
                    }
                };
                let reviewer = match &reviewer_model {
                    Some(key) => providers.get(key).cloned(),
                    None => None,
                };
                let mut tools: std::collections::HashMap<String, Arc<dyn aether_tools::Tool>> = std::collections::HashMap::new();
                for t in aether_tools::default_tools() {
                    tools.insert(t.name().to_string(), t);
                }
                let agent = aether_core::agent_loop::Agent::new(
                    controller,
                    controller_model,
                    executor_model,
                    providers,
                    Some(store),
                    session_id,
                    None,
                    None,
                    false,
                    8,
                    cwd,
                    aether_permissions::Policy {
                        read: aether_permissions::Permission::Allow,
                        edit: aether_permissions::Permission::Allow,
                        bash: aether_permissions::Permission::Ask,
                        delete: aether_permissions::Permission::Ask,
                        git_commit: aether_permissions::Permission::Ask,
                        network: aether_permissions::Permission::Ask,
                    },
                    tools,
                    true,
                    None,
                    30,
                    128000,
                    3,
                    reviewer,
                    reviewer_model,
                    frontend,
                );
                let outcome = agent.run(&task, Mode::Build, None, None).await;
                let result = match outcome {
                    Ok(o) => RunResult { ok: true, text: o.result, error: None },
                    Err(e) => RunResult { ok: false, text: String::new(), error: Some(e.to_string()) },
                };
                let _ = tx.send(result);
            });
        });
    }
}

// ---------------------------------------------------------------------------
// Rebuild providers after setup
// ---------------------------------------------------------------------------

fn rebuild_providers(
    cfg_path: &std::path::Path,
) -> Result<(Arc<dyn ModelProvider>, std::collections::HashMap<String, Arc<dyn ModelProvider>>, Option<Arc<dyn ModelProvider>>)> {
    let cfg = aether_config::Config::load(Some(cfg_path.to_path_buf()))?;
    let mut providers = std::collections::HashMap::new();
    for (k, mcfg) in &cfg.models {
        if let Ok(p) = aether_models::build_provider(mcfg) {
            providers.insert(k.clone(), Arc::from(p));
        }
    }
    let controller_cfg = cfg
        .model(&cfg.agent.controller_model)
        .ok_or_else(|| anyhow::anyhow!("controller model missing"))?;
    let controller: Arc<dyn ModelProvider> = Arc::from(aether_models::build_provider(controller_cfg)?);
    let reviewer = match &cfg.agent.reviewer_model {
        Some(key) => providers.get(key).cloned(),
        None => None,
    };
    Ok((controller, providers, reviewer))
}

// ---------------------------------------------------------------------------
// Event loop
// ---------------------------------------------------------------------------

pub async fn run(app: App) -> Result<()> {
    let terminal = ratatui::init();
    let result = run_inner(app, terminal).await;
    ratatui::restore();
    result
}

async fn run_inner(mut app: App, mut terminal: DefaultTerminal) -> Result<()> {
    let mut events = EventStream::new();
    let mut tick = tokio::time::interval(Duration::from_millis(100));

    loop {
        // Drain any pending run result.
        if let Some(rx) = &mut app.result_rx {
            match rx.try_recv() {
                Ok(r) => {
                    app.run_phase = if r.ok { RunPhase::Done } else { RunPhase::Failed };
                    app.run_result = r.text;
                    app.run_error = r.error;
                }
                Err(mpsc::error::TryRecvError::Empty) => {}
                Err(mpsc::error::TryRecvError::Disconnected) => {
                    app.result_rx = None;
                    app.run_phase = RunPhase::Failed;
                    app.run_error = Some("agent task ended unexpectedly".into());
                }
            }
        }

        terminal.draw(|f| ui(f, &app))?;

        let event = tokio::select! {
            _ = tick.tick() => None,
            ev = events.next() => ev,
        };
        if let Some(Ok(Event::Key(key))) = event {
            if key.kind == KeyEventKind::Press {
                handle_key(&mut app, key);
            }
        }

        if matches!(app.screen, Screen::Home) && app.task.value.contains("__quit__") {
            break;
        }
        // Hard exit on Ctrl-C (crossterm translates it to a key event in some envs).
        if matches!(
            event,
            Some(Ok(Event::Key(KeyEvent {
                code: KeyCode::Char('c'),
                modifiers: KeyModifiers::CONTROL,
                kind: KeyEventKind::Press,
                ..
            })))
        ) {
            break;
        }
        // Explicit quit: only on Home, press q (lowercase, no modifier).
        if matches!(app.screen, Screen::Home)
            && matches!(
                event,
                Some(Ok(Event::Key(KeyEvent {
                    code: KeyCode::Char('q'),
                    modifiers: KeyModifiers::NONE,
                    kind: KeyEventKind::Press,
                    ..
                })))
            )
            && app.task.value.is_empty()
        {
            break;
        }
    }
    Ok(())
}

fn handle_key(app: &mut App, key: KeyEvent) {
    match app.screen {
        Screen::Setup => handle_setup_key(app, key),
        Screen::Home => handle_home_key(app, key),
        Screen::Run => handle_run_key(app, key),
    }
}

fn handle_setup_key(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Tab | KeyCode::BackTab => {
            app.setup_focus = match app.setup_focus {
                SetupFocus::ApiKey => SetupFocus::BaseUrl,
                SetupFocus::BaseUrl => SetupFocus::Save,
                SetupFocus::Save => SetupFocus::ApiKey,
            };
        }
        KeyCode::Enter => {
            match app.setup_focus {
                SetupFocus::Save => app.save_setup(),
                SetupFocus::ApiKey => app.setup_focus = SetupFocus::BaseUrl,
                SetupFocus::BaseUrl => app.setup_focus = SetupFocus::Save,
            }
            // After save success, move to Home.
            if app.setup_focus == SetupFocus::Save
                && app.setup_status.as_deref() == Some("Saved. Press Enter to continue.")
            {
                app.screen = Screen::Home;
            }
        }
        KeyCode::Esc => {
            // Skip setup if user already has a key configured elsewhere.
            if std::env::var("OPENAI_API_KEY").is_ok() {
                app.screen = Screen::Home;
            }
        }
        KeyCode::Backspace => edit_input(app, key, |i| i.backspace()),
        KeyCode::Delete => edit_input(app, key, |i| i.delete()),
        KeyCode::Left => edit_input(app, key, |i| i.left()),
        KeyCode::Right => edit_input(app, key, |i| i.right()),
        KeyCode::Home => edit_input(app, key, |i| i.home()),
        KeyCode::End => edit_input(app, key, |i| i.end()),
        KeyCode::Char(c) => edit_input(app, key, |i| i.insert(c)),
        _ => {}
    }
}

fn edit_input<F: FnOnce(&mut Input)>(app: &mut App, _key: KeyEvent, f: F) {
    if app.screen == Screen::Setup {
        match app.setup_focus {
            SetupFocus::ApiKey => f(&mut app.api_key),
            SetupFocus::BaseUrl => f(&mut app.base_url),
            SetupFocus::Save => {}
        }
    } else if app.screen == Screen::Home {
        f(&mut app.task);
    } else if app.screen == Screen::Run {
        // Run screen has no editable input.
    }
}

fn handle_home_key(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Enter => {
            if !app.task.value.trim().is_empty() {
                app.start_run();
            }
        }
        KeyCode::Backspace => edit_input(app, key, |i| i.backspace()),
        KeyCode::Delete => edit_input(app, key, |i| i.delete()),
        KeyCode::Left => edit_input(app, key, |i| i.left()),
        KeyCode::Right => edit_input(app, key, |i| i.right()),
        KeyCode::Home => edit_input(app, key, |i| i.home()),
        KeyCode::End => edit_input(app, key, |i| i.end()),
        KeyCode::Char(c) => {
            if c != 'q' {
                edit_input(app, key, |i| i.insert(c));
            }
        }
        KeyCode::Esc => {
            // Move cursor to end (helps when input is long).
            app.task.end();
        }
        _ => {}
    }
}

fn handle_run_key(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Enter | KeyCode::Esc => {
            // Go back to Home once the run is finished.
            if matches!(app.run_phase, RunPhase::Done | RunPhase::Failed) {
                app.screen = Screen::Home;
                app.result_rx = None;
            }
        }
        KeyCode::Char('q') => {
            if matches!(app.run_phase, RunPhase::Done | RunPhase::Failed) {
                app.screen = Screen::Home;
                app.result_rx = None;
            }
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

const ACCENT: Color = Color::Rgb(124, 166, 180);
const INK: Color = Color::Rgb(232, 232, 232);
const MUTED: Color = Color::Rgb(140, 146, 150);
const WARN: Color = Color::Rgb(232, 169, 76);
const ERR: Color = Color::Rgb(188, 72, 74);

fn ui(f: &mut Frame, app: &App) {
    let area = f.area();
    // Outer block.
    let outer = Block::default().borders(Borders::ALL).title(Span::styled(
        " aether ",
        Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
    ));
    f.render_widget(outer, area);

    let inner = Rect {
        x: area.x + 1,
        y: area.y + 1,
        width: area.width.saturating_sub(2),
        height: area.height.saturating_sub(2),
    };

    match app.screen {
        Screen::Setup => render_setup(f, app, inner),
        Screen::Home => render_home(f, app, inner),
        Screen::Run => render_run(f, app, inner),
    }
}

fn render_setup(f: &mut Frame, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Length(3), // title
            Constraint::Length(3), // api key
            Constraint::Length(3), // base url
            Constraint::Length(3), // save
            Constraint::Length(3), // status
            Constraint::Min(0),
        ])
        .split(area);

    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("Welcome to aether.", Style::default().fg(INK).add_modifier(Modifier::BOLD)),
            Span::raw("  "),
            Span::styled(
                "Set your OpenAI-compatible API credentials to get started.",
                Style::default().fg(MUTED),
            ),
        ])),
        chunks[0],
    );

    let focused = |_focus: SetupFocus| Style::default().fg(ACCENT).add_modifier(Modifier::BOLD);
    let normal = Style::default().fg(INK);
    let label_dim = Style::default().fg(MUTED);

    let api_block = Block::default()
        .borders(Borders::ALL)
        .border_style(if app.setup_focus == SetupFocus::ApiKey { focused(SetupFocus::ApiKey) } else { normal })
        .title(Span::styled(" API key (env: OPENAI_API_KEY) ", label_dim));
    f.render_widget(api_block.clone(), chunks[1]);
    let api_inner = api_block.inner(chunks[1]);
    let (api_display, api_cursor) = masked(&app.api_key.value, app.api_key.cursor);
    let api_para = Paragraph::new(api_display).scroll((0, 0));
    f.render_widget(api_para, api_inner);
    if app.setup_focus == SetupFocus::ApiKey {
        f.set_cursor_position(Position {
            x: api_inner.x + api_cursor as u16,
            y: api_inner.y,
        });
    }

    let url_block = Block::default()
        .borders(Borders::ALL)
        .border_style(if app.setup_focus == SetupFocus::BaseUrl { focused(SetupFocus::BaseUrl) } else { normal })
        .title(Span::styled(" Base URL ", label_dim));
    f.render_widget(url_block.clone(), chunks[2]);
    let url_inner = url_block.inner(chunks[2]);
    f.render_widget(
        Paragraph::new(app.base_url.value.clone()),
        url_inner,
    );
    if app.setup_focus == SetupFocus::BaseUrl {
        f.set_cursor_position(Position {
            x: url_inner.x + app.base_url.cursor as u16,
            y: url_inner.y,
        });
    }

    let save_style = if app.setup_focus == SetupFocus::Save { focused(SetupFocus::Save) } else { normal };
    let save = Paragraph::new(Line::from(vec![
        Span::styled("[ ", save_style),
        Span::styled("Save", save_style.add_modifier(Modifier::BOLD)),
        Span::styled(" ]  ", save_style),
        Span::styled("(writes to ", label_dim),
        Span::styled(app.cfg_path.display().to_string(), Style::default().fg(INK)),
        Span::styled(")", label_dim),
    ]));
    f.render_widget(save, chunks[3]);

    if let Some(s) = &app.setup_status {
        let color = if s.starts_with("Saved") { WARN } else { ERR };
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(s.clone(), Style::default().fg(color)))),
            chunks[4],
        );
    }
}

fn render_home(f: &mut Frame, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Length(2), // help
            Constraint::Length(3), // task
            Constraint::Length(2), // status
            Constraint::Length(6), // models
            Constraint::Min(0),
        ])
        .split(area);

    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("aether", Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)),
            Span::raw("  ·  "),
            Span::styled("Type a task, Enter to run · q to quit", Style::default().fg(MUTED)),
        ])),
        chunks[0],
    );

    let task_block = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(" Task ", Style::default().fg(MUTED)));
    f.render_widget(task_block.clone(), chunks[1]);
    let task_inner = task_block.inner(chunks[1]);
    f.render_widget(Paragraph::new(app.task.value.clone()), task_inner);
    f.set_cursor_position(Position {
        x: task_inner.x + app.task.cursor as u16,
        y: task_inner.y,
    });

    if let Some(s) = &app.home_status {
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(s.clone(), Style::default().fg(WARN)))),
            chunks[2],
        );
    }

    let items: Vec<ListItem> = app
        .models
        .iter()
        .map(|(k, name)| {
            ListItem::new(Line::from(vec![
                Span::styled(format!("{k:<14} "), Style::default().fg(ACCENT)),
                Span::raw(name.clone()),
            ]))
        })
        .collect();
    let models = List::new(items)
        .block(Block::default().borders(Borders::ALL).title(Span::styled(
            " Models ",
            Style::default().fg(MUTED),
        )));
    f.render_widget(models, chunks[3]);
}

fn render_run(f: &mut Frame, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Length(3), // header
            Constraint::Min(6),    // output
            Constraint::Length(2), // status / hint
        ])
        .split(area);

    let status = match app.run_phase {
        RunPhase::Running => {
            let secs = app.run_started.elapsed().as_secs();
            Span::styled(format!(" Running...  ({secs}s)"), Style::default().fg(WARN))
        }
        RunPhase::Done => Span::styled(" Done.", Style::default().fg(ACCENT)),
        RunPhase::Failed => Span::styled(" Failed.", Style::default().fg(ERR)),
    };
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("Task: ", Style::default().fg(MUTED)),
            Span::styled(app.run_task.clone(), Style::default().fg(INK)),
            status,
        ])),
        chunks[0],
    );

    let body = match app.run_phase {
        RunPhase::Running => "(working... press Ctrl-C to force-quit)".to_string(),
        RunPhase::Done => app.run_result.clone(),
        RunPhase::Failed => app.run_error.clone().unwrap_or_else(|| "(unknown error)".into()),
    };
    let out_block = Block::default().borders(Borders::ALL).title(Span::styled(
        " Output ",
        Style::default().fg(MUTED),
    ));
    f.render_widget(out_block.clone(), chunks[1]);
    let para = Paragraph::new(body).wrap(Wrap { trim: false }).scroll((0, 0));
    f.render_widget(para, out_block.inner(chunks[1]));

    let hint = match app.run_phase {
        RunPhase::Running => Span::styled("waiting for result...", Style::default().fg(MUTED)),
        _ => Span::styled("Press Enter / q / Esc to go back.", Style::default().fg(MUTED)),
    };
    f.render_widget(Paragraph::new(Line::from(hint)), chunks[2]);
}

use ratatui::layout::Position;
fn masked(value: &str, cursor: usize) -> (String, usize) {
    // Replace all but the last 4 chars with •; preserve the cursor's effective position.
    let n = value.chars().count();
    if n <= 4 {
        (value.to_string(), cursor.min(value.len()))
    } else {
        let stars: String = "•".repeat(n - 4);
        let tail: String = value.chars().rev().take(4).collect::<Vec<_>>().into_iter().rev().collect();
        let display = format!("{stars}{tail}");
        let cur = cursor.min(display.len());
        (display, cur)
    }
}

// ---------------------------------------------------------------------------
// Public entry point used by main.rs
// ---------------------------------------------------------------------------

/// Construct and run the TUI. Returns when the user quits.
pub async fn run_tui(cli: Cli, _original_args: Vec<String>) -> Result<()> {
    // Reuse the same setup as the CLI's run() function. We can't share code
    // trivially, so duplicate the minimal config/provider wiring here.
    let cfg_path = cli.config.clone().unwrap_or_else(aether_config::Config::default_path);
    let cfg_missing = !cfg_path.exists();
    let cfg = aether_config::Config::load(cli.config.clone()).unwrap_or_default();

    // Base URL: prefer the controller model's base_url if we have one, otherwise
    // a sensible default.
    let base_url = cfg
        .model(&cfg.agent.controller_model)
        .map(|m| m.base_url.clone())
        .unwrap_or_else(|| "https://api.openai.com/v1".to_string());

    // Decide whether we can launch Home directly. If the OPENAI_API_KEY env
    // var is missing AND we don't have a config.toml with one, force the
    // user through Setup before we try to build any provider.
    let key_present = std::env::var("OPENAI_API_KEY").is_ok();

    // Build providers + controller.
    let mut providers: std::collections::HashMap<String, Arc<dyn ModelProvider>> = std::collections::HashMap::new();
    for (key, mcfg) in &cfg.models {
        if let Ok(p) = aether_models::build_provider(mcfg) {
            providers.insert(key.clone(), Arc::from(p));
        }
    }
    let controller_cfg = match cfg.model(&cfg.agent.controller_model) {
        Some(c) => c.clone(),
        None => {
            // No controller configured yet — show the setup screen.
            let placeholder = aether_config::ModelConfig {
                provider: "openai_compatible".into(),
                base_url: base_url.clone(),
                model: "gpt-4o-mini".into(),
                api_key_env: "OPENAI_API_KEY".into(),
                extra_body: None,
            };
            // We won't actually call the placeholder provider at runtime, but
            // build_provider still wants a real API key to be set. Provide a
            // dummy so the Setup screen can render even before the user
            // enters one — the runtime path never touches this provider.
            std::env::set_var("OPENAI_API_KEY", "PENDING_SETUP");
            let ctrl = aether_models::build_provider(&placeholder).ok();
            if std::env::var("OPENAI_API_KEY").as_deref() == Ok("PENDING_SETUP") {
                std::env::remove_var("OPENAI_API_KEY");
            }
            let ctrl = match ctrl {
                Some(p) => Arc::from(p),
                None => {
                    // Should never happen with a dummy key, but fall back.
                    return Err(anyhow::anyhow!("failed to build placeholder provider"));
                }
            };
            let app = App {
                screen: Screen::Setup,
                cfg_path: cfg_path.clone(),
                setup_focus: SetupFocus::ApiKey,
                api_key: Input::default(),
                base_url: Input { value: base_url.clone(), cursor: base_url.len() },
                setup_status: None,
                task: Input::default(),
                home_status: None,
                models: Vec::new(),
                run_phase: RunPhase::Running,
                run_started: Instant::now(),
                run_task: String::new(),
                run_result: String::new(),
                run_error: None,
                controller_model: cfg.agent.controller_model.clone(),
                executor_model: cfg.agent.executor_model.clone(),
                reviewer_model: cfg.agent.reviewer_model.clone(),
                provider: None,
                providers: std::collections::HashMap::new(),
                session_id: String::new(),
                cwd: std::env::current_dir()?,
                controller_provider: ctrl,
                result_rx: None,
            };
            return run(app).await;
        }
    };
    // If the API key is missing, route to Setup regardless of whether the
    // controller cfg exists.
    if !key_present {
        let placeholder = aether_config::ModelConfig {
            provider: "openai_compatible".into(),
            base_url: base_url.clone(),
            model: "gpt-4o-mini".into(),
            api_key_env: "OPENAI_API_KEY".into(),
            extra_body: None,
        };
        std::env::set_var("OPENAI_API_KEY", "PENDING_SETUP");
        let ctrl = aether_models::build_provider(&placeholder).ok();
        if std::env::var("OPENAI_API_KEY").as_deref() == Ok("PENDING_SETUP") {
            std::env::remove_var("OPENAI_API_KEY");
        }
        let ctrl = match ctrl {
            Some(p) => Arc::from(p),
            None => {
                // Could not even build a placeholder — give up.
                return Err(anyhow::anyhow!("failed to construct placeholder provider"));
            }
        };
        let app = App {
            screen: Screen::Setup,
            cfg_path: cfg_path.clone(),
            setup_focus: SetupFocus::ApiKey,
            api_key: Input::default(),
            base_url: Input { value: base_url.clone(), cursor: base_url.len() },
            setup_status: Some("Set OPENAI_API_KEY to enable aether.".into()),
            task: Input::default(),
            home_status: None,
            models: Vec::new(),
            run_phase: RunPhase::Running,
            run_started: Instant::now(),
            run_task: String::new(),
            run_result: String::new(),
            run_error: None,
            controller_model: cfg.agent.controller_model.clone(),
            executor_model: cfg.agent.executor_model.clone(),
            reviewer_model: cfg.agent.reviewer_model.clone(),
            provider: None,
            providers: std::collections::HashMap::new(),
            session_id: String::new(),
            cwd: std::env::current_dir()?,
            controller_provider: ctrl,
            result_rx: None,
        };
        return run(app).await;
    }
    let controller_provider: Arc<dyn ModelProvider> = Arc::from(aether_models::build_provider(&controller_cfg)?);

    let reviewer = match &cfg.agent.reviewer_model {
        Some(key) => providers.get(key).cloned(),
        None => None,
    };

    let models: Vec<(String, String)> = cfg
        .models
        .iter()
        .map(|(k, m)| (k.clone(), m.model.clone()))
        .collect();

    let session_id = aether_sessions::SessionStore::open(
        &aether_config::Config::default_dir().join("sessions.db"),
    )?
    .new_session()?;

    let mut app = App::new(
        cfg_path,
        cfg_missing,
        controller_provider,
        cfg.agent.controller_model.clone(),
        cfg.agent.executor_model.clone(),
        cfg.agent.reviewer_model.clone(),
        providers,
        session_id,
        std::env::current_dir()?,
        std::env::var("OPENAI_API_KEY").ok(),
        base_url,
        models,
    );
    app.provider = reviewer;
    run(app).await
}
