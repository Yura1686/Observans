#!/usr/bin/env bash
# Observans — Linux Uninstaller

set -euo pipefail

INSTALL_DIR="${OBSERVANS_INSTALL_DIR:-$HOME/.local/opt/observans}"
BIN_DIR="${OBSERVANS_BIN_DIR:-$HOME/.local/bin}"
LAUNCHER="$BIN_DIR/observans"

_ok()   { printf "  [++++] SYS     %s\n" "$1"; }
_info() { printf "  [....] SYS     %s\n" "$1"; }

printf "\n  +================================================================+\n"
printf "  |              OBSERVANS  —  Linux Uninstaller                   |\n"
printf "  +================================================================+\n\n"

_info "Removing $INSTALL_DIR ..."
rm -rf "$INSTALL_DIR"
_ok "Removed: $INSTALL_DIR"

if [ -f "$LAUNCHER" ]; then
  _info "Removing launcher $LAUNCHER ..."
  rm -f "$LAUNCHER"
  _ok "Removed: $LAUNCHER"
fi

_ok "Observans uninstalled successfully"
printf "\n"

