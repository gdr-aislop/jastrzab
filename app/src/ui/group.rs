//! The group PIN panel, shown to the host of a group share, and the dialog that adds someone's
//! existing share to the group ("adoption").

use adw::prelude::*;
use jastrzab_core::engine::Event;
use jastrzab_core::i18n::tr;
use jastrzab_core::view::View;

use super::dialogs;
use super::widgets::*;
use crate::controller::Controller;

pub struct GroupSection {
    group: adw::PreferencesGroup,
    pin: gtk::Label,
}

impl GroupSection {
    pub fn new(controller: &Controller, parent: &adw::ApplicationWindow) -> Self {
        let group = adw::PreferencesGroup::builder().title(tr("Group PIN")).description(tr("Others can join your group share with this PIN.")).build();

        let pin_box = gtk::Box::builder().spacing(12).build();
        // The PIN comes from the server, so it is capped like any other outside text.
        let pin = external_text_label();
        pin.set_xalign(0.5);
        pin.set_justify(gtk::Justification::Center);
        pin.set_selectable(true);
        pin.add_css_class("title-1");
        pin.add_css_class("numeric");
        let copy = icon_button("edit-copy-symbolic", &tr("Copy PIN"));
        {
            let controller = controller.clone();
            copy.connect_clicked(move |_| controller.send(Event::CopyPin));
        }
        pin_box.append(&pin);
        pin_box.append(&copy);
        group.add(&pin_box);

        let list = boxed_list();
        list.set_margin_top(12);
        let add = adw::ActionRow::builder().title(tr("Add Existing Share…")).activatable(true).build();
        add.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
        {
            let (controller, parent) = (controller.clone(), parent.clone());
            add.connect_activated(move |_| dialogs::adopt_share(&parent, &controller));
        }
        list.append(&add);
        group.add(&list);
        group.set_visible(false);
        Self { group, pin }
    }

    pub fn widget(&self) -> &gtk::Widget {
        self.group.upcast_ref()
    }

    pub fn render(&self, view: &View) {
        match &view.group_pin {
            Some(pin) => {
                set_label(&self.pin, pin);
                set_visible(&self.group, true);
            }
            None => set_visible(&self.group, false),
        }
    }
}
