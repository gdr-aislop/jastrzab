#!/usr/bin/env python3
"""Dump the AT-SPI tree of a running app and flag unlabelled controls.

    a11y_dump.py [app-name-prefix] [--json]
"""
import sys

import gi

gi.require_version("Atspi", "2.0")
from gi.repository import Atspi  # noqa: E402

ALL = "--all" in sys.argv
INTERACTIVE = {"push button", "toggle button", "spin button", "text", "entry", "check box", "switch",
               "combo box", "menu button", "password text", "list item", "radio button"}


def first_label(node):
    for i in range(node.get_child_count()):
        child = node.get_child_at_index(i)
        if child is None:
            continue
        if child.get_role_name() == "label" and (child.get_name() or "").strip():
            return child.get_name()
        found = first_label(child)
        if found:
            return found
    return ""


def walk(node, out, missing, depth=0):
    try:
        role = node.get_role_name()
        name = node.get_name() or ""
        desc = node.get_description() or ""
        showing = node.get_state_set().contains(Atspi.StateType.SHOWING)
    except Exception as exc:  # the node went away
        out.append("  " * depth + f"<error {exc}>")
        return
    if not showing and depth > 1 and not ALL:
        return
    flag = ""
    if role == "list item" and not name.strip():
        # libadwaita's combo rows keep their current value in an unnamed list item whose only
        # content is a label: the label is its name.
        name = first_label(node)
    if role in INTERACTIVE and not name.strip():
        flag = "   <-- NO ACCESSIBLE NAME"
        missing.append(role)
    extra = f"  desc={desc!r}" if desc else ""
    out.append("  " * depth + f"[{role}] {name!r}{extra}{flag}")
    for i in range(node.get_child_count()):
        child = node.get_child_at_index(i)
        if child is not None:
            walk(child, out, missing, depth + 1)


def main():
    prefix = next((a for a in sys.argv[1:] if not a.startswith("--")), "jastrzab")
    desktop = Atspi.get_desktop(0)
    out, missing, found = [], [], False
    for i in range(desktop.get_child_count()):
        app = desktop.get_child_at_index(i)
        if app is not None and (app.get_name() or "").startswith(prefix):
            found = True
            walk(app, out, missing)
    print("\n".join(out))
    print(f"\nunlabelled interactive controls: {len(missing)} {missing}")
    if not found:
        sys.exit("target app not found on the accessibility bus")
    sys.exit(1 if missing else 0)


main()
