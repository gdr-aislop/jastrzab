#!/usr/bin/env python3
"""UI spike for a Phosh/libadwaita Jastrzab client (a Hauk-compatible location sharer).

Throwaway research prototype: no networking, no location. It only uses API
available in GTK 4.8 and libadwaita 1.2 (PureOS Crimson) and exists to
validate layout, sizing and accessibility on a 360 px wide phone screen.

    python3 jastrzab_spike.py [idle|connecting|active|group|join|prefs|confirm|adopt]
"""
import os
import sys
import time

import gi

gi.require_version("Gtk", "4.0")
gi.require_version("Adw", "1")
from gi.repository import Adw, Gio, GLib, Gtk, Pango  # noqa: E402

STATE = sys.argv[1] if len(sys.argv) > 1 else "idle"
APP_ID = "org.example.Jastrzab"

MODES = ["Just Me", "Create Group", "Join Group"]
UNITS = ["Minutes", "Hours", "Days"]

# status id -> (icon, style class, title)
STATUS = {
    "inactive": ("location-services-disabled-symbolic", None, "Not sharing"),
    "connecting": ("network-transmit-receive-symbolic", None, "Connecting…"),
    "waiting": ("find-location-symbolic", "warning", "Waiting for location fix"),
    "coarse": ("find-location-symbolic", "warning", "Approximate location only"),
    "active": ("location-services-active-symbolic", "success", "Sharing location"),
    "offline": ("network-offline-symbolic", "error", "Server unreachable"),
}


# --- GPS signal model ---------------------------------------------------
# GeoClue only reports: accuracy (metres), a free-text Description
# ("GPS GGA"/"GPS RMC" for NMEA fixes) and a Timestamp. It exposes no fix type
# and no satellite count, and the gnss-share socket that carries raw NMEA is
# 0660 root:geoclue. The indicator is therefore built on accuracy, source hint
# and age only. Thresholds below are a proposal to tune on a device.
SIGNAL_CSS = b"""
levelbar.signal > trough > block { min-width: 14px; margin-right: 3px; }
levelbar.signal.q-poor > trough > block:not(.empty) { background-color: @error_color; }
levelbar.signal.q-fair > trough > block:not(.empty) { background-color: @warning_color; }
levelbar.signal.q-good > trough > block:not(.empty),
levelbar.signal.q-excellent > trough > block:not(.empty) { background-color: @success_color; }
"""

# level -> (bars, title, consequence for viewers, hint)
SIGNALS = {
    "off": (0, "Not checking", "", "Check the signal before you start, so viewers see you in the right place."),
    "searching": (0, "Searching for GPS…", "Nobody can see you yet.",
                  "Go outside or stand near a window. Roofs, walls and vehicles block GPS. Keep this screen open."),
    "poor": (1, "Poor", "Viewers will only see your rough area.",
             "Move to open sky and wait a moment for a better fix."),
    "fair": (2, "Fair", "Viewers may see you on the wrong street.", "A clearer view of the sky will improve this."),
    "good": (3, "Good", "Viewers will see you at the right building.", ""),
    "excellent": (4, "Excellent", "Viewers will see you in the right spot.", ""),
    "lost": (0, "Signal lost", "Viewers see your last known spot.",
             "Move to open sky. Sharing continues and resumes when a signal returns."),
}


def level_for(accuracy_m):
    """Map a GeoClue accuracy (metres) to a level key.

    GeoClue turns a GPS fix's HDOP into 0, 1, 3, 50, 100 or 300 m (gclue-location.c,
    get_accuracy_from_hdop, marked FIXME "rough estimates"), so the cut-offs sit on
    those buckets: HDOP <= 2 excellent, <= 5 good, <= 20 fair, above that poor.
    -1 means unknown and is treated as poor.
    """
    if accuracy_m < 0:
        return "poor"
    if accuracy_m <= 1:
        return "excellent"
    if accuracy_m <= 5:
        return "good"
    if accuracy_m <= 100:
        return "fair"
    return "poor"


def accuracy_text(m):
    # Below 10 m GeoClue's number is a coarse HDOP bucket, not a measurement,
    # so no figure is shown.
    if m < 0:
        return "Unknown"
    if m < 10:
        return "Precise"
    return f"About {int(round(m / 5.0) * 5)} m"



def label_icon_button(icon, tooltip):
    """Icon-only button with a tooltip and an explicit accessible label."""
    btn = Gtk.Button(icon_name=icon, valign=Gtk.Align.CENTER, tooltip_text=tooltip)
    btn.add_css_class("flat")
    btn.update_property([Gtk.AccessibleProperty.LABEL], [tooltip])
    return btn


def spin_row(title, subtitle, lower, upper, value, digits=0):
    adj = Gtk.Adjustment(lower=lower, upper=upper, step_increment=1,
                         page_increment=10, value=value)
    spin = Gtk.SpinButton(adjustment=adj, digits=digits, numeric=True,
                          valign=Gtk.Align.CENTER)
    row = Adw.ActionRow(title=title, subtitle=subtitle or "", activatable_widget=spin)
    row.add_suffix(spin)
    # AdwActionRow labels its activatable widget (LABELLED_BY / DESCRIBED_BY)
    # by itself, so no manual accessible relation is needed here.
    return row, spin


class MainWindow(Adw.ApplicationWindow):
    def __init__(self, app):
        super().__init__(application=app, title="Jastrzab")
        # Phone-sized default; Phosh maximizes the window anyway. Never call
        # set_resizable(False): GTK then sizes the window to its content.
        self.set_default_size(360, 720)

        self.toasts = Adw.ToastOverlay()
        root = Gtk.Box(orientation=Gtk.Orientation.VERTICAL)
        root.append(self._header())
        root.append(self.toasts)
        self.set_content(root)

        # All page content lives in a vertical-only scroller so that no
        # amount of content can raise the window's minimum size.
        scroller = Gtk.ScrolledWindow(hscrollbar_policy=Gtk.PolicyType.NEVER,
                                      vexpand=True)
        self.toasts.set_child(scroller)
        clamp = Adw.Clamp(maximum_size=480, tightening_threshold=360)
        scroller.set_child(clamp)
        page = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=24,
                       margin_top=18, margin_bottom=18, margin_start=12, margin_end=12)
        clamp.set_child(page)

        page.append(self._status_group())
        page.append(self._signal_group())
        page.append(self._share_group())
        page.append(self._primary_button())
        page.append(self._pin_group())
        page.append(self._links_group())

        self._deadline = None
        self._on_mode_changed()
        self.set_status("inactive")
        self.set_signal("off")

    # -- construction -----------------------------------------------------

    def _header(self):
        hb = Adw.HeaderBar()
        menu = Gio.Menu()
        menu.append("_Preferences", "app.preferences")
        menu.append("_About Jastrzab", "app.about")
        btn = Gtk.MenuButton(icon_name="open-menu-symbolic", menu_model=menu,
                             primary=True, tooltip_text="Main Menu")
        btn.update_property([Gtk.AccessibleProperty.LABEL], ["Main Menu"])
        hb.pack_end(btn)
        return hb

    def _status_group(self):
        grp = Adw.PreferencesGroup()
        lb = Gtk.ListBox(selection_mode=Gtk.SelectionMode.NONE)
        lb.add_css_class("boxed-list")
        self.status_row = Adw.ActionRow(title="", subtitle="")
        self.status_icon = Gtk.Image(pixel_size=32)
        self.status_row.add_prefix(self.status_icon)
        # The countdown is a separate numeric label (tabular figures), not
        # part of the button text, so the button's accessible name is stable.
        self.countdown = Gtk.Label(valign=Gtk.Align.CENTER)
        self.countdown.add_css_class("numeric")
        self.countdown.add_css_class("title-4")
        self.status_row.add_suffix(self.countdown)
        lb.append(self.status_row)
        grp.add(lb)
        return grp

    def _signal_group(self):
        css = Gtk.CssProvider()
        css.load_from_data(SIGNAL_CSS)
        Gtk.StyleContext.add_provider_for_display(
            self.get_display(), css, Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION)

        grp = Adw.PreferencesGroup()
        self.signal_row = Adw.ExpanderRow(title="GPS signal")
        self.signal_icon = Gtk.Image(icon_name="find-location-symbolic")
        self.signal_row.add_prefix(self.signal_icon)

        self.signal_bar = Gtk.LevelBar(mode=Gtk.LevelBarMode.DISCRETE,
                                       min_value=0, max_value=4, valign=Gtk.Align.CENTER)
        self.signal_bar.add_css_class("signal")
        # Decorative: the row text carries the meaning for assistive tech.
        self.signal_bar.update_state([Gtk.AccessibleState.HIDDEN], [True])
        self.signal_row.add_action(self.signal_bar)  # 1.2 name for "suffix"

        self.check_btn = Gtk.Button(label="Check", valign=Gtk.Align.CENTER)
        self.check_btn.connect("clicked", self._on_check_clicked)
        self.signal_row.add_action(self.check_btn)

        self.sig_acc = Adw.ActionRow(title="Accuracy")
        self.sig_src = Adw.ActionRow(title="Source")
        self.sig_age = Adw.ActionRow(title="Last update")
        self.sig_view = Adw.ActionRow(title="What viewers see")
        self.sig_hint = Adw.ActionRow(title="What to try")
        for row in (self.sig_acc, self.sig_src, self.sig_age, self.sig_view, self.sig_hint):
            row.set_subtitle_lines(0)      # wrap; subtitle is never ellipsized
            row.set_title_lines(0)
            self.signal_row.add_row(row)
        grp.add(self.signal_row)
        return grp

    def _on_check_clicked(self, *_):
        # Idle-only control: start or stop the pre-share signal check.
        self.set_signal("off" if self.signal_bar.get_visible() else "searching")

    def set_signal(self, key, accuracy=None, source="gps", age=0):
        # While sharing, the monitor is tied to the session and never "off":
        # a started share with no fix yet is "searching".
        if self._deadline is not None and key == "off":
            key = "searching"
        bars, title, consequence, hint = SIGNALS[key]
        measured = key in ("poor", "fair", "good", "excellent")
        self.signal_bar.set_value(bars)
        for k in ("poor", "fair", "good", "excellent"):
            self.signal_bar.remove_css_class("q-" + k)
        self.signal_bar.add_css_class("q-" + key)
        self.signal_bar.set_visible(key != "off")
        sharing = self._deadline is not None
        # Idle: "Check" starts monitoring, "Stop" ends it. While sharing the
        # monitor is tied to the session, so there is no button.
        self.check_btn.set_visible(not sharing)
        self.check_btn.set_label("Check" if key == "off" else "Stop")
        for cls in ("success", "warning", "error"):
            self.signal_icon.remove_css_class(cls)
        self.signal_icon.add_css_class({"excellent": "success", "good": "success",
                                        "fair": "warning", "poor": "warning",
                                        "searching": "warning", "lost": "error"}.get(key, "dim-label"))
        self.signal_icon.set_from_icon_name(
            "location-services-active-symbolic" if measured else
            "location-services-disabled-symbolic" if key == "lost" else "find-location-symbolic")

        sub = title
        if measured and source != "gps":
            sub = f"No GPS fix · network location, {accuracy_text(accuracy).lower()}"
        elif measured:
            if accuracy >= 10:
                sub += f" · {accuracy_text(accuracy).lower()}"
        if key == "lost":
            sub += f" · last seen {age} s ago"
        self.signal_row.set_subtitle(sub)

        self.sig_acc.set_visible(measured)
        self.sig_src.set_visible(measured)
        self.sig_age.set_visible(measured or key == "lost")
        self.sig_view.set_visible(bool(consequence))
        self.sig_hint.set_visible(bool(hint))
        if measured:
            self.sig_acc.set_subtitle(accuracy_text(accuracy) if accuracy < 10
                                      else accuracy_text(accuracy) + " (estimate)")
            self.sig_src.set_subtitle("Satellites (GPS)" if source == "gps"
                                      else "Wi-Fi or mobile network (approximate)")
            self.sig_age.set_subtitle("Just now")
        elif key == "lost":
            self.sig_age.set_subtitle(f"{age} seconds ago")
        # Before sharing starts this is a preview, not a fact.
        self.sig_view.set_title("What viewers see" if sharing else "If you share now")
        self.sig_view.set_subtitle(consequence)
        self.sig_hint.set_subtitle(hint)

        # One short, stable accessible description; it changes with the level,
        # not every second.
        text = f"GPS signal: {sub}"
        self.signal_row.update_property([Gtk.AccessibleProperty.DESCRIPTION], [text])

    def _share_group(self):
        grp = Adw.PreferencesGroup(title="New Share")
        self.share_group = grp

        self.duration_row, self.duration = spin_row("Duration", None, 1, 999, 30)
        grp.add(self.duration_row)

        self.unit_row = Adw.ComboRow(title="Unit", model=Gtk.StringList.new(UNITS))
        grp.add(self.unit_row)

        self.mode_row = Adw.ComboRow(title="Sharing mode", model=Gtk.StringList.new(MODES))
        self.mode_row.connect("notify::selected", self._on_mode_changed)
        grp.add(self.mode_row)

        self.adopt_switch = Gtk.Switch(valign=Gtk.Align.CENTER, active=True)
        self.adopt_row = Adw.ActionRow(title="Allow Adoption",
                                       subtitle="Others may add you to their group share",
                                       activatable_widget=self.adopt_switch)
        self.adopt_row.set_subtitle_lines(0)
        help_btn = label_icon_button("help-about-symbolic", "What Is Adoption?")
        help_btn.connect("clicked", lambda *_: self.explain_adoption())
        self.adopt_row.add_suffix(help_btn)
        self.adopt_row.add_suffix(self.adopt_switch)
        grp.add(self.adopt_row)

        self.nick_row = Adw.EntryRow(title="Nickname",
                                     input_purpose=Gtk.InputPurpose.NAME)
        grp.add(self.nick_row)
        self.pin_entry_row = Adw.EntryRow(title="Group PIN",
                                          input_purpose=Gtk.InputPurpose.DIGITS)
        grp.add(self.pin_entry_row)
        return grp

    def _primary_button(self):
        self.main_btn = Gtk.Button(label="Start Sharing", halign=Gtk.Align.CENTER)
        self.main_btn.add_css_class("pill")
        self.main_btn.add_css_class("suggested-action")
        self.main_btn.connect("clicked", self._on_main_clicked)
        return self.main_btn

    def _pin_group(self):
        grp = Adw.PreferencesGroup(
            title="Group PIN",
            description="Others can join your group share with this PIN.")
        box = Gtk.Box(spacing=12, halign=Gtk.Align.CENTER)
        self.pin_label = Gtk.Label(selectable=True)
        self.pin_label.add_css_class("title-1")
        self.pin_label.add_css_class("numeric")
        copy = label_icon_button("edit-copy-symbolic", "Copy PIN")
        copy.connect("clicked", lambda *_: self.toast("PIN copied"))
        box.append(self.pin_label)
        box.append(copy)
        grp.add(box)
        lb = Gtk.ListBox(selection_mode=Gtk.SelectionMode.NONE, margin_top=12)
        lb.add_css_class("boxed-list")
        add = Adw.ActionRow(title="Add Existing Share…", activatable=True)
        add.add_suffix(Gtk.Image(icon_name="go-next-symbolic"))
        add.connect("activated", lambda *_: self.adopt_dialog())
        lb.append(add)
        grp.add(lb)
        self.pin_group = grp
        grp.set_visible(False)
        return grp

    def _links_group(self):
        new_btn = label_icon_button("list-add-symbolic", "New Link")
        new_btn.set_sensitive(False)
        new_btn.connect("clicked", lambda *_: self.add_link("K3QZ-9WTA", "Your location only"))
        self.new_link_btn = new_btn
        grp = Adw.PreferencesGroup(title="Active Links", header_suffix=new_btn)
        self.links = Gtk.ListBox(selection_mode=Gtk.SelectionMode.NONE)
        self.links.add_css_class("boxed-list")
        empty = Gtk.Label(label="No active links", margin_top=12, margin_bottom=12,
                          wrap=True)
        empty.add_css_class("dim-label")
        self.links.set_placeholder(empty)
        grp.add(self.links)
        return grp

    # -- behaviour ----------------------------------------------------------

    def _on_mode_changed(self, *_):
        mode = self.mode_row.get_selected()
        self.adopt_row.set_visible(mode == 0)
        self.nick_row.set_visible(mode in (1, 2))
        self.pin_entry_row.set_visible(mode == 2)

    def set_status(self, key):
        icon, style, title = STATUS[key]
        self.status_icon.set_from_icon_name(icon)
        for cls in ("success", "warning", "error"):
            self.status_icon.remove_css_class(cls)
        if style:
            self.status_icon.add_css_class(style)
        self.status_row.set_title(title)
        if key == "inactive":
            self.status_row.set_subtitle("Choose how long to share, then start")
        elif self._deadline:
            end = time.strftime("%H:%M", time.localtime(self._deadline))
            self.status_row.set_subtitle(f"Until {end} · hauk.example.org")

    def _tick(self):
        if not self._deadline:
            self.countdown.set_label("")
            return False
        left = max(0, int(self._deadline - time.time()))
        h, rem = divmod(left, 3600)
        m, s = divmod(rem, 60)
        self.countdown.set_label(f"{h}:{m:02}:{s:02}" if h else f"{m}:{s:02}")
        return True

    def start(self, group=False):
        self._deadline = time.time() + 29 * 60 + 51
        self.set_status("active")
        self._tick()
        GLib.timeout_add_seconds(1, self._tick)
        self.share_group.set_sensitive(False)
        self.main_btn.set_label("Stop Sharing")
        self.main_btn.remove_css_class("suggested-action")
        self.main_btn.add_css_class("destructive-action")
        self.new_link_btn.set_sensitive(True)
        if group:
            self.pin_label.set_label("482 913")
            self.pin_group.set_visible(True)
            self.add_link("TQ7M-2XRB", "Group share (host)")
        else:
            self.add_link("XPHU-64H8", "Your location only")
        # A new session has no fix yet.
        self.set_signal("searching")

    def add_link(self, link_id, kind):
        row = Adw.ActionRow(title=link_id, subtitle=kind)
        copy = label_icon_button("edit-copy-symbolic", f"Copy Link {link_id}")
        copy.connect("clicked", lambda *_: self.toast("Link copied"))
        stop = label_icon_button("process-stop-symbolic", f"Stop Link {link_id}")
        stop.connect("clicked", lambda *_: self.links.remove(row))
        row.add_suffix(copy)
        row.add_suffix(stop)
        self.links.append(row)

    def _on_main_clicked(self, *_):
        if self._deadline:
            self.confirm_stop()
        else:
            self.start(group=self.mode_row.get_selected() == 1)
            self.toast("Sharing started", "Copy Link")

    def toast(self, title, button=None):
        t = Adw.Toast(title=title)
        if button:
            t.set_button_label(button)
        self.toasts.add_toast(t)

    def explain_adoption(self):
        d = Adw.MessageDialog(
            transient_for=self, heading="Share Adoption",
            body="If you allow adoption, someone who creates a group share can "
                 "add your existing share to their group map, without any "
                 "action from you. Encrypted shares cannot be adopted.")
        d.add_response("close", "_Close")
        d.present()

    def confirm_stop(self):
        d = Adw.MessageDialog(transient_for=self, heading="Stop Sharing?",
                              body="Everyone with one of your links will stop "
                                   "seeing your location.")
        d.add_response("cancel", "_Cancel")
        d.add_response("stop", "_Stop Sharing")
        d.set_response_appearance("stop", Adw.ResponseAppearance.DESTRUCTIVE)
        d.set_default_response("cancel")
        d.set_close_response("cancel")
        d.present()

    def adopt_dialog(self):
        d = Adw.MessageDialog(transient_for=self, heading="Add Existing Share",
                              body="Paste the link or ID of a share that allows adoption.")
        lb = Gtk.ListBox(selection_mode=Gtk.SelectionMode.NONE)
        lb.add_css_class("boxed-list")
        lb.append(Adw.EntryRow(title="Link or ID", input_purpose=Gtk.InputPurpose.URL))
        lb.append(Adw.EntryRow(title="Nickname", input_purpose=Gtk.InputPurpose.NAME))
        d.set_extra_child(lb)
        d.add_response("cancel", "_Cancel")
        d.add_response("add", "_Add")
        d.set_response_appearance("add", Adw.ResponseAppearance.SUGGESTED)
        d.present()


class Preferences(Adw.PreferencesWindow):
    def __init__(self, parent):
        super().__init__(transient_for=parent, modal=True, default_width=360,
                         default_height=720)
        page = Adw.PreferencesPage(title="General", icon_name="preferences-system-symbolic")
        self.add(page)

        srv = Adw.PreferencesGroup(title="Server")
        srv.add(Adw.EntryRow(title="Server address", text="https://hauk.example.org/",
                             input_purpose=Gtk.InputPurpose.URL))
        srv.add(Adw.EntryRow(title="Username"))
        srv.add(Adw.PasswordEntryRow(title="Password"))
        srv.add(Adw.EntryRow(title="Preferred link ID"))
        page.add(srv)

        upd = Adw.PreferencesGroup(title="Location Updates")
        upd.add(spin_row("Update interval", "Seconds between updates", 1, 3600, 1)[0])
        upd.add(spin_row("Minimum distance", "Meters moved before sending", 0, 1000, 0)[0])
        page.add(upd)

        sec = Adw.PreferencesGroup(title="Privacy")
        e2e = Adw.ExpanderRow(title="End-to-End Encryption",
                              subtitle="Viewers must enter a password",
                              show_enable_switch=True, enable_expansion=False)
        e2e.add_row(Adw.PasswordEntryRow(title="Encryption password"))
        sec.add(e2e)
        page.add(sec)

        net = Adw.PreferencesGroup(title="Network")
        net.add(Adw.ComboRow(title="Proxy", model=Gtk.StringList.new(
            ["System Default", "None", "HTTP", "SOCKS"])))
        net.add(spin_row("Connection timeout", "Seconds", 1, 120, 10)[0])
        page.add(net)

        app_grp = Adw.PreferencesGroup(title="Behavior")
        sw = Gtk.Switch(valign=Gtk.Align.CENTER, active=True)
        row = Adw.ActionRow(title="Confirm Before Stopping", activatable_widget=sw)
        row.add_suffix(sw)
        app_grp.add(row)
        page.add(app_grp)


class App(Adw.Application):
    def __init__(self):
        super().__init__(application_id=APP_ID, flags=Gio.ApplicationFlags.NON_UNIQUE)

    def do_activate(self):
        win = MainWindow(self)
        for name, cb in (("preferences", lambda *_: Preferences(win).present()),
                         ("about", lambda *_: self.about(win))):
            act = Gio.SimpleAction.new(name, None)
            act.connect("activate", cb)
            self.add_action(act)
        self.set_accels_for_action("app.preferences", ["<Control>comma"])
        win.present()

        if STATE == "connecting":
            win.set_status("connecting")
            win.main_btn.set_sensitive(False)
            win.share_group.set_sensitive(False)
        elif STATE == "active":
            win.start()
            win.set_signal("good", accuracy=3)
        elif STATE == "group":
            win.mode_row.set_selected(1)
            win.nick_row.set_text("Alex")
            win.start(group=True)
            win.set_signal("good", accuracy=3)
            win.add_link("K3QZ-9WTA", "Your location only")
        elif STATE == "join":
            win.mode_row.set_selected(2)
        elif STATE.startswith("sig-"):
            kind = STATE[4:]
            if kind in ("excellent", "good", "fair", "poor", "searching", "lost"):
                win.start()
            args = {"excellent": dict(accuracy=1), "good": dict(accuracy=3),
                    "fair": dict(accuracy=50), "poor": dict(accuracy=300),
                    "net": dict(accuracy=80, source="network"), "lost": dict(age=42)}
            if kind == "net":
                win.start(); win.set_signal("fair", **args["net"])
            elif kind == "checking":
                win.set_signal("searching")
            elif kind != "off":
                win.set_signal(kind, **args.get(kind, {}))
            if os.environ.get("SIG_EXPAND"):
                win.signal_row.set_expanded(True)
        elif STATE == "prefs":
            GLib.timeout_add(800, lambda: Preferences(win).present() or False)
        elif STATE == "confirm":
            win.start()
            win.set_signal("good", accuracy=3)
            GLib.timeout_add(800, lambda: win.confirm_stop() or False)
        elif STATE == "adopt":
            win.mode_row.set_selected(1)
            win.start(group=True)
            win.set_signal("good", accuracy=3)
            GLib.timeout_add(800, lambda: win.adopt_dialog() or False)

        if os.environ.get("SPIKE_SHOT"):
            # Test hook: signal the screenshot harness, then exit.
            GLib.timeout_add(2500, lambda: print("ready-for-shot", flush=True) or False)
            GLib.timeout_add(int(os.environ.get("SPIKE_LINGER", "6000")),
                             lambda: self.quit() or False)

    def about(self, win):
        Adw.AboutWindow(transient_for=win, application_name="Jastrzab",
                        application_icon="find-location-symbolic",
                        version="0.1", license_type=Gtk.License.APACHE_2_0).present()


if __name__ == "__main__":
    App().run([sys.argv[0]])
