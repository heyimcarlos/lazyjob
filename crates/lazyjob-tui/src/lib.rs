pub mod action;
pub mod app;
pub mod event_loop;
pub mod keybindings;
pub mod layout;
pub mod render;
pub mod theme;
pub mod views;
pub mod widgets;

use std::sync::Arc;

use tokio::sync::broadcast;

use lazyjob_core::config::Config;
use lazyjob_core::db::Database;
use lazyjob_core::repositories::RalphLoopRunRepository;

use crate::app::{App, RalphUpdate};

pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

pub async fn run(config: Arc<Config>) -> anyhow::Result<()> {
    let (_ralph_tx, ralph_rx) = broadcast::channel::<RalphUpdate>(64);

    let mut app = match Database::connect(&config.database_url).await {
        Ok(db) => {
            let repo = RalphLoopRunRepository::new(db.pool().clone());
            let recovered = repo.recover_pending().await.unwrap_or(0);
            if recovered > 0 {
                tracing::info!("Recovered {recovered} stale ralph loop runs marked as failed");
            }
            App::new(config, ralph_rx).with_pool(db.pool().clone())
        }
        Err(err) => {
            tracing::warn!("Could not connect to database at startup: {err}");
            App::new(config, ralph_rx)
        }
    };

    app.load_jobs().await;
    app.load_applications().await;
    app.load_dashboard_stats().await;
    event_loop::run_event_loop(app).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_returns_crate_version() {
        assert_eq!(version(), "0.1.0");
    }

    // learning test: proves ratatui TestBackend can render a paragraph and capture output
    #[test]
    fn ratatui_test_backend_renders_paragraph() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        use ratatui::layout::Rect;
        use ratatui::widgets::Paragraph;

        let backend = TestBackend::new(20, 3);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|frame| {
                let area = Rect::new(0, 0, 20, 1);
                let paragraph = Paragraph::new("Hello, LazyJob!");
                frame.render_widget(paragraph, area);
            })
            .unwrap();

        let buffer = terminal.backend().buffer().clone();
        let line: String = (0..15)
            .map(|x| {
                buffer
                    .cell((x, 0))
                    .unwrap()
                    .symbol()
                    .chars()
                    .next()
                    .unwrap_or(' ')
            })
            .collect();
        assert_eq!(line, "Hello, LazyJob!");
    }

    // learning test: proves ratatui Layout splits a Rect into the expected number of chunks
    #[test]
    fn ratatui_layout_splits_correctly() {
        use ratatui::layout::{Constraint, Direction, Layout, Rect};

        let area = Rect::new(0, 0, 100, 50);
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Fill(1),
                Constraint::Length(1),
            ])
            .split(area);

        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks[0].height, 3);
        assert_eq!(chunks[2].height, 1);
        assert_eq!(chunks[1].height, 50 - 3 - 1);
    }

    // learning test: proves crossterm KeyEvent can be constructed for testing
    #[test]
    fn crossterm_key_event_constructible() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

        let key = KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE);
        assert_eq!(key.code, KeyCode::Char('q'));
        assert_eq!(key.modifiers, KeyModifiers::NONE);
    }
}
