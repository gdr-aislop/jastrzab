//! Dialogs. `AdwMessageDialog` is the only dialog type in libadwaita 1.2. Response labels carry no
//! mnemonic underscores: libadwaita 1.2.2 gives such a button the accessible name "_Cancel Cancel",
//! which a screen reader would read out literally, and mnemonics mean nothing on a touch screen. Every dialog is
//! presented from an idle callback, after the caller's window is on screen: in a headless Phoc a
//! dialog presented in the same main-loop iteration as its parent was mapped but never shown.

use adw::prelude::*;
use jastrzab_core::engine::Event;
use jastrzab_core::i18n::tr;
use jastrzab_core::view::DialogSpec;

use super::widgets::*;
use crate::controller::Controller;

fn present_deferred(dialog: &adw::MessageDialog) {
    let dialog = dialog.clone();
    glib::idle_add_local_once(move || dialog.present());
}

/// An error or a decision the user has to read. `open_preferences` adds a button for it.
pub fn error(parent: &adw::ApplicationWindow, spec: &DialogSpec, controller: &Controller) {
    let dialog = adw::MessageDialog::new(Some(parent), Some(&spec.heading), Some(&spec.body));
    dialog.add_response("close", &tr("Close"));
    dialog.set_close_response("close");
    if spec.open_preferences {
        dialog.add_response("preferences", &tr("Preferences"));
        dialog.set_response_appearance("preferences", adw::ResponseAppearance::Suggested);
        dialog.set_default_response(Some("preferences"));
        let controller = controller.clone();
        dialog.connect_response(None, move |_, response| {
            if response == "preferences" {
                controller.open_preferences();
            }
        });
    } else {
        dialog.set_default_response(Some("close"));
    }
    present_deferred(&dialog);
}

pub fn explain_adoption(parent: &adw::ApplicationWindow) {
    let dialog = adw::MessageDialog::new(
        Some(parent),
        Some(&tr("Share Adoption")),
        Some(&tr(
            "When adoption is allowed, the host of a group share can add your existing share to their group by entering its link. You don't have to do anything: you simply show up on their map under the nickname they choose. This is handy when you are driving and can't type.\n\nAdoption does not work with end-to-end encrypted shares.",
        )),
    );
    dialog.add_response("close", &tr("Close"));
    dialog.set_close_response("close");
    present_deferred(&dialog);
}

/// "Stop Sharing?" with Cancel as the default, since stopping is the destructive choice.
pub fn confirm_stop(parent: &adw::ApplicationWindow, controller: &Controller) {
    let dialog = adw::MessageDialog::new(Some(parent), Some(&tr("Stop Sharing?")), Some(&tr("People following your link will stop seeing your location.")));
    dialog.add_response("cancel", &tr("Cancel"));
    dialog.add_response("stop", &tr("Stop Sharing"));
    dialog.set_response_appearance("stop", adw::ResponseAppearance::Destructive);
    dialog.set_default_response(Some("cancel"));
    dialog.set_close_response("cancel");
    let controller = controller.clone();
    dialog.connect_response(None, move |_, response| {
        if response == "stop" {
            controller.send(Event::Stop);
        }
    });
    present_deferred(&dialog);
}

/// Asked when the window is closed while sharing. `answer` gets `true` to keep sharing in the
/// background and `false` to stop; Cancel (Esc) leaves the window open.
pub fn keep_sharing(parent: &adw::ApplicationWindow, answer: impl Fn(Option<bool>) + 'static) {
    let dialog = adw::MessageDialog::new(
        Some(parent),
        Some(&tr("Keep Sharing?")),
        Some(&tr("Your location is still being shared. You can keep sharing in the background and stop from the notification.")),
    );
    dialog.add_response("cancel", &tr("Cancel"));
    dialog.add_response("stop", &tr("Stop Sharing"));
    dialog.add_response("keep", &tr("Keep Sharing"));
    dialog.set_response_appearance("stop", adw::ResponseAppearance::Destructive);
    dialog.set_response_appearance("keep", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("keep"));
    dialog.set_close_response("cancel");
    dialog.connect_response(None, move |_, response| match response {
        "keep" => answer(Some(true)),
        "stop" => answer(Some(false)),
        _ => answer(None),
    });
    present_deferred(&dialog);
}

/// Add someone's existing share to this group: their link (or its ID) and the nickname to show.
pub fn adopt_share(parent: &adw::ApplicationWindow, controller: &Controller) {
    let dialog = adw::MessageDialog::new(
        Some(parent),
        Some(&tr("Add Existing Share")),
        Some(&tr("Enter the link or ID of the share to add, and the nickname to show on the map.")),
    );
    let list = boxed_list();
    let target = adw::EntryRow::builder().title(tr("Share link or ID")).input_purpose(gtk::InputPurpose::Url).build();
    let nickname = adw::EntryRow::builder().title(tr("Nickname")).input_purpose(gtk::InputPurpose::Name).build();
    list.append(&target);
    list.append(&nickname);
    dialog.set_extra_child(Some(&list));
    dialog.add_response("cancel", &tr("Cancel"));
    dialog.add_response("add", &tr("Add"));
    dialog.set_response_appearance("add", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("add"));
    dialog.set_close_response("cancel");
    let controller = controller.clone();
    let parent = parent.clone();
    dialog.connect_response(None, move |_, response| {
        if response != "add" {
            return;
        }
        let nick = nickname.text().trim().to_string();
        match hauk::share_id::extract(&target.text()) {
            Some(id) if !nick.is_empty() => controller.send(Event::Adopt { target: id, nickname: nick }),
            Some(_) => error(&parent, &DialogSpec::error(tr("Nickname Required"), tr("Enter the nickname to show for this share.")), &controller),
            None => error(&parent, &DialogSpec::error(tr("Not a Share Link"), tr("Paste the link or ID of the share you want to add.")), &controller),
        }
    });
    present_deferred(&dialog);
}

pub fn about(parent: &adw::ApplicationWindow) {
    let about = adw::AboutWindow::builder()
        .transient_for(parent)
        .modal(true)
        .application_name(tr("Jastrzab"))
        .application_icon(jastrzab_core::paths::APP_ID)
        .version(env!("CARGO_PKG_VERSION"))
        .developer_name("gdr-aislop")
        .comments(tr("Share your location with a link, using your own Hauk server."))
        .website("https://github.com/gdr-aislop/jastrzab")
        .issue_url("https://github.com/gdr-aislop/jastrzab/issues")
        .license_type(gtk::License::Gpl30)
        .build();
    about.add_legal_section(&tr("Hauk"), Some("© Marius Lindvall"), gtk::License::Apache20, None);
    let about_clone = about.clone();
    glib::idle_add_local_once(move || about_clone.present());
}
