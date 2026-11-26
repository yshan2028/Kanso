use iced::border;
use iced::widget::{button, container, text, text_input};
use iced::{Background, Color, Element, Length, Theme};

pub const BG_MAIN: Color = Color::from_rgb(0.035, 0.035, 0.043); // #09090B
pub const BG_SIDEBAR: Color = Color::from_rgb(0.051, 0.051, 0.067); // #0D0D11
pub const BG_CARD: Color = Color::from_rgb(0.078, 0.078, 0.102); // #14141A
pub const BG_ELEVATED: Color = Color::from_rgb(0.106, 0.106, 0.141); // #1B1B24
pub const BORDER: Color = Color::from_rgb(0.153, 0.153, 0.165); // #27272A

pub const TEXT_PRIMARY: Color = Color::from_rgb(0.957, 0.957, 0.961); // #F4F4F5
pub const TEXT_SECONDARY: Color = Color::from_rgb(0.631, 0.631, 0.667); // #A1A1AA
pub const TEXT_MUTED: Color = Color::from_rgb(0.443, 0.443, 0.478); // #71717A

pub const ACCENT_SKY: Color = Color::from_rgb(0.220, 0.741, 0.973); // #38BDF8
pub const ACCENT_GREEN: Color = Color::from_rgb(0.204, 0.827, 0.600); // #34D399
pub const ACCENT_AMBER: Color = Color::from_rgb(0.984, 0.749, 0.141); // #FBBF24
pub const ACCENT_ROSE: Color = Color::from_rgb(0.984, 0.443, 0.522); // #FB7185
pub const ACCENT_VIOLET: Color = Color::from_rgb(0.655, 0.545, 0.980); // #A78BFA

pub fn kanso_theme() -> Theme {
    Theme::custom(
        "Kanso Dark".to_string(),
        iced::theme::Palette {
            background: BG_MAIN,
            text: TEXT_PRIMARY,
            primary: ACCENT_SKY,
            success: ACCENT_GREEN,
            warning: ACCENT_AMBER,
            danger: ACCENT_ROSE,
        },
    )
}

pub fn divider<'a, M: 'a>() -> Element<'a, M> {
    container(text("").size(1))
        .width(Length::Fill)
        .height(1)
        .style(|_| container::Style {
            background: Some(Background::Color(BORDER)),
            ..Default::default()
        })
        .into()
}

pub fn divider_v<'a, M: 'a>() -> Element<'a, M> {
    container(text("").size(1))
        .width(1)
        .height(Length::Fill)
        .style(|_| container::Style {
            background: Some(Background::Color(BORDER)),
            ..Default::default()
        })
        .into()
}

pub fn sidebar_style(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(BG_SIDEBAR)),
        border: border::Border {
            color: BORDER,
            width: 0.0,
            radius: 0.0.into(),
        },
        ..Default::default()
    }
}

pub fn panel_style(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(BG_MAIN)),
        ..Default::default()
    }
}

pub fn card_style(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(BG_CARD)),
        border: border::Border {
            color: BORDER,
            width: 1.0,
            radius: 10.0.into(),
        },
        ..Default::default()
    }
}

pub fn elevated_style(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(BG_ELEVATED)),
        border: border::Border {
            color: BORDER,
            width: 1.0,
            radius: 8.0.into(),
        },
        ..Default::default()
    }
}

pub fn alert_banner_style(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(Color::from_rgba(0.984, 0.749, 0.141, 0.09))),
        border: border::Border {
            color: Color::from_rgba(0.984, 0.749, 0.141, 0.35),
            width: 1.0,
            radius: 0.0.into(),
        },
        ..Default::default()
    }
}

pub fn badge_style(color: Color) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        background: Some(Background::Color(Color::from_rgba(
            color.r, color.g, color.b, 0.14,
        ))),
        border: border::Border {
            color: Color::from_rgba(color.r, color.g, color.b, 0.35),
            width: 1.0,
            radius: 6.0.into(),
        },
        ..Default::default()
    }
}

pub fn primary_btn_style(_theme: &Theme, status: button::Status) -> button::Style {
    let bg = match status {
        button::Status::Hovered => Color::from_rgb(0.42, 0.82, 0.99),
        button::Status::Disabled => Color::from_rgba(0.220, 0.741, 0.973, 0.35),
        _ => ACCENT_SKY,
    };
    button::Style {
        background: Some(Background::Color(bg)),
        text_color: BG_MAIN,
        border: border::Border {
            radius: 7.0.into(),
            ..Default::default()
        },
        ..Default::default()
    }
}

pub fn warning_btn_style(_theme: &Theme, status: button::Status) -> button::Style {
    let bg = match status {
        button::Status::Hovered => Color::from_rgb(0.99, 0.82, 0.30),
        _ => ACCENT_AMBER,
    };
    button::Style {
        background: Some(Background::Color(bg)),
        text_color: BG_MAIN,
        border: border::Border {
            radius: 6.0.into(),
            ..Default::default()
        },
        ..Default::default()
    }
}

pub fn danger_btn_style(_theme: &Theme, status: button::Status) -> button::Style {
    let alpha = match status {
        button::Status::Hovered => 0.28,
        _ => 0.16,
    };
    button::Style {
        background: Some(Background::Color(Color::from_rgba(
            ACCENT_ROSE.r,
            ACCENT_ROSE.g,
            ACCENT_ROSE.b,
            alpha,
        ))),
        text_color: ACCENT_ROSE,
        border: border::Border {
            color: Color::from_rgba(ACCENT_ROSE.r, ACCENT_ROSE.g, ACCENT_ROSE.b, 0.45),
            width: 1.0,
            radius: 7.0.into(),
        },
        ..Default::default()
    }
}

pub fn secondary_btn_style(_theme: &Theme, status: button::Status) -> button::Style {
    let bg = match status {
        button::Status::Hovered => BG_ELEVATED,
        _ => BG_CARD,
    };
    button::Style {
        background: Some(Background::Color(bg)),
        text_color: TEXT_PRIMARY,
        border: border::Border {
            color: BORDER,
            width: 1.0,
            radius: 7.0.into(),
        },
        ..Default::default()
    }
}

pub fn nav_btn_style(active: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| {
        let bg = if active {
            Some(Background::Color(BG_ELEVATED))
        } else if matches!(status, button::Status::Hovered) {
            Some(Background::Color(Color::from_rgba(1.0, 1.0, 1.0, 0.04)))
        } else {
            None
        };
        button::Style {
            background: bg,
            text_color: if active { TEXT_PRIMARY } else { TEXT_SECONDARY },
            border: border::Border {
                color: if active { BORDER } else { Color::TRANSPARENT },
                width: 1.0,
                radius: 8.0.into(),
            },
            ..Default::default()
        }
    }
}

pub fn input_style(_theme: &Theme, status: text_input::Status) -> text_input::Style {
    let border_color = match status {
        text_input::Status::Focused { .. } => ACCENT_SKY,
        _ => BORDER,
    };
    text_input::Style {
        background: Background::Color(BG_MAIN),
        border: border::Border {
            color: border_color,
            width: 1.0,
            radius: 7.0.into(),
        },
        icon: TEXT_MUTED,
        placeholder: TEXT_MUTED,
        value: TEXT_PRIMARY,
        selection: Color::from_rgba(ACCENT_SKY.r, ACCENT_SKY.g, ACCENT_SKY.b, 0.3),
    }
}
