use crate::config::CtxConfig;
use crate::rate;
use crate::state::{self, CtxState, LogEntry, TrackedSymbol};
use crate::trails;
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::ExecutableCommand;
use ratatui::prelude::*;
use ratatui::widgets::*;
use std::collections::BTreeMap;
use std::io::stdout;
use std::time::SystemTime;

#[derive(Clone, Copy, PartialEq)]
enum Panel {
    Modules,
    Symbols,
    Log,
    Timeline,
}

const PANELS: [Panel; 4] = [Panel::Modules, Panel::Symbols, Panel::Log, Panel::Timeline];

struct App {
    cfg: CtxConfig,
    state: CtxState,
    trails: Vec<trails::Trail>,
    last_mtime: Option<SystemTime>,
    running_daemon: bool,
    active_panel: usize,
    module_list: Vec<ModuleInfo>,
    selected_module: usize,
    symbol_scroll: usize,
    log_scroll: usize,
    quit: bool,
}

fn state_mtime(cfg: &CtxConfig) -> Option<SystemTime> {
    std::fs::metadata(cfg.ctx_path.join(".state")).ok()?.modified().ok()
}

#[derive(Clone)]
struct ModuleInfo {
    name: String,
    sym_count: usize,
    total_strength: f64,
    max_strength: f64,
    wph: usize,
    symbols: Vec<TrackedSymbol>,
}

impl App {
    fn new(cfg: CtxConfig, state: CtxState, running_daemon: bool) -> Self {
        let module_list = build_module_list(&state);
        let last_mtime = state_mtime(&cfg);
        let worker_trails = trails::load(&cfg);
        Self {
            cfg,
            state,
            trails: worker_trails,
            last_mtime,
            running_daemon,
            active_panel: 0,
            module_list,
            selected_module: 0,
            symbol_scroll: 0,
            log_scroll: 0,
            quit: false,
        }
    }

    /// Re-read `.state` when the daemon has rewritten it. Cheap enough to call
    /// every tick: one `stat` unless the file actually moved.
    fn refresh(&mut self) {
        let mtime = state_mtime(&self.cfg);
        if mtime == self.last_mtime {
            return;
        }
        self.last_mtime = mtime;

        // ponytail: the daemon writes .state non-atomically, so a torn read just
        // fails to parse and we keep the current view until the next tick.
        // Upgrade path: have save() write .state.tmp and rename it into place.
        let Some(new_state) = state::try_load(&self.cfg) else { return };

        // Trails are appended by workers on their own schedule; reload alongside
        // state so attribution catches up with the changes it explains.
        self.trails = trails::load(&self.cfg);

        let selected = self.module_list.get(self.selected_module).map(|m| m.name.clone());
        self.state = new_state;
        self.module_list = build_module_list(&self.state);

        // Modules re-sort as heat shifts — follow the selection by name, not index.
        self.selected_module = selected
            .and_then(|name| self.module_list.iter().position(|m| m.name == name))
            .unwrap_or(0);

        let sym_max = self.selected_module_symbols().len().saturating_sub(1);
        self.symbol_scroll = self.symbol_scroll.min(sym_max);
        self.log_scroll = self.log_scroll.min(self.state.log.len().saturating_sub(1));
    }

    fn selected_module_symbols(&self) -> &[TrackedSymbol] {
        self.module_list.get(self.selected_module).map(|m| m.symbols.as_slice()).unwrap_or(&[])
    }

    fn handle_key(&mut self, key: KeyCode) {
        match key {
            KeyCode::Char('q') | KeyCode::Esc => self.quit = true,
            KeyCode::Tab => {
                self.active_panel = (self.active_panel + 1) % PANELS.len();
            }
            KeyCode::BackTab => {
                self.active_panel = if self.active_panel == 0 { PANELS.len() - 1 } else { self.active_panel - 1 };
            }
            KeyCode::Up | KeyCode::Char('k') => self.move_up(),
            KeyCode::Down | KeyCode::Char('j') => self.move_down(),
            _ => {}
        }
    }

    fn move_up(&mut self) {
        match PANELS[self.active_panel] {
            Panel::Modules => {
                if self.selected_module > 0 {
                    self.selected_module -= 1;
                    self.symbol_scroll = 0;
                }
            }
            Panel::Symbols => {
                if self.symbol_scroll > 0 { self.symbol_scroll -= 1; }
            }
            Panel::Log => {
                if self.log_scroll > 0 { self.log_scroll -= 1; }
            }
            Panel::Timeline => {}
        }
    }

    fn move_down(&mut self) {
        match PANELS[self.active_panel] {
            Panel::Modules => {
                if self.selected_module + 1 < self.module_list.len() {
                    self.selected_module += 1;
                    self.symbol_scroll = 0;
                }
            }
            Panel::Symbols => {
                let max = self.selected_module_symbols().len().saturating_sub(1);
                if self.symbol_scroll < max { self.symbol_scroll += 1; }
            }
            Panel::Log => {
                let max = self.state.log.len().saturating_sub(1);
                if self.log_scroll < max { self.log_scroll += 1; }
            }
            Panel::Timeline => {}
        }
    }
}

pub fn render_view(cfg: &CtxConfig, state: &CtxState, running: bool) {
    enable_raw_mode().expect("failed to enable raw mode");
    stdout().execute(EnterAlternateScreen).expect("failed to enter alternate screen");
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout())).expect("failed to create terminal");

    let mut app = App::new(cfg.clone(), state.clone(), running);

    loop {
        terminal.draw(|f| draw(f, &app)).expect("failed to draw");

        if event::poll(std::time::Duration::from_millis(100)).unwrap_or(false) {
            if let Ok(Event::Key(key)) = event::read() {
                if key.kind == KeyEventKind::Press {
                    app.handle_key(key.code);
                }
            }
        }

        app.refresh();

        if app.quit { break; }
    }

    disable_raw_mode().expect("failed to disable raw mode");
    stdout().execute(LeaveAlternateScreen).expect("failed to leave alternate screen");
}

fn draw(f: &mut Frame, app: &App) {
    let size = f.area();

    let header_area = Rect::new(size.x, size.y, size.width, 3);
    let body_area = Rect::new(size.x, size.y + 3, size.width, size.height.saturating_sub(5));
    let footer_area = Rect::new(size.x, size.height.saturating_sub(2), size.width, 2);

    draw_header(f, header_area, app);
    draw_body(f, body_area, app);
    draw_footer(f, footer_area, app);
}

fn draw_header(f: &mut Frame, area: Rect, app: &App) {
    let sym_count = app.state.symbols.len();
    let log_count = app.state.log.len();
    let hot_count = app.state.symbols.values().filter(|s| s.total_heat() > 0.5).count();
    let status = if app.running_daemon { "▶ running" } else { "■ stopped" };
    let status_color = if app.running_daemon { Color::Green } else { Color::Red };

    let title = Line::from(vec![
        Span::styled(" ctx ", Style::default().fg(Color::Cyan).bold()),
        Span::raw("│ "),
        Span::styled(format!("{} sym", sym_count), Style::default().fg(Color::White)),
        Span::raw(" │ "),
        Span::styled(format!("{} hot", hot_count), Style::default().fg(Color::Yellow)),
        Span::raw(" │ "),
        Span::styled(format!("{} log", log_count), Style::default().fg(Color::White)),
        Span::raw(" │ "),
        Span::styled(status, Style::default().fg(status_color)),
    ]);

    let block = Block::default().borders(Borders::BOTTOM).border_style(Style::default().fg(Color::DarkGray));
    let para = Paragraph::new(title).block(block);
    f.render_widget(para, area);
}

fn draw_body(f: &mut Frame, area: Rect, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(35), Constraint::Percentage(35), Constraint::Percentage(30)])
        .split(area);

    draw_modules_panel(f, chunks[0], app);
    draw_center_panel(f, chunks[1], app);
    draw_right_panel(f, chunks[2], app);
}

fn draw_modules_panel(f: &mut Frame, area: Rect, app: &App) {
    let is_active = PANELS[app.active_panel] == Panel::Modules;
    let border_color = if is_active { Color::Cyan } else { Color::DarkGray };

    let block = Block::default()
        .title(" Modules [Tab] ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color));

    let inner = block.inner(area);
    f.render_widget(block, area);

    let items: Vec<ListItem> = app.module_list.iter().enumerate().map(|(i, m)| {
        let heat_bar = heat_bar_str(m.total_strength, 8);
        let activity = rate::activity_label(m.wph);
        let style = if i == app.selected_module {
            Style::default().bg(Color::DarkGray).fg(Color::White)
        } else {
            Style::default()
        };

        let strength_color = if m.total_strength > 10.0 { Color::Red }
            else if m.total_strength > 5.0 { Color::Yellow }
            else { Color::Cyan };

        ListItem::new(Line::from(vec![
            Span::styled(heat_bar.clone(), Style::default().fg(strength_color)),
            Span::raw(" "),
            Span::styled(
                format!("{:<18}", truncate(&m.name, 18)),
                style,
            ),
            Span::styled(format!(" {:>2}", m.sym_count), Style::default().fg(Color::DarkGray)),
            Span::raw(" "),
            Span::styled(activity.to_string(), Style::default().fg(Color::DarkGray)),
        ]))
    }).collect();

    let list = List::new(items);
    f.render_widget(list, inner);
}

fn draw_center_panel(f: &mut Frame, area: Rect, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    draw_symbols_panel(f, chunks[0], app);
    draw_timeline_panel(f, chunks[1], app);
}

fn draw_symbols_panel(f: &mut Frame, area: Rect, app: &App) {
    let is_active = PANELS[app.active_panel] == Panel::Symbols;
    let border_color = if is_active { Color::Cyan } else { Color::DarkGray };

    let module_name = app.module_list.get(app.selected_module).map(|m| m.name.as_str()).unwrap_or("—");
    let title = format!(" {} ", module_name);

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color));

    let inner = block.inner(area);
    f.render_widget(block, area);

    let symbols = app.selected_module_symbols();
    let visible = inner.height as usize;
    let start = app.symbol_scroll;
    let end = (start + visible).min(symbols.len());

    let items: Vec<ListItem> = symbols[start..end].iter().map(|sym| {
        let dots = trail_dots(sym.total_heat());
        let kind_color = match sym.kind.as_str() {
            "type" => Color::Yellow,
            "function" => Color::Green,
            "component" => Color::Magenta,
            _ => Color::White,
        };

        let deps_info = if !sym.used_by.is_empty() {
            format!(" ◄{}", sym.used_by.len())
        } else {
            String::new()
        };

        ListItem::new(Line::from(vec![
            Span::styled(dots.clone(), Style::default().fg(Color::Yellow)),
            Span::raw(" "),
            Span::styled(
                format!("{:<20}", truncate(&sym.name, 20)),
                Style::default().fg(kind_color),
            ),
            Span::styled(
                format!("{:<8}", &sym.kind),
                Style::default().fg(Color::DarkGray),
            ),
            Span::styled(deps_info, Style::default().fg(Color::Red)),
        ]))
    }).collect();

    let list = List::new(items);
    f.render_widget(list, inner);
}

fn draw_timeline_panel(f: &mut Frame, area: Rect, app: &App) {
    let is_active = PANELS[app.active_panel] == Panel::Timeline;
    let border_color = if is_active { Color::Cyan } else { Color::DarkGray };

    let block = Block::default()
        .title(" Timeline ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color));

    let inner = block.inner(area);
    f.render_widget(block, area);

    if app.state.log.is_empty() { return; }

    let first_ts = app.state.log.first().map(|e| e.timestamp).unwrap_or(0);
    let last_ts = app.state.log.last().map(|e| e.timestamp).unwrap_or(0);
    let span = (last_ts - first_ts).max(1);
    let width = inner.width as usize;

    let mut module_buckets: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for entry in &app.state.log {
        let module = entry.path.split('/').next().unwrap_or("?").to_string();
        let pos = ((entry.timestamp - first_ts) as f64 / span as f64 * (width - 1) as f64) as usize;
        module_buckets.entry(module).or_insert_with(|| vec![0; width])[pos.min(width - 1)] += 1;
    }

    let mut sorted: Vec<_> = module_buckets.into_iter().collect();
    sorted.sort_by(|a, b| {
        let at: usize = a.1.iter().sum();
        let bt: usize = b.1.iter().sum();
        bt.cmp(&at)
    });

    let max_rows = inner.height as usize;
    let chars = [' ', '░', '▒', '▓', '█'];

    for (i, (module, counts)) in sorted.iter().take(max_rows).enumerate() {
        let max_c = *counts.iter().max().unwrap_or(&1).max(&1);
        let row: String = counts.iter().take(width.saturating_sub(10)).map(|&c| {
            let level = if max_c > 0 { (c as f64 / max_c as f64 * 4.0) as usize } else { 0 };
            chars[level.min(4)]
        }).collect();

        let total: usize = counts.iter().sum();
        let color = if total >= 20 { Color::Red } else if total >= 10 { Color::Yellow } else { Color::Cyan };

        let line = Line::from(vec![
            Span::styled(format!("{:<8}", truncate(module, 8)), Style::default().fg(color)),
            Span::raw("│"),
            Span::styled(row, Style::default().fg(color)),
            Span::styled(format!("{:>3}", total), Style::default().fg(Color::DarkGray)),
        ]);

        if i < inner.height as usize {
            f.render_widget(Paragraph::new(line), Rect::new(inner.x, inner.y + i as u16, inner.width, 1));
        }
    }
}

fn draw_right_panel(f: &mut Frame, area: Rect, app: &App) {
    let is_active = PANELS[app.active_panel] == Panel::Log;
    let border_color = if is_active { Color::Cyan } else { Color::DarkGray };

    let block = Block::default()
        .title(" Log ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color));

    let inner = block.inner(area);
    f.render_widget(block, area);

    let visible = inner.height as usize;
    let total = app.state.log.len();
    let start = if total > app.log_scroll + visible { total - app.log_scroll - visible } else { 0 };
    let end = (start + visible).min(total);

    let entries: Vec<&LogEntry> = app.state.log[start..end].iter().rev().collect();

    for (i, entry) in entries.iter().enumerate() {
        if i >= inner.height as usize { break; }

        let dt = chrono::DateTime::from_timestamp(entry.timestamp, 0)
            .map(|d| d.format("%H:%M").to_string())
            .unwrap_or_default();

        let op_color = match entry.op.as_str() {
            "W" => Color::Green,
            "D" => Color::Red,
            _ => Color::White,
        };

        let short_path = entry.path.split('/').last().unwrap_or(&entry.path);
        let worker = trails::attribute(&app.trails, &entry.path, entry.timestamp);

        // Unattributed changes keep the full width; the worker tag only steals
        // space from the path when there is actually a worker to name.
        let path_width = if worker.is_some() { 10 } else { 16 };

        let mut spans = vec![
            Span::styled(&dt, Style::default().fg(Color::DarkGray)),
            Span::raw(" "),
            Span::styled(&entry.op, Style::default().fg(op_color).bold()),
            Span::raw(" "),
            Span::styled(truncate(short_path, path_width).to_string(), Style::default().fg(Color::White)),
        ];

        if let Some(w) = worker {
            spans.push(Span::raw(" "));
            spans.push(Span::styled(
                truncate(&w, 10).to_string(),
                Style::default().fg(Color::Cyan),
            ));
        }

        let line = Line::from(spans);

        f.render_widget(Paragraph::new(line), Rect::new(inner.x, inner.y + i as u16, inner.width, 1));
    }
}

fn draw_footer(f: &mut Frame, area: Rect, app: &App) {
    let panel_names = ["Modules", "Symbols", "Log", "Timeline"];
    let spans: Vec<Span> = panel_names.iter().enumerate().map(|(i, name)| {
        if i == app.active_panel {
            Span::styled(format!(" {} ", name), Style::default().fg(Color::Black).bg(Color::Cyan).bold())
        } else {
            Span::styled(format!(" {} ", name), Style::default().fg(Color::DarkGray))
        }
    }).collect();

    let mut all_spans = vec![Span::raw(" ")];
    for (i, s) in spans.into_iter().enumerate() {
        all_spans.push(s);
        if i < 3 { all_spans.push(Span::raw(" │ ")); }
    }
    all_spans.push(Span::raw("    "));
    all_spans.push(Span::styled("Tab", Style::default().fg(Color::Cyan)));
    all_spans.push(Span::raw(" cycle  "));
    all_spans.push(Span::styled("↑↓", Style::default().fg(Color::Cyan)));
    all_spans.push(Span::raw(" navigate  "));
    all_spans.push(Span::styled("q", Style::default().fg(Color::Cyan)));
    all_spans.push(Span::raw(" quit"));

    let footer = Paragraph::new(Line::from(all_spans))
        .block(Block::default().borders(Borders::TOP).border_style(Style::default().fg(Color::DarkGray)));
    f.render_widget(footer, area);
}

fn build_module_list(state: &CtxState) -> Vec<ModuleInfo> {
    let mut modules: BTreeMap<String, ModuleInfo> = BTreeMap::new();

    for sym in state.symbols.values() {
        let parts: Vec<&str> = sym.fqn.split('/').collect();
        let module = if parts.len() >= 2 {
            parts[..parts.len() - 1].join("/")
        } else {
            parts[0].to_string()
        };

        let info = modules.entry(module.clone()).or_insert(ModuleInfo {
            name: module,
            sym_count: 0,
            total_strength: 0.0,
            max_strength: 0.0,
            wph: 0,
            symbols: Vec::new(),
        });
        info.sym_count += 1;
        info.total_strength += sym.total_heat();
        if sym.total_heat() > info.max_strength {
            info.max_strength = sym.total_heat();
        }
        info.symbols.push(sym.clone());
    }

    for (name, info) in modules.iter_mut() {
        info.wph = rate::writes_per_hour(&state.rates, name);
        info.symbols.sort_by(|a, b| {
            b.total_heat().partial_cmp(&a.total_heat()).unwrap_or(std::cmp::Ordering::Equal)
        });
    }

    let mut sorted: Vec<_> = modules.into_values().collect();
    sorted.sort_by(|a, b| {
        b.total_strength.partial_cmp(&a.total_strength).unwrap_or(std::cmp::Ordering::Equal)
    });
    sorted
}

fn heat_bar_str(strength: f64, width: usize) -> String {
    let filled = ((strength / 5.0) * width as f64).clamp(0.0, width as f64) as usize;
    let empty = width - filled;
    format!("{}{}", "█".repeat(filled), "░".repeat(empty))
}

/// Five buckets spanning the range heat actually occupies.
///
/// The old scale was `strength as usize`, which saturated at 1.0 — and every
/// touched symbol starts at 1.0, so every row rendered five filled dots and the
/// column carried no information. These thresholds are spread over the observed
/// distribution instead, so a symbol edited once reads differently from one
/// edited repeatedly.
pub fn trail_dots(strength: f64) -> String {
    let filled = match strength {
        s if s >= 4.0 => 5,
        s if s >= 2.5 => 4,
        s if s >= 1.5 => 3,
        s if s >= 0.75 => 2,
        s if s > 0.0 => 1,
        _ => 0,
    };
    format!("{}{}", "●".repeat(filled), "○".repeat(5 - filled))
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max { s.to_string() } else { format!("{}…", &s[..max - 1]) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn scratch(name: &str) -> CtxConfig {
        let root = std::env::temp_dir().join(name);
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join(".ctx")).unwrap();
        CtxConfig::new(root)
    }

    fn state_with(names: &[&str]) -> CtxState {
        let mut st = CtxState::new();
        for n in names {
            st.add_symbol(
                format!("app/mod/{}", n),
                n.to_string(),
                "function".into(),
                PathBuf::from("src/mod.ts"),
                1,
                1.0,
            );
        }
        st
    }

    #[test]
    fn refresh_picks_up_daemon_writes() {
        let cfg = scratch("ctx-view-refresh-test");
        state::save(&cfg, &state_with(&["alpha"]));

        let mut app = App::new(cfg.clone(), state::load(&cfg), true);
        assert_eq!(app.state.symbols.len(), 1);

        // Simulate the daemon recording a second symbol.
        std::thread::sleep(std::time::Duration::from_millis(10));
        state::save(&cfg, &state_with(&["alpha", "beta"]));

        app.refresh();
        assert_eq!(app.state.symbols.len(), 2, "refresh should reload .state");
        assert_eq!(app.module_list.iter().map(|m| m.sym_count).sum::<usize>(), 2);
    }

    #[test]
    fn refresh_is_a_noop_when_state_is_unchanged() {
        let cfg = scratch("ctx-view-noop-test");
        state::save(&cfg, &state_with(&["alpha"]));

        let mut app = App::new(cfg.clone(), state::load(&cfg), true);
        app.selected_module = 0;
        app.log_scroll = 0;
        app.refresh();

        assert_eq!(app.state.symbols.len(), 1);
    }

    #[test]
    fn refresh_keeps_current_view_on_a_torn_write() {
        let cfg = scratch("ctx-view-torn-test");
        state::save(&cfg, &state_with(&["alpha"]));
        let mut app = App::new(cfg.clone(), state::load(&cfg), true);

        // A half-flushed .state must not blank the view.
        std::thread::sleep(std::time::Duration::from_millis(10));
        std::fs::write(cfg.ctx_path.join(".state"), "{\"symbols\": {\"a\"").unwrap();

        app.refresh();
        assert_eq!(app.state.symbols.len(), 1, "torn read should keep the old state");
    }
}
