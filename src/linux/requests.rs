//! Request widgets retain the exact session snapshot that created each control.
use super::{style, widgets};
use crate::{
    actors::monitor,
    domain::{interaction::Response, session::Session},
};
use gtk::{glib, pango, prelude::*};
use std::{collections::BTreeMap, rc::Rc};

pub struct Card {
    pub widget: gtk::Box,
    pub targets: Vec<gtk::Widget>,
}
impl Card {
    pub fn new(session: &Session, monitor: &monitor::Handle) -> Self {
        let card = gtk::Box::new(gtk::Orientation::Vertical, 8);
        style::class(&card, "request-card");
        let mut targets = Vec::new();
        let interaction = session.interaction();
        if interaction.questions.len() == 1 {
            card.pack_start(
                &widgets::label(&interaction.questions[0].title, "question", true),
                false,
                false,
                0,
            );
        }
        if let Some(approval) = &session.approval {
            if interaction.questions.is_empty()
                || (interaction.questions.len() == 1
                    && interaction.questions[0].title.trim() != approval.body.trim())
            {
                let body = widgets::label(
                    &approval.body,
                    if interaction.questions.is_empty() {
                        "question"
                    } else {
                        "secondary"
                    },
                    true,
                );
                body.set_selectable(true);
                card.pack_start(&body, false, false, 0);
            }
        }
        if interaction.questions.len() <= 1 {
            if let Some(command) = &interaction.command {
                card.pack_start(&command_block(command), false, false, 0);
            }
        }
        let buttons = interaction
            .actions
            .iter()
            .map(|a| {
                let b = widgets::action(&a.label);
                if a.label == "Deny" || a.label == "Cancel turn" {
                    style::class(&b, "subtle");
                }
                b.set_tooltip_text(Some(&a.detail));
                let h = monitor.clone();
                let s = session.clone();
                let id = a.id.clone();
                b.connect_clicked(move |_| {
                    let _ = h.submit(&s, Response::Action(id.clone()));
                });
                (b, a.detail.clone())
            })
            .collect::<Vec<_>>();
        let width = buttons
            .iter()
            .map(|(b, _)| b.preferred_width().1)
            .sum::<i32>()
            + (buttons.len().saturating_sub(1) * 6) as i32;
        let context = buttons.iter().any(|(_, detail)| !detail.is_empty());
        let actions = gtk::Box::new(
            if width <= 302 && !context {
                gtk::Orientation::Horizontal
            } else {
                gtk::Orientation::Vertical
            },
            6,
        );
        for (button, detail) in buttons {
            targets.push(button.clone().upcast());
            if !detail.is_empty() {
                let block = widgets::label(&detail, "command", true);
                block.set_selectable(true);
                actions.pack_start(&block, false, false, 0);
            }
            actions.pack_start(&button, false, false, 0);
        }
        if !interaction.actions.is_empty() {
            card.pack_start(&actions, false, false, 0);
        }
        if interaction.questions.len() == 1 {
            let q = &interaction.questions[0];
            for choice in &q.choices {
                let button = widgets::action(&choice.label);
                button.set_tooltip_text(Some(&choice.description));
                let h = monitor.clone();
                let s = session.clone();
                let id = q.id.clone();
                let value = choice.label.clone();
                button.connect_clicked(move |_| {
                    let _ = h.submit(
                        &s,
                        Response::Answers(BTreeMap::from([(id.clone(), vec![value.clone()])])),
                    );
                });
                targets.push(button.clone().upcast());
                card.pack_start(&button, false, false, 0);
            }
            if q.freeform {
                let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
                let entry = gtk::Entry::new();
                entry.set_placeholder_text(Some("Your own answer"));
                entry.set_visibility(!q.secret);
                if let Some(accessible) = entry.accessible() {
                    gtk::atk::prelude::AtkObjectExt::set_name(&accessible, &q.title);
                }
                entry.set_hexpand(true);
                let send = widgets::action("Send answer");
                let h = monitor.clone();
                let s = session.clone();
                let id = q.id.clone();
                let e = entry.clone();
                send.connect_clicked(move |_| {
                    let value = e.text().to_string();
                    if !value.trim().is_empty() {
                        let _ = h.submit(
                            &s,
                            Response::Answers(BTreeMap::from([(id.clone(), vec![value])])),
                        );
                    }
                });
                row.pack_start(&entry, true, true, 0);
                row.pack_start(&send, false, false, 0);
                targets.push(entry.upcast());
                targets.push(send.upcast());
                card.pack_start(&row, false, false, 0);
            }
        } else if !interaction.questions.is_empty() {
            let mut groups = Vec::new();
            let form = gtk::Box::new(gtk::Orientation::Vertical, 0);
            for (index, q) in interaction.questions.iter().enumerate() {
                let group = gtk::Box::new(gtk::Orientation::Vertical, 0);
                style::class(&group, "question-group");
                if index > 0 {
                    group.set_margin_top(14);
                }
                let title = widgets::label(&q.title, "question", true);
                title.set_margin_bottom(10);
                group.pack_start(&title, false, false, 0);
                if index == 0 {
                    if let Some(approval) = &session.approval {
                        if !approval.body.trim().is_empty()
                            && !interaction
                                .questions
                                .iter()
                                .any(|q| q.title.trim() == approval.body.trim())
                        {
                            let explanation = widgets::label(&approval.body, "secondary", true);
                            explanation.set_selectable(true);
                            explanation.set_margin_bottom(10);
                            group.pack_start(&explanation, false, false, 0);
                        }
                    }
                    if let Some(command) = &interaction.command {
                        group.pack_start(&command_block(command), false, false, 0);
                    }
                }
                let mut options = Vec::<(gtk::RadioButton, String)>::new();
                for choice in &q.choices {
                    let radio = if let Some((first, _)) = options.first() {
                        gtk::RadioButton::with_label_from_widget(first, &choice.label)
                    } else {
                        gtk::RadioButton::with_label(&choice.label)
                    };
                    style::class(&radio, "answer");
                    radio.set_tooltip_text(Some(&choice.description));
                    if let Some(label) = radio.child().and_then(|w| w.downcast::<gtk::Label>().ok())
                    {
                        label.set_line_wrap(true);
                        label.set_line_wrap_mode(pango::WrapMode::WordChar);
                    }
                    targets.push(radio.clone().upcast());
                    group.pack_start(&radio, false, false, 0);
                    options.push((radio, choice.label.clone()));
                }
                let custom = if q.freeform {
                    let row = gtk::Box::new(gtk::Orientation::Horizontal, 7);
                    style::class(&row, "custom-answer");
                    let radio = if let Some((first, _)) = options.first() {
                        gtk::RadioButton::from_widget(first)
                    } else {
                        gtk::RadioButton::new()
                    };
                    style::class(&radio, "custom-radio");
                    radio.set_can_focus(false);
                    radio.set_valign(gtk::Align::Start);
                    radio.set_tooltip_text(Some("Your own answer"));
                    let entry = gtk::Entry::new();
                    entry.set_placeholder_text(Some("Your own answer"));
                    entry.set_visibility(!q.secret);
                    if let Some(accessible) = entry.accessible() {
                        gtk::atk::prelude::AtkObjectExt::set_name(&accessible, &q.title);
                    }
                    entry.set_hexpand(true);
                    let r = radio.clone();
                    entry.connect_changed(move |e| {
                        if !e.text().is_empty() {
                            r.set_active(true);
                        }
                    });
                    let r = radio.clone();
                    entry.connect_activate(move |_| r.set_active(true));
                    let r = radio.clone();
                    entry.connect_focus_in_event(move |_, _| {
                        r.set_active(true);
                        glib::Propagation::Proceed
                    });
                    row.pack_start(&radio, false, false, 0);
                    row.pack_start(&entry, true, true, 0);
                    targets.push(entry.clone().upcast());
                    group.pack_start(&row, false, false, 0);
                    Some((radio, entry))
                } else {
                    None
                };
                form.pack_start(&group, false, false, 0);
                groups.push((q.id.clone(), options, custom));
            }
            card.pack_start(&form, false, false, 0);
            let groups = Rc::new(groups);
            let send = widgets::action("Send answer");
            send.set_margin_top(3); // Card spacing (8) + 3 = the mockup's 11px gap.
            let h = monitor.clone();
            let s = session.clone();
            send.connect_clicked(move |_| {
                let mut values = BTreeMap::new();
                for (id, options, custom) in groups.iter() {
                    let value = custom
                        .as_ref()
                        .filter(|(r, _)| r.is_active())
                        .map(|(_, e)| e.text().to_string())
                        .or_else(|| {
                            options
                                .iter()
                                .find(|(r, _)| r.is_active())
                                .map(|(_, v)| v.clone())
                        })
                        .unwrap_or_default();
                    if value.trim().is_empty() {
                        return;
                    }
                    values.insert(id.clone(), vec![value]);
                }
                let _ = h.submit(&s, Response::Answers(values));
            });
            targets.push(send.clone().upcast());
            card.pack_start(&send, false, false, 0);
        }
        Self {
            widget: card,
            targets,
        }
    }
}
fn command_projection(
    label: &gtk::Label,
    full: &str,
    width: i32,
    expanded: bool,
) -> (String, bool) {
    let layout = label.create_pango_layout(Some(full));
    layout.set_width(width.max(1) * pango::SCALE);
    layout.set_wrap(pango::WrapMode::WordChar);
    let overflow = layout.line_count() > 2;
    if expanded || !overflow {
        return (full.to_owned(), overflow);
    }
    let line = layout.line_readonly(1).unwrap();
    let end = (line.start_index() + line.length()) as usize;
    let mut prefix = full[..end.min(full.len())].trim_end().to_owned();
    loop {
        let candidate = format!("{prefix}…");
        layout.set_text(&candidate);
        if layout.line_count() <= 2 || prefix.is_empty() {
            return (candidate, true);
        }
        super::model::pop_grapheme(&mut prefix);
    }
}
fn command_block(text: &str) -> gtk::Box {
    let group = gtk::Box::new(gtk::Orientation::Vertical, 4);
    let caption = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    caption.pack_start(
        &widgets::label("Command", "secondary", false),
        true,
        true,
        0,
    );
    let expand = widgets::icon_button("pan-down-symbolic", "Expand command");
    expand.set_can_focus(false);
    caption.pack_end(&expand, false, false, 0);
    group.pack_start(&caption, false, false, 0);
    let command = widgets::label(text, "command", true);
    command.set_selectable(true);
    group.pack_start(&command, false, false, 0);
    let expanded = Rc::new(std::cell::Cell::new(false));
    let e = expanded.clone();
    let c = command.clone();
    let full = text.to_owned();
    expand.connect_clicked(move |b| {
        e.set(!e.get());
        let (projected, _) = command_projection(&c, &full, c.allocated_width() - 20, e.get());
        c.set_text(&projected);
        b.set_tooltip_text(Some(if e.get() {
            "Collapse command"
        } else {
            "Expand command"
        }));
    });
    let b = expand.clone();
    let full = text.to_owned();
    command.connect_size_allocate(move |label, rect| {
        let (projected, overflow) =
            command_projection(label, &full, rect.width() - 20, expanded.get());
        b.set_visible(overflow);
        if label.text() != projected {
            label.set_text(&projected);
        }
    });
    group
}
