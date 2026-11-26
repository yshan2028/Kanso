mod app;
mod theme;
mod views;

use app::KansoApp;
use iced::{Size, window};

fn main() -> iced::Result {
    iced::application(KansoApp::new, KansoApp::update, KansoApp::view)
        .title("Kanso (簡素) — AI 多仓工程控制塔")
        .subscription(KansoApp::subscription)
        .theme(KansoApp::theme)
        .window(window::Settings {
            size: Size::new(1360.0, 860.0),
            min_size: Some(Size::new(1024.0, 680.0)),
            ..Default::default()
        })
        .run()
}
