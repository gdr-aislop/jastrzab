#!/bin/sh
# Runs inside the Crimson chroot, inside phoc and a D-Bus session: starts the app and dumps its
# accessibility tree.
export HOME=/root/e2e-home; rm -rf $HOME; mkdir -p $HOME/.config/io.github.gdr_aislop.jastrzab
printf 'server_url = "http://127.0.0.1:8080/"\n' > $HOME/.config/io.github.gdr_aislop.jastrzab/settings.toml
export JASTRZAB_FAKE_LOCATION="${JASTRZAB_FAKE_LOCATION:-52.52,13.40,3}"
export NO_AT_BRIDGE=0 GTK_A11Y=atspi
/root/target/debug/jastrzab > /root/e2e-app.log 2>&1 &
sleep 5
echo ready-for-shot; sleep 2
python3 /src/scripts/ui-e2e/a11y_dump.py jastrzab --all
status=$?
kill %1 2>/dev/null
exit $status
