#!/usr/bin/env python3
"""Window-growth experiments for GTK 4.8 / libadwaita 1.2.

Each scenario builds a window, waits until it is mapped, then grows the
content (long text, many rows, ...) and reports the window size and the
window's measured minimum size before and after.  One JSON line per sample.
"""
import json
import sys

import gi

gi.require_version("Gtk", "4.0")
gi.require_version("Adw", "1")
from gi.repository import Adw, GLib, Gtk, Pango  # noqa: E402

SCENARIO = sys.argv[1]
SHOT = sys.argv[2] if len(sys.argv) > 2 else None  # optional: pause for screenshot

LONG_URL = "https://hauk.example.org/?" + "XPHU-64H8-" * 12
ROWS = 40


def out(stage, win):
    mw = win.measure(Gtk.Orientation.HORIZONTAL, -1)[0]
    mh = win.measure(Gtk.Orientation.VERTICAL, -1)[0]
    surf = win.get_surface()
    print(json.dumps({
        "scenario": SCENARIO, "stage": stage,
        "alloc": [win.get_width(), win.get_height()],
        "surface": [surf.get_width(), surf.get_height()] if surf else None,
        "min": [mw, mh],
        "maximized": win.is_maximized(),
        "resizable": win.get_resizable(),
    }), flush=True)


def header(box):
    hb = Adw.HeaderBar()
    hb.set_title_widget(Adw.WindowTitle(title="Hauk", subtitle=SCENARIO))
    box.append(hb)


def build(app):
    win = Adw.ApplicationWindow(application=app)
    if SCENARIO not in ("nonresizable-nodefault",):
        win.set_default_size(360, 720)
    if SCENARIO.startswith("nonresizable"):
        win.set_resizable(False)

    root = Gtk.Box(orientation=Gtk.Orientation.VERTICAL)
    header(root)
    win.set_content(root)

    grow = None

    if SCENARIO == "naive-label":
        lbl = Gtk.Label(label="Short", xalign=0, margin_start=12, margin_end=12)
        root.append(lbl)
        grow = lambda: lbl.set_label(LONG_URL)

    elif SCENARIO == "wrapped-label":
        lbl = Gtk.Label(label="Short", xalign=0, margin_start=12, margin_end=12,
                        wrap=True, wrap_mode=Pango.WrapMode.WORD_CHAR)
        root.append(lbl)
        grow = lambda: lbl.set_label(LONG_URL)

    elif SCENARIO == "ellipsized-label":
        lbl = Gtk.Label(label="Short", xalign=0, margin_start=12, margin_end=12,
                        ellipsize=Pango.EllipsizeMode.MIDDLE)
        root.append(lbl)
        grow = lambda: lbl.set_label(LONG_URL)

    elif SCENARIO == "actionrow-long":
        lb = Gtk.ListBox(selection_mode=Gtk.SelectionMode.NONE, margin_start=12,
                         margin_end=12, margin_top=12)
        lb.add_css_class("boxed-list")
        row = Adw.ActionRow(title="XPHU-64H8", subtitle="Your location only")
        btn = Gtk.Button(icon_name="edit-copy-symbolic", valign=Gtk.Align.CENTER)
        row.add_suffix(btn)
        lb.append(row)
        root.append(lb)
        grow = lambda: row.set_subtitle(LONG_URL)

    elif SCENARIO in ("naive-list", "scrolled-list", "nonresizable", "nonresizable-nodefault"):
        lb = Gtk.ListBox(selection_mode=Gtk.SelectionMode.NONE)
        lb.add_css_class("boxed-list")
        clamp = Adw.Clamp(maximum_size=600, margin_start=12, margin_end=12,
                          margin_top=12, margin_bottom=12)
        clamp.set_child(lb)
        if SCENARIO == "naive-list":
            root.append(clamp)
        else:
            sw = Gtk.ScrolledWindow(hscrollbar_policy=Gtk.PolicyType.NEVER,
                                    vexpand=True)
            sw.set_child(clamp)
            root.append(sw)
        lb.append(Adw.ActionRow(title="Row 0"))

        def add_rows():
            for i in range(1, ROWS):
                lb.append(Adw.ActionRow(title=f"Row {i}", subtitle="Group share (member)"))
        grow = add_rows

    elif SCENARIO == "wide-buttons":
        # Three text buttons in a row: their combined minimum width
        # exceeds 360 px.
        bx = Gtk.Box(spacing=6, margin_start=12, margin_end=12, margin_top=12)
        root.append(bx)

        def add():
            for t in ("Create new sharing link", "Merge an existing link", "Stop sharing"):
                bx.append(Gtk.Button(label=t))
        grow = add

    else:
        raise SystemExit(f"unknown scenario {SCENARIO}")

    win.present()

    def stage(n):
        if n == 0:
            out("before", win)
            grow()
            GLib.timeout_add(1500, stage, 1)
        elif n == 1:
            out("after", win)
            if SHOT:
                print(json.dumps({"scenario": SCENARIO, "stage": "ready-for-shot"}), flush=True)
                GLib.timeout_add(4000, stage, 2)
            else:
                app.quit()
        else:
            app.quit()
        return False

    GLib.timeout_add(1500, stage, 0)


app = Adw.Application(application_id="org.example.HaukSizeTest")
app.connect("activate", build)
app.run([])
