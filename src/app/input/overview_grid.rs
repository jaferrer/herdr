//! Keyboard and mouse for the overview grid.
//!
//! The grid observes and moves focus, so the input surface is deliberately
//! small: move, open, close. Typing into a session happens in the session.
//!
//! Everything here works off `AppState` alone. Navigation depends on how many
//! cells there are and how they are laid out, never on live terminal contents,
//! so input needs no runtime registry.

use crossterm::event::{KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind};

use crate::app::state::AppState;
use crate::ui::overview_grid::{cell_at, grid_shape};

impl AppState {
    pub(crate) fn handle_overview_grid_key(&mut self, key: KeyEvent) {
        let count = self.overview_cell_count();
        let cols = grid_shape(count, self.overview_grid_area()).0.max(1) as isize;

        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => self.close_overview_grid(),
            KeyCode::Left | KeyCode::Char('h') => self.move_overview_selection(-1, count),
            KeyCode::Right | KeyCode::Char('l') | KeyCode::Tab => {
                self.move_overview_selection(1, count)
            }
            KeyCode::BackTab => self.move_overview_selection(-1, count),
            KeyCode::Up | KeyCode::Char('k') => self.move_overview_selection(-cols, count),
            KeyCode::Down | KeyCode::Char('j') => self.move_overview_selection(cols, count),
            // A session can close while the grid is open; refusing to focus a
            // stale cell keeps the grid up instead of jumping somewhere
            // arbitrary. Re-clamping puts the selection back in range.
            KeyCode::Enter if !self.focus_overview_selection() => {
                self.move_overview_selection(0, count)
            }
            _ => {}
        }
    }

    /// The grid draws over the terminal surface, so that is the area its
    /// geometry is measured against.
    pub(crate) fn overview_grid_area(&self) -> ratatui::layout::Rect {
        self.view.terminal_area
    }

    /// Consumes every mouse event while the grid is up: the panes underneath
    /// are covered, so a stray click must never reach them.
    pub(crate) fn handle_overview_grid_mouse(&mut self, mouse: MouseEvent) {
        if !matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left)) {
            return;
        }
        let count = self.overview_cell_count();
        let Some(idx) = cell_at(count, self.overview_grid_area(), mouse.column, mouse.row) else {
            return;
        };
        // Clicking the already-selected cell opens it: one click to aim, one to
        // enter, with no double-click timing to get wrong.
        if idx == self.overview_grid_selected {
            self.focus_overview_selection();
        } else {
            self.overview_grid_selected = idx;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::state::Mode;
    use crate::ui::overview_grid::cell_rects;
    use crossterm::event::KeyModifiers;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::empty())
    }

    fn click(column: u16, row: u16) -> MouseEvent {
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column,
            row,
            modifiers: KeyModifiers::empty(),
        }
    }

    fn state_with_sessions(count: usize) -> AppState {
        let mut state = AppState::test_new();
        state.workspaces = (0..count)
            .map(|idx| crate::workspace::Workspace::test_new(&format!("ws{idx}")))
            .collect();
        state.active = Some(0);
        state.view.terminal_area = ratatui::layout::Rect::new(0, 0, 120, 40);
        state.open_overview_grid_at_current();
        state
    }

    #[test]
    fn escape_and_q_close_the_grid() {
        for code in [KeyCode::Esc, KeyCode::Char('q')] {
            let mut state = state_with_sessions(3);

            state.handle_overview_grid_key(key(code));

            assert_eq!(state.mode, Mode::Terminal);
        }
    }

    #[test]
    fn horizontal_movement_wraps_at_both_ends() {
        let mut state = state_with_sessions(4);
        state.overview_grid_selected = 0;

        state.handle_overview_grid_key(key(KeyCode::Right));
        assert_eq!(state.overview_grid_selected, 1);

        state.handle_overview_grid_key(key(KeyCode::Left));
        state.handle_overview_grid_key(key(KeyCode::Left));
        assert_eq!(state.overview_grid_selected, 3);
    }

    #[test]
    fn vertical_movement_steps_a_whole_row() {
        let mut state = state_with_sessions(4);
        state.overview_grid_selected = 0;

        state.handle_overview_grid_key(key(KeyCode::Down));

        assert_eq!(state.overview_grid_selected, 2);
    }

    #[test]
    fn tab_and_backtab_cycle_cells() {
        let mut state = state_with_sessions(3);
        state.overview_grid_selected = 0;

        state.handle_overview_grid_key(key(KeyCode::Tab));
        assert_eq!(state.overview_grid_selected, 1);

        state.handle_overview_grid_key(key(KeyCode::BackTab));
        assert_eq!(state.overview_grid_selected, 0);
    }

    #[test]
    fn enter_opens_the_selected_session() {
        let mut state = state_with_sessions(3);
        state.overview_grid_selected = 2;

        state.handle_overview_grid_key(key(KeyCode::Enter));

        assert_eq!(state.mode, Mode::Terminal);
        assert_eq!(state.active, Some(2));
    }

    #[test]
    fn enter_on_a_stale_cell_keeps_the_grid_open() {
        let mut state = state_with_sessions(2);
        state.overview_grid_selected = 9;

        state.handle_overview_grid_key(key(KeyCode::Enter));

        assert_eq!(state.mode, Mode::OverviewGrid);
        assert!(state.overview_grid_selected < 2);
    }

    #[test]
    fn keys_in_an_empty_grid_do_nothing_harmful() {
        let mut state = AppState::test_new();
        state.workspaces.clear();
        state.active = None;
        state.view.terminal_area = ratatui::layout::Rect::new(0, 0, 80, 24);
        state.open_overview_grid_at_current();

        for code in [KeyCode::Down, KeyCode::Right, KeyCode::Enter] {
            state.handle_overview_grid_key(key(code));
        }

        assert_eq!(state.mode, Mode::OverviewGrid);
        assert_eq!(state.overview_grid_selected, 0);
    }

    #[test]
    fn a_click_selects_and_a_second_click_opens() {
        let mut state = state_with_sessions(4);
        state.overview_grid_selected = 0;
        let target = cell_rects(4, state.overview_grid_area())[3];

        state.handle_overview_grid_mouse(click(target.x, target.y));
        assert_eq!(state.overview_grid_selected, 3);
        assert_eq!(state.mode, Mode::OverviewGrid);

        state.handle_overview_grid_mouse(click(target.x, target.y));
        assert_eq!(state.mode, Mode::Terminal);
        assert_eq!(state.active, Some(3));
    }

    #[test]
    fn a_click_outside_any_cell_changes_nothing() {
        let mut state = state_with_sessions(2);
        state.overview_grid_selected = 1;

        state.handle_overview_grid_mouse(click(500, 500));

        assert_eq!(state.mode, Mode::OverviewGrid);
        assert_eq!(state.overview_grid_selected, 1);
    }
}
