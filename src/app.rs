use std::collections::{HashMap, HashSet};
use std::io;
use std::sync::mpsc;
use std::sync::mpsc::{Receiver, SyncSender};
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
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table};

use crate::collector::Collector;
use crate::model::{SessionRow, SessionStatus, Snapshot};
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

    let (cmd_tx, cmd_rx) = mpsc::sync_channel::<WorkerCmd>(1);
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

#[derive(Debug, Clone, Copy)]
enum WorkerCmd {
    Refresh,
}

#[derive(Debug)]
enum WorkerMsg {
    Snapshot(Snapshot),
    Error(String),
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

    // Stable sort: most recent activity first, then host, then thread id.
    out.sort_by(|a, b| {
        let a_ts = a.last_activity_unix_s.unwrap_or(i64::MIN);
        let b_ts = b.last_activity_unix_s.unwrap_or(i64::MIN);
        b_ts.cmp(&a_ts)
            .then_with(|| a.root.host.cmp(&b.root.host))
            .then_with(|| a.root.thread_id.cmp(&b.root.thread_id))
    });

    out
}

struct App {
    refresh: Duration,
    debug: bool,
    last_refresh_request: Instant,
    last_snapshot: Option<Snapshot>,
    display_sessions: Vec<DisplaySessionRow>,
    last_error: Option<String>,
    cmd_tx: SyncSender<WorkerCmd>,
    msg_rx: Receiver<WorkerMsg>,
}

impl App {
    fn new(
        refresh_ms: u64,
        debug: bool,
        cmd_tx: SyncSender<WorkerCmd>,
        msg_rx: Receiver<WorkerMsg>,
    ) -> Self {
        Self {
            refresh: Duration::from_millis(refresh_ms.max(100)),
            debug,
            last_refresh_request: Instant::now() - Duration::from_secs(999),
            last_snapshot: None,
            display_sessions: Vec::new(),
            last_error: None,
            cmd_tx,
            msg_rx,
        }
    }

    fn request_refresh(&mut self) {
        self.last_refresh_request = Instant::now();
        let _ = self.cmd_tx.try_send(WorkerCmd::Refresh);
    }

    fn poll_worker(&mut self) {
        while let Ok(msg) = self.msg_rx.try_recv() {
            match msg {
                WorkerMsg::Snapshot(snap) => {
                    self.display_sessions = group_sessions_for_display(&snap.sessions, self.debug);
                    self.last_snapshot = Some(snap);
                    self.last_error = None;
                }
                WorkerMsg::Error(e) => self.last_error = Some(e),
            }
        }
    }
}

fn run_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
) -> anyhow::Result<()> {
    loop {
        if app.last_refresh_request.elapsed() >= app.refresh {
            app.request_refresh();
        }

        app.poll_worker();

        terminal.draw(|f| draw_ui(f, app)).context("draw ui")?;

        if event::poll(Duration::from_millis(50)).unwrap_or(false) {
            match event::read().context("read event")? {
                Event::Key(k) if k.kind == KeyEventKind::Press => match k.code {
                    KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                    KeyCode::Char('r') => app.request_refresh(),
                    _ => {}
                },
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
    f.render_widget(table, chunks[1]);
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

    let mut spans = Vec::new();
    spans.push(Span::styled(
        "codex-ps  ",
        Style::default().add_modifier(Modifier::BOLD),
    ));
    spans.push(Span::raw(format!("hosts: {host_sel}  ")));
    spans.push(Span::raw(format!("sessions: {display_rows}  ")));
    if raw_threads != display_rows {
        spans.push(Span::raw(format!("threads: {raw_threads}  ")));
    }
    if host_errs > 0 {
        spans.push(Span::styled(
            format!("errors: {host_errs}  "),
            Style::default().fg(Color::Red),
        ));
    }
    spans.push(Span::raw(format!(
        "refresh: {}ms  ",
        app.refresh.as_millis()
    )));

    if let Some(err) = app.last_error.as_ref() {
        spans.push(Span::styled(
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
        spans.push(Span::styled(
            format!("updated: {age}s ago"),
            Style::default().fg(Color::DarkGray),
        ));
    }

    Paragraph::new(Line::from(spans)).block(Block::default().borders(Borders::NONE))
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
        Constraint::Length(18), // TITLE
        Constraint::Length(36), // BRANCH
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
        .highlight_style(Style::default())
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
        SessionStatus::Waiting => ("WAIT", Style::default().fg(Color::Yellow)),
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
    let branch = s.root.git_branch.as_deref().unwrap_or("unknown");
    let why = s.reason.as_deref().unwrap_or("");

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
