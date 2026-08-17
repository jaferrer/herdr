use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use super::text::truncate_end;
use super::widgets::{render_modal_header, render_modal_shell};
use crate::app::connection_picker::ConnectionRow;
use crate::app::AppState;

const MODAL_WIDTH: u16 = 60;
const CHROME_ROWS: u16 = 6; // header, spacing, hint, borders

pub(super) fn render_connection_picker(app: &AppState, frame: &mut Frame, area: Rect) {
    super::dim_background(frame, area);

    let rows = app.connection_picker.rows();
    let height = (rows.len() as u16)
        .saturating_add(CHROME_ROWS)
        .min(area.height);
    let Some(inner) = render_modal_shell(frame, area, MODAL_WIDTH, height, &app.palette) else {
        return;
    };
    if inner.height < 3 {
        return;
    }

    let [header, body, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .areas(inner);

    render_modal_header(frame, header, "connections", &app.palette);

    if let Some(input) = &app.connection_picker.adding {
        render_add_form(app, frame, body, input);
    } else {
        render_rows(app, frame, body, &rows);
    }

    let hint = match (
        &app.connection_picker.message,
        &app.connection_picker.adding,
    ) {
        (Some(message), _) => message.clone(),
        (None, Some(_)) => "host, user@host, or an ssh config alias".to_string(),
        (None, None) => "↵ use · ^d forget · esc close".to_string(),
    };
    let hint_style = if app.connection_picker.message.is_some() {
        Style::default().fg(app.palette.red)
    } else {
        Style::default().fg(app.palette.overlay1)
    };
    frame.render_widget(
        Paragraph::new(truncate_end(&hint, footer.width as usize)).style(hint_style),
        footer,
    );
}

fn render_add_form(app: &AppState, frame: &mut Frame, area: Rect, input: &str) {
    let p = &app.palette;
    let lines = vec![
        Line::from(Span::styled(
            "add an ssh destination",
            Style::default().fg(p.overlay1),
        )),
        Line::from(vec![
            Span::styled("› ", Style::default().fg(p.accent)),
            Span::styled(
                truncate_end(input, area.width.saturating_sub(3) as usize),
                Style::default().fg(p.text),
            ),
            Span::styled("▏", Style::default().fg(p.accent)),
        ]),
    ];
    frame.render_widget(Paragraph::new(lines), area);
}

fn render_rows(app: &AppState, frame: &mut Frame, area: Rect, rows: &[ConnectionRow]) {
    let p = &app.palette;
    let selected = app.connection_picker.selected;
    let lines: Vec<Line> = rows
        .iter()
        .enumerate()
        .map(|(idx, row)| {
            let is_selected = idx == selected;
            let marker = if is_selected { "❯ " } else { "  " };
            let (label, detail, active) = match row {
                ConnectionRow::Local { active } => ("local".to_string(), String::new(), *active),
                ConnectionRow::Remote {
                    name,
                    target,
                    active,
                } => (
                    name.clone(),
                    // The name defaults to the target, so only show it when the
                    // user has renamed the profile and the two differ.
                    if name == target {
                        String::new()
                    } else {
                        target.clone()
                    },
                    *active,
                ),
                ConnectionRow::Add => ("+ ssh".to_string(), String::new(), false),
            };

            let label_style = Style::default()
                .fg(if is_selected { p.text } else { p.subtext0 })
                .add_modifier(if is_selected {
                    Modifier::BOLD
                } else {
                    Modifier::empty()
                });
            let mut spans = vec![
                Span::styled(marker, Style::default().fg(p.accent)),
                Span::styled(
                    if active { "● " } else { "  " },
                    Style::default().fg(p.green),
                ),
                Span::styled(
                    truncate_end(&label, area.width.saturating_sub(4) as usize),
                    label_style,
                ),
            ];
            if !detail.is_empty() {
                spans.push(Span::styled(
                    format!("  {detail}"),
                    Style::default().fg(p.overlay1),
                ));
            }
            Line::from(spans)
        })
        .collect();

    frame.render_widget(Paragraph::new(lines), area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::connection_picker::ConnectionPickerState;
    use crate::remote::profiles::RemoteProfiles;
    use ratatui::{backend::TestBackend, Terminal};

    fn state_with(profiles: &[(&str, &str)], active: Option<&str>) -> AppState {
        let mut state = AppState::test_new();
        let mut store = RemoteProfiles::default();
        for (name, target) in profiles {
            store.upsert(name, target).unwrap();
        }
        store.set_active(active).unwrap();
        state.connection_picker = ConnectionPickerState {
            profiles: store,
            selected: 0,
            adding: None,
            message: None,
        };
        state
    }

    fn rendered(state: &AppState) -> String {
        let mut terminal = Terminal::new(TestBackend::new(100, 24)).unwrap();
        terminal
            .draw(|frame| render_connection_picker(state, frame, frame.area()))
            .unwrap();
        terminal
            .backend()
            .buffer()
            .content()
            .chunks(100)
            .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn every_destination_and_the_add_row_are_drawn() {
        let state = state_with(&[("work", "workbox")], None);

        let screen = rendered(&state);

        assert!(screen.contains("connections"));
        assert!(screen.contains("local"));
        assert!(screen.contains("work"));
        assert!(screen.contains("+ ssh"));
    }

    #[test]
    fn the_active_destination_is_marked() {
        let state = state_with(&[("work", "workbox")], Some("work"));

        let screen = rendered(&state);
        let work_line = screen
            .lines()
            .find(|line| line.contains("work"))
            .expect("work row drawn");

        assert!(
            work_line.contains('●'),
            "active marker missing: {work_line}"
        );
    }

    #[test]
    fn the_add_form_shows_what_is_typed() {
        let mut state = state_with(&[], None);
        state.connection_picker.adding = Some("user@box".to_string());

        let screen = rendered(&state);

        assert!(screen.contains("user@box"));
        assert!(screen.contains("ssh config alias"));
    }

    #[test]
    fn a_rejection_message_replaces_the_hint() {
        let mut state = state_with(&[], None);
        state.connection_picker.message = Some("SSH target must not start with '-'".to_string());

        let screen = rendered(&state);

        assert!(screen.contains("must not start with"));
    }

    #[test]
    fn a_tiny_area_does_not_panic() {
        let state = state_with(&[("work", "workbox")], None);
        for (w, h) in [(1, 1), (10, 3), (40, 6)] {
            let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
            terminal
                .draw(|frame| render_connection_picker(&state, frame, frame.area()))
                .unwrap();
        }
    }
}
