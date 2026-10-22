//! The "New Share" form: how long, and whether alone or in a group.

use adw::prelude::*;
use jastrzab_core::i18n::tr;
use jastrzab_core::settings::{DurationUnit, Mode, Settings};
use jastrzab_core::view::View;

use super::dialogs;
use super::widgets::*;
use crate::controller::FormValues;

pub struct ShareForm {
    group: adw::PreferencesGroup,
    duration: gtk::SpinButton,
    unit: adw::ComboRow,
    mode: adw::ComboRow,
    adopt_row: adw::ActionRow,
    adopt: gtk::Switch,
    nickname: adw::EntryRow,
    pin: adw::EntryRow,
}

impl ShareForm {
    pub fn new(settings: &Settings, parent: &adw::ApplicationWindow) -> Self {
        let group = adw::PreferencesGroup::builder().title(tr("New Share")).build();

        let (duration_row, duration) = spin_row(&tr("Duration"), "", 1.0, 9999.0, f64::from(settings.duration));
        group.add(&duration_row);

        let unit = adw::ComboRow::builder()
            .title(tr("Unit"))
            .model(&gtk::StringList::new(&[&tr("Minutes"), &tr("Hours"), &tr("Days")]))
            .selected(DurationUnit::ALL.iter().position(|u| *u == settings.unit).unwrap_or(0) as u32)
            .build();
        group.add(&unit);

        let mode =
            adw::ComboRow::builder().title(tr("Sharing mode")).model(&gtk::StringList::new(&[&tr("Just Me"), &tr("Create Group"), &tr("Join Group")])).build();
        group.add(&mode);

        let (adopt_row, adopt) = switch_row(&tr("Allow Adoption"), &tr("Others may add you to their group share"), settings.adoptable);
        let help = icon_button("help-about-symbolic", &tr("What Is Adoption?"));
        {
            let parent = parent.clone();
            help.connect_clicked(move |_| dialogs::explain_adoption(&parent));
        }
        adopt_row.add_suffix(&help);
        group.add(&adopt_row);

        let nickname = adw::EntryRow::builder().title(tr("Nickname")).input_purpose(gtk::InputPurpose::Name).text(&settings.nickname).build();
        group.add(&nickname);
        // DIGITS, not PIN: GTK 4.8 hides the characters for PIN, and the group PIN is meant to be
        // read out and shared. squeekboard shows its number layout for DIGITS.
        let pin = adw::EntryRow::builder().title(tr("Group PIN")).input_purpose(gtk::InputPurpose::Digits).build();
        group.add(&pin);

        let form = Self { group, duration, unit, mode, adopt_row, adopt, nickname, pin };
        form.update_mode_rows();
        let rows = (form.adopt_row.clone(), form.nickname.clone(), form.pin.clone());
        let mode = form.mode.clone();
        mode.connect_selected_notify(move |mode| Self::apply_mode_rows(mode.selected(), &rows.0, &rows.1, &rows.2));
        form
    }

    pub fn widget(&self) -> &gtk::Widget {
        self.group.upcast_ref()
    }

    fn update_mode_rows(&self) {
        Self::apply_mode_rows(self.mode.selected(), &self.adopt_row, &self.nickname, &self.pin);
    }

    fn apply_mode_rows(selected: u32, adopt: &adw::ActionRow, nickname: &adw::EntryRow, pin: &adw::EntryRow) {
        let mode = Mode::ALL.get(selected as usize).copied().unwrap_or_default();
        set_visible(adopt, mode == Mode::Alone);
        set_visible(nickname, mode.is_group());
        set_visible(pin, mode == Mode::JoinGroup);
    }

    pub fn values(&self) -> FormValues {
        FormValues {
            duration: self.duration.value_as_int().max(1) as u32,
            unit: DurationUnit::ALL.get(self.unit.selected() as usize).copied().unwrap_or_default(),
            mode: Mode::ALL.get(self.mode.selected() as usize).copied().unwrap_or_default(),
            nickname: self.nickname.text().to_string(),
            group_pin: self.pin.text().trim().to_string(),
            adoptable: self.adopt.is_active(),
        }
    }

    pub fn render(&self, view: &View) {
        set_sensitive(&self.group, view.form_sensitive);
    }
}
