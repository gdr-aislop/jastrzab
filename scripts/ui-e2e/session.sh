#!/bin/sh
# session.sh <profile> <command...>: run the command in a phoc inside a private D-Bus session (inside the
# Crimson chroot). A Secret Service (gnome-keyring, unlocked with an empty password) is started
# so the password round trip through libsecret is real.
#
# The session bus doubles as the "system" bus (DBUS_SYSTEM_BUS_ADDRESS), so the app's GeoClue and
# logind calls reach whatever this test provides there: the real GeoClue daemon when
# WITH_GEOCLUE=1, or the fakes in fakes.py.
exec dbus-run-session -- sh -c '
  eval "$(echo "" | gnome-keyring-daemon --unlock --start --components=secrets 2>/dev/null)"
  export GNOME_KEYRING_CONTROL SSH_AUTH_SOCK
  export DBUS_SYSTEM_BUS_ADDRESS="$DBUS_SESSION_BUS_ADDRESS"
  if [ "$WITH_GEOCLUE" = 1 ]; then
    /usr/libexec/geoclue > /root/geoclue.log 2>&1 &
    sleep 1
    # GeoClue holds every GetClient until the user'"'"'s location agent has registered (Phosh is
    # the agent on a phone); its demo agent stands in for it.
    /usr/libexec/geoclue-2.0/demos/agent > /root/geoclue-agent.log 2>&1 &
    sleep 2
  fi
  exec sh /src/scripts/ui-e2e/run_in_phoc.sh "$@"
' sh "$@"
