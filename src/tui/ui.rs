use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::Frame;

use super::app::{App, Mode};
use crate::core::stack::PatchStatus;

/// Main render dispatch.
pub fn render(frame: &mut Frame, app: &App) {
    let width = frame.size().width as usize;
    let shortcut_lines = build_shortcut_lines(app.shortcuts(), width);
    let status_height = shortcut_lines.len() + if app.notification.is_some() { 1 } else { 0 };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),                    // header
            Constraint::Min(5),                       // main content
            Constraint::Length(status_height as u16), // status bar
        ])
        .split(frame.size());

    render_header(frame, app, chunks[0]);

    match &app.mode {
        Mode::DiffView => render_diff_view(frame, app, chunks[1]),
        Mode::HistoryView => render_history_view(frame, app, chunks[1]),
        Mode::Help => render_help_view(frame, app, chunks[1]),
        _ => render_stack_view(frame, app, chunks[1]),
    }

    render_status_bar(frame, app, chunks[2], shortcut_lines);
}

fn render_header(frame: &mut Frame, app: &App, area: Rect) {
    let mode_str = match &app.mode {
        Mode::Normal => "NORMAL",
        Mode::Select => "SELECT",
        Mode::DiffView => "DIFF",
        Mode::HistoryView => "HISTORY",
        Mode::Help => "HELP",
        Mode::InsertChoice => "INSERT",
        Mode::Confirm { .. } => "CONFIRM",
    };

    let mode_color = match &app.mode {
        Mode::Normal => Color::Green,
        Mode::Select => Color::Yellow,
        Mode::DiffView => Color::Magenta,
        Mode::Help | Mode::HistoryView => Color::Blue,
        Mode::InsertChoice => Color::Cyan,
        Mode::Confirm { .. } => Color::Red,
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

    frame.render_widget(Paragraph::new(Line::from(spans)), area);
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

    // Render newest (highest index) at the top of the list
    let items: Vec<ListItem> = (0..n)
        .rev()
        .map(|i| {
            let patch = &app.stack.patches[i];
            let is_cursor = i == app.cursor;
            let is_selected = selection.map_or(false, |(lo, hi)| i >= lo && i <= hi);
            let is_expanded = app.expanded == Some(i);

            let pos_marker = if is_cursor { "▶" } else { " " };
            let connector = if i == n - 1 {
                "┌"
            } else if i == 0 {
                "└"
            } else {
                "│"
            };

            let (status_icon, status_color) = match patch.status {
                PatchStatus::Clean => ("●", Color::Green),
                PatchStatus::Conflict => ("✗", Color::Red),
                PatchStatus::Editing => ("✎", Color::Yellow),
                PatchStatus::Submitted => ("◈", Color::Cyan),
                PatchStatus::Merged => ("✓", Color::DarkGray),
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
                Span::styled(
                    format!("{} ", connector),
                    Style::default().fg(Color::DarkGray),
                ),
                Span::styled(
                    format!("{} ", status_icon),
                    Style::default().fg(status_color),
                ),
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

            if let Some(pr_num) = patch.pr_number {
                spans.push(Span::styled(
                    format!("  PR#{}", pr_num),
                    Style::default().fg(Color::Cyan).bold(),
                ));
            } else if patch.pr_branch.is_some() {
                spans.push(Span::styled(
                    "  ◈ submitted",
                    Style::default().fg(Color::Cyan).add_modifier(Modifier::DIM),
                ));
            }

            let mut lines = vec![Line::from(spans)];

            // Expanded: show author, timestamp, PR branch, and body preview
            if is_expanded {
                lines.push(Line::from(vec![
                    Span::raw("       "),
                    Span::styled(
                        format!("{} • {}", patch.author, patch.timestamp),
                        Style::default().fg(Color::DarkGray).italic(),
                    ),
                ]));
                if let Some(ref branch) = patch.pr_branch {
                    lines.push(Line::from(vec![
                        Span::raw("       "),
                        Span::styled(
                            format!("branch: {}", branch),
                            Style::default().fg(Color::Cyan).add_modifier(Modifier::DIM),
                        ),
                    ]));
                }
                if let Some(ref url) = patch.pr_url {
                    lines.push(Line::from(vec![
                        Span::raw("       "),
                        Span::styled(
                            url.clone(),
                            Style::default()
                                .fg(Color::Blue)
                                .add_modifier(Modifier::UNDERLINED),
                        ),
                    ]));
                }
                for body_line in patch.body.lines().take(5) {
                    lines.push(Line::from(vec![
                        Span::raw("       "),
                        Span::styled(body_line.to_string(), Style::default().fg(Color::DarkGray)),
                    ]));
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

    let list = List::new(items)
        .block(
            Block::default()
                .title(" Stack (newest on top) ")
                .title_style(Style::default().fg(Color::Cyan).bold())
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::DarkGray)),
        )
        .highlight_symbol(""); // we handle the cursor marker ourselves

    // Use ListState so ratatui auto-scrolls to keep the cursor visible.
    // Visual index: items are reversed, so cursor at data index `i`
    // is at visual position `n - 1 - i`.
    let visual_cursor = n - 1 - app.cursor;
    let mut list_state = ListState::default();
    list_state.select(Some(visual_cursor));
    frame.render_stateful_widget(list, area, &mut list_state);

    // Confirm and InsertChoice overlays
    match &app.mode {
        Mode::Confirm { ref prompt, .. } => render_overlay(frame, prompt, Color::Yellow, area),
        Mode::InsertChoice => render_overlay(
            frame,
            "Insert: (a) after cursor  (t) at top  (Esc) cancel",
            Color::Cyan,
            area,
        ),
        _ => {}
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
            let marker = if i == app.history.position() {
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
                Span::styled(entry.description.clone(), Style::default().fg(Color::Gray)),
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

fn render_help_view(frame: &mut Frame, app: &App, area: Rect) {
    let lines: Vec<Line> = app
        .help_text()
        .lines()
        .map(|line| {
            if line.is_empty() {
                Line::from("")
            } else if line.starts_with(' ') && line.contains("   ") {
                // Key-description lines: split at the multi-space gap
                let trimmed = line.trim_start();
                if let Some(pos) = trimmed.find("   ") {
                    let key_part = &trimmed[..pos];
                    let desc_part = trimmed[pos..].trim_start();
                    Line::from(vec![
                        Span::styled(
                            format!("   {:16}", key_part),
                            Style::default().fg(Color::Yellow),
                        ),
                        Span::styled(desc_part.to_string(), Style::default().fg(Color::Gray)),
                    ])
                } else {
                    Line::from(Span::styled(
                        format!("   {}", trimmed),
                        Style::default().fg(Color::Gray),
                    ))
                }
            } else {
                // Section headers
                Line::from(Span::styled(
                    format!(" {}", line.trim()),
                    Style::default().fg(Color::Cyan).bold(),
                ))
            }
        })
        .collect();

    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::default()
                    .title(" Keyboard Shortcuts (q/Esc to close) ")
                    .title_style(Style::default().fg(Color::Blue).bold())
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::DarkGray)),
            )
            .wrap(Wrap { trim: false }),
        area,
    );
}

/// Parse a shortcut string into styled (key, action) pairs.
fn parse_shortcut_pair(part: &str) -> Vec<Span<'static>> {
    if let Some(colon_pos) = part.find(':') {
        let key = &part[..colon_pos];
        let action = &part[colon_pos + 1..];
        vec![
            Span::styled(key.to_string(), Style::default().fg(Color::Cyan).bold()),
            Span::styled(format!(":{}", action), Style::default().fg(Color::DarkGray)),
        ]
    } else {
        vec![Span::styled(
            part.to_string(),
            Style::default().fg(Color::DarkGray),
        )]
    }
}

/// Build shortcut lines that wrap at terminal width.
fn build_shortcut_lines(shortcuts: &str, width: usize) -> Vec<Line<'static>> {
    let sep = "  ";
    let parts: Vec<&str> = shortcuts.split(sep).filter(|s| !s.is_empty()).collect();

    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut current_spans: Vec<Span<'static>> = vec![Span::raw(" ")];
    let mut current_width: usize = 1; // leading space

    for (i, part) in parts.iter().enumerate() {
        let part_width = if i > 0 {
            sep.len() + part.len()
        } else {
            part.len()
        };

        // If adding this part overflows, start a new line
        if i > 0 && current_width + part_width > width.saturating_sub(1) {
            lines.push(Line::from(current_spans));
            current_spans = vec![Span::raw(" ")];
            current_width = 1;
        }

        // Add separator between pairs on the same line
        if current_width > 1 {
            current_spans.push(Span::styled(
                sep.to_string(),
                Style::default().fg(Color::DarkGray),
            ));
            current_width += sep.len();
        }

        current_spans.extend(parse_shortcut_pair(part));
        current_width += part.len();
    }

    if current_spans.len() > 1 {
        lines.push(Line::from(current_spans));
    }

    if lines.is_empty() {
        lines.push(Line::from(" "));
    }

    lines
}

/// Render the bottom status bar: notification (if any) + wrapped shortcuts.
fn render_status_bar(frame: &mut Frame, app: &App, area: Rect, shortcut_lines: Vec<Line<'static>>) {
    let mut lines: Vec<Line> = Vec::new();

    // Notification line (yellow, above shortcuts)
    if let Some(ref msg) = app.notification {
        lines.push(Line::from(vec![
            Span::styled(" ▸ ", Style::default().fg(Color::Yellow).bold()),
            Span::styled(msg.clone(), Style::default().fg(Color::Yellow)),
        ]));
    }

    lines.extend(shortcut_lines);

    frame.render_widget(Paragraph::new(lines), area);
}

/// Render a centered overlay dialog.
fn render_overlay(frame: &mut Frame, text: &str, color: Color, parent_area: Rect) {
    let width = (text.len() as u16 + 6).min(parent_area.width.saturating_sub(4));
    let height = 3;
    let x = parent_area.x + (parent_area.width.saturating_sub(width)) / 2;
    let y = parent_area.y + (parent_area.height.saturating_sub(height)) / 2;
    let dialog_area = Rect::new(x, y, width, height);

    // Background
    frame.render_widget(
        Paragraph::new("").style(Style::default().bg(Color::Black)),
        dialog_area,
    );
    // Dialog
    frame.render_widget(
        Paragraph::new(Line::from(vec![Span::styled(
            format!(" {} ", text),
            Style::default().fg(color).bold(),
        )]))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(color)),
        ),
        dialog_area,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcuts_single_line_wide() {
        let shortcuts = "a:one  b:two  c:three";
        let lines = build_shortcut_lines(shortcuts, 200);
        assert_eq!(lines.len(), 1);
    }

    #[test]
    fn shortcuts_wrap_narrow() {
        let shortcuts = "↑k/↓j:move  V:select  Ctrl+↑↓:reorder  e:edit  i:insert  x:remove  d:diff  r:rebase  p:submit  s:sync  ?:help  q:quit";
        let lines = build_shortcut_lines(shortcuts, 60);
        assert!(
            lines.len() >= 2,
            "expected wrapping at width 60, got {} lines",
            lines.len()
        );
    }

    #[test]
    fn shortcuts_very_narrow() {
        let shortcuts = "a:one  b:two  c:three  d:four  e:five";
        let lines = build_shortcut_lines(shortcuts, 20);
        assert!(
            lines.len() >= 3,
            "expected 3+ lines at width 20, got {}",
            lines.len()
        );
    }

    #[test]
    fn shortcuts_empty() {
        let lines = build_shortcut_lines("", 80);
        assert_eq!(lines.len(), 1); // at least one empty line
    }

    #[test]
    fn shortcuts_single_item_fits() {
        let lines = build_shortcut_lines("q:quit", 80);
        assert_eq!(lines.len(), 1);
    }
}
