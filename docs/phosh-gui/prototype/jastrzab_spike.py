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
        page.append(self._share_group())
        page.append(self._primary_button())
        page.append(self._pin_group())
        page.append(self._links_group())

        self._deadline = None
        self._on_mode_changed()
        self.set_status("inactive")

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
        upd.add(spin_row("GPS fallback delay", "Seconds without GPS before using network location", 5, 600, 45)[0])
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
        elif STATE == "group":
            win.mode_row.set_selected(1)
            win.nick_row.set_text("Alex")
            win.start(group=True)
            win.add_link("K3QZ-9WTA", "Your location only")
        elif STATE == "join":
            win.mode_row.set_selected(2)
        elif STATE == "prefs":
            GLib.timeout_add(800, lambda: Preferences(win).present() or False)
        elif STATE == "confirm":
            win.start()
            GLib.timeout_add(800, lambda: win.confirm_stop() or False)
        elif STATE == "adopt":
            win.mode_row.set_selected(1)
            win.start(group=True)
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
