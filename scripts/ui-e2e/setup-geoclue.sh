#!/bin/sh
# Configures the real GeoClue (PureOS Crimson's geoclue-2.0 2.7.0) in the test chroot for the UI
# checks: only the static source is on, at a fixed position, and Jastrzab is allowed without a
# location agent (Phosh is the agent on a phone). Run once inside the chroot.
set -e
mkdir -p /etc/geoclue/conf.d
cat > /etc/geoclue/conf.d/90-jastrzab-test.conf <<CONF
[network-nmea]
enable=false
[3g]
enable=false
[cdma]
enable=false
[modem-gps]
enable=false
[wifi]
enable=false
[compass]
enable=false
[static-source]
enable=true
[io.github.gdr_aislop.jastrzab]
allowed=true
system=true
users=
CONF
# latitude, longitude, altitude, accuracy: one per line.
printf '52.5200\n13.4050\n34.0\n3.0\n' > /etc/geolocation
