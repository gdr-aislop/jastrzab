//! Preferences. Changes apply immediately and there is no Save button: spin rows, combos and
//! switches apply when changed; text entries apply on Return or their apply button, so a
//! half-typed server address never reaches the network layer or the log scrubber.

use adw::prelude::*;
use jastrzab_core::AppPaths;
use jastrzab_core::i18n::{tr, tr_args};
use jastrzab_core::settings::{ProxyKind, TlsPolicy};

use super::widgets::*;
use crate::controller::Controller;
use crate::platform::secrets::Kind;

pub fn show(parent: &adw::ApplicationWindow, controller: &Controller) {
    let settings = controller.settings();
    let window = adw::PreferencesWindow::builder().transient_for(parent).modal(true).default_width(360).default_height(648).build();
    let page = adw::PreferencesPage::new();

    // --- Server ----------------------------------------------------------------------------
    let server = adw::PreferencesGroup::builder().title(tr("Server")).build();
    let address =
        adw::EntryRow::builder().title(tr("Server address")).input_purpose(gtk::InputPurpose::Url).text(&settings.server_url).show_apply_button(true).build();
    {
        let controller = controller.clone();
        address.connect_apply(move |row| {
            let text = row.text().trim().to_string();
            controller.update_settings(|s| s.server_url = text);
        });
    }
    server.add(&address);
    let username = adw::EntryRow::builder().title(tr("Username")).text(&settings.username).show_apply_button(true).build();
    {
        let controller = controller.clone();
        username.connect_apply(move |row| {
            let text = row.text().trim().to_string();
            controller.update_settings(|s| s.username = text);
        });
    }
    server.add(&username);
    let password = adw::PasswordEntryRow::builder().title(tr("Password")).show_apply_button(true).build();
    controller.load_secret(Kind::ServerPassword, {
        let password = password.clone();
        move |secret| password.set_text(&secret)
    });
    {
        let controller = controller.clone();
        password.connect_apply(move |row| controller.store_secret(Kind::ServerPassword, row.text().to_string()));
    }
    server.add(&password);
    let link = adw::EntryRow::builder().title(tr("Preferred link ID")).text(&settings.preferred_link_id).show_apply_button(true).build();
    {
        let controller = controller.clone();
        link.connect_apply(move |row| {
            let text = row.text().trim().to_string();
            controller.update_settings(|s| s.preferred_link_id = text);
        });
    }
    server.add(&link);
    page.add(&server);

    // --- Location updates ------------------------------------------------------------------
    let updates = adw::PreferencesGroup::builder().title(tr("Location Updates")).build();
    let (interval_row, interval) = spin_row(
        &tr("Update interval"),
        &tr("Seconds between positions sent to the server. Longer saves battery."),
        1.0,
        3600.0,
        f64::from(settings.interval_secs),
    );
    wrap_row(&interval_row);
    {
        let controller = controller.clone();
        interval.connect_value_changed(move |spin| {
            let secs = spin.value_as_int().max(1) as u32;
            controller.update_settings(|s| s.interval_secs = secs);
        });
    }
    updates.add(&interval_row);
    let (distance_row, distance) =
        spin_row(&tr("Minimum distance"), &tr("Metres to move before sending a new position. 0 sends every update."), 0.0, 100_000.0, settings.min_distance_m);
    wrap_row(&distance_row);
    {
        let controller = controller.clone();
        distance.connect_value_changed(move |spin| {
            let metres = spin.value();
            controller.update_settings(|s| s.min_distance_m = metres);
        });
    }
    updates.add(&distance_row);
    page.add(&updates);

    // --- Privacy ---------------------------------------------------------------------------
    let privacy = adw::PreferencesGroup::builder().title(tr("Privacy")).build();
    let encryption = adw::ExpanderRow::builder()
        .title(tr("End-to-End Encryption"))
        .subtitle(tr("The server only stores scrambled positions. Viewers enter the password. Not available for group shares."))
        .show_enable_switch(true)
        .enable_expansion(settings.e2e_enabled)
        .build();
    {
        let controller = controller.clone();
        encryption.connect_enable_expansion_notify(move |row| {
            let on = row.enables_expansion();
            controller.update_settings(|s| s.e2e_enabled = on);
        });
    }
    let enc_password = adw::PasswordEntryRow::builder().title(tr("Encryption password")).show_apply_button(true).build();
    controller.load_secret(Kind::EncryptionPassword, {
        let enc_password = enc_password.clone();
        move |secret| enc_password.set_text(&secret)
    });
    {
        let controller = controller.clone();
        enc_password.connect_apply(move |row| controller.store_secret(Kind::EncryptionPassword, row.text().to_string()));
    }
    encryption.add_row(&enc_password);
    privacy.add(&encryption);
    page.add(&privacy);

    // --- Network ---------------------------------------------------------------------------
    let network = adw::PreferencesGroup::builder().title(tr("Network")).build();
    let proxy = adw::ComboRow::builder()
        .title(tr("Proxy"))
        .model(&gtk::StringList::new(&[&tr("System Default"), &tr("None"), &tr("HTTP"), &tr("SOCKS")]))
        .selected(match settings.proxy {
            ProxyKind::System => 0,
            ProxyKind::None => 1,
            ProxyKind::Http => 2,
            ProxyKind::Socks => 3,
        })
        .build();
    let proxy_host = adw::EntryRow::builder().title(tr("Proxy host")).text(&settings.proxy_host).show_apply_button(true).build();
    let (proxy_port_row, proxy_port) = spin_row(&tr("Proxy port"), "", 1.0, 65535.0, f64::from(settings.proxy_port));
    let proxy_rows_sensitive = {
        let (host, port) = (proxy_host.clone(), proxy_port_row.clone());
        move |kind: ProxyKind| {
            let manual = matches!(kind, ProxyKind::Http | ProxyKind::Socks);
            set_sensitive(&host, manual);
            set_sensitive(&port, manual);
        }
    };
    proxy_rows_sensitive(settings.proxy);
    {
        let controller = controller.clone();
        proxy.connect_selected_notify(move |row| {
            let kind = match row.selected() {
                1 => ProxyKind::None,
                2 => ProxyKind::Http,
                3 => ProxyKind::Socks,
                _ => ProxyKind::System,
            };
            proxy_rows_sensitive(kind);
            controller.update_settings(|s| s.proxy = kind);
        });
    }
    {
        let controller = controller.clone();
        proxy_host.connect_apply(move |row| {
            let text = row.text().trim().to_string();
            controller.update_settings(|s| s.proxy_host = text);
        });
    }
    {
        let controller = controller.clone();
        proxy_port.connect_value_changed(move |spin| {
            let port = spin.value_as_int().clamp(1, 65535) as u16;
            controller.update_settings(|s| s.proxy_port = port);
        });
    }
    network.add(&proxy);
    network.add(&proxy_host);
    network.add(&proxy_port_row);

    let (timeout_row, timeout) = spin_row(&tr("Connection timeout"), &tr("Seconds to wait for the server."), 1.0, 300.0, f64::from(settings.timeout_secs));
    {
        let controller = controller.clone();
        timeout.connect_value_changed(move |spin| {
            let secs = spin.value_as_int().max(1) as u32;
            controller.update_settings(|s| s.timeout_secs = secs);
        });
    }
    network.add(&timeout_row);

    let tls = adw::ComboRow::builder()
        .title(tr("HTTPS certificate validation"))
        .model(&gtk::StringList::new(&[&tr("Always validate"), &tr("Accept unknown issuer for .onion"), &tr("Accept any certificate for .onion")]))
        .selected(match settings.tls {
            TlsPolicy::Always => 0,
            TlsPolicy::OnionAnyChain => 1,
            TlsPolicy::OnionAnyCertificate => 2,
        })
        .build();
    {
        let controller = controller.clone();
        tls.connect_selected_notify(move |row| {
            let policy = match row.selected() {
                1 => TlsPolicy::OnionAnyChain,
                2 => TlsPolicy::OnionAnyCertificate,
                _ => TlsPolicy::Always,
            };
            controller.update_settings(|s| s.tls = policy);
        });
    }
    network.add(&tls);
    page.add(&network);

    // --- Behavior --------------------------------------------------------------------------
    let behavior = adw::PreferencesGroup::builder().title(tr("Behavior")).build();
    let (confirm_row, confirm) = switch_row(&tr("Confirm Before Stopping"), "", settings.confirm_stop);
    {
        let controller = controller.clone();
        confirm.connect_active_notify(move |switch| {
            let on = switch.is_active();
            controller.update_settings(|s| s.confirm_stop = on);
        });
    }
    behavior.add(&confirm_row);
    let (offline_row, offline) = switch_row(
        &tr("Pause When Offline"),
        &tr("Stop sending and switch the GPS off while the system reports no network. Turn this off if your server is on a network without internet access."),
        settings.pause_when_offline,
    );
    {
        let controller = controller.clone();
        offline.connect_active_notify(move |switch| {
            let on = switch.is_active();
            controller.update_settings(|s| s.pause_when_offline = on);
        });
    }
    behavior.add(&offline_row);
    page.add(&behavior);

    // --- Diagnostics -----------------------------------------------------------------------
    let diagnostics = adw::PreferencesGroup::builder().title(tr("Diagnostics")).build();
    let (anonymize_row, anonymize) =
        switch_row(&tr("Anonymize Logs"), &tr("Hide the server address, user name and share IDs in log files."), settings.anonymize_logs);
    {
        let controller = controller.clone();
        anonymize.connect_active_notify(move |switch| {
            let on = switch.is_active();
            controller.update_settings(|s| s.anonymize_logs = on);
        });
    }
    diagnostics.add(&anonymize_row);
    let logs = adw::ActionRow::builder().title(tr("Open Log Folder")).activatable(true).build();
    logs.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
    {
        let window = window.clone();
        logs.connect_activated(move |_| open_log_folder(&window));
    }
    diagnostics.add(&logs);
    page.add(&diagnostics);

    window.add(&page);
    let window_clone = window.clone();
    glib::idle_add_local_once(move || window_clone.present());
}

fn open_log_folder(window: &adw::PreferencesWindow) {
    let Some(paths) = AppPaths::resolve() else { return };
    // Logging buffers for up to a minute; what the user is about to open must be on disk.
    crate::crash_reporting::flush_logs();
    let dir = paths.logs_dir();
    let uri = gio::File::for_path(&dir).uri();
    if let Err(err) = gio::AppInfo::launch_default_for_uri(&uri, Some(&crate::host_env::launch_context())) {
        tracing::warn!(%err, "couldn't open the log folder");
        window.add_toast(adw::Toast::new(&tr_args("Logs are in {path}", &[("path", &dir.display().to_string())])));
    }
}
