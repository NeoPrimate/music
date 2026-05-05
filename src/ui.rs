use crate::app::{App, ConfirmKind, DownloadStatus, Tab};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, Borders, Clear, Gauge, List, ListItem, ListState, Paragraph, Tabs, Wrap,
};
use ratatui::Frame;

pub fn render(f: &mut Frame, app: &App) {
    let area = f.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .split(area);

    render_tabs(f, app, chunks[0]);
    match app.tab {
        Tab::Download => render_download(f, app, chunks[1]),
        Tab::Library => render_library(f, app, chunks[1]),
    }
    render_footer(f, app, chunks[2]);

    if let Some(c) = &app.confirm {
        render_confirm(f, area, c);
    }
}

fn render_tabs(f: &mut Frame, app: &App, area: Rect) {
    let titles = vec!["Download", "Library"];
    let selected = match app.tab {
        Tab::Download => 0,
        Tab::Library => 1,
    };
    let tabs = Tabs::new(titles)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("yt-music-tui"),
        )
        .select(selected)
        .highlight_style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        );
    f.render_widget(tabs, area);
}

fn collect_active(app: &App) -> Vec<(String, DownloadStatus)> {
    let mut v: Vec<(String, DownloadStatus)> = app
        .statuses
        .iter()
        .filter(|(_, s)| matches!(s, DownloadStatus::Running { .. } | DownloadStatus::Queued))
        .map(|(k, s)| (k.clone(), s.clone()))
        .collect();
    v.sort_by(|a, b| a.0.cmp(&b.0));
    v
}

fn render_download(f: &mut Frame, app: &App, area: Rect) {
    let active = collect_active(app);
    let active_height: u16 = if active.is_empty() {
        0
    } else {
        (active.len() as u16) * 4 + 2
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(5), Constraint::Length(active_height)])
        .split(area);

    render_playlists(f, app, chunks[0]);
    if active_height > 0 {
        render_active_downloads(f, &active, chunks[1]);
    }
}

fn status_badge(status: Option<&DownloadStatus>) -> (String, Style) {
    match status {
        Some(DownloadStatus::InLibrary { tracks_in_music }) => (
            format!("  📚 in library ({tracks_in_music})"),
            Style::default().fg(Color::Blue),
        ),
        Some(DownloadStatus::Queued) => (
            "  ⏳ queued".to_string(),
            Style::default().fg(Color::Gray),
        ),
        Some(DownloadStatus::Running {
            imported_so_far,
            track_total,
            ..
        }) => {
            let counts = if *track_total > 0 {
                format!(" {imported_so_far}/{track_total}")
            } else if *imported_so_far > 0 {
                format!(" {imported_so_far}")
            } else {
                String::new()
            };
            (
                format!("  ⟳{counts}"),
                Style::default().fg(Color::Yellow),
            )
        }
        Some(DownloadStatus::Done {
            tracks_imported,
            tracks_expected,
        }) => match tracks_expected {
            Some(exp) if *tracks_imported >= *exp => (
                format!("  ✓ {tracks_imported}/{exp} imported"),
                Style::default().fg(Color::Green),
            ),
            Some(exp) => (
                format!("  ◐ {tracks_imported}/{exp} imported (partial)"),
                Style::default().fg(Color::Yellow),
            ),
            None => {
                if *tracks_imported == 0 {
                    (
                        "  ✓ already up to date".to_string(),
                        Style::default().fg(Color::Green),
                    )
                } else {
                    (
                        format!("  ✓ {tracks_imported} new imported"),
                        Style::default().fg(Color::Green),
                    )
                }
            }
        },
        Some(DownloadStatus::Failed { message }) => (
            format!("  ✗ {}", truncate(message, 60)),
            Style::default().fg(Color::Red),
        ),
        None => (String::new(), Style::default()),
    }
}

fn render_playlists(f: &mut Frame, app: &App, area: Rect) {
    let items: Vec<ListItem> = app
        .playlists
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let cursor = if i == app.download_cursor { "▶ " } else { "  " };
            let mark = if app.download_selected.contains(&i) {
                "[x] "
            } else {
                "[ ] "
            };
            let (badge, badge_style) = status_badge(app.statuses.get(&p.title));
            let title_style = match app.statuses.get(&p.title) {
                Some(DownloadStatus::Done { .. }) => Style::default().fg(Color::Green),
                Some(DownloadStatus::Failed { .. }) => Style::default().fg(Color::Red),
                Some(DownloadStatus::Running { .. } | DownloadStatus::Queued) => {
                    Style::default().fg(Color::Yellow)
                }
                Some(DownloadStatus::InLibrary { .. }) => Style::default().fg(Color::Blue),
                None => Style::default(),
            };
            ListItem::new(Line::from(vec![
                Span::raw(cursor),
                Span::raw(mark),
                Span::styled(p.title.clone(), title_style),
                Span::styled(badge, badge_style),
            ]))
        })
        .collect();

    let title = format!(
        "Playlists ({} total, {} selected)",
        app.playlists.len(),
        app.download_selected.len()
    );
    let list = List::new(items).block(Block::default().borders(Borders::ALL).title(title));
    let mut state = ListState::default();
    if !app.playlists.is_empty() {
        state.select(Some(app.download_cursor));
    }
    f.render_stateful_widget(list, area, &mut state);
}

fn render_active_downloads(f: &mut Frame, active: &[(String, DownloadStatus)], area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!("Active downloads ({})", active.len()));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let constraints: Vec<Constraint> = active.iter().map(|_| Constraint::Length(4)).collect();
    if constraints.is_empty() {
        return;
    }
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(inner);

    for (i, (title, status)) in active.iter().enumerate() {
        let chunk = chunks[i];
        let row = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Length(1),
            ])
            .split(chunk);

        let (header_right, percent_text, percent_u16, color, body) = match status {
            DownloadStatus::Queued => (
                "queued".to_string(),
                "0%".to_string(),
                0u16,
                Color::Gray,
                String::new(),
            ),
            DownloadStatus::Running {
                track_index,
                track_total,
                track_title,
                percent,
                speed,
                eta,
                imported_so_far,
            } => {
                let speed_s = match speed {
                    Some(s) if *s > 0.0 => format!("{:.2} MiB/s", s / 1024.0 / 1024.0),
                    _ => "-- MiB/s".to_string(),
                };
                let eta_s = match eta {
                    Some(s) => format!("{:02}:{:02}", s / 60, s % 60),
                    None => "--:--".to_string(),
                };
                let counts = if *track_total > 0 {
                    format!("{}/{}", track_index, track_total)
                } else {
                    format!("{}", track_index)
                };
                let header_right = format!(
                    "{counts}  {speed_s}  ETA {eta_s}  imported {imported_so_far}"
                );
                let pct_u = (*percent).clamp(0.0, 100.0) as u16;
                (
                    header_right,
                    format!("{:.0}%", percent),
                    pct_u,
                    Color::Yellow,
                    format!("Currently: {track_title}"),
                )
            }
            _ => continue,
        };

        let header = Line::from(vec![
            Span::styled(
                format!("{title}   "),
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::styled(header_right, Style::default().fg(Color::DarkGray)),
        ]);
        f.render_widget(Paragraph::new(header), row[0]);

        let gauge = Gauge::default()
            .gauge_style(Style::default().fg(color))
            .percent(percent_u16)
            .label(percent_text);
        f.render_widget(gauge, row[1]);

        if !body.is_empty() {
            f.render_widget(
                Paragraph::new(body).style(Style::default().fg(Color::DarkGray)),
                row[2],
            );
        }
    }
}

fn render_library(f: &mut Frame, app: &App, area: Rect) {
    let items: Vec<ListItem> = app
        .library
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let cursor = if i == app.library_cursor { "▶ " } else { "  " };
            let mark = if app.library_selected.contains(&i) {
                "[x] "
            } else {
                "[ ] "
            };
            ListItem::new(Line::from(vec![
                Span::raw(cursor),
                Span::raw(mark),
                Span::raw(p.name.clone()),
                Span::styled(
                    format!("  ({} tracks)", p.track_count),
                    Style::default().fg(Color::DarkGray),
                ),
            ]))
        })
        .collect();

    let title = format!(
        "Music.app user playlists ({} total, {} selected)",
        app.library.len(),
        app.library_selected.len()
    );
    let list = List::new(items).block(Block::default().borders(Borders::ALL).title(title));
    let mut state = ListState::default();
    if !app.library.is_empty() {
        state.select(Some(app.library_cursor));
    }
    f.render_stateful_widget(list, area, &mut state);
}

fn render_footer(f: &mut Frame, app: &App, area: Rect) {
    let hint = match app.tab {
        Tab::Download => {
            "↑↓/jk move  ␣ select  a all  ⏎/d download  Tab switch  r refresh  q quit"
        }
        Tab::Library => {
            "↑↓/jk move  ␣ select  a all  x del playlist  X del + tracks  Tab switch  r refresh  q quit"
        }
    };
    let line = if let Some(s) = &app.status_msg {
        format!("{hint}  |  {s}")
    } else {
        hint.to_string()
    };
    f.render_widget(
        Paragraph::new(line).style(Style::default().fg(Color::DarkGray)),
        area,
    );
}

fn render_confirm(f: &mut Frame, area: Rect, c: &ConfirmKind) {
    let popup = centered_rect(60, 40, area);
    f.render_widget(Clear, popup);
    let (title, lines) = match c {
        ConfirmKind::DeletePlaylists(names) => {
            let mut lines: Vec<Line> = vec![
                Line::from(format!(
                    "Delete {} Music.app playlist(s)? (tracks remain in library)",
                    names.len()
                )),
                Line::from(""),
            ];
            for n in names.iter().take(8) {
                lines.push(Line::from(format!("  • {n}")));
            }
            if names.len() > 8 {
                lines.push(Line::from(format!("  … and {} more", names.len() - 8)));
            }
            lines.push(Line::from(""));
            lines.push(
                Line::from("Press y to confirm, n / Esc to cancel.")
                    .style(Style::default().fg(Color::Yellow)),
            );
            ("Confirm delete (playlist only)", lines)
        }
        ConfirmKind::DeletePlaylistsAndTracks(names) => {
            let mut lines: Vec<Line> = vec![
                Line::from(format!(
                    "Delete {} playlist(s) AND remove their tracks from the Music library?",
                    names.len()
                ))
                .style(Style::default().fg(Color::Red)),
                Line::from("(frees disk space — files go to Music.app's trash)"),
                Line::from(""),
            ];
            for n in names.iter().take(8) {
                lines.push(Line::from(format!("  • {n}")));
            }
            if names.len() > 8 {
                lines.push(Line::from(format!("  … and {} more", names.len() - 8)));
            }
            lines.push(Line::from(""));
            lines.push(
                Line::from("Press y to confirm, n / Esc to cancel.")
                    .style(Style::default().fg(Color::Yellow)),
            );
            ("Confirm delete (playlist + tracks)", lines)
        }
    };
    let para = Paragraph::new(lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(title)
                .style(Style::default().fg(Color::Red)),
        )
        .wrap(Wrap { trim: false });
    f.render_widget(para, popup);
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}
