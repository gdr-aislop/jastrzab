"""Fake system services for the UI checks, on the (shared) session/"system" bus.

* org.freedesktop.Notifications: records every call and can invoke an action, like Phosh.
* org.freedesktop.login1: hands out a pipe for each Inhibit call; the lock is held while the
  client keeps its end open, exactly like the real thing.
"""
import os
import select
import threading

import gi

gi.require_version("Gio", "2.0")
from gi.repository import Gio, GLib  # noqa: E402

NOTIFICATIONS_XML = """<node><interface name="org.freedesktop.Notifications">
 <method name="GetCapabilities"><arg type="as" direction="out"/></method>
 <method name="Notify"><arg type="s" direction="in"/><arg type="u" direction="in"/><arg type="s" direction="in"/>
  <arg type="s" direction="in"/><arg type="s" direction="in"/><arg type="as" direction="in"/>
  <arg type="a{sv}" direction="in"/><arg type="i" direction="in"/><arg type="u" direction="out"/></method>
 <method name="CloseNotification"><arg type="u" direction="in"/></method>
 <method name="GetServerInformation"><arg type="s" direction="out"/><arg type="s" direction="out"/>
  <arg type="s" direction="out"/><arg type="s" direction="out"/></method>
 <signal name="ActionInvoked"><arg type="u"/><arg type="s"/></signal>
 <signal name="NotificationClosed"><arg type="u"/><arg type="u"/></signal>
</interface></node>"""

LOGIND_XML = """<node><interface name="org.freedesktop.login1.Manager">
 <method name="Inhibit"><arg type="s" direction="in"/><arg type="s" direction="in"/><arg type="s" direction="in"/>
  <arg type="s" direction="in"/><arg type="h" direction="out"/></method>
</interface></node>"""


class Fakes:
    def __init__(self):
        self.notifications = []   # dicts, one per Notify call
        self.closed = []          # ids passed to CloseNotification
        self.inhibitors = []      # (what, who, why, mode, write_fd)
        self._next_id = 100
        self._conn = None
        self._ready = threading.Event()
        self._thread = threading.Thread(target=self._run, daemon=True)

    def start(self):
        self._thread.start()
        self._ready.wait(10)
        return self

    def _run(self):
        ctx = GLib.MainContext.new()
        ctx.push_thread_default()
        self._loop = GLib.MainLoop.new(ctx, False)
        self._owned = 0

        def acquired(conn, name):
            self._conn = conn
            self._owned += 1
            if self._owned >= 2:
                self._ready.set()

        def on_bus(conn, name):
            conn.register_object("/org/freedesktop/Notifications", Gio.DBusNodeInfo.new_for_xml(NOTIFICATIONS_XML).interfaces[0], self._notifications_call, None, None)
            conn.register_object("/org/freedesktop/login1", Gio.DBusNodeInfo.new_for_xml(LOGIND_XML).interfaces[0], self._logind_call, None, None)

        Gio.bus_own_name(Gio.BusType.SESSION, "org.freedesktop.Notifications", Gio.BusNameOwnerFlags.NONE, on_bus, acquired, None)
        Gio.bus_own_name(Gio.BusType.SESSION, "org.freedesktop.login1", Gio.BusNameOwnerFlags.NONE, None, acquired, None)
        self._loop.run()

    # -- Notifications ----------------------------------------------------------------------
    def _notifications_call(self, conn, sender, path, iface, method, params, invocation, _=None):
        if method == "GetCapabilities":
            invocation.return_value(GLib.Variant("(as)", (["body", "actions", "body-markup", "icon-static"],)))
        elif method == "GetServerInformation":
            invocation.return_value(GLib.Variant("(ssss)", ("fake-phosh", "test", "0", "1.2")))
        elif method == "Notify":
            app, replaces, icon, summary, body, actions, hints, timeout = params.unpack()
            nid = replaces or self._next_id
            if not replaces:
                self._next_id += 1
            self.notifications.append(dict(app=app, replaces=replaces, id=nid, icon=icon, summary=summary, body=body, actions=list(actions), hints=hints))
            invocation.return_value(GLib.Variant("(u)", (nid,)))
        elif method == "CloseNotification":
            self.closed.append(params.unpack()[0])
            invocation.return_value(None)

    def invoke_action(self, key):
        """Tap an action on the latest notification, as Phosh does."""
        nid = self.notifications[-1]["id"]
        self._conn.emit_signal(None, "/org/freedesktop/Notifications", "org.freedesktop.Notifications", "ActionInvoked", GLib.Variant("(us)", (nid, key)))

    # -- logind -----------------------------------------------------------------------------
    def _logind_call(self, conn, sender, path, iface, method, params, invocation, _=None):
        what, who, why, mode = params.unpack()
        read_fd, write_fd = os.pipe()
        self.inhibitors.append((what, who, why, mode, write_fd))
        # The list owns read_fd from here on and closes it itself; closing it here too would
        # abort GLib ("file descriptor closed twice").
        invocation.return_value_with_unix_fd_list(GLib.Variant("(h)", (0,)), Gio.UnixFDList.new_from_array([read_fd]))

    def inhibitors_held(self):
        """How many inhibit locks the client still holds (its end of the pipe is open)."""
        held = 0
        for *_, write_fd in self.inhibitors:
            poller = select.poll()
            poller.register(write_fd, select.POLLERR)
            if not poller.poll(0):
                held += 1
        return held
