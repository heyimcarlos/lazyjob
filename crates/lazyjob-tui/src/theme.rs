use ratatui::style::{Color, Modifier, Style};

pub struct Theme {
    pub primary: Color,
    pub secondary: Color,
    pub success: Color,
    pub warning: Color,
    pub error: Color,
    pub text_primary: Color,
    pub text_secondary: Color,
    pub text_muted: Color,
    pub bg_primary: Color,
    pub bg_secondary: Color,
    pub border: Color,
    pub border_focused: Color,
}

impl Theme {
    pub const DARK: Self = Self {
        primary: Color::LightBlue,
        secondary: Color::DarkGray,
        success: Color::LightGreen,
        warning: Color::LightYellow,
        error: Color::LightRed,
        text_primary: Color::White,
        text_secondary: Color::Gray,
        text_muted: Color::DarkGray,
        bg_primary: Color::Reset,
        bg_secondary: Color::DarkGray,
        border: Color::DarkGray,
        border_focused: Color::LightBlue,
    };

    pub fn selected_style(&self) -> Style {
        Style::default()
            .fg(self.text_primary)
            .bg(self.primary)
            .add_modifier(Modifier::BOLD)
    }

    pub fn status_bar_style(&self) -> Style {
        Style::default()
            .fg(self.text_secondary)
            .bg(self.bg_secondary)
    }

    pub fn focused_border_style(&self) -> Style {
        Style::default().fg(self.border_focused)
    }

    pub fn tab_style(&self) -> Style {
        Style::default().fg(self.text_muted)
    }

    pub fn title_style(&self) -> Style {
        Style::default()
            .fg(self.primary)
            .add_modifier(Modifier::BOLD)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dark_theme_has_colors() {
        let t = &Theme::DARK;
        assert_eq!(t.primary, Color::LightBlue);
        assert_eq!(t.text_primary, Color::White);
        assert_eq!(t.error, Color::LightRed);
    }

    #[test]
    fn selected_style_has_bold() {
        let style = Theme::DARK.selected_style();
        assert!(style.add_modifier.contains(Modifier::BOLD));
    }
}
