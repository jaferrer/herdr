use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph},
    Frame,
};

use super::status::{state_icon, state_label, state_label_color};
use super::text::truncate_end;
use crate::app::overview_grid::OverviewCell;
use crate::app::AppState;
use crate::terminal::TerminalRuntimeRegistry;

/// Rows and columns for `count` cells, as close to square as the area allows.
/// Cells need width to be readable, so wide-but-short terminals get fewer rows
/// rather than columns too narrow to show a name.
pub(crate) fn grid_shape(count: usize, area: Rect) -> (u16, u16) {
    if count == 0 || area.width == 0 || area.height == 0 {
        return (0, 0);
    }
    const MIN_CELL_WIDTH: u16 = 18;
    const MIN_CELL_HEIGHT: u16 = 3;

    let max_cols = (area.width / MIN_CELL_WIDTH).max(1);
    let max_rows = (area.height / MIN_CELL_HEIGHT).max(1);

    let ideal_cols = (count as f32).sqrt().ceil() as u16;
    let cols = ideal_cols.clamp(1, max_cols);
    let rows = (count as u16).div_ceil(cols).clamp(1, max_rows);
    (cols, rows)
}

/// Rect of each visible cell, in cell order. Cells past the last visible slot
/// are dropped rather than squeezed into unreadable slivers.
pub(crate) fn cell_rects(count: usize, area: Rect) -> Vec<Rect> {
    let (cols, rows) = grid_shape(count, area);
    if cols == 0 || rows == 0 {
        return Vec::new();
    }
    let visible = count.min((cols as usize) * (rows as usize));
    let cell_w = area.width / cols;
    let cell_h = area.height / rows;

    (0..visible)
        .map(|idx| {
            let col = (idx as u16) % cols;
            let row = (idx as u16) / cols;
            Rect::new(area.x + col * cell_w, area.y + row * cell_h, cell_w, cell_h)
        })
        .collect()
}

/// Cell index at a screen position, for mouse selection.
pub(crate) fn cell_at(count: usize, area: Rect, col: u16, row: u16) -> Option<usize> {
    cell_rects(count, area).into_iter().position(|rect| {
        col >= rect.x && col < rect.x + rect.width && row >= rect.y && row < rect.y + rect.height
    })
}

pub(super) fn render_overview_grid(
    app: &AppState,
    terminal_runtimes: &TerminalRuntimeRegistry,
    frame: &mut Frame,
    area: Rect,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let p = &app.palette;
    let cells = app.overview_cells_from(terminal_runtimes);
    if cells.is_empty() {
        frame.render_widget(
            Paragraph::new("no open sessions").style(Style::default().fg(p.overlay1)),
            area,
        );
        return;
    }

    let rects = cell_rects(cells.len(), area);
    for (idx, rect) in rects.iter().enumerate() {
        let Some(cell) = cells.get(idx) else { continue };
        render_cell(app, frame, *rect, cell, idx == app.overview_grid_selected);
    }
}

fn render_cell(app: &AppState, frame: &mut Frame, area: Rect, cell: &OverviewCell, selected: bool) {
    let p = &app.palette;
    let border_color = if selected {
        p.accent
    } else if cell.is_current {
        p.blue
    } else {
        p.surface1
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(if selected {
            BorderType::Thick
        } else {
            BorderType::Plain
        })
        .border_style(Style::default().fg(border_color));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let mut lines = vec![Line::from(vec![
        {
            let (symbol, style) = state_icon(cell.state, cell.seen, app.status_indicators, p);
            Span::styled(format!("{symbol} "), style)
        },
        Span::styled(
            truncate_end(
                &cell.workspace_label,
                inner.width.saturating_sub(2) as usize,
            ),
            Style::default().fg(p.text).add_modifier(if selected {
                Modifier::BOLD
            } else {
                Modifier::empty()
            }),
        ),
    ])];

    if inner.height > 1 {
        let detail = match &cell.agent_label {
            Some(agent) => format!("{} · {}", cell.tab_label, agent),
            None => cell.tab_label.clone(),
        };
        lines.push(Line::from(Span::styled(
            truncate_end(&detail, inner.width as usize),
            Style::default().fg(p.overlay1),
        )));
    }

    if inner.height > 2 {
        lines.push(Line::from(Span::styled(
            truncate_end(state_label(cell.state, cell.seen), inner.width as usize),
            Style::default().fg(state_label_color(cell.state, cell.seen, p)),
        )));
    }

    frame.render_widget(Paragraph::new(lines), inner);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_grid_has_no_cells() {
        assert_eq!(grid_shape(0, Rect::new(0, 0, 80, 24)), (0, 0));
        assert!(cell_rects(0, Rect::new(0, 0, 80, 24)).is_empty());
    }

    #[test]
    fn cell_counts_stay_close_to_square() {
        let area = Rect::new(0, 0, 120, 40);

        assert_eq!(grid_shape(1, area), (1, 1));
        assert_eq!(grid_shape(2, area), (2, 1));
        assert_eq!(grid_shape(4, area), (2, 2));
        assert_eq!(grid_shape(15, area), (4, 4));
    }

    #[test]
    fn every_cell_gets_a_rect_inside_the_area() {
        let area = Rect::new(3, 2, 120, 40);

        let rects = cell_rects(15, area);

        assert_eq!(rects.len(), 15);
        for rect in rects {
            assert!(rect.x >= area.x && rect.y >= area.y);
            assert!(rect.x + rect.width <= area.x + area.width);
            assert!(rect.y + rect.height <= area.y + area.height);
            assert!(rect.width > 0 && rect.height > 0);
        }
    }

    #[test]
    fn a_narrow_area_reduces_columns_instead_of_shredding_cells() {
        let narrow = Rect::new(0, 0, 20, 40);

        let (cols, _) = grid_shape(9, narrow);

        assert_eq!(cols, 1);
        assert!(cell_rects(9, narrow).iter().all(|rect| rect.width >= 18));
    }

    #[test]
    fn a_tiny_area_still_yields_usable_cells_or_none() {
        for (w, h) in [(0, 0), (1, 1), (4, 2), (18, 3)] {
            let area = Rect::new(0, 0, w, h);
            for rect in cell_rects(6, area) {
                assert!(rect.width > 0 && rect.height > 0);
            }
        }
    }

    #[test]
    fn clicks_map_back_to_the_cell_under_them() {
        let area = Rect::new(0, 0, 120, 40);
        let rects = cell_rects(4, area);

        for (idx, rect) in rects.iter().enumerate() {
            assert_eq!(cell_at(4, area, rect.x, rect.y), Some(idx));
            assert_eq!(
                cell_at(4, area, rect.x + rect.width - 1, rect.y + rect.height - 1),
                Some(idx)
            );
        }
        assert_eq!(cell_at(4, area, 200, 200), None);
    }
}
