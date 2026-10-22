//! Small builders and "set only if changed" helpers shared by the views.
//!
//! The builders enforce the window-size rules of the design (§3) by construction, so a view cannot
//! forget them: text from outside the app can never widen the window, icon-only buttons always
//! have a name for assistive technology, and touch targets are 44 px.
//!
//! The setters do nothing when the value is already shown. Views are re-rendered after every
//! engine event, and writing an unchanged label would make a screen reader read it again.

use adw::prelude::*;
use glib::translate::{IntoGlib, ToGlibPtr};
use gtk::pango;

/// Touch targets are at least 44 px (design §5).
pub const TOUCH_TARGET: i32 = 44;

/// A label for text the app does not control (link IDs, nicknames, server messages, URLs). It
/// wraps anywhere and reports a one-character natural width, so no amount of text can make the
/// window wider than the phone.
pub fn external_text_label() -> gtk::Label {
    let label = gtk::Label::builder().xalign(0.0).wrap(true).wrap_mode(pango::WrapMode::WordChar).max_width_chars(1).selectable(false).build();
    label.set_hexpand(true);
    label
}

/// An icon-only button. The tooltip is only the accessible *description* in GTK 4.8, so the name
/// is set explicitly too.
pub fn icon_button(icon: &str, label: &str) -> gtk::Button {
    let button = gtk::Button::builder().icon_name(icon).valign(gtk::Align::Center).tooltip_text(label).build();
    button.add_css_class("flat");
    button.set_width_request(TOUCH_TARGET);
    button.set_height_request(TOUCH_TARGET);
    set_accessible_label(&button, label);
    button
}

/// An `AdwActionRow` with a spin button as its suffix and activatable widget (libadwaita 1.2 has
/// no `AdwSpinRow`). The row labels the spin button for assistive technology by itself.
pub fn spin_row(title: &str, subtitle: &str, lower: f64, upper: f64, value: f64) -> (adw::ActionRow, gtk::SpinButton) {
    let adjustment = gtk::Adjustment::new(value, lower, upper, 1.0, 10.0, 0.0);
    let spin = gtk::SpinButton::builder().adjustment(&adjustment).numeric(true).valign(gtk::Align::Center).build();
    let row = adw::ActionRow::builder().title(title).subtitle(subtitle).activatable_widget(&spin).build();
    row.add_suffix(&spin);
    (row, spin)
}

/// An `AdwActionRow` with a switch (libadwaita 1.2 has no `AdwSwitchRow`).
pub fn switch_row(title: &str, subtitle: &str, active: bool) -> (adw::ActionRow, gtk::Switch) {
    let switch = gtk::Switch::builder().active(active).valign(gtk::Align::Center).build();
    let row = adw::ActionRow::builder().title(title).subtitle(subtitle).activatable_widget(&switch).build();
    row.set_subtitle_lines(0);
    row.add_suffix(&switch);
    (row, switch)
}

pub fn boxed_list() -> gtk::ListBox {
    let list = gtk::ListBox::builder().selection_mode(gtk::SelectionMode::None).build();
    list.add_css_class("boxed-list");
    list
}

/// The 2-line wrapped subtitle every row of ours uses.
pub fn wrap_row(row: &adw::ActionRow) {
    row.set_subtitle_lines(0);
    row.set_title_lines(0);
    // Titles and subtitles can carry text from the server or other users: never parse them as markup.
    row.set_use_markup(false);
}

// --- accessibility --------------------------------------------------------------------------
//
// gtk4-rs only binds GtkAccessible's update functions from GTK 4.10, so on 4.8 the two C
// functions are declared here. They exist in GTK 4.0 and later; libgtk-4 is already linked.

unsafe extern "C" {
    fn gtk_accessible_update_property_value(
        accessible: *mut std::ffi::c_void,
        n_properties: std::ffi::c_int,
        properties: *mut std::ffi::c_int,
        values: *const glib::gobject_ffi::GValue,
    );
    fn gtk_accessible_update_state_value(
        accessible: *mut std::ffi::c_void,
        n_states: std::ffi::c_int,
        states: *mut std::ffi::c_int,
        values: *const glib::gobject_ffi::GValue,
    );
}

fn update_property(widget: &impl IsA<gtk::Widget>, property: gtk::AccessibleProperty, value: glib::Value) {
    let mut property = property.into_glib();
    // SAFETY: `widget` is a live widget (every GtkWidget is a GtkAccessible), and one property
    // is paired with one value of the type GTK documents for it (a string).
    unsafe { gtk_accessible_update_property_value(widget.as_ptr().cast(), 1, &mut property, value.to_glib_none().0) }
}

/// The accessible *name*. In GTK 4.8 a tooltip becomes only the accessible *description*, so an
/// icon-only button with a tooltip would otherwise have no name.
pub fn set_accessible_label(widget: &impl IsA<gtk::Widget>, text: &str) {
    update_property(widget, gtk::AccessibleProperty::Label, text.to_value());
}

pub fn set_accessible_description(widget: &impl IsA<gtk::Widget>, text: &str) {
    update_property(widget, gtk::AccessibleProperty::Description, text.to_value());
}

/// Marks a purely decorative widget as invisible to assistive technology.
pub fn hide_from_accessibility(widget: &impl IsA<gtk::Widget>) {
    let mut state = gtk::AccessibleState::Hidden.into_glib();
    let value = true.to_value();
    // SAFETY: as above, with one state and a boolean value.
    unsafe { gtk_accessible_update_state_value(widget.as_ptr().cast(), 1, &mut state, value.to_glib_none().0) }
}

// --- set-if-changed -------------------------------------------------------------------------

pub fn set_label(label: &gtk::Label, text: &str) {
    if label.label() != text {
        label.set_label(text);
    }
}

pub fn set_title(row: &impl IsA<adw::PreferencesRow>, text: &str) {
    if row.title() != text {
        row.set_title(text);
    }
}

pub fn set_subtitle(row: &adw::ActionRow, text: &str) {
    if row.subtitle().as_deref().unwrap_or("") != text {
        row.set_subtitle(text);
    }
}

pub fn set_expander_subtitle(row: &adw::ExpanderRow, text: &str) {
    if row.subtitle() != text {
        row.set_subtitle(text);
    }
}

pub fn set_icon(image: &gtk::Image, name: &str) {
    if image.icon_name().as_deref() != Some(name) {
        image.set_icon_name(Some(name));
    }
}

pub fn set_visible(widget: &impl IsA<gtk::Widget>, visible: bool) {
    if widget.is_visible() != visible {
        widget.set_visible(visible);
    }
}

pub fn set_sensitive(widget: &impl IsA<gtk::Widget>, sensitive: bool) {
    if widget.is_sensitive() != sensitive {
        widget.set_sensitive(sensitive);
    }
}

/// Makes exactly one of `classes` (or none, when `active` is empty) present on the widget.
pub fn set_style_class(widget: &impl IsA<gtk::Widget>, classes: &[&str], active: &str) {
    for class in classes {
        let has = widget.has_css_class(class);
        if *class == active && !has {
            widget.add_css_class(class);
        } else if *class != active && has {
            widget.remove_css_class(class);
        }
    }
}

/// `H:MM:SS`, or `M:SS` under an hour.
pub fn format_countdown(remaining_secs: i64) -> String {
    let left = remaining_secs.max(0);
    let (h, m, s) = (left / 3600, left % 3600 / 60, left % 60);
    if h > 0 { format!("{h}:{m:02}:{s:02}") } else { format!("{m}:{s:02}") }
}

/// The local wall-clock time, `17:23`.
pub fn format_clock(unix_secs: f64) -> String {
    glib::DateTime::from_unix_local(unix_secs as i64).ok().and_then(|time| time.format("%H:%M").ok()).map_or_else(String::new, |text| text.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn countdown_formats() {
        assert_eq!(format_countdown(29 * 60 + 51), "29:51");
        assert_eq!(format_countdown(3600 + 5), "1:00:05");
        assert_eq!(format_countdown(-3), "0:00");
        assert_eq!(format_countdown(86_399), "23:59:59");
    }
}
