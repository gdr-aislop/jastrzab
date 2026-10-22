#!/usr/bin/env python3
"""Drives the real app through its accessibility tree (AT-SPI), inside a headless Phoc.

    drive.py <scenario> [shots-dir]

Scenarios: solo (check the signal, share, verify the points on a real Hauk backend, stop),
prefs (open Preferences and Main Menu, audit labels), group (host a group share).

Needs a Hauk backend at HAUK_URL (default http://127.0.0.1:8080/) with password HAUK_PASSWORD
(default testpw). Run through session.sh, which provides phoc, a D-Bus session and a keyring.
"""
import json
import os
import subprocess
import sys
import time
import urllib.request

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import gi

gi.require_version("Atspi", "2.0")
gi.require_version("Secret", "1")
from gi.repository import Atspi, Secret  # noqa: E402

HAUK_URL = os.environ.get("HAUK_URL", "http://127.0.0.1:8080/")
HAUK_PASSWORD = os.environ.get("HAUK_PASSWORD", "testpw")
HOME = "/root/e2e-home"
APP_ID = "io.github.gdr_aislop.jastrzab"
SHOTS = sys.argv[2] if len(sys.argv) > 2 else "/root/shots"
BINARY = os.environ.get("JASTRZAB_BIN") or "/root/target/debug/jastrzab"

failures = []


def check(condition, message):
    print(("ok   " if condition else "FAIL ") + message, flush=True)
    if not condition:
        failures.append(message)


def shot(name):
    os.makedirs(SHOTS, exist_ok=True)
    subprocess.run(["grim", f"{SHOTS}/{name}.png"], check=False)


# --- AT-SPI helpers ----------------------------------------------------------------------------

def app_node():
    desktop = Atspi.get_desktop(0)
    for i in range(desktop.get_child_count()):
        node = desktop.get_child_at_index(i)
        if node is not None and (node.get_name() or "") == "jastrzab":
            return node
    return None


def walk(node):
    yield node
    for i in range(node.get_child_count()):
        child = node.get_child_at_index(i)
        if child is not None:
            yield from walk(child)


def find_all(role=None, name=None, contains=None):
    root = app_node()
    if root is None:
        return []
    found = []
    for node in walk(root):
        try:
            if role and node.get_role_name() != role:
                continue
            n = node.get_name() or ""
            if name is not None and n != name:
                continue
            if contains is not None and contains not in n:
                continue
            found.append(node)
        except Exception:
            pass
    return found


def wait_for(fn, timeout=15, what=""):
    deadline = time.time() + timeout
    while time.time() < deadline:
        try:
            value = fn()
        except Exception:
            value = None
        if value:
            return value
        time.sleep(0.3)
    check(False, f"timed out waiting for {what}")
    return None


def click(role, name, timeout=15, last=False):
    def pick():
        nodes = [n for n in find_all(role, name) if n.get_state_set().contains(Atspi.StateType.VISIBLE) and Atspi.Action.get_n_actions(n) > 0]
        return (nodes[-1] if last else nodes[0]) if nodes else None
    node = wait_for(pick, timeout, f"{role} {name!r}")
    if node is None:
        return False
    Atspi.Action.do_action(node, 0)
    return True


def click_text(text, timeout=15):
    """Activates whatever contains the visible label `text`: the nearest ancestor with an action."""
    def pick():
        for label in find_all("label", text):
            node = label
            while node is not None:
                if Atspi.Action.get_n_actions(node) > 0 and node.get_role_name() in ("list item", "push button", "menu item", "toggle button", "check box"):
                    return node
                node = node.get_parent()
        return None
    node = wait_for(pick, timeout, f"something labelled {text!r}")
    if node is None:
        return False
    Atspi.Action.do_action(node, 0)
    return True


def click_in_dialog(name, timeout=15):
    """Clicks a button in a dialog window: any top-level that is not the main 'Jastrzab' frame."""
    def pick():
        root = app_node()
        for i in range(root.get_child_count()):
            top = root.get_child_at_index(i)
            if top is None or top.get_name() == "Jastrzab":
                continue
            for node in walk(top):
                if node.get_role_name() == "push button" and (node.get_name() or "").startswith(name):
                    return node
        return None
    node = wait_for(pick, timeout, f"dialog button {name!r}")
    if node is None:
        return False
    Atspi.Action.do_action(node, 0)
    return True


def labels():
    return [n.get_name() for n in find_all("label") if n.get_name()]


def has_label(text):
    return any(text in label for label in labels())


# --- the Hauk backend --------------------------------------------------------------------------

def fetch_points(link_id):
    try:
        opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))  # not via the sandbox proxy
        body = opener.open(f"{HAUK_URL}api/fetch.php?id={link_id}", timeout=5).read().decode()
        data = json.loads(body)
        return data.get("points", [])
    except Exception as exc:
        return f"error: {exc}"


# --- the app -----------------------------------------------------------------------------------

def start_app(extra_settings="", fake="52.52,13.40,3", url=None, form_mode=None):
    subprocess.run(["rm", "-rf", HOME])
    cfg = f"{HOME}/.config/{APP_ID}"
    os.makedirs(cfg, exist_ok=True)
    with open(f"{cfg}/settings.toml", "w") as f:
        f.write(f'server_url = "{url or HAUK_URL}"\ninterval_secs = 2\n{extra_settings}')
    # The app has its own XDG_CONFIG_HOME, so desktop settings (text scale, ...) the harness was
    # given for the compositor are written where the app's GSettings keyfile backend looks.
    extra = os.environ.get("PHOC_KEYFILE_EXTRA", "").strip()
    if extra:
        os.makedirs(f"{HOME}/.config/glib-2.0/settings", exist_ok=True)
        with open(f"{HOME}/.config/glib-2.0/settings/keyfile", "w") as f:
            f.write(extra + "\n")
    schema = Secret.Schema.new(APP_ID, Secret.SchemaFlags.NONE, {"kind": Secret.SchemaAttributeType.STRING, "server": Secret.SchemaAttributeType.STRING})
    Secret.password_store_sync(schema, {"kind": "server-password", "server": url or HAUK_URL}, Secret.COLLECTION_SESSION, "test", HAUK_PASSWORD, None)
    # The sandbox's own HTTP proxy would otherwise be used for the local backend.
    clean = {k: v for k, v in os.environ.items() if "proxy" not in k.lower()}
    env = dict(clean, HOME=HOME, XDG_CONFIG_HOME=f"{HOME}/.config", NO_AT_BRIDGE="0", GTK_A11Y="atspi", RUST_LOG="info,jastrzab=debug")
    if fake:
        env["JASTRZAB_FAKE_LOCATION"] = fake
    if form_mode:
        env["JASTRZAB_FORM_MODE"] = form_mode
    proc = subprocess.Popen([BINARY], env=env, stdout=open("/root/e2e-app.log", "w"), stderr=subprocess.STDOUT)
    wait_for(lambda: app_node() is not None and find_all("push button", "Start Sharing"), 20, "the main window")
    return proc


def log_text():
    d = f"{HOME}/.local/state/{APP_ID}/logs"
    try:
        return "".join(open(f"{d}/{name}").read() for name in sorted(os.listdir(d)))
    except OSError:
        return ""


def end_app(proc):
    proc.terminate()
    try:
        proc.wait(timeout=8)
    except subprocess.TimeoutExpired:
        proc.kill()
        check(False, "the app exits on SIGTERM")


# --- scenarios ---------------------------------------------------------------------------------

def solo():
    proc = start_app()
    check(has_label("Not sharing"), "starts idle")
    shot("e2e-1-idle")

    click("push button", "Check")
    wait_for(lambda: has_label("Good") or any("Good" in l for l in labels()), 15, "the signal to reach Good")
    check(any(l.startswith("Good") for l in labels()), "the signal check shows Good for a 3 m fix")
    check(any(l == "Stop" for l in labels()), "the Check button became Stop")
    shot("e2e-2-checking")

    click("push button", "Start Sharing")
    wait_for(lambda: has_label("Sharing location"), 20, "Sharing location")
    check(has_label("Sharing location"), "the share started against the real backend")
    check(not any(l == "Not checking" for l in labels()), "the signal row is never 'Not checking' while sharing")
    time.sleep(6)  # let the "Sharing started" toast go before the screenshot
    shot("e2e-3-sharing")

    items = wait_for(lambda: [n for n in find_all("list item") if n.get_name() and "-" in n.get_name() and len(n.get_name()) == 9], 10, "a link row")
    link_id = items[0].get_name() if items else None
    check(link_id is not None, f"a link row is shown ({link_id})")
    points = wait_for(lambda: (p := fetch_points(link_id)) and isinstance(p, list) and len(p) >= 1 and p, 20, "a point on the server")
    check(bool(points) and abs(points[-1][0] - 52.52) < 0.01, f"the server received the position: {points[-1] if points else None}")
    first_count = len(points) if points else 0
    time.sleep(7)
    later = fetch_points(link_id)
    check(isinstance(later, list) and later and later[-1][2] > points[-1][2], "later points keep arriving")

    click("push button", "Stop Sharing")
    time.sleep(1)
    shot("e2e-confirm-stop")
    click_in_dialog("Stop Sharing")  # the confirmation dialog's response
    wait_for(lambda: has_label("Not sharing"), 15, "Not sharing")
    check(has_label("Not sharing"), "stopping returns to idle")
    shot("e2e-4-stopped")
    after = fetch_points(link_id)
    check(not (isinstance(after, list) and after), "the share is gone on the server")
    end_app(proc)

    # The log is buffered for up to a minute and flushed on a clean exit, so it is read afterwards.
    log = log_text()
    check("starting up" in log, "the log has the start-up line")
    import re
    # (A timestamp such as 18:55:52.527098 is not a coordinate: it follows a colon.)
    check(re.search(r"(?<![\d:])52\.52\d*", log) is None and HAUK_PASSWORD not in log, "no coordinates or password in the log")
    check(link_id is not None and link_id not in log, "the share ID is scrubbed from the log")
    check("<id>" in log or "api/create.php" in log, "requests were logged (scrubbed)")


def prefs():
    proc = start_app()
    click("push button", "Main Menu")
    shot("e2e-menu")
    click("menu item", "Preferences")
    wait_for(lambda: find_all("label", "Server"), 10, "the Preferences window")
    time.sleep(1)
    shot("e2e-prefs")
    unnamed = []
    for node in find_all():
        try:
            if node.get_role_name() in ("push button", "toggle button", "spin button", "check box", "switch", "combo box") and not (node.get_name() or "").strip():
                unnamed.append(node.get_role_name())
        except Exception:
            pass
    check(not unnamed, f"every control in Preferences has a name {unnamed}")
    end_app(proc)


def frame_size():
    frame = next(iter(find_all("frame", "Jastrzab")), None)
    ext = Atspi.Component.get_extents(frame, Atspi.CoordType.SCREEN)
    return ext.width, ext.height


class Hostile:
    """A fake Hauk server that answers with the worst text a server could send."""

    def __init__(self):
        import http.server
        import threading
        self.post_body = None
        outer = self

        class Handler(http.server.BaseHTTPRequestHandler):
            def do_POST(self):
                self.rfile.read(int(self.headers.get("Content-Length", 0)))
                path = self.path.rsplit("/", 1)[-1]
                long_id = "L" + "-ABCDEFGHIJ" * 8
                if path == "create.php":
                    body = f"OK\n{'S' * 300}\nhttps://hauk.example/?{'X' * 200}\n{long_id}\n"
                elif path == "post.php":
                    body = outer.post_body or "OK\nhttps://hauk.example/?%s\n" + ",".join(f"{long_id}{n}" for n in range(25)) + "\n"
                elif path == "new-link.php":
                    self.send_response(302)
                    self.send_header("Location", "https://elsewhere.example/api/new-link.php")
                    self.send_header("Content-Length", "0")
                    self.end_headers()
                    return
                else:
                    body = "OK\n"
                data = body.encode()
                self.send_response(200)
                self.send_header("X-Hauk-Version", "1.6.3")
                self.send_header("Content-Length", str(len(data)))
                self.end_headers()
                self.wfile.write(data)

            def log_message(self, *args):
                pass

        self.server = http.server.ThreadingHTTPServer(("127.0.0.1", 8081), Handler)
        threading.Thread(target=self.server.serve_forever, daemon=True).start()


def worstcase():
    """Design rules 1, 2 and 8: nothing a server sends may resize the window."""
    hostile = Hostile()
    proc = start_app(url="http://127.0.0.1:8081/")
    wait_for(lambda: frame_size() and frame_size()[0] > 0, 10, "the window size")
    idle = frame_size()
    check(idle[0] > 0, f"window size when idle: {idle}")
    expect = os.environ.get("EXPECT_SIZE")
    if expect:
        want = tuple(int(v) for v in expect.split("x"))
        check(idle == want, f"the window fits the screen ({idle}), so its minimum size is no larger than {want}")

    click("push button", "Start Sharing")
    wait_for(lambda: has_label("Sharing location"), 20, "Sharing location")
    wait_for(lambda: len([n for n in find_all("list item") if (n.get_name() or "").startswith("L-")]) >= 20, 20, "25 long links")
    time.sleep(1)
    sharing = frame_size()
    check(sharing == idle, f"25 links with 80-character IDs do not change the window size ({idle} -> {sharing})")
    shot("e2e-worst-links")

    click("push button", "New Link")
    dialog_text = wait_for(lambda: next(iter(l for l in labels() if "Received HTTP 302" in l), None), 15, "a dialog for the redirect")
    check(dialog_text is not None, f"a redirect is reported, not followed ({dialog_text!r})")
    click_in_dialog("Close")
    time.sleep(1)
    check(frame_size() == idle, "the dialog does not change the window size either")

    hostile.post_body = "Error: " + "the server is very unhappy " * 40 + "\n"
    wait_for(lambda: has_label("Server reported a problem"), 20, "the server message in the status row")
    time.sleep(1)
    problem = frame_size()
    check(problem == idle, f"a 1000-character server message does not change the window size ({idle} -> {problem})")
    shot("e2e-worst-message")
    end_app(proc)
    hostile.server.shutdown()


def services():
    """The notification (resident, actions, updates) and the suspend inhibitor, against fakes."""
    from fakes import Fakes
    fakes = Fakes().start()
    proc = start_app()
    check(fakes.inhibitors_held() == 0, "no inhibitor before sharing")
    click("push button", "Start Sharing")
    wait_for(lambda: has_label("Sharing location"), 20, "Sharing location")
    first = wait_for(lambda: fakes.notifications[0] if fakes.notifications else None, 10, "a notification")
    check(first is not None and first["app"] == "Jastrzab", "a notification was sent")
    if first:
        check(first["hints"].get("resident") is True, "it is resident, so Phosh keeps it after an action")
        check(first["hints"].get("desktop-entry") == APP_ID, "it names the app's desktop entry")
        keys = first["actions"][0::2]
        check(keys == ["default", "copy-link", "stop-sharing"], f"actions: {keys}")
        check("127.0.0.1" in first["body"], f"body names the server: {first['body']!r}")
    check(fakes.inhibitors_held() == 1, "a suspend inhibitor is held while sharing")
    if fakes.inhibitors:
        what, who, why, mode, _ = fakes.inhibitors[0]
        check((what, mode) == ("sleep", "block"), f"it is a sleep block lock ({what}/{mode})")
    # The GPS signal arrives and the title follows it, replacing the same notification.
    wait_for(lambda: len(fakes.notifications) >= 2, 20, "the notification to be updated")
    if len(fakes.notifications) >= 2:
        a, b = fakes.notifications[0], fakes.notifications[1]
        check(b["replaces"] == a["id"], "the update replaces the first notification")
        check(a["summary"] != b["summary"] and b["summary"] == "Sharing location", f"title: {a['summary']!r} -> {b['summary']!r}")
    time.sleep(6)
    check(len(fakes.notifications) <= 3, f"the notification is not re-sent on every fix ({len(fakes.notifications)} sends)")
    shot("e2e-services")
    fakes.invoke_action("stop-sharing")
    wait_for(lambda: has_label("Not sharing"), 15, "Not sharing after the notification's Stop action")
    check(has_label("Not sharing"), "the notification's Stop action ends the share")
    time.sleep(1)
    check(fakes.inhibitors_held() == 0, "the inhibitor is released")
    check(fakes.notifications and fakes.notifications[-1]["id"] in fakes.closed, "the notification is withdrawn")
    end_app(proc)


def geoclue():
    """The real GeoClue 2.7.0 (WITH_GEOCLUE=1): the static source at 52.52, 13.405."""
    proc = start_app(fake=None)
    click("push button", "Check")
    wait_for(lambda: any("network location" in l or l.split(" ")[0] in ("Excellent", "Good", "Fair", "Poor") for l in labels()), 30, "a fix from GeoClue")
    summary = [l for l in labels() if "network location" in l or l.split(" ")[0] in ("Excellent", "Good", "Fair", "Poor")]
    check(bool(summary), f"GeoClue delivered a fix: {summary}")
    shot("e2e-geoclue-check")
    click("push button", "Start Sharing")
    wait_for(lambda: has_label("Sharing location"), 30, "Sharing location")
    check(has_label("Sharing location"), "sharing with GeoClue as the source")
    items = wait_for(lambda: [n for n in find_all("list item") if n.get_name() and len(n.get_name()) == 9 and "-" in n.get_name()], 10, "a link row")
    link_id = items[0].get_name() if items else None
    points = wait_for(lambda: (p := fetch_points(link_id)) and isinstance(p, list) and len(p) >= 1 and p, 30, "a point on the server")
    check(bool(points) and abs(points[-1][0] - 52.52) < 0.01 and abs(points[-1][1] - 13.405) < 0.01, f"GeoClue's position reached the server: {points[-1] if points else None}")
    click("push button", "Stop Sharing")
    click_in_dialog("Stop Sharing")
    wait_for(lambda: has_label("Not sharing"), 15, "Not sharing")
    end_app(proc)


def log_paths():
    d = f"{HOME}/.local/state/{APP_ID}"
    return d, f"{d}/logs", f"{d}/crashes"


def logging():
    """Buffered log lines reach disk on the 60 s timer and on exit, not before; the file name and
    the line format are the shared scheme."""
    proc = start_app()
    _, logs, _ = log_paths()
    time.sleep(8)
    files = sorted(os.listdir(logs))
    check(len(files) == 1 and files[0].startswith("jastrzab.2"), f"one dated log file: {files}")
    path = f"{logs}/{files[0]}"
    check(os.path.getsize(path) == 0, "nothing has been written yet: lines are held in the 64 KiB buffer")
    time.sleep(58)
    text = open(path).read()
    check("starting up" in text, "the 60 s timer flushed the buffer to disk")
    import re
    first = text.splitlines()[0]
    check(re.match(r"^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d\.\d{6}Z +INFO jastrzab: starting up version=", first) is not None, f"line format: {first[:90]!r}")
    check("\x1b" not in text, "no colour codes in the file")
    size = os.path.getsize(path)
    click("push button", "Check")
    time.sleep(5)
    check(os.path.getsize(path) == size, "new lines stay buffered until the next flush")
    end_app(proc)
    check(os.path.getsize(path) > size, "a clean exit flushes what was buffered")
    check("asked to terminate" in open(path).read(), "the termination was logged")


def crash():
    """SIGSEGV: a minidump is written, and the log gets the crash line plus the lines before it."""
    proc = start_app()
    d, logs, crashes = log_paths()
    click("push button", "Check")
    time.sleep(4)
    servers = subprocess.run(["pgrep", "-f", "crash-handler-server"], capture_output=True, text=True).stdout.split()
    check(len(servers) == 1, f"the crash-dump server is running as a child process ({servers})")
    proc.send_signal(11)
    proc.wait(timeout=15)
    time.sleep(2)
    dumps = [f for f in os.listdir(crashes) if f.endswith(".dmp")]
    check(len(dumps) == 1 and os.path.getsize(f"{crashes}/{dumps[0]}") > 10000, f"a minidump was written: {dumps}")
    check(subprocess.run(["pgrep", "-f", "crash-handler-server"], capture_output=True).returncode != 0, "the crash-dump server has exited")
    log = "".join(open(f"{logs}/{f}").read() for f in sorted(os.listdir(logs)))
    check("starting up" in log, "the lines buffered before the crash were flushed to disk")
    check("handling event=\"check-signal\"" in log, "including events from seconds before the crash")
    check("wrote a native-crash minidump" in log and dumps and dumps[0] in log.replace("~", ""), "the crash server logged the dump's path")
    magic = open(f"{crashes}/{dumps[0]}", "rb").read(4) if dumps else b""
    check(magic == b"MDMP", f"it is a Breakpad minidump ({magic!r})")


def group():
    """Host a group share through the UI: nickname, PIN, host label, adoption dialog."""
    # (A combo row cannot be operated through AT-SPI, and a headless Phoc has no keyboard focus
    # for key synthesis, so debug builds can preselect the mode from the environment.)
    proc = start_app(extra_settings='nickname = "Ann"\n', form_mode="create-group")
    check(any(n.get_name() == "Nickname" for n in find_all("text")), "the nickname field is shown for a group")
    shot("e2e-group-form")
    click("push button", "Start Sharing")
    wait_for(lambda: has_label("Sharing location"), 20, "Sharing location")
    pin = wait_for(lambda: next(iter(l for l in labels() if len(l) == 6 and l.isdigit()), None), 10, "the group PIN")
    check(pin is not None, f"the group PIN is shown ({pin})")
    check(has_label("Group share (host)"), "the link row says host")
    check(any(l == "Add Existing Share…" for l in labels()), "the adoption row is offered")
    link = next(iter(n.get_name() for n in find_all("list item") if n.get_name() and len(n.get_name()) == 9 and "-" in n.get_name()), None)
    points = wait_for(lambda: (p := fetch_points(link)) is not None and isinstance(p, dict) and p, 20, "a group point")
    check(bool(points) and "Ann" in points, f"the server shows the nickname: {list(points) if points else points}")
    time.sleep(6)
    shot("e2e-group-active")
    click("push button", "Stop Sharing")
    click_in_dialog("Stop Sharing")
    wait_for(lambda: has_label("Not sharing"), 15, "Not sharing")
    end_app(proc)


SCENARIOS = {"solo": solo, "prefs": prefs, "group": group, "worstcase": worstcase, "services": services, "geoclue": geoclue, "logging": logging, "crash": crash}

if __name__ == "__main__":
    name = sys.argv[1] if len(sys.argv) > 1 else "solo"
    SCENARIOS[name]()
    print(f"\n{len(failures)} failure(s)")
    sys.exit(1 if failures else 0)
