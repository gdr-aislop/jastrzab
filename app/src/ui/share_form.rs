//! The "New Share" form: how long (a number and a unit on one row, like Hauk's), and whether alone
//! or in a group.

use adw::prelude::*;
use jastrzab_core::i18n::tr;
use jastrzab_core::settings::{DurationUnit, Mode, Settings};
use jastrzab_core::view::View;

use super::dialogs;
use super::widgets::*;
use crate::controller::FormValues;

/// Four digits are enough for any duration the server accepts (31 days is 44 640 minutes, but
/// nobody types that: hours and days exist).
const MAX_DIGITS: usize = 4;

/// The number typed in the duration row; nothing usable counts as 0, which the engine rejects
/// with its *Invalid Duration* message.
fn parse_duration(text: &str) -> u32 {
    text.trim().parse().unwrap_or(0)
}

pub struct ShareForm {
    group: adw::PreferencesGroup,
    duration: adw::EntryRow,
    unit: gtk::DropDown,
    mode: adw::ComboRow,
    adopt_row: adw::ActionRow,
    adopt: gtk::Switch,
    nickname: adw::EntryRow,
    pin: adw::EntryRow,
}

impl ShareForm {
    pub fn new(settings: &Settings, parent: &adw::ApplicationWindow) -> Self {
        let group = adw::PreferencesGroup::builder().title(tr("New Share")).build();

        // An entry row with the unit as its suffix rather than a spin button beside a dropdown:
        // the number takes whatever width is left, so the row fits a 320 px screen at large text.
        let duration = adw::EntryRow::builder().title(tr("Duration")).input_purpose(gtk::InputPurpose::Digits).text(settings.duration.to_string()).build();
        duration.connect_changed(|row| {
            let text = row.text();
            let digits: String = text.chars().filter(char::is_ascii_digit).take(MAX_DIGITS).collect();
            if digits != text {
                row.set_text(&digits);
                row.set_position(-1);
            }
        });
        let unit = gtk::DropDown::from_strings(&[&tr("Minutes"), &tr("Hours"), &tr("Days")]);
        unit.set_selected(DurationUnit::ALL.iter().position(|u| *u == settings.unit).unwrap_or(0) as u32);
        unit.set_valign(gtk::Align::Center);
        set_accessible_label(&unit, &tr("Unit"));
        duration.add_suffix(&unit);
        group.add(&duration);

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

        // Development aid for the headless UI checks, which cannot operate a combo row: debug
        // builds only, like the scripted location source.
        #[cfg(debug_assertions)]
        if let Ok(name) = std::env::var("JASTRZAB_FORM_MODE")
            && let Some(index) = ["alone", "create-group", "join-group"].iter().position(|m| *m == name)
        {
            mode.set_selected(index as u32);
        }

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
            duration: parse_duration(&self.duration.text()),
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

#[cfg(test)]
mod tests {
    use super::parse_duration;

    #[test]
    fn the_typed_number_is_used() {
        assert_eq!(parse_duration("30"), 30);
        assert_eq!(parse_duration(" 7 "), 7);
        assert_eq!(parse_duration("9999"), 9999);
    }

    #[test]
    fn nothing_usable_is_zero_so_the_engine_asks_for_a_duration() {
        for text in ["", "  ", "abc", "-5", "1.5", "99999999999999999999"] {
            assert_eq!(parse_duration(text), 0, "{text:?}");
        }
    }
}
