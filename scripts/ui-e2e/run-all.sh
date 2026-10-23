#!/bin/sh
# run-all.sh: every headless UI scenario, one after the other. Run inside the Crimson chroot with
# the debug binary built (`cargo build -p jastrzab`) and a Hauk backend up (see README.md).
# Exits non-zero if any scenario fails.
cd /src/scripts/ui-e2e || exit 2
failed=0
run() { # run <label> <profile> <scenario> [env assignments...]
    label=$1; profile=$2; scenario=$3; shift 3
    echo "=== $label"
    if env "$@" sh /src/scripts/ui-e2e/session.sh "$profile" sh /src/scripts/ui-e2e/scenario.sh "$scenario" 2>&1 \
        | grep -v "dbus-daemon\|SpiRegistry\|wlr_renderer" | tee /tmp/ui-e2e-last.log | grep -E "^(ok|FAIL)|failure"; then :; fi
    grep -q "^0 failure" /tmp/ui-e2e-last.log || failed=$((failed + 1))
}
run "solo"                  phone           solo
run "group"                 phone           group
run "preferences"           phone           prefs
run "resume after a kill"   phone           resume
run "services"              phone           services
run "logging"               phone           logging
run "crash"                 phone           crash
run "hostile server, phone"           phone           worstcase EXPECT_SIZE=360x720
run "hostile server, landscape"       landscape       worstcase EXPECT_SIZE=720x360
run "hostile server, 720x288"         landscape-short worstcase EXPECT_SIZE=720x288
run "hostile server, 320x640"         narrow          worstcase EXPECT_SIZE=320x640
run "hostile server, text at 1.5x"    phone           worstcase EXPECT_SIZE=360x720 "PHOC_KEYFILE_EXTRA=
[org/gnome/desktop/interface]
text-scaling-factor=1.5"
run "unknown icon theme"    phone           icons "PHOC_KEYFILE_EXTRA=
[org/gnome/desktop/interface]
icon-theme='nonexistent'"
run "network gone and back"  phone           network
run "real GeoClue"          phone           geoclue WITH_GEOCLUE=1
echo
echo "$failed scenario(s) failed"
exit "$failed"
