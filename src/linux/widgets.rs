use super::{model, style};
use crate::domain::session::Activity;
use gtk::{cairo, glib, pango, prelude::*};
use std::{cell::Cell, rc::Rc};

pub fn mouse_transparent_layer() -> gtk::DrawingArea {
    let layer = gtk::DrawingArea::new();
    layer.set_can_focus(false);
    layer.set_has_tooltip(false);
    layer.connect_realize(|layer| {
        if let Some(window) = layer.window() {
            // Native input transparency covers crossing, motion, buttons and wheel events.
            window.set_pass_through(true);
        }
    });
    layer
}

pub fn label(text: &str, class: &str, wrap: bool) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.set_xalign(0.0);
    style::class(&label, class);
    label.set_line_wrap(wrap);
    label.set_line_wrap_mode(pango::WrapMode::WordChar);
    label
}
pub fn message(text: &str, dark: bool) -> gtk::Label {
    let label = label("", "message", true);
    label.set_selectable(true);
    label.set_markup(&model::markup(
        if text.trim_end().is_empty() {
            "No messages yet"
        } else {
            text.trim_end()
        },
        dark,
    ));
    if let Some(layout) = label.layout() {
        layout.set_spacing(3 * pango::SCALE);
    }
    label
}
pub fn icon_button(icon: &str, tooltip: &str) -> gtk::Button {
    let button = gtk::Button::new();
    style::class(&button, "icon-button");
    let image = gtk::Image::from_icon_name(Some(icon), gtk::IconSize::Menu);
    image.set_pixel_size(14);
    button.set_image(Some(&image));
    button.set_tooltip_text(Some(tooltip));
    if let Some(accessible) = button.accessible() {
        gtk::atk::prelude::AtkObjectExt::set_name(&accessible, tooltip);
    }
    button
}
pub fn action(caption: &str) -> gtk::Button {
    let b = gtk::Button::with_label(caption);
    style::class(&b, "action");
    b.set_halign(gtk::Align::Start);
    b
}
pub fn meta(icon: &str, text: &str) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    style::class(&row, "metadata-row");
    let image: gtk::Widget = if icon == "vcs-branch-symbolic" {
        let area = gtk::DrawingArea::new();
        area.set_size_request(12, 12);
        area.connect_draw(|w, c| {
            let fg = w.style_context().color(gtk::StateFlags::NORMAL);
            c.set_source_rgba(fg.red(), fg.green(), fg.blue(), 0.55);
            c.set_line_width(1.2);
            c.move_to(3.0, 3.3);
            c.line_to(3.0, 8.7);
            c.move_to(9.0, 4.3);
            c.curve_to(9.0, 7.0, 3.0, 5.0, 3.0, 8.0);
            let _ = c.stroke();
            for (x, y) in [(3.0, 2.0), (9.0, 3.0), (3.0, 10.0)] {
                c.arc(x, y, 1.3, 0.0, std::f64::consts::TAU);
                let _ = c.stroke();
            }
            glib::Propagation::Proceed
        });
        area.upcast()
    } else {
        let image = gtk::Image::from_icon_name(Some(icon), gtk::IconSize::Menu);
        image.set_pixel_size(12);
        image.upcast()
    };
    row.pack_start(&image, false, false, 0);
    let text = label(text, "metadata", false);
    text.set_selectable(true);
    text.set_ellipsize(pango::EllipsizeMode::Middle);
    text.set_hexpand(true);
    text.set_tooltip_text(Some(text.text().as_str()));
    row.pack_start(&text, true, true, 0);
    row
}
pub fn application_icon(client: crate::domain::session::Client) -> gtk::Image {
    let desktop_ids: &[&str] = match client {
        crate::domain::session::Client::VSCode => &[
            "code.desktop",
            "com.visualstudio.code.desktop",
            "com.microsoft.VSCode.desktop",
            "code-url-handler.desktop",
        ],
        crate::domain::session::Client::Terminal => &[
            "org.gnome.Terminal.desktop",
            "org.gnome.Ptyxis.desktop",
            "org.gnome.Console.desktop",
        ],
        _ => &[],
    };
    for id in desktop_ids {
        if let Some(icon) = gtk::gio::DesktopAppInfo::new(id).and_then(|app| app.icon()) {
            let image = gtk::Image::from_gicon(&icon, gtk::IconSize::LargeToolbar);
            image.set_pixel_size(24);
            image.set_tooltip_text(Some(client.label()));
            return image;
        }
    }
    let names = match client {
        crate::domain::session::Client::VSCode => {
            vec!["com.visualstudio.code", "visual-studio-code", "code"]
        }
        crate::domain::session::Client::Terminal => {
            vec!["org.gnome.Terminal", "utilities-terminal"]
        }
        _ => vec!["application-x-executable"],
    };
    let theme = gtk::IconTheme::default();
    let name = names
        .iter()
        .find(|n| theme.as_ref().is_some_and(|t| t.has_icon(n)))
        .copied()
        .unwrap_or("application-x-executable");
    let image = gtk::Image::from_icon_name(Some(name), gtk::IconSize::LargeToolbar);
    image.set_pixel_size(24);
    image.set_tooltip_text(Some(client.label()));
    image
}
pub fn application_pixbuf(
    client: crate::domain::session::Client,
) -> Option<gtk::gdk_pixbuf::Pixbuf> {
    let image = application_icon(client);
    let theme = gtk::IconTheme::default()?;
    if image.storage_type() == gtk::ImageType::Gicon {
        theme
            .lookup_by_gicon(&image.gicon().0, 24, gtk::IconLookupFlags::FORCE_SIZE)?
            .load_icon()
            .ok()
    } else {
        theme
            .load_icon(
                image.icon_name()?.as_str(),
                24,
                gtk::IconLookupFlags::FORCE_SIZE,
            )
            .ok()
            .flatten()
    }
}
pub(super) fn rounded(c: &cairo::Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    use std::f64::consts::{FRAC_PI_2, PI};
    c.new_sub_path();
    c.arc(x + w - r, y + r, r, -FRAC_PI_2, 0.0);
    c.arc(x + w - r, y + h - r, r, 0.0, FRAC_PI_2);
    c.arc(x + r, y + h - r, r, FRAC_PI_2, PI);
    c.arc(x + r, y + r, r, PI, PI + FRAC_PI_2);
    c.close_path();
}
pub fn robot(activity: Activity) -> gtk::DrawingArea {
    let area = gtk::DrawingArea::new();
    area.set_has_window(false);
    area.set_size_request(14, 11);
    area.set_halign(gtk::Align::Center);
    area.set_valign(gtk::Align::Center);
    area.connect_draw(move |w, c| {
        let fg = w.style_context().color(gtk::StateFlags::NORMAL);
        let _ = c.save();
        c.scale(
            w.allocated_width() as f64 / 22.0,
            w.allocated_height() as f64 / 16.0,
        );
        c.translate(0.8, 0.0);
        c.set_source_rgba(fg.red(), fg.green(), fg.blue(), 0.6);
        c.set_fill_rule(cairo::FillRule::EvenOdd);
        rounded(c, 1.4, 0.4, 17.2, 15.2, 4.8);
        rounded(c, 3.0, 3.4, 14.0, 9.2, 3.2);
        let _ = c.fill();
        rounded(c, 0.0, 4.9, 3.2, 6.2, 1.3);
        rounded(c, 17.2, 4.9, 3.2, 6.2, 1.3);
        let _ = c.fill();
        match activity {
            Activity::Working => {
                if style::dark(w) {
                    c.set_source_rgb(0.188, 0.82, 0.345)
                } else {
                    c.set_source_rgb(0.0, 0.463, 0.216)
                }
            }
            Activity::Confirmation => c.set_source_rgb(1.0, 0.584, 0.0),
            _ => c.set_source_rgba(fg.red(), fg.green(), fg.blue(), 0.6),
        };
        for x in [7.45, 12.55] {
            c.arc(x, 8.0, 1.8, 0.0, std::f64::consts::TAU);
            let _ = c.fill();
        }
        let _ = c.restore();
        glib::Propagation::Proceed
    });
    area
}
pub fn status(activity: Activity, finished: bool, phase: Rc<Cell<f64>>) -> gtk::DrawingArea {
    let area = gtk::DrawingArea::new();
    area.set_has_window(false);
    area.set_size_request(18, 18);
    area.set_halign(gtk::Align::Center);
    area.set_valign(gtk::Align::Center);
    area.set_tooltip_text(Some(if finished {
        "Mark as seen"
    } else {
        activity.label()
    }));
    area.connect_draw(move |w, c| {
        use std::f64::consts::{PI, TAU};
        let _ = c.save();
        c.translate(9.0, 9.0);
        if finished || activity == Activity::Confirmation {
            if finished {
                if style::dark(w) {
                    c.set_source_rgb(0.45, 0.78, 1.0)
                } else {
                    c.set_source_rgb(0.0, 0.4, 0.745)
                }
            } else {
                c.set_source_rgb(1.0, 0.584, 0.0)
            };
            c.arc(0.0, 0.0, 8.0, 0.0, TAU);
            let _ = c.fill();
            c.set_source_rgb(1.0, 1.0, 1.0);
            c.set_line_width(1.7);
            c.set_line_cap(cairo::LineCap::Round);
            if finished {
                c.move_to(-3.5, 0.0);
                c.line_to(-0.8, 2.5);
                c.line_to(4.0, -3.0);
                let _ = c.stroke();
            } else {
                c.move_to(0.0, -4.0);
                c.line_to(0.0, 0.8);
                let _ = c.stroke();
                c.arc(0.0, 4.0, 0.8, 0.0, TAU);
                let _ = c.fill();
            }
        } else if activity == Activity::Working {
            if style::dark(w) {
                c.set_source_rgb(0.188, 0.82, 0.345)
            } else {
                c.set_source_rgb(0.0, 0.463, 0.216)
            };
            c.rotate(phase.get());
            c.set_line_width(1.3);
            for a in [0.0, PI] {
                c.arc(0.0, 0.0, 5.4, a, a + 2.25);
                let _ = c.stroke();
                let end = a + 2.25;
                c.move_to(5.4 * end.cos(), 5.4 * end.sin());
                c.rel_line_to(-2.0, -0.3);
                c.rel_line_to(1.5, 2.0);
                c.close_path();
                let _ = c.fill();
            }
        } else if activity == Activity::Unknown {
            let fg = w.style_context().color(gtk::StateFlags::NORMAL);
            c.set_source_rgba(fg.red(), fg.green(), fg.blue(), 0.45);
            c.select_font_face("sans", cairo::FontSlant::Normal, cairo::FontWeight::Normal);
            c.set_font_size(14.0);
            c.move_to(-4.0, 5.0);
            let _ = c.show_text("?");
        }
        let _ = c.restore();
        glib::Propagation::Proceed
    });
    area
}

/// Chevron and disclosure use the guide's 240 ms cubic ease-out timing.
pub fn chevron(revealer: &gtk::Revealer) -> gtk::DrawingArea {
    use std::time::{Duration, Instant};
    let area = gtk::DrawingArea::new();
    area.set_has_window(false);
    area.set_size_request(14, 14);
    area.set_halign(gtk::Align::Center);
    area.set_valign(gtk::Align::Center);
    let from = Rc::new(Cell::new(0.0));
    let target = Rc::new(Cell::new(0.0));
    let start = Rc::new(Cell::new(Instant::now()));
    let progress = Rc::new(Cell::new(0.0));
    let running = Rc::new(Cell::new(false));
    let p = progress.clone();
    area.connect_draw(move |w, c| {
        let fg = w.style_context().color(gtk::StateFlags::NORMAL);
        c.set_source_rgba(fg.red(), fg.green(), fg.blue(), 0.55);
        c.translate(7.0, 7.0);
        c.rotate(p.get() * std::f64::consts::FRAC_PI_2);
        c.set_line_width(1.3);
        c.set_line_cap(cairo::LineCap::Round);
        c.move_to(-2.0, -4.0);
        c.line_to(2.0, 0.0);
        c.line_to(-2.0, 4.0);
        let _ = c.stroke();
        glib::Propagation::Proceed
    });
    let weak = area.downgrade();
    revealer.connect_reveal_child_notify(move |revealer| {
        let Some(area) = weak.upgrade() else {
            return;
        };
        let elapsed = (start.get().elapsed().as_secs_f64() / 0.240).min(1.0);
        let sampled = from.get() + (target.get() - from.get()) * (1.0 - (1.0 - elapsed).powi(3));
        from.set(sampled);
        target.set(if revealer.reveals_child() { 1.0 } else { 0.0 });
        start.set(Instant::now());
        if gtk::Settings::default().is_some_and(|s| !s.is_gtk_enable_animations()) {
            progress.set(target.get());
            area.queue_draw();
            return;
        }
        if !running.replace(true) {
            let from = from.clone();
            let target = target.clone();
            let start = start.clone();
            let p = progress.clone();
            let active = running.clone();
            area.add_tick_callback(move |w, _| {
                let elapsed = (start.get().elapsed().as_secs_f64()
                    / Duration::from_millis(240).as_secs_f64())
                .min(1.0);
                p.set(from.get() + (target.get() - from.get()) * (1.0 - (1.0 - elapsed).powi(3)));
                w.queue_draw();
                if elapsed >= 1.0 {
                    active.set(false);
                    glib::ControlFlow::Break
                } else {
                    glib::ControlFlow::Continue
                }
            });
        }
    });
    area
}
