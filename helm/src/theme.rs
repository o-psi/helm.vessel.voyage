//! Application-owned roles and passive, per-screen terminal color adaptation.
use ratatui::{
    buffer::Buffer,
    style::{Color, Modifier, Style},
};
use std::collections::HashMap;
use termprofile::{DetectorSettings, TermProfile};

/// Application palette. Roles describe meaning, independently of terminal capabilities.
#[derive(Clone, Copy)]
pub(crate) enum Role {
    Primary,
    Muted,
    Focus,
    Selection,
    Hover,
    SearchMatch,
    Link,
    TableHeader,
    Running,
    AwaitingInput,
    Failed,
    Completed,
    PrivateTerminal,
    Code,
}

impl Role {
    pub(crate) fn style(self) -> Style {
        match self {
            Self::Primary => Style::default(),
            Self::Muted => Style::default().fg(Color::DarkGray),
            Self::Focus => Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
            Self::Selection => Style::default().add_modifier(Modifier::REVERSED | Modifier::BOLD),
            Self::Hover => Style::default().add_modifier(Modifier::UNDERLINED | Modifier::BOLD),
            Self::SearchMatch => Style::default().add_modifier(Modifier::REVERSED),
            Self::Link => Style::default()
                .fg(Color::Blue)
                .add_modifier(Modifier::UNDERLINED),
            Self::TableHeader => Style::default()
                .fg(Color::Magenta)
                .add_modifier(Modifier::BOLD),
            Self::Running => Style::default().fg(Color::Blue),
            Self::AwaitingInput => Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
            Self::Failed => Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            Self::Completed => Style::default().fg(Color::Green),
            Self::PrivateTerminal => Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
            Self::Code => Style::default().fg(Color::Yellow).bg(Color::DarkGray),
        }
    }
}

pub(crate) struct TerminalStyles {
    profile: TermProfile,
    // Local to this screen/profile. Bound adversarial syntax/legacy colors without
    // a global lock or an additional cache dependency.
    colors: HashMap<Color, Color>,
}

impl TerminalStyles {
    pub(crate) fn from_env() -> anyhow::Result<Self> {
        let override_value = std::env::var("HELM_COLOR");
        let profile = match override_value.as_deref() {
            Ok("never") => TermProfile::NoColor,
            Ok("16") => TermProfile::Ansi16,
            Ok("256") => TermProfile::Ansi256,
            Ok("truecolor") => TermProfile::TrueColor,
            Ok("auto") | Err(std::env::VarError::NotPresent) => {
                // NO_COLOR specifies *any* nonempty value, not only booleans.
                if std::env::var_os("NO_COLOR").is_some_and(|value| !value.is_empty()) {
                    TermProfile::NoColor
                } else {
                    TermProfile::detect(
                        &std::io::stdout(),
                        // Defaults may run `tmux info`; never spawn or read input here.
                        DetectorSettings::new().enable_tmux_info(false),
                    )
                }
            }
            _ => anyhow::bail!("HELM_COLOR must be auto, never, 16, 256, or truecolor"),
        };
        Ok(Self {
            profile,
            colors: HashMap::new(),
        })
    }

    fn color(&mut self, color: Color) -> Color {
        if self.profile < TermProfile::Ansi16 {
            return Color::Reset;
        }
        if !matches!(color, Color::Rgb(..) | Color::Indexed(_)) {
            return color;
        }
        if let Some(converted) = self.colors.get(&color) {
            return *converted;
        }
        let converted = self.profile.adapt_color(color).unwrap_or(Color::Reset);
        if self.colors.len() >= 4096 {
            self.colors.clear();
        }
        self.colors.insert(color, converted);
        converted
    }

    /// O(visible terminal cells), never O(transcript/history size). Mutate the
    /// completed frame so overlay and syntax colors cannot bypass the fallback.
    pub(crate) fn apply(&mut self, buffer: &mut Buffer) {
        if self.profile == TermProfile::TrueColor {
            return;
        }
        for cell in &mut buffer.content {
            cell.fg = self.color(cell.fg);
            cell.bg = self.color(cell.bg);
            cell.underline_color = self.color(cell.underline_color);
            if self.profile == TermProfile::NoTty {
                cell.modifier = Modifier::empty();
            }
        }
    }
}
