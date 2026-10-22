//! The status row (what the share is doing) and the offer to resume an unfinished one.

use std::cell::Cell;

use adw::prelude::*;
use jastrzab_core::engine::Event;
use jastrzab_core::i18n::{tr, tr_args};
use jastrzab_core::view::{PhaseView, StatusDetail, View};

use super::widgets::*;
use crate::controller::Controller;

pub struct StatusSection {
    group: adw::PreferencesGroup,
    row: adw::ActionRow,
    icon: gtk::Image,
    countdown: gtk::Label,
    expires: Cell<Option<f64>>,
}

impl StatusSection {
    pub fn new() -> Self {
        let group = adw::PreferencesGroup::new();
        let list = boxed_list();
        let row = adw::ActionRow::new();
        wrap_row(&row);
        let icon = gtk::Image::builder().pixel_size(32).build();
        row.add_prefix(&icon);
        // The countdown is its own numeric label (tabular figures), not part of any button text,
        // so a screen reader does not re-read a button every second.
        let countdown = gtk::Label::builder().valign(gtk::Align::Center).build();
        countdown.add_css_class("numeric");
        countdown.add_css_class("title-4");
        row.add_suffix(&countdown);
        list.append(&row);
        group.add(&list);
        Self { group, row, icon, countdown, expires: Cell::new(None) }
    }

    pub fn widget(&self) -> &gtk::Widget {
        self.group.upcast_ref()
    }

    pub fn render(&self, view: &View) {
        let status = &view.status;
        set_icon(&self.icon, status.icon);
        set_style_class(&self.icon, &["success", "warning", "error"], status.class);
        set_title(&self.row, &status.title);
        let subtitle = match &status.detail {
            StatusDetail::ChooseDuration => tr("Choose how long to share, then start"),
            StatusDetail::None => String::new(),
            StatusDetail::Until { expires_unix, host } => tr_args("Until {time} · {server}", &[("time", &format_clock(*expires_unix)), ("server", host)]),
            StatusDetail::Message(text) => text.clone(),
        };
        set_subtitle(&self.row, &subtitle);
        self.expires.set(if view.phase == PhaseView::Active { view.expires_unix } else { None });
        self.tick();
    }

    /// Updates the countdown. Called once a second, and only while the window is in front.
    pub fn tick(&self) {
        let text = match self.expires.get() {
            Some(expires) => {
                let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64());
                format_countdown((expires - now).ceil() as i64)
            }
            None => String::new(),
        };
        set_label(&self.countdown, &text);
    }

    pub fn is_counting(&self) -> bool {
        self.expires.get().is_some()
    }
}

/// "Resume unfinished share?", shown at the top of the page when one was found on disk. The
/// original asks in a dialog at launch; the HIG says not to pop one up unexpectedly.
pub struct ResumeSection {
    group: adw::PreferencesGroup,
    row: adw::ActionRow,
}

impl ResumeSection {
    pub fn new(controller: &Controller) -> Self {
        let group = adw::PreferencesGroup::new();
        let list = boxed_list();
        let row = adw::ActionRow::builder().title(tr("Resume unfinished share?")).build();
        wrap_row(&row);
        let resume = gtk::Button::builder().label(tr("Resume")).valign(gtk::Align::Center).build();
        resume.add_css_class("suggested-action");
        let dismiss = icon_button("window-close-symbolic", &tr("Dismiss"));
        {
            let controller = controller.clone();
            resume.connect_clicked(move |_| controller.send(Event::Resume));
        }
        {
            let controller = controller.clone();
            dismiss.connect_clicked(move |_| controller.send(Event::DismissResume));
        }
        row.add_suffix(&resume);
        row.add_suffix(&dismiss);
        list.append(&row);
        group.add(&list);
        group.set_visible(false);
        Self { group, row }
    }

    pub fn widget(&self) -> &gtk::Widget {
        self.group.upcast_ref()
    }

    pub fn render(&self, view: &View) {
        match &view.resume_offer {
            Some(offer) => {
                let text = tr_args("{count} active · until {time}", &[("count", &offer.links.to_string()), ("time", &format_clock(offer.expires_unix))]);
                set_subtitle(&self.row, &text);
                set_visible(&self.group, true);
            }
            None => set_visible(&self.group, false),
        }
    }
}
