use std::collections::{HashMap, HashSet};
use std::io;
use std::sync::mpsc;
use std::sync::mpsc::{Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use anyhow::Context;
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table, TableState};

use crate::collector::Collector;
use crate::model::{SessionRow, SessionStatus, Snapshot};
use crate::names::SessionNameKey;
use crate::util::truncate_middle;

pub fn run_tui(
    collector: Collector,
    hosts: Vec<String>,
    refresh_ms: u64,
    debug: bool,
) -> anyhow::Result<()> {
    enable_raw_mode().context("enable raw mode")?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen).context("enter alternate screen")?;

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend).context("create terminal")?;
    terminal.clear().ok();

    let (cmd_tx, cmd_rx) = mpsc::channel::<WorkerCmd>();
    let (msg_tx, msg_rx) = mpsc::channel::<WorkerMsg>();

    let worker = thread::spawn(move || worker_loop(collector, hosts, debug, cmd_rx, msg_tx));

    let mut app = App::new(refresh_ms, debug, cmd_tx, msg_rx);
    app.request_refresh();

    let res = run_loop(&mut terminal, &mut app);

    // Stop the worker (drop sender, then join).
    drop(app);
    let _ = worker.join();

    disable_raw_mode().ok();
    execute!(io::stdout(), LeaveAlternateScreen).ok();
    terminal.show_cursor().ok();

    res
}

#[derive(Debug, Clone)]
enum WorkerCmd {
    Refresh,
    SetName { key: SessionNameKey, name: String },
    ClearName { key: SessionNameKey },
}

#[derive(Debug)]
enum WorkerMsg {
    Snapshot(Snapshot),
    Error(String),
    Status(String),
    NameUpdated {
        key: SessionNameKey,
        name: Option<String>,
    },
}

fn worker_loop(
    mut collector: Collector,
    hosts: Vec<String>,
    debug: bool,
    cmd_rx: Receiver<WorkerCmd>,
    msg_tx: mpsc::Sender<WorkerMsg>,
) {
    while let Ok(cmd) = cmd_rx.recv() {
        match cmd {
            WorkerCmd::Refresh => match collector.collect(&hosts, debug) {
                Ok(snap) => {
                    let _ = msg_tx.send(WorkerMsg::Snapshot(snap));
                }
                Err(e) => {
                    let _ = msg_tx.send(WorkerMsg::Error(format!("{e}")));
                }
            },
            WorkerCmd::SetName { key, name } => match collector.set_session_name(key.clone(), name)
            {
                Ok(normalized) => {
                    let _ = msg_tx.send(WorkerMsg::NameUpdated {
                        key: key.clone(),
                        name: normalized.clone(),
                    });
                    let tid = short_thread_id(&key.thread_id);
                    let _ = msg_tx.send(WorkerMsg::Status(format!(
                        "Saved name for ({}) {tid}",
                        key.host
                    )));
                }
                Err(e) => {
                    let _ = msg_tx.send(WorkerMsg::Error(format!("failed to save name: {e}")));
                }
            },
            WorkerCmd::ClearName { key } => match collector.clear_session_name(key.clone()) {
                Ok(()) => {
                    let _ = msg_tx.send(WorkerMsg::NameUpdated {
                        key: key.clone(),
                        name: None,
                    });
                    let tid = short_thread_id(&key.thread_id);
                    let _ = msg_tx.send(WorkerMsg::Status(format!(
                        "Cleared name for ({}) {tid}",
                        key.host
                    )));
                }
                Err(e) => {
                    let _ = msg_tx.send(WorkerMsg::Error(format!("failed to clear name: {e}")));
                }
            },
        }
    }
}

#[derive(Clone, Debug)]
struct SubagentSummary {
    total: usize,
    working: usize,
    unknown: usize,
    waiting: usize,
}

#[derive(Clone, Debug)]
struct DisplaySessionRow {
    root: SessionRow,
    status: SessionStatus,
    last_activity_unix_s: Option<i64>,
    reason: Option<String>,
    subagents: SubagentSummary,
}

fn group_sessions_for_display(sessions: &[SessionRow], debug: bool) -> Vec<DisplaySessionRow> {
    let mut ids: HashSet<(String, String)> = HashSet::new();
    for s in sessions {
        ids.insert((s.host.clone(), s.thread_id.clone()));
    }

    #[derive(Default)]
    struct Agg {
        root: Option<SessionRow>,
        subs: Vec<SessionRow>,
    }

    let mut groups: HashMap<(String, String), Agg> = HashMap::new();
    for s in sessions {
        let root_id = match s.subagent_parent_thread_id.as_ref() {
            Some(parent) if ids.contains(&(s.host.clone(), parent.clone())) => parent.clone(),
            _ => s.thread_id.clone(),
        };
        let key = (s.host.clone(), root_id.clone());
        let entry = groups.entry(key).or_default();
        if s.thread_id == root_id {
            entry.root = Some(s.clone());
        } else {
            entry.subs.push(s.clone());
        }
    }

    let mut out: Vec<DisplaySessionRow> = Vec::new();
    for ((_host, _root_id), agg) in groups {
        let Some(root) = agg.root else {
            // Shouldn't happen with the root-id selection fallback, but fail-loud by omission.
            continue;
        };

        let mut status_score: i32 = 0;
        let mut last_ts: Option<i64> = root.last_activity_unix_s;
        let mut sub_summary = SubagentSummary {
            total: agg.subs.len(),
            working: 0,
            unknown: 0,
            waiting: 0,
        };

        let mut all_rows: Vec<&SessionRow> = Vec::with_capacity(1 + agg.subs.len());
        all_rows.push(&root);
        for sub in &agg.subs {
            all_rows.push(sub);
            match sub.status {
                SessionStatus::Working => sub_summary.working += 1,
                SessionStatus::Unknown => sub_summary.unknown += 1,
                SessionStatus::Waiting => sub_summary.waiting += 1,
            }
        }

        for r in &all_rows {
            let score = match r.status {
                SessionStatus::Working => 2,
                SessionStatus::Unknown => 1,
                SessionStatus::Waiting => 0,
            };
            status_score = status_score.max(score);
            last_ts = match (last_ts, r.last_activity_unix_s) {
                (None, x) => x,
                (x, None) => x,
                (Some(a), Some(b)) => Some(a.max(b)),
            };
        }

        let status = match status_score {
            2 => SessionStatus::Working,
            1 => SessionStatus::Unknown,
            _ => SessionStatus::Waiting,
        };

        let reason = if debug {
            all_rows
                .iter()
                .filter(|r| r.status == status)
                .max_by_key(|r| r.last_activity_unix_s.unwrap_or(i64::MIN))
                .and_then(|r| r.debug.as_ref())
                .and_then(|d| d.status_reason.clone())
        } else {
            None
        };

        out.push(DisplaySessionRow {
            root,
            status,
            last_activity_unix_s: last_ts,
            reason,
            subagents: sub_summary,
        });
    }

    // Stable sort:
    // 1) named sessions first (scanability)
    // 2) most recent activity
    // 3) host, then thread id (deterministic tiebreakers)
    out.sort_by(|a, b| {
        let a_named = a.root.name.as_ref().is_some_and(|s| !s.trim().is_empty());
        let b_named = b.root.name.as_ref().is_some_and(|s| !s.trim().is_empty());
        let a_ts = a.last_activity_unix_s.unwrap_or(i64::MIN);
        let b_ts = b.last_activity_unix_s.unwrap_or(i64::MIN);
        b_named
            .cmp(&a_named)
            .then_with(|| b_ts.cmp(&a_ts))
            .then_with(|| a.root.host.cmp(&b.root.host))
            .then_with(|| a.root.thread_id.cmp(&b.root.thread_id))
    });

    out
}

struct App {
    refresh: Duration,
    debug: bool,
    refresh_in_flight: bool,
    last_refresh_sent: Instant,
    last_snapshot: Option<Snapshot>,
    display_sessions: Vec<DisplaySessionRow>,
    selected: Option<SessionNameKey>,
    rename_modal: Option<RenameModal>,
    last_error: Option<String>,
    last_status: Option<(Instant, String)>,
    last_warning_seen: Option<String>,
    cmd_tx: Sender<WorkerCmd>,
    msg_rx: Receiver<WorkerMsg>,
}

#[derive(Clone, Debug)]
struct RenameModal {
    key: SessionNameKey,
    buffer: String,
}

impl App {
    fn new(
        refresh_ms: u64,
        debug: bool,
        cmd_tx: Sender<WorkerCmd>,
        msg_rx: Receiver<WorkerMsg>,
    ) -> Self {
        Self {
            refresh: Duration::from_millis(refresh_ms.max(100)),
            debug,
            refresh_in_flight: false,
            last_refresh_sent: Instant::now() - Duration::from_secs(999),
            last_snapshot: None,
            display_sessions: Vec::new(),
            selected: None,
            rename_modal: None,
            last_error: None,
            last_status: None,
            last_warning_seen: None,
            cmd_tx,
            msg_rx,
        }
    }

    fn request_refresh(&mut self) {
        if self.refresh_in_flight {
            return;
        }
        self.refresh_in_flight = true;
        self.last_refresh_sent = Instant::now();
        let _ = self.cmd_tx.send(WorkerCmd::Refresh);
    }

    fn poll_worker(&mut self) {
        while let Ok(msg) = self.msg_rx.try_recv() {
            match msg {
                WorkerMsg::Snapshot(snap) => {
                    let names_warning = snap
                        .warnings
                        .as_ref()
                        .and_then(|w| w.iter().find(|s| s.starts_with("names store")))
                        .cloned();

                    self.display_sessions = group_sessions_for_display(&snap.sessions, self.debug);
                    self.last_snapshot = Some(snap);
                    self.last_error = None;
                    self.refresh_in_flight = false;
                    self.reconcile_selection();

                    if self.debug {
                        if let Some(w) = names_warning {
                            if self.last_warning_seen.as_deref() != Some(&w) {
                                self.last_warning_seen = Some(w.clone());
                                self.last_status = Some((Instant::now(), format!("WARN: {w}")));
                            }
                        }
                    }
                }
                WorkerMsg::Error(e) => {
                    self.last_error = Some(e);
                    if self.refresh_in_flight {
                        self.refresh_in_flight = false;
                    }
                }
                WorkerMsg::Status(msg) => {
                    self.last_status = Some((Instant::now(), msg));
                }
                WorkerMsg::NameUpdated { key, name } => {
                    if let Some(snap) = self.last_snapshot.as_mut() {
                        for row in &mut snap.sessions {
                            if row.host == key.host && row.thread_id == key.thread_id {
                                row.name = name.clone();
                            }
                        }
                        self.display_sessions =
                            group_sessions_for_display(&snap.sessions, self.debug);
                        self.reconcile_selection();
                    }
                    self.last_error = None;
                }
            }
        }
    }

    fn reconcile_selection(&mut self) {
        if self.display_sessions.is_empty() {
            self.selected = None;
            return;
        }

        if let Some(sel) = self.selected.as_ref() {
            if self
                .display_sessions
                .iter()
                .any(|s| s.root.host == sel.host && s.root.thread_id == sel.thread_id)
            {
                return;
            }
        }

        let first = &self.display_sessions[0].root;
        self.selected = Some(SessionNameKey {
            host: first.host.clone(),
            thread_id: first.thread_id.clone(),
        });
    }

    fn selected_index(&self) -> Option<usize> {
        let sel = self.selected.as_ref()?;
        self.display_sessions
            .iter()
            .position(|s| s.root.host == sel.host && s.root.thread_id == sel.thread_id)
    }

    fn select_prev(&mut self) {
        let Some(idx) = self.selected_index() else {
            self.reconcile_selection();
            return;
        };
        let next = idx.saturating_sub(1);
        let row = &self.display_sessions[next].root;
        self.selected = Some(SessionNameKey {
            host: row.host.clone(),
            thread_id: row.thread_id.clone(),
        });
    }

    fn select_next(&mut self) {
        let Some(idx) = self.selected_index() else {
            self.reconcile_selection();
            return;
        };
        let next = (idx + 1).min(self.display_sessions.len().saturating_sub(1));
        let row = &self.display_sessions[next].root;
        self.selected = Some(SessionNameKey {
            host: row.host.clone(),
            thread_id: row.thread_id.clone(),
        });
    }

    fn start_rename(&mut self) {
        self.reconcile_selection();
        let Some(sel) = self.selected.clone() else {
            return;
        };

        let existing = self
            .display_sessions
            .iter()
            .find(|s| s.root.host == sel.host && s.root.thread_id == sel.thread_id)
            .and_then(|s| s.root.name.clone())
            .unwrap_or_default();

        self.rename_modal = Some(RenameModal {
            key: sel,
            buffer: existing,
        });
    }

    fn commit_rename(&mut self) {
        let Some(modal) = self.rename_modal.take() else {
            return;
        };
        let key = modal.key;
        let trimmed = modal.buffer.trim().to_string();
        if trimmed.is_empty() {
            let _ = self.cmd_tx.send(WorkerCmd::ClearName { key });
        } else {
            let _ = self.cmd_tx.send(WorkerCmd::SetName { key, name: trimmed });
        }
    }

    fn clear_name(&mut self) {
        self.reconcile_selection();
        let Some(key) = self.selected.clone() else {
            return;
        };
        let _ = self.cmd_tx.send(WorkerCmd::ClearName { key });
    }

    fn handle_key(&mut self, code: KeyCode) -> bool {
        if self.rename_modal.is_some() {
            match code {
                KeyCode::Esc => self.rename_modal = None,
                KeyCode::Enter => self.commit_rename(),
                KeyCode::Backspace => {
                    if let Some(modal) = self.rename_modal.as_mut() {
                        modal.buffer.pop();
                    }
                }
                KeyCode::Char(c) => {
                    if !c.is_control() {
                        if let Some(modal) = self.rename_modal.as_mut() {
                            modal.buffer.push(c);
                        }
                    }
                }
                _ => {}
            }
            return false;
        }

        match code {
            KeyCode::Char('q') | KeyCode::Char('Q') | KeyCode::Esc => return true,
            KeyCode::Char('r') | KeyCode::Char('R') => self.request_refresh(),
            KeyCode::Up => self.select_prev(),
            KeyCode::Down => self.select_next(),
            KeyCode::Char('n') | KeyCode::Char('N') => self.start_rename(),
            KeyCode::Char('x') | KeyCode::Char('X') => self.clear_name(),
            _ => {}
        }
        false
    }
}

fn run_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
) -> anyhow::Result<()> {
    loop {
        if app.rename_modal.is_none() && app.last_refresh_sent.elapsed() >= app.refresh {
            app.request_refresh();
        }

        app.poll_worker();

        terminal.draw(|f| draw_ui(f, app)).context("draw ui")?;

        if event::poll(Duration::from_millis(50)).unwrap_or(false) {
            match event::read().context("read event")? {
                Event::Key(k) if k.kind == KeyEventKind::Press => {
                    if app.handle_key(k.code) {
                        return Ok(());
                    }
                }
                _ => {}
            }
        }
    }
}

fn draw_ui(f: &mut ratatui::Frame, app: &App) {
    let area = f.area();

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(2), Constraint::Min(3)].as_ref())
        .split(area);

    let header = header_line(app, chunks[0]);
    f.render_widget(header, chunks[0]);

    let table = sessions_table(app, chunks[1]);
    let mut state = TableState::default();
    state.select(app.selected_index());
    f.render_stateful_widget(table, chunks[1], &mut state);

    if let Some(modal) = app.rename_modal.as_ref() {
        render_rename_modal(f, modal, area);
    }
}

fn header_line(app: &App, area: Rect) -> Paragraph {
    let now = SystemTime::now();
    let display_rows = app.display_sessions.len();
    let raw_threads = app
        .last_snapshot
        .as_ref()
        .map(|s| s.sessions.len())
        .unwrap_or(0);
    let host_sel = app
        .last_snapshot
        .as_ref()
        .map(|s| s.host.as_str())
        .unwrap_or("?");
    let host_errs = app
        .last_snapshot
        .as_ref()
        .and_then(|s| s.host_errors.as_ref())
        .map(|v| v.len())
        .unwrap_or(0);

    let mut header_spans = Vec::new();
    header_spans.push(Span::styled(
        "codex-ps  ",
        Style::default().add_modifier(Modifier::BOLD),
    ));
    header_spans.push(Span::raw(format!("hosts: {host_sel}  ")));
    header_spans.push(Span::raw(format!("sessions: {display_rows}  ")));
    if raw_threads != display_rows {
        header_spans.push(Span::raw(format!("threads: {raw_threads}  ")));
    }
    if host_errs > 0 {
        header_spans.push(Span::styled(
            format!("errors: {host_errs}  "),
            Style::default().fg(Color::Red),
        ));
    }
    header_spans.push(Span::raw(format!(
        "refresh: {}ms  ",
        app.refresh.as_millis()
    )));

    if let Some(err) = app.last_error.as_ref() {
        header_spans.push(Span::styled(
            truncate_middle(err, area.width.saturating_sub(30) as usize),
            Style::default().fg(Color::Red),
        ));
    } else {
        let now_s = crate::util::system_time_to_unix_s(now).unwrap_or(0);
        let updated_s = app
            .last_snapshot
            .as_ref()
            .map(|s| s.generated_at_unix_s)
            .unwrap_or(0);
        let age = now_s.saturating_sub(updated_s);
        header_spans.push(Span::styled(
            format!("updated: {age}s ago"),
            Style::default().fg(Color::DarkGray),
        ));
    }

    let mut lines = Vec::new();
    lines.push(Line::from(header_spans));

    let mut help_spans = Vec::new();
    if app.rename_modal.is_some() {
        help_spans.push(Span::styled(
            "Keys: ",
            Style::default().add_modifier(Modifier::BOLD),
        ));
        help_spans.push(Span::raw("Enter save  Esc cancel  Backspace delete"));
    } else {
        help_spans.push(Span::styled(
            "Keys: ",
            Style::default().add_modifier(Modifier::BOLD),
        ));
        help_spans.push(Span::raw("↑/↓ select  n name  x clear  r refresh  q quit"));
    }

    if let Some((at, msg)) = app.last_status.as_ref() {
        if at.elapsed() <= Duration::from_secs(4) {
            help_spans.push(Span::raw("   "));
            help_spans.push(Span::styled(
                format!("Status: {msg}"),
                Style::default().fg(Color::Green),
            ));
        }
    }

    lines.push(Line::from(help_spans));

    Paragraph::new(lines).block(Block::default().borders(Borders::NONE))
}

fn sessions_table(app: &App, _area: Rect) -> Table {
    let sessions = app.display_sessions.as_slice();

    let mut header_cells = vec![
        Cell::from("HOST"),
        Cell::from("PID"),
        Cell::from("TID"),
        Cell::from("SUB"),
        Cell::from("STATE"),
        Cell::from("AGE"),
        Cell::from("NAME"),
        Cell::from("TITLE"),
        Cell::from("BRANCH"),
        Cell::from("PWD"),
    ];
    if app.debug {
        header_cells.push(Cell::from("WHY"));
    }

    let header = Row::new(header_cells)
        .style(Style::default().add_modifier(Modifier::BOLD))
        .bottom_margin(0);

    let rows = sessions.iter().map(|s| row_for_session(s, app.debug));

    // Rough width budget (60–120 cols). Keep it stable and let long cells truncate.
    let mut constraints = vec![
        Constraint::Length(6),  // HOST
        Constraint::Length(8),  // PID
        Constraint::Length(14), // TID
        Constraint::Length(10), // SUB
        Constraint::Length(5),  // STATE
        Constraint::Length(6),  // AGE
        Constraint::Length(22), // NAME
        Constraint::Length(18), // TITLE
        Constraint::Length(28), // BRANCH
        Constraint::Min(18),    // PWD
    ];
    if app.debug {
        constraints.push(Constraint::Min(18)); // WHY
    }

    Table::new(rows, constraints)
        .header(header)
        .block(
            Block::default()
                .borders(Borders::TOP)
                .title("Active Codex Sessions"),
        )
        .column_spacing(1)
        .highlight_symbol("> ")
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
}

fn short_thread_id(thread_id: &str) -> String {
    let tid = thread_id.trim();
    if tid.len() <= 14 {
        return tid.to_string();
    }
    let left = &tid[..8.min(tid.len())];
    let right = &tid[tid.len().saturating_sub(5)..];
    format!("{left}…{right}")
}

fn shorten_home_path(path: &str) -> String {
    let p = path.trim();
    let Some(home_os) = std::env::var_os("HOME") else {
        return p.to_string();
    };
    let home = home_os.to_string_lossy();
    if home.is_empty() {
        return p.to_string();
    }

    if p == home.as_ref() {
        return "~".into();
    }
    if let Some(rest) = p.strip_prefix(home.as_ref()) {
        if rest.starts_with(std::path::MAIN_SEPARATOR) {
            return format!("~{rest}");
        }
    }
    p.to_string()
}

fn format_subagents(s: &SubagentSummary, debug: bool) -> String {
    if s.total == 0 {
        return "0".into();
    }
    if !debug {
        return s.total.to_string();
    }
    let mut parts = Vec::new();
    if s.working > 0 {
        parts.push(format!("{}W", s.working));
    }
    if s.unknown > 0 {
        parts.push(format!("{}U", s.unknown));
    }
    if s.waiting > 0 {
        parts.push(format!("{}WT", s.waiting));
    }
    if parts.is_empty() {
        return s.total.to_string();
    }
    format!("{} ({})", s.total, parts.join("/"))
}

fn row_for_session(s: &DisplaySessionRow, debug: bool) -> Row {
    let pid = if s.root.pids.is_empty() {
        "unknown".to_string()
    } else if s.root.pids.len() == 1 {
        s.root.pids[0].to_string()
    } else {
        format!("{}+", s.root.pids[0])
    };

    let (state_text, state_style) = match s.status {
        SessionStatus::Working => ("WORK", Style::default().fg(Color::Green)),
        SessionStatus::Waiting => ("IDLE", Style::default().fg(Color::Yellow)),
        SessionStatus::Unknown => ("UNK", Style::default().fg(Color::Red)),
    };

    let tid = short_thread_id(&s.root.thread_id);
    let sub = format_subagents(&s.subagents, debug);

    let age = s
        .last_activity_unix_s
        .map(|ts| {
            let now = crate::util::system_time_to_unix_s(SystemTime::now()).unwrap_or(ts);
            let delta = now.saturating_sub(ts);
            if delta < 60 {
                format!("{delta}s")
            } else if delta < 3600 {
                format!("{}m", delta / 60)
            } else {
                format!("{}h", delta / 3600)
            }
        })
        .unwrap_or_else(|| "?".into());

    let title = s.root.title.as_deref().unwrap_or("unknown");
    let name = s
        .root
        .name
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("(unset)");
    let branch = s.root.git_branch.as_deref().unwrap_or("unknown");
    let why = s.reason.as_deref().unwrap_or("");

    let name = truncate_middle(name, 22);
    let title = truncate_middle(title, 18);
    let branch = branch.to_string();
    let pwd = s
        .root
        .cwd
        .as_deref()
        .map(shorten_home_path)
        .unwrap_or_else(|| "unknown".into());
    let pwd = truncate_middle(&pwd, 44);
    let host = truncate_middle(&s.root.host, 6);
    let why = truncate_middle(why, 60);

    let mut cells = vec![
        Cell::from(host),
        Cell::from(pid),
        Cell::from(tid),
        Cell::from(sub),
        Cell::from(Span::styled(state_text, state_style)),
        Cell::from(age),
        Cell::from(name),
        Cell::from(title),
        Cell::from(branch),
        Cell::from(pwd),
    ];
    if debug {
        cells.push(Cell::from(why));
    }

    let mut row = Row::new(cells);

    if debug {
        row = row.style(Style::default().fg(Color::White));
    }

    row
}

fn render_rename_modal(f: &mut ratatui::Frame, modal: &RenameModal, area: Rect) {
    let width = area.width.min(80).max(40);
    let height = area.height.min(9).max(7);
    let rect = centered_rect(width, height, area);

    f.render_widget(Clear, rect);

    let tid = short_thread_id(&modal.key.thread_id);
    let title = format!("Name session ({}) {tid}", modal.key.host);

    let input_max = rect.width.saturating_sub(4) as usize;
    let input = format!("> {}_", modal.buffer);
    let input = truncate_middle(&input, input_max);

    let lines = vec![
        Line::raw(""),
        Line::raw(input),
        Line::raw(""),
        Line::styled(
            "Enter = Save    Esc = Cancel",
            Style::default().fg(Color::DarkGray),
        ),
    ];

    let widget = Paragraph::new(lines).block(Block::default().borders(Borders::ALL).title(title));
    f.render_widget(widget, rect);
}

fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    let x = area.x + (area.width.saturating_sub(width) / 2);
    let y = area.y + (area.height.saturating_sub(height) / 2);
    Rect {
        x,
        y,
        width,
        height,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(thread_id: &str, name: Option<&str>, last_activity_unix_s: Option<i64>) -> SessionRow {
        SessionRow {
            host: "local".into(),
            thread_id: thread_id.into(),
            pids: Vec::new(),
            tty: None,
            title: Some("t".into()),
            name: name.map(|s| s.to_string()),
            cwd: None,
            repo_root: None,
            git_branch: None,
            git_commit: None,
            session_source: None,
            forked_from_id: None,
            subagent_parent_thread_id: None,
            subagent_depth: None,
            status: SessionStatus::Waiting,
            last_activity_unix_s,
            rollout_path: None,
            debug: None,
        }
    }

    #[test]
    fn named_rows_sort_above_unnamed_rows() {
        let named_old = row("a", Some("release triage"), Some(100));
        let unnamed_new = row("b", None, Some(200));

        let out = group_sessions_for_display(&[unnamed_new, named_old], false);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].root.thread_id, "a");
        assert_eq!(out[1].root.thread_id, "b");
    }
}
