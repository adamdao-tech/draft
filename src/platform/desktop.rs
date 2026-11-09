//! The desktop side of [`crate::platform`]: the real thing, mostly re-exported.

pub use std::fs;
pub use std::time::Instant;

pub use tokio::time::sleep;

pub mod dialog {
    pub use rfd::AsyncFileDialog;
}

pub fn spawn<F: FnOnce() + Send + 'static>(work: F) {
    std::thread::spawn(work);
}

pub fn process_id() -> u32 {
    std::process::id()
}

pub type WindowId = dioxus::desktop::tao::window::WindowId;

pub fn window_id() -> WindowId {
    dioxus::desktop::window().id()
}

pub fn set_title(title: &str) {
    dioxus::desktop::window().set_title(title);
}

pub fn set_fullscreen(on: bool) {
    dioxus::desktop::window().set_fullscreen(on);
}

pub fn focus_window() {
    dioxus::desktop::window().set_focus();
}

pub fn show_window() {
    dioxus::desktop::window().set_visible(true);
}

pub fn close_window() {
    let window = dioxus::desktop::window();
    window.set_close_behavior(dioxus::desktop::WindowCloseBehaviour::WindowCloses);
    window.close();
}
