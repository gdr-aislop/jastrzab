//! The "Active Links" list: one row per link the server is following, each with Copy and Stop.

use std::cell::RefCell;

use adw::prelude::*;
use jastrzab_core::engine::Event;
use jastrzab_core::i18n::{tr, tr_args};
use jastrzab_core::view::{LinkView, View};

use super::widgets::*;
use crate::controller::Controller;

pub struct LinksSection {
    group: adw::PreferencesGroup,
    list: gtk::ListBox,
    new_link: gtk::Button,
    shown: RefCell<Vec<LinkView>>,
    controller: Controller,
}

impl LinksSection {
    pub fn new(controller: &Controller, adoptable: impl Fn() -> bool + 'static) -> Self {
        let new_link = icon_button("list-add-symbolic", &tr("New Link"));
        new_link.set_sensitive(false);
        {
            let controller = controller.clone();
            new_link.connect_clicked(move |_| controller.send(Event::NewLink { adoptable: adoptable() }));
        }
        let group = adw::PreferencesGroup::builder().title(tr("Active Links")).header_suffix(&new_link).build();
        let list = boxed_list();
        let empty = gtk::Label::builder().label(tr("No active links")).margin_top(12).margin_bottom(12).wrap(true).build();
        empty.add_css_class("dim-label");
        list.set_placeholder(Some(&empty));
        group.add(&list);
        Self { group, list, new_link, shown: RefCell::new(Vec::new()), controller: controller.clone() }
    }

    pub fn widget(&self) -> &gtk::Widget {
        self.group.upcast_ref()
    }

    pub fn render(&self, view: &View) {
        set_sensitive(&self.new_link, view.can_add_link);
        if *self.shown.borrow() == view.links {
            return;
        }
        *self.shown.borrow_mut() = view.links.clone();
        while let Some(child) = self.list.first_child() {
            self.list.remove(&child);
        }
        for link in &view.links {
            self.list.append(&self.row(link));
        }
    }

    fn row(&self, link: &LinkView) -> adw::ActionRow {
        let row = adw::ActionRow::builder().title(&link.id).subtitle(&link.subtitle).build();
        wrap_row(&row);
        let copy = icon_button("edit-copy-symbolic", &tr_args("Copy Link {id}", &[("id", &link.id)]));
        let stop = icon_button("process-stop-symbolic", &tr_args("Stop Link {id}", &[("id", &link.id)]));
        {
            let (controller, id) = (self.controller.clone(), link.id.clone());
            copy.connect_clicked(move |_| controller.send(Event::CopyLink(Some(id.clone()))));
        }
        {
            let (controller, id) = (self.controller.clone(), link.id.clone());
            stop.connect_clicked(move |_| controller.send(Event::StopLink(id.clone())));
        }
        row.add_suffix(&copy);
        row.add_suffix(&stop);
        row
    }
}
