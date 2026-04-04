use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph, Wrap};
use ratatui::Frame;

use super::app::{App, Mode};
use crate::core::stack::PatchStatus;

/// Main render dispatch.
pub fn render(frame: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(5),
            Constraint::Length(3),
        ])
        .split(frame.size());

    render_header(frame, app, chunks[0]);

    match &app.mode {
        Mode::DiffView => render_diff_view(frame, app, chunks[1]),
        Mode::HistoryView => render_history_view(frame, app, chunks[1]),
        _ => render_stack_view(frame, app, chunks[1]),
    }

    render_status_bar(frame, app, chunks[2]);
}

fn render_header(frame: &mut Frame, app: &App, area: Rect) {
    let mode_str = match &app.mode {
        Mode::Normal => "NORMAL",
        Mode::Select => "SELECT",
        Mode::DiffView => "DIFF",
        Mode::HistoryView => "HISTORY",
        Mode::Confirm { .. } => "CONFIRM",
    };

    let mode_color = match &app.mode {
        Mode::Normal => Color::Green,
        Mode::Select => Color::Yellow,
        Mode::DiffView => Color::Magenta,
        Mode::Confirm { .. } => Color::Red,
        _ => Color::Blue,
    };

    let mut spans = vec![
        Span::styled(
            " pilegit ",
            Style::default().fg(Color::Black).bg(Color::Cyan).bold(),
        ),
        Span::raw("  "),
        Span::styled(
            format!(" {} ", mode_str),
            Style::default().fg(Color::Black).bg(mode_color).bold(),
        ),
        Span::raw("  "),
        Span::styled(
            format!("base: {} │ {} commits", app.stack.base, app.stack.len()),
            Style::default().fg(Color::DarkGray),
        ),
    ];

    if app.history.can_undo() {
        spans.push(Span::raw("  "));
        spans.push(Span::styled(
            format!("undo:{}", app.history.position()),
            Style::default().fg(Color::DarkGray),
        ));
    }

    let header = Paragraph::new(Line::from(spans)).block(
        Block::default()
            .borders(Borders::BOTTOM)
            .border_style(Style::default().fg(Color::DarkGray)),
    );
    frame.render_widget(header, area);
}

fn render_stack_view(frame: &mut Frame, app: &App, area: Rect) {
    if app.stack.is_empty() {
        let empty = Paragraph::new("  No commits in stack. Branch is up to date with base.")
            .style(Style::default().fg(Color::DarkGray))
            .block(
                Block::default()
                    .title(" Stack ")
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::DarkGray)),
            );
        frame.render_widget(empty, area);
        return;
    }

    let selection = app.selection_range();
    let n = app.stack.len();

    // Iterate in reverse so newest commit appears at the top of the list.
    // The data index `i` still maps correctly to app.cursor and selection.
    let items: Vec<ListItem> = (0..n)
        .rev()
        .map(|i| {
            let patch = &app.stack.patches[i];
            let is_cursor = i == app.cursor;
            let is_selected = selection.map_or(false, |(lo, hi)| i >= lo && i <= hi);
            let is_expanded = app.expanded == Some(i);

            let pos_marker = if is_cursor { "▶" } else { " " };
            let connector = if i == n - 1 { "┌" } else if i == 0 { "└" } else { "│" };

            let status_icon = match patch.status {
                PatchStatus::Clean => "●",
                PatchStatus::Conflict => "✗",
                PatchStatus::Editing => "✎",
                PatchStatus::Submitted => "◈",
                PatchStatus::Merged => "✓",
            };

            let status_color = match patch.status {
                PatchStatus::Clean => Color::Green,
                PatchStatus::Conflict => Color::Red,
                PatchStatus::Editing => Color::Yellow,
                PatchStatus::Submitted => Color::Cyan,
                PatchStatus::Merged => Color::DarkGray,
            };

            let hash_short = &patch.hash[..patch.hash.len().min(8)];

            let mut spans = vec![
                Span::styled(
                    format!(" {} ", pos_marker),
                    if is_cursor {
                        Style::default().fg(Color::Cyan).bold()
                    } else {
                        Style::default().fg(Color::DarkGray)
                    },
                ),
                Span::styled(format!("{} ", connector), Style::default().fg(Color::DarkGray)),
                Span::styled(format!("{} ", status_icon), Style::default().fg(status_color)),
                Span::styled(
                    format!("{} ", hash_short),
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::DIM),
                ),
                Span::styled(
                    patch.subject.clone(),
                    if is_cursor {
                        Style::default().fg(Color::White).bold()
                    } else if is_selected {
                        Style::default().fg(Color::Cyan)
                    } else {
                        Style::default().fg(Color::Gray)
                    },
                ),
            ];

            if let Some(pr) = patch.pr_number {
                spans.push(Span::styled(
                    format!("  PR#{}", pr),
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::DIM),
                ));
            }

            let mut lines = vec![Line::from(spans)];

            if is_expanded {
                lines.push(Line::from(vec![
                    Span::raw("       "),
                    Span::styled(
                        format!("{} • {}", patch.author, patch.timestamp),
                        Style::default().fg(Color::DarkGray).italic(),
                    ),
                ]));
                if !patch.body.is_empty() {
                    for body_line in patch.body.lines().take(5) {
                        lines.push(Line::from(vec![
                            Span::raw("       "),
                            Span::styled(body_line.to_string(), Style::default().fg(Color::DarkGray)),
                        ]));
                    }
                }
            }

            let style = if is_selected {
                Style::default().bg(Color::DarkGray)
            } else {
                Style::default()
            };

            ListItem::new(lines).style(style)
        })
        .collect();

    let list = List::new(items).block(
        Block::default()
            .title(" Stack (newest on top) ")
            .title_style(Style::default().fg(Color::Cyan).bold())
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray)),
    );

    frame.render_widget(list, area);

    // Confirm overlay
    if let Mode::Confirm { ref prompt, .. } = app.mode {
        render_confirm_dialog(frame, prompt, area);
    }
}

fn render_diff_view(frame: &mut Frame, app: &App, area: Rect) {
    let visible_height = area.height.saturating_sub(2) as usize;
    let start = app.diff_scroll;
    let end = (start + visible_height).min(app.diff_content.len());
    let visible: Vec<Line> = app.diff_content[start..end]
        .iter()
        .map(|line| {
            let color = if line.starts_with('+') && !line.starts_with("+++") {
                Color::Green
            } else if line.starts_with('-') && !line.starts_with("---") {
                Color::Red
            } else if line.starts_with("@@") {
                Color::Cyan
            } else if line.starts_with("diff") || line.starts_with("index") {
                Color::Yellow
            } else {
                Color::Gray
            };
            Line::from(Span::styled(line.clone(), Style::default().fg(color)))
        })
        .collect();

    let title = if !app.stack.is_empty() && app.cursor < app.stack.len() {
        format!(" Diff: {} ", app.stack.patches[app.cursor].subject)
    } else {
        " Diff ".to_string()
    };

    let diff = Paragraph::new(visible)
        .block(
            Block::default()
                .title(title)
                .title_style(Style::default().fg(Color::Magenta).bold())
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::DarkGray)),
        )
        .wrap(Wrap { trim: false });

    frame.render_widget(diff, area);
}

fn render_history_view(frame: &mut Frame, app: &App, area: Rect) {
    let entries = app.history.list();
    let items: Vec<ListItem> = entries
        .iter()
        .enumerate()
        .rev()
        .map(|(i, entry)| {
            let marker = if i + 1 == app.history.position() {
                "→"
            } else {
                " "
            };
            ListItem::new(Line::from(vec![
                Span::styled(format!(" {} ", marker), Style::default().fg(Color::Cyan)),
                Span::styled(
                    format!("{} ", entry.timestamp.format("%H:%M:%S")),
                    Style::default().fg(Color::DarkGray),
                ),
                Span::styled(
                    entry.description.clone(),
                    Style::default().fg(Color::Gray),
                ),
                Span::styled(
                    format!("  ({} patches)", entry.snapshot.len()),
                    Style::default().fg(Color::DarkGray),
                ),
            ]))
        })
        .collect();

    let list = List::new(items).block(
        Block::default()
            .title(" Undo History ")
            .title_style(Style::default().fg(Color::Blue).bold())
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray)),
    );
    frame.render_widget(list, area);
}

fn render_status_bar(frame: &mut Frame, app: &App, area: Rect) {
    let status = Paragraph::new(Line::from(vec![
        Span::raw(" "),
        Span::styled(&app.status_msg, Style::default().fg(Color::DarkGray)),
    ]))
    .block(
        Block::default()
            .borders(Borders::TOP)
            .border_style(Style::default().fg(Color::DarkGray)),
    );
    frame.render_widget(status, area);
}

fn render_confirm_dialog(frame: &mut Frame, prompt: &str, parent_area: Rect) {
    let width = (prompt.len() as u16 + 6).min(parent_area.width.saturating_sub(4));
    let height = 3;
    let x = parent_area.x + (parent_area.width.saturating_sub(width)) / 2;
    let y = parent_area.y + (parent_area.height.saturating_sub(height)) / 2;
    let dialog_area = Rect::new(x, y, width, height);

    // Clear background
    frame.render_widget(
        Paragraph::new("").style(Style::default().bg(Color::Black)),
        dialog_area,
    );

    let dialog = Paragraph::new(Line::from(vec![Span::styled(
        format!(" {} ", prompt),
        Style::default().fg(Color::Yellow).bold(),
    )]))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Yellow)),
    );
    frame.render_widget(dialog, dialog_area);
}
