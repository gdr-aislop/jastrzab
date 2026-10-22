#!/bin/sh
# Run a command inside a headless phoc (PureOS Crimson chroot).
#   run_in_phoc.sh <profile> <cmd...>
# profiles:
#   phone      720x1440 @2  -> 360x720 logical  (Librem 5 portrait), auto-maximize on
#   phone-osk  720x880  @2  -> 360x440 logical  (portrait with on-screen keyboard up)
#   landscape  1440x720 @2  -> 720x360 logical
#   desktop    1920x1080 @1, auto-maximize off
# env: SHOT=<png path> takes a grim screenshot when the command prints "ready-for-shot"
#      PHOC_KEYFILE_EXTRA=<text> extra lines appended to the GSettings keyfile
set -e
profile=$1; shift

export XDG_RUNTIME_DIR=/tmp/xdg-$$
mkdir -p "$XDG_RUNTIME_DIR"; chmod 700 "$XDG_RUNTIME_DIR"
export XDG_CONFIG_HOME=$XDG_RUNTIME_DIR/config
mkdir -p "$XDG_CONFIG_HOME/glib-2.0/settings"
export GSETTINGS_BACKEND=keyfile
export WLR_BACKENDS=headless WLR_RENDERER=pixman WLR_HEADLESS_OUTPUTS=1 WLR_LIBINPUT_NO_DEVICES=1
export GDK_BACKEND=wayland LANG=C.UTF-8 NO_AT_BRIDGE=${NO_AT_BRIDGE:-1}
export GSK_RENDERER=cairo

automax=true
case $profile in
  phone)     mode=720x1440; scale=2 ;;
  phone-osk) mode=720x880;  scale=2 ;;
  phone-tall) mode=720x2600; scale=2 ;;
  landscape) mode=1440x720; scale=2 ;;
  desktop)   mode=1920x1080; scale=1; automax=false ;;
  *) echo "unknown profile $profile"; exit 2 ;;
esac

cat > "$XDG_CONFIG_HOME/glib-2.0/settings/keyfile" <<EOF
[sm/puri/phoc]
auto-maximize=$automax
${PHOC_KEYFILE_EXTRA}
EOF

cat > "$XDG_RUNTIME_DIR/phoc.ini" <<EOF
[core]
xwayland=false
[output:HEADLESS-1]
mode = $mode
scale = $scale
EOF

cmdfile=$XDG_RUNTIME_DIR/cmd.sh
log=$XDG_RUNTIME_DIR/app.log
{
  echo '#!/bin/sh'
  printf '%s ' "$@"; echo "> $log 2>&1 &"
  echo 'pid=$!'
  if [ -n "$SHOT" ]; then
    echo "until grep -q ready-for-shot $log || ! kill -0 \$pid 2>/dev/null; do sleep 0.2; done"
    echo "sleep 1; grim '$SHOT'"
  fi
  echo 'wait $pid'
  echo 'for p in /proc/[0-9]*; do [ "$(cat $p/comm 2>/dev/null)" = phoc ] && kill ${p#/proc/}; done'
} > "$cmdfile"
chmod +x "$cmdfile"

timeout 60 phoc -C "$XDG_RUNTIME_DIR/phoc.ini" -E "$cmdfile" > "$XDG_RUNTIME_DIR/phoc.log" 2>&1 || true
cat "$log"
grep -i "error\|critical" "$XDG_RUNTIME_DIR/phoc.log" | grep -v "Failed to open\|libinput\|xkb" | head -5 || true
rm -rf "$XDG_RUNTIME_DIR"
