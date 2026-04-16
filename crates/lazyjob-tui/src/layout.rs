use ratatui::layout::{Constraint, Direction, Layout, Rect};

pub const HEADER_HEIGHT: u16 = 3;
pub const STATUS_BAR_HEIGHT: u16 = 1;

pub struct AppLayout {
    pub header: Rect,
    pub body: Rect,
    pub status_bar: Rect,
}

impl AppLayout {
    pub fn compute(frame_size: Rect) -> Self {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(HEADER_HEIGHT),
                Constraint::Fill(1),
                Constraint::Length(STATUS_BAR_HEIGHT),
            ])
            .split(frame_size);
        Self {
            header: chunks[0],
            body: chunks[1],
            status_bar: chunks[2],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_layout_dimensions() {
        let area = Rect::new(0, 0, 120, 40);
        let layout = AppLayout::compute(area);
        assert_eq!(layout.header.height, HEADER_HEIGHT);
        assert_eq!(layout.status_bar.height, STATUS_BAR_HEIGHT);
        assert_eq!(layout.body.height, 40 - HEADER_HEIGHT - STATUS_BAR_HEIGHT);
        assert_eq!(layout.header.width, 120);
        assert_eq!(layout.body.width, 120);
        assert_eq!(layout.status_bar.width, 120);
    }

    #[test]
    fn app_layout_small_terminal() {
        let area = Rect::new(0, 0, 40, 10);
        let layout = AppLayout::compute(area);
        assert_eq!(layout.header.height, HEADER_HEIGHT);
        assert_eq!(layout.status_bar.height, STATUS_BAR_HEIGHT);
        assert_eq!(layout.body.height, 10 - HEADER_HEIGHT - STATUS_BAR_HEIGHT);
    }
}
