//! The GPS signal row (design §4.4): whether the phone has a fix, how good it is, and what that
//! means for the people watching.
//!
//! The bar is decorative and hidden from assistive technology. The meaning is in text: the row
//! summary, and the detail rows when it is expanded. The accessible description changes only when
//! the level changes, not every second.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use jastrzab_core::engine::Event;
use jastrzab_core::i18n::tr;
use jastrzab_core::signal::{SignalButton, SignalView};
use jastrzab_core::view::View;

use super::widgets::*;
use crate::controller::Controller;

const QUALITY_CLASSES: [&str; 5] = ["q-poor", "q-fair", "q-good", "q-excellent", "q-none"];
const ICON_CLASSES: [&str; 4] = ["success", "warning", "error", "dim-label"];

pub struct SignalSection {
    group: adw::PreferencesGroup,
    row: adw::ExpanderRow,
    icon: gtk::Image,
    bar: gtk::LevelBar,
    button: gtk::Button,
    button_mode: Rc<Cell<SignalButton>>,
    accuracy: adw::ActionRow,
    source: adw::ActionRow,
    age: adw::ActionRow,
    viewers: adw::ActionRow,
    hint: adw::ActionRow,
    description: RefCell<String>,
}

impl SignalSection {
    pub fn new(controller: &Controller) -> Self {
        let group = adw::PreferencesGroup::new();
        let row = adw::ExpanderRow::builder().title(tr("GPS signal")).build();
        row.set_subtitle(&tr("Not checking"));
        // The expander row is a list item of its own: without this it has a description (set on
        // every level change) but no name.
        set_accessible_label(&row, &tr("GPS signal"));
        let icon = gtk::Image::from_icon_name("find-location-symbolic");
        row.add_prefix(&icon);

        let bar = gtk::LevelBar::builder().mode(gtk::LevelBarMode::Discrete).min_value(0.0).max_value(4.0).valign(gtk::Align::Center).build();
        bar.add_css_class("signal");
        hide_from_accessibility(&bar);
        // `AdwExpanderRow` in libadwaita 1.2 calls its end widgets "actions".
        row.add_action(&bar);

        let button = gtk::Button::builder().label(tr("Check")).valign(gtk::Align::Center).build();
        let button_mode = Rc::new(Cell::new(SignalButton::Check));
        {
            let (controller, mode) = (controller.clone(), button_mode.clone());
            button.connect_clicked(move |_| match mode.get() {
                SignalButton::Check => controller.send(Event::CheckSignal(true)),
                SignalButton::Stop => controller.send(Event::CheckSignal(false)),
                SignalButton::Hidden => {}
            });
        }
        row.add_action(&button);

        let detail = |title: &str| {
            let r = adw::ActionRow::builder().title(title).build();
            wrap_row(&r);
            row.add_row(&r);
            r
        };
        let accuracy = detail(&tr("Accuracy"));
        let source = detail(&tr("Source"));
        let age = detail(&tr("Last update"));
        let viewers = detail(&tr("What viewers see"));
        let hint = detail(&tr("What to try"));
        group.add(&row);
        Self { group, row, icon, bar, button, button_mode, accuracy, source, age, viewers, hint, description: RefCell::new(String::new()) }
    }

    pub fn widget(&self) -> &gtk::Widget {
        self.group.upcast_ref()
    }

    pub fn render(&self, view: &View) {
        let s: &SignalView = &view.signal;
        set_expander_subtitle(&self.row, &s.summary);
        set_icon(&self.icon, s.icon);
        set_style_class(&self.icon, &ICON_CLASSES, s.icon_class);

        if self.bar.value() != f64::from(s.bars) {
            self.bar.set_value(f64::from(s.bars));
        }
        set_style_class(&self.bar, &QUALITY_CLASSES, s.css_class);
        set_visible(&self.bar, s.button != SignalButton::Check);

        self.button_mode.set(s.button);
        set_visible(&self.button, s.button != SignalButton::Hidden);
        let label = if s.button == SignalButton::Check { tr("Check") } else { tr("Stop") };
        if self.button.label().as_deref() != Some(label.as_str()) {
            self.button.set_label(&label);
        }

        let optional = |row: &adw::ActionRow, text: &Option<String>| {
            set_visible(row, text.is_some());
            if let Some(text) = text {
                set_subtitle(row, text);
            }
        };
        optional(&self.accuracy, &s.accuracy);
        optional(&self.source, &s.source);
        optional(&self.age, &s.last_update);
        optional(&self.viewers, &s.viewers);
        optional(&self.hint, &s.hint);
        set_title(&self.viewers, &s.viewers_title);

        if *self.description.borrow() != s.description {
            *self.description.borrow_mut() = s.description.clone();
            set_accessible_description(&self.row, &s.description);
        }
    }
}
