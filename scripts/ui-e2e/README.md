# Headless UI checks

These scripts run the real app in a headless [Phoc](https://gitlab.gnome.org/World/Phosh/phoc) (the
Phosh compositor) on **PureOS Crimson's own libraries**, and drive it through its accessibility
tree (AT-SPI). They exist because an Xvfb test cannot reproduce how Phosh sizes a window, and
because a GTK app built against a newer GTK than the phone has can quietly use API the phone lacks.

What they check, against a **real Hauk backend**, the **real GeoClue 2.7.0**, a real keyring
(gnome-keyring) and fakes for the notification server and logind:

| Scenario | Checks |
|---|---|
| `solo` | Check the GPS signal, share, see points arrive on the server (fields and timestamps), later points, stop through the confirmation dialog, the share is gone, the log hides the share ID and has no coordinates or password |
| `group` | Host a group share: nickname, PIN, host label, nickname reaches the server |
| `prefs` | Open Main Menu and Preferences; every control has an accessible name; the keyring password round-trips |
| `worstcase` | A hostile server (25 links with 80-character IDs, a 1000-character message): the window never changes size. Run with `EXPECT_SIZE` on the `phone`, `landscape`, `landscape-short` (720×288) and `narrow` (320×640) profiles to show the minimum size fits |
| `services` | The notification is resident, has the Copy and Stop actions, is replaced (not re-sent) as the signal changes, its Stop action ends the share and withdraws it; a logind sleep inhibitor is held while sharing and released after |
| `duration` | the duration is one row (a number and a unit dropdown named *Unit*), shows the last values, starts a share from them and saves them |
| `icons` | an icon theme the system lacks (as in a Flatpak whose host uses a theme the runtime does not have): the log shows the theme lacks some of the app's icons, and none is still missing thanks to the bundled ones |
| `network` | (a fake NetworkManager) the app listens to GLib's NetworkManager backend; the network going away shows at once and pauses posting; after a 20 s grace period the GPS goes off and the signal row says *Paused*, while the phone is still kept awake; a blip shorter than that leaves the GPS alone; when the network returns location restarts and posting resumes |
| `geoclue` | (`WITH_GEOCLUE=1`) the app gets a fix from the real GeoClue and the position reaches the server |
| `logging` | One dated file, nothing on disk for the first minute, flushed by the 60 s timer and on exit, the line format |
| `crash` | `kill -SEGV`: a minidump is written, the crash server logs it into the same file, the lines before it are flushed |

## One-time set-up

```sh
# A Crimson root file system (as root):
debootstrap --variant=minbase crimson /path/to/crimson-root https://repo.pureos.net/pureos/
# add crimson-updates and crimson-security to its /etc/apt/sources.list, then inside it:
apt-get install --no-install-recommends \
    build-essential pkg-config git curl ca-certificates gettext \
    libgtk-4-dev libadwaita-1-dev libsoup-3.0-dev libsecret-1-dev libgeoclue-2-dev \
    phoc grim wtype fonts-cantarell adwaita-icon-theme librsvg2-common dbus dbus-x11 \
    at-spi2-core gir1.2-atspi-2.0 gir1.2-secret-1 python3-gi gnome-keyring geoclue-2.0 \
    gsettings-desktop-schemas glib-networking
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
sh /src/scripts/ui-e2e/setup-geoclue.sh      # GeoClue: static source + allow Jastrzab
```

A Hauk backend on `http://127.0.0.1:8080/` with password `testpw` (`HAUK_URL`, `HAUK_PASSWORD`
override): copy `backend-php/` from the Hauk repository, make `include/config.php` from
`config-sample.php` with `"storage_backend" => REDIS`, a `password_hash` for `testpw` and
`"public_url" => 'http://127.0.0.1:8080/'`, then `redis-server` and `php -S 127.0.0.1:8080 -t backend-php`.

## Running

```sh
export CROOT=/path/to/crimson-root
scripts/ui-e2e/chroot.sh sh -c 'cd /src && cargo build -p jastrzab'
scripts/ui-e2e/chroot.sh sh /src/scripts/ui-e2e/session.sh phone sh /src/scripts/ui-e2e/scenario.sh solo
EXPECT_SIZE=720x288 scripts/ui-e2e/chroot.sh sh /src/scripts/ui-e2e/session.sh landscape-short \
    sh /src/scripts/ui-e2e/scenario.sh worstcase
WITH_GEOCLUE=1 scripts/ui-e2e/chroot.sh sh /src/scripts/ui-e2e/session.sh phone sh /src/scripts/ui-e2e/scenario.sh geoclue
```

Screenshots go to `/root/shots` in the chroot (`SHOT=` for `inspect.sh`). `inspect.sh` dumps the
whole accessibility tree and fails on any unnamed control.

## Limits

* A headless Phoc has no keyboard focus and AT-SPI cannot activate libadwaita combo rows or
  activatable list rows, so those are not clicked: debug builds read `JASTRZAB_FORM_MODE`
  (`alone`, `create-group`, `join-group`) to preselect the sharing mode, and `JASTRZAB_FAKE_LOCATION`
  can replace GeoClue (`lat,lon[,accuracies[,description]]`). Both exist only with
  `debug_assertions`.
* Nothing here runs Phosh itself: the real location agent, lock screen, on-screen keyboard and
  suspend behaviour need a device.
