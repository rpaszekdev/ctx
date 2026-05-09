use crate::rate;
use crate::state::{CtxState, LogEntry, TrackedSymbol};
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::ExecutableCommand;
use ratatui::prelude::*;
use ratatui::widgets::*;
use std::collections::BTreeMap;
use std::io::stdout;

#[derive(Clone, Copy, PartialEq)]
enum Panel {
    Modules,
    Symbols,
    Log,
    Timeline,
}

const PANELS: [Panel; 4] = [Panel::Modules, Panel::Symbols, Panel::Log, Panel::Timeline];

struct App {
    state: CtxState,
    running_daemon: bool,
    active_panel: usize,
    module_list: Vec<ModuleInfo>,
    selected_module: usize,
    symbol_scroll: usize,
    log_scroll: usize,
    quit: bool,
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
    fn new(state: CtxState, running_daemon: bool) -> Self {
        let module_list = build_module_list(&state);
        Self {
            state,
            running_daemon,
            active_panel: 0,
            module_list,
            selected_module: 0,
            symbol_scroll: 0,
            log_scroll: 0,
            quit: false,
        }
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

pub fn render_view(state: &CtxState, running: bool) {
    enable_raw_mode().expect("failed to enable raw mode");
    stdout().execute(EnterAlternateScreen).expect("failed to enter alternate screen");
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout())).expect("failed to create terminal");

    let mut app = App::new(state.clone(), running);

    loop {
        terminal.draw(|f| draw(f, &app)).expect("failed to draw");

        if event::poll(std::time::Duration::from_millis(100)).unwrap_or(false) {
            if let Ok(Event::Key(key)) = event::read() {
                if key.kind == KeyEventKind::Press {
                    app.handle_key(key.code);
                }
            }
        }

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
    let hot_count = app.state.symbols.values().filter(|s| s.trail_strength > 0.5).count();
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
        let dots = trail_dots(sym.trail_strength);
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

        let line = Line::from(vec![
            Span::styled(&dt, Style::default().fg(Color::DarkGray)),
            Span::raw(" "),
            Span::styled(&entry.op, Style::default().fg(op_color).bold()),
            Span::raw(" "),
            Span::styled(truncate(short_path, 16).to_string(), Style::default().fg(Color::White)),
        ]);

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
        info.total_strength += sym.trail_strength;
        if sym.trail_strength > info.max_strength {
            info.max_strength = sym.trail_strength;
        }
        info.symbols.push(sym.clone());
    }

    for (name, info) in modules.iter_mut() {
        info.wph = rate::writes_per_hour(&state.rates, name);
        info.symbols.sort_by(|a, b| {
            b.trail_strength.partial_cmp(&a.trail_strength).unwrap_or(std::cmp::Ordering::Equal)
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

fn trail_dots(strength: f64) -> String {
    let filled = strength.clamp(0.0, 5.0) as usize;
    let empty = 5 - filled;
    format!("{}{}", "●".repeat(filled), "○".repeat(empty))
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max { s.to_string() } else { format!("{}…", &s[..max - 1]) }
}
