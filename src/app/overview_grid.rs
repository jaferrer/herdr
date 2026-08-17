//! Overview grid — every open session at a glance.
//!
//! The grid is a *projection* of `AppState`, never a copy: cells are derived on
//! demand from the same workspaces and terminals the sidebar reads, so it can
//! never drift out of sync with the runtime. The only state it owns is which
//! cell is selected.
//!
//! It observes and moves focus. Typing into a session and reordering stay where
//! they already work: the pane itself and the sidebar.

use super::state::{AppState, Mode};
use crate::detect::AgentState;
use crate::layout::PaneId;
use crate::terminal::TerminalRuntimeRegistry;
use crate::ui::sidebar::{workspace_list_entries_expanded, WorkspaceListEntry};

/// One cell: the focused pane of one tab.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OverviewCell {
    pub ws_idx: usize,
    pub tab_idx: usize,
    pub pane_id: PaneId,
    pub workspace_label: String,
    pub tab_label: String,
    pub agent_label: Option<String>,
    pub state: AgentState,
    pub seen: bool,
    /// Whether this cell is the session the user is currently focused on.
    pub is_current: bool,
}

impl AppState {
    #[cfg(test)]
    pub(crate) fn overview_cells(&self) -> Vec<OverviewCell> {
        self.overview_cells_from(&TerminalRuntimeRegistry::new())
    }

    /// Cells in sidebar order: workspaces grouped the way they are listed
    /// (worktree children stay with their parent), then tabs in tab order.
    ///
    /// One cell per **tab**, taken from its focused pane. Derived from tabs
    /// rather than from agent details on purpose: a tab running only a shell is
    /// still an open session, and the grid answers "what is open".
    pub(crate) fn overview_cells_from(
        &self,
        terminal_runtimes: &TerminalRuntimeRegistry,
    ) -> Vec<OverviewCell> {
        let active_tab = self
            .active
            .and_then(|ws_idx| self.workspaces.get(ws_idx))
            .map(|ws| ws.active_tab_index());

        workspace_list_entries_expanded(self)
            .into_iter()
            .flat_map(|WorkspaceListEntry::Workspace { ws_idx, .. }| {
                let Some(ws) = self.workspaces.get(ws_idx) else {
                    return Vec::new();
                };
                let workspace_label = ws.display_name_from(&self.terminals, terminal_runtimes);
                let details = ws.pane_details(&self.terminals);

                ws.tabs
                    .iter()
                    .enumerate()
                    .map(|(tab_idx, tab)| {
                        let pane_id = tab.layout.focused();
                        let detail = details.iter().find(|detail| detail.pane_id == pane_id);
                        let (state, seen) = tab
                            .panes
                            .get(&pane_id)
                            .and_then(|pane| {
                                self.terminals
                                    .get(&pane.attached_terminal_id)
                                    .map(|terminal| (terminal.state, pane.seen))
                            })
                            .unwrap_or((AgentState::Unknown, true));

                        OverviewCell {
                            ws_idx,
                            tab_idx,
                            pane_id,
                            workspace_label: workspace_label.clone(),
                            tab_label: ws
                                .tab_display_name(tab_idx)
                                .unwrap_or_else(|| (tab_idx + 1).to_string()),
                            agent_label: detail.and_then(|detail| detail.agent_kind_label.clone()),
                            state,
                            seen,
                            is_current: self.active == Some(ws_idx) && active_tab == Some(tab_idx),
                        }
                    })
                    .collect()
            })
            .collect()
    }

    #[cfg(test)]
    pub(crate) fn open_overview_grid(&mut self) {
        self.open_overview_grid_at_current();
    }

    /// Opens with the session the user is on already selected, so closing
    /// immediately leaves them exactly where they were.
    ///
    /// Needs no runtime registry: cell order and which cell is current depend
    /// only on workspaces and tabs. Live titles are read later, at render.
    pub(crate) fn open_overview_grid_at_current(&mut self) {
        let cells = self.overview_cells_from(&TerminalRuntimeRegistry::new());
        self.overview_grid_selected = cells.iter().position(|cell| cell.is_current).unwrap_or(0);
        self.mode = Mode::OverviewGrid;
    }

    pub(crate) fn close_overview_grid(&mut self) {
        if self.mode == Mode::OverviewGrid {
            self.mode = Mode::Terminal;
        }
    }

    /// Moves the selection by `delta` cells, wrapping at both ends.
    pub(crate) fn move_overview_selection(&mut self, delta: isize, cell_count: usize) {
        if cell_count == 0 {
            self.overview_grid_selected = 0;
            return;
        }
        let count = cell_count as isize;
        let current = self.overview_grid_selected.min(cell_count - 1) as isize;
        self.overview_grid_selected = (current + delta).rem_euclid(count) as usize;
    }

    /// How many cells the grid shows. Cheap enough for input paths: it walks
    /// tabs, never terminal contents.
    pub(crate) fn overview_cell_count(&self) -> usize {
        self.overview_cells_from(&TerminalRuntimeRegistry::new())
            .len()
    }

    /// Focuses the selected session and leaves the grid. Returns false when the
    /// selection no longer exists, which happens if a session closed while the
    /// grid was open.
    pub(crate) fn focus_overview_selection(&mut self) -> bool {
        let cells = self.overview_cells_from(&TerminalRuntimeRegistry::new());
        let Some(cell) = cells.get(self.overview_grid_selected) else {
            return false;
        };
        let (ws_idx, tab_idx) = (cell.ws_idx, cell.tab_idx);
        self.switch_workspace_tab(ws_idx, tab_idx);
        self.mode = Mode::Terminal;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::Workspace;

    fn app_with_workspaces(labels: &[&str]) -> AppState {
        let mut app = AppState::test_new();
        app.workspaces = labels
            .iter()
            .map(|label| Workspace::test_new(label))
            .collect();
        app.active = Some(0);
        app
    }

    #[test]
    fn cells_follow_sidebar_order() {
        let app = app_with_workspaces(&["first", "second", "third"]);

        let labels: Vec<_> = app
            .overview_cells()
            .into_iter()
            .map(|cell| cell.workspace_label)
            .collect();

        assert_eq!(labels, vec!["first", "second", "third"]);
    }

    #[test]
    fn the_grid_opens_on_the_session_the_user_is_already_on() {
        let mut app = app_with_workspaces(&["first", "second", "third"]);
        app.active = Some(2);

        app.open_overview_grid();

        assert_eq!(app.mode, Mode::OverviewGrid);
        assert_eq!(app.overview_grid_selected, 2);
        assert!(app.overview_cells()[2].is_current);
    }

    #[test]
    fn an_empty_grid_opens_without_panicking() {
        let mut app = AppState::test_new();
        app.workspaces.clear();
        app.active = None;

        app.open_overview_grid();

        assert!(app.overview_cells().is_empty());
        assert_eq!(app.overview_grid_selected, 0);
    }

    #[test]
    fn selection_wraps_in_both_directions() {
        let mut app = app_with_workspaces(&["a", "b", "c"]);
        let count = app.overview_cells().len();

        app.move_overview_selection(-1, count);
        assert_eq!(app.overview_grid_selected, 2);

        app.move_overview_selection(1, count);
        assert_eq!(app.overview_grid_selected, 0);

        app.move_overview_selection(2, count);
        assert_eq!(app.overview_grid_selected, 2);
    }

    #[test]
    fn moving_in_an_empty_grid_stays_put() {
        let mut app = AppState::test_new();
        app.workspaces.clear();

        app.move_overview_selection(3, 0);

        assert_eq!(app.overview_grid_selected, 0);
    }

    #[test]
    fn focusing_a_cell_switches_workspace_and_leaves_the_grid() {
        let mut app = app_with_workspaces(&["first", "second"]);
        app.open_overview_grid();
        let count = app.overview_cells().len();
        app.move_overview_selection(1, count);

        assert!(app.focus_overview_selection());

        assert_eq!(app.active, Some(1));
        assert_eq!(app.mode, Mode::Terminal);
    }

    #[test]
    fn focusing_a_stale_selection_reports_failure() {
        let mut app = app_with_workspaces(&["only"]);
        app.open_overview_grid();
        app.overview_grid_selected = 7;

        assert!(!app.focus_overview_selection());
        assert_eq!(app.mode, Mode::OverviewGrid);
    }

    #[test]
    fn closing_returns_to_the_terminal() {
        let mut app = app_with_workspaces(&["only"]);
        app.open_overview_grid();

        app.close_overview_grid();

        assert_eq!(app.mode, Mode::Terminal);
    }
}
