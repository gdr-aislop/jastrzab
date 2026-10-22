#!/bin/sh
# Runs one drive.py scenario inside phoc (called through session.sh).
export NO_AT_BRIDGE=0 GTK_A11Y=atspi
exec python3 /src/scripts/ui-e2e/drive.py "$@"
