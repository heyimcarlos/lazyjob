use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Tabs};

use crate::action::ViewId;
use crate::app::App;
use crate::keybindings::KeyContext;
use crate::layout::AppLayout;

pub fn render(frame: &mut Frame, app: &mut App) {
    let layout = AppLayout::compute(frame.area());

    render_header(frame, layout.header, app);
    render_body(frame, layout.body, app);
    render_status_bar(frame, layout.status_bar, app);

    if app.help_open {
        let ctx = KeyContext::from_view_id(app.active_view);
        app.views
            .help_overlay
            .render_overlay(frame, frame.area(), app.theme, &app.keymap, &ctx);
    }
}

fn render_header(frame: &mut Frame, area: Rect, app: &App) {
    let tab_titles: Vec<&str> = ViewId::ALL.iter().map(|v| v.label()).collect();
    let selected = app.active_view.tab_index();

    let tabs = Tabs::new(tab_titles)
        .block(
            Block::default()
                .borders(Borders::BOTTOM)
                .title(" LazyJob ")
                .title_style(app.theme.title_style()),
        )
        .select(selected)
        .highlight_style(app.theme.selected_style())
        .style(app.theme.tab_style())
        .divider("|");

    frame.render_widget(tabs, area);
}

fn render_body(frame: &mut Frame, area: Rect, app: &mut App) {
    let theme = app.theme;
    app.active_view_mut().render(frame, area, theme);
}

fn render_status_bar(frame: &mut Frame, area: Rect, app: &App) {
    let now = chrono::Local::now().format("%H:%M");
    let view_label = app.active_view.label();

    let text = Line::from(vec![
        Span::raw(" "),
        Span::styled(
            format!("[{}]", view_label),
            Style::default().fg(app.theme.primary),
        ),
        Span::raw(format!("  Press ? for help  {now} ")),
    ]);

    let paragraph = Paragraph::new(text).style(app.theme.status_bar_style());
    frame.render_widget(paragraph, area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn test_app() -> App {
        App::new_for_test()
    }

    #[test]
    fn render_without_panic() {
        let mut app = test_app();
        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| render(f, &mut app)).unwrap();
    }

    #[test]
    fn render_all_views_without_panic() {
        let mut app = test_app();
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();

        for view in ViewId::ALL {
            app.active_view = view;
            terminal.draw(|f| render(f, &mut app)).unwrap();
        }
    }

    #[test]
    fn render_with_help_overlay() {
        let mut app = test_app();
        app.help_open = true;
        let backend = TestBackend::new(100, 40);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| render(f, &mut app)).unwrap();

        let buffer = terminal.backend().buffer().clone();
        let all_text: String = (0..40)
            .flat_map(|y| (0..100).map(move |x| (x, y)))
            .map(|(x, y)| buffer.cell((x, y)).unwrap().symbol().to_string())
            .collect();
        assert!(all_text.contains("Quit"));
    }

    #[test]
    fn render_dispatches_to_correct_view() {
        let mut app = test_app();
        app.active_view = ViewId::Jobs;
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| render(f, &mut app)).unwrap();

        let buffer = terminal.backend().buffer().clone();
        let all_text: String = (0..24)
            .flat_map(|y| (0..80).map(move |x| (x, y)))
            .map(|(x, y)| buffer.cell((x, y)).unwrap().symbol().to_string())
            .collect();
        assert!(all_text.contains("Jobs"));
    }

    #[test]
    fn header_shows_all_tabs() {
        let mut app = test_app();
        app.active_view = ViewId::Jobs;
        let backend = TestBackend::new(120, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| render(f, &mut app)).unwrap();

        let buffer = terminal.backend().buffer().clone();
        let header_area: String = (0..3)
            .flat_map(|y| (0..120).map(move |x| (x, y)))
            .map(|(x, y)| buffer.cell((x, y)).unwrap().symbol().to_string())
            .collect();
        assert!(header_area.contains("Dashboard"));
        assert!(header_area.contains("Ralph"));
    }
}
