#!/usr/bin/env bash
# Observans — Linux Installer
# Usage: curl -fsSL https://github.com/<owner>/<repo>/releases/latest/download/install.sh | bash

set -euo pipefail

REPO="${OBSERVANS_REPO:-Yura1686/Observans}"
APP="observans"
INSTALL_DIR="${OBSERVANS_INSTALL_DIR:-$HOME/.local/opt/observans}"
BIN_DIR="${OBSERVANS_BIN_DIR:-$HOME/.local/bin}"
LAUNCHER="$BIN_DIR/$APP"

# BEGIN AUTO-GENERATED: release_manifest
ARTIFACT_LINUX_X64="Observans-linux-x64.tar.gz"
FFMPEG_BIN_REL="ffmpeg/bin/ffmpeg"
# END AUTO-GENERATED: release_manifest

RUNTIME_DIR="$INSTALL_DIR/_observans_runtime"
FFMPEG_BIN="$RUNTIME_DIR/$FFMPEG_BIN_REL"
DEFAULT_BASE_URL="https://github.com/${REPO}/releases/latest/download"
BASE_URL="${OBSERVANS_RELEASE_BASE_URL:-$DEFAULT_BASE_URL}"
WIDTH=66

_banner() {
  printf "\n  +%s+\n" "$(printf '=%.0s' $(seq 1 $WIDTH))"
  printf "  |%s|\n" "$(printf '%*s' $(( (${#1} + WIDTH) / 2 )) "$1" | printf '%-*s' $WIDTH "$(cat)")"
  printf "  +%s+\n\n" "$(printf '=%.0s' $(seq 1 $WIDTH))"
}

_section() {
  local dashes=$(( WIDTH - 6 - ${#1} ))
  printf "\n  +--[ %s ]%s+\n" "$1" "$(printf -- '-%.0s' $(seq 1 $dashes))"
}

_info()  { printf "  [....] %-6s  %s\n" "$1" "$2"; }
_ok()    { printf "  [++++] %-6s  %s\n" "$1" "$2"; }
_wait()  { printf "  [~~~~] %-6s  %s\n" "$1" "$2"; }
_warn()  { printf "  [!!!!] %-6s  %s\n" "$1" "$2"; }
_fatal() { printf "  [XXXX] %-6s  %s\n" "$1" "$2"; exit 1; }

_banner "OBSERVANS  —  Linux Installer"

_section "DETECT"

ARCH="$(uname -m)"
case "$ARCH" in
  x86_64|amd64) ARTIFACT="$ARTIFACT_LINUX_X64" ;;
  *) _fatal "SYS" "Unsupported architecture: $ARCH" ;;
esac

_ok "SYS" "Architecture : $ARCH"
_ok "SYS" "Artifact     : $ARTIFACT"

_section "DOWNLOAD"

TMPDIR_INST="$(mktemp -d)"
trap 'rm -rf "$TMPDIR_INST"' EXIT

ARCHIVE="$TMPDIR_INST/$ARTIFACT"
SHA_FILE="$TMPDIR_INST/$ARTIFACT.sha256"

_wait "NET" "Downloading $ARTIFACT ..."
if command -v curl &>/dev/null; then
  curl -fsSL --progress-bar "$BASE_URL/$ARTIFACT" -o "$ARCHIVE"
  curl -fsSL "$BASE_URL/$ARTIFACT.sha256" -o "$SHA_FILE"
elif command -v wget &>/dev/null; then
  wget -q --show-progress "$BASE_URL/$ARTIFACT" -O "$ARCHIVE"
  wget -q "$BASE_URL/$ARTIFACT.sha256" -O "$SHA_FILE"
else
  _fatal "NET" "curl or wget required"
fi
_ok "NET" "Download complete"

_section "VERIFY"

_wait "SYS" "Verifying SHA-256 ..."
(
  cd "$TMPDIR_INST"
  sha256sum -c "$SHA_FILE" --status 2>/dev/null
) || _fatal "SYS" "Checksum mismatch — download may be corrupted"
_ok "SYS" "Checksum OK"

_section "INSTALL"

_info "SYS" "Removing old installation ..."
rm -rf "$INSTALL_DIR"
mkdir -p "$INSTALL_DIR" "$BIN_DIR"

_info "SYS" "Extracting to $INSTALL_DIR ..."
tar -xzf "$ARCHIVE" -C "$TMPDIR_INST"
EXTRACTED="$(find "$TMPDIR_INST" -maxdepth 1 -type d -name "Observans-*" | head -1)"
cp -a "$EXTRACTED/." "$INSTALL_DIR/"
chmod +x "$INSTALL_DIR/$APP"

_info "SYS" "Creating launcher at $LAUNCHER ..."
cat > "$LAUNCHER" <<EOF
#!/usr/bin/env bash
exec "$INSTALL_DIR/$APP" "\$@"
EOF
chmod +x "$LAUNCHER"

_ok "SYS" "Installed to   : $INSTALL_DIR"
_ok "SYS" "Launcher       : $LAUNCHER"

_section "FFMPEG"

if [ -x "$FFMPEG_BIN" ]; then
  _ok "SYS" "Bundled FFmpeg present: $FFMPEG_BIN"
else
  _fatal "SYS" "Bundled FFmpeg is missing at $FFMPEG_BIN (release may be incomplete)"
fi

_section "DONE"

if echo ":$PATH:" | grep -q ":$BIN_DIR:"; then
  _ok "SYS" "Run with: observans"
else
  _warn "SYS" "$BIN_DIR is not in PATH"
  _info "SYS" "Add to ~/.bashrc or ~/.zshrc:"
  printf "\n    export PATH=\"\$HOME/.local/bin:\$PATH\"\n\n"
fi
