use gtk::{gdk, prelude::*};
pub fn dark(widget: &impl IsA<gtk::Widget>) -> bool {
    widget
        .style_context()
        .lookup_color("theme_bg_color")
        .is_some_and(|c| c.red() + c.green() + c.blue() < 1.5)
}
pub fn install(window: &gtk::Window) -> gtk::CssProvider {
    let provider = gtk::CssProvider::new();
    let dark = dark(window);
    let tokens = if dark {
        "@define-color message #cfcfcf; @define-color request #373737; @define-color command #2a2a2a; @define-color footer #2c2c2c; @define-color working #30d158; @define-color finished #73c7ff;"
    } else {
        "@define-color message #414141; @define-color request #e7e7e7; @define-color command #dadada; @define-color footer #e7e7e7; @define-color working #007637; @define-color finished #0066be;"
    };
    let css = format!("{tokens}\n{}", include_str!("style.css"));
    provider
        .load_from_data(css.as_bytes())
        .expect("valid built-in GTK stylesheet");
    if let Some(screen) = gdk::Screen::default() {
        gtk::StyleContext::add_provider_for_screen(
            &screen,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
    provider
}
pub fn class(widget: &impl IsA<gtk::Widget>, name: &str) {
    widget.style_context().add_class(name);
    if matches!(name, "icon-button" | "robot-button") {
        widget.set_halign(gtk::Align::Center);
        widget.set_valign(gtk::Align::Center);
        widget.set_size_request(28, 28);
    }
}
