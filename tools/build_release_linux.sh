#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MANIFEST_PATH="$ROOT_DIR/tools/release_manifest.json"
TARGET_ID="${1:-linux-x64}"
DIST_DIR="${OBSERVANS_DIST_DIR:-$ROOT_DIR/dist}"
WORK_DIR="${OBSERVANS_WORK_DIR:-$ROOT_DIR/.release-work/$TARGET_ID}"

require_cmd() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "missing required command: $1" >&2
    exit 1
  }
}

read_manifest() {
  eval "$(
    python3 - "$MANIFEST_PATH" "$TARGET_ID" <<'PY'
import json
import shlex
import sys

manifest_path, target_id = sys.argv[1], sys.argv[2]
payload = json.load(open(manifest_path, encoding="utf-8"))
target = payload["targets"][target_id]
source = payload["ffmpeg_sources"][target["ffmpeg_source"]]
pairs = {
    "DISPLAY_NAME": payload["display_name"],
    "BINARY_NAME": payload["binary_name"],
    "TARGET_OS": target["os"],
    "RUST_TARGET": target["rust_target"],
    "ARTIFACT_NAME": target["artifact_name"],
    "ARCHIVE_FORMAT": target["archive_format"],
    "BUNDLE_DIR": target["bundle_dir"],
    "ENTRY_EXECUTABLE": target["entry_executable"],
    "LAUNCHER_KIND": target["launcher_kind"],
    "FFMPEG_SOURCE_ID": target["ffmpeg_source"],
    "FFMPEG_ASSET": target["ffmpeg_asset"],
    "FFMPEG_SHA256": target["ffmpeg_sha256"],
    "FFMPEG_BASE_URL": source["base_url"],
    "FFMPEG_CHECKSUMS_ASSET": source["checksums_asset"],
}
for key, value in pairs.items():
    print(f"{key}={shlex.quote(str(value))}")
PY
  )"
}

download_file() {
  local url="$1"
  local destination="$2"

  if command -v curl >/dev/null 2>&1; then
    curl -fsSL --retry 3 "$url" -o "$destination"
  else
    wget -q "$url" -O "$destination"
  fi
}

resolve_checksum() {
  local checksums_file="$1"
  local asset_name="$2"
  local configured="$3"

  if [[ "$configured" != "auto" ]]; then
    printf '%s' "$configured"
    return
  fi

  python3 - "$checksums_file" "$asset_name" <<'PY'
import sys

checksums_path, asset_name = sys.argv[1], sys.argv[2]
with open(checksums_path, encoding="utf-8", errors="replace") as handle:
    for line in handle:
        parts = line.strip().split()
        if len(parts) >= 2 and parts[-1] == asset_name:
            print(parts[0])
            raise SystemExit(0)
raise SystemExit(f"checksum for {asset_name} not found")
PY
}

write_build_meta() {
  local destination="$1"
  local git_commit
  git_commit="$(git -C "$ROOT_DIR" rev-parse --short HEAD 2>/dev/null || printf 'unknown')"
  local built_at
  built_at="$(date -u +"%Y-%m-%dT%H:%M:%SZ")"

  cat >"$destination" <<EOF
{
  "display_name": "$DISPLAY_NAME",
  "binary_name": "$BINARY_NAME",
  "target_id": "$TARGET_ID",
  "target_os": "$TARGET_OS",
  "rust_target": "$RUST_TARGET",
  "artifact_name": "$ARTIFACT_NAME",
  "launcher_kind": "$LAUNCHER_KIND",
  "ffmpeg_source": "$FFMPEG_SOURCE_ID",
  "ffmpeg_asset": "$FFMPEG_ASSET",
  "git_commit": "$git_commit",
  "built_at": "$built_at"
}
EOF
}

write_linux_launcher() {
  local destination="$1"
  local target_binary="$2"

  cat >"$destination" <<EOF
#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="\$(cd "\$(dirname "\${BASH_SOURCE[0]}")" && pwd)"
cd "\$SCRIPT_DIR"
exec "\$SCRIPT_DIR/$target_binary" "\$@"
EOF
  chmod +x "$destination"
}

resolve_release_repository() {
  if [[ -n "${OBSERVANS_RELEASE_REPOSITORY:-}" ]]; then
    printf '%s' "$OBSERVANS_RELEASE_REPOSITORY"
    return
  fi

  if [[ -n "${GITHUB_REPOSITORY:-}" ]]; then
    printf '%s' "$GITHUB_REPOSITORY"
    return
  fi

  local remote_url
  remote_url="$(git -C "$ROOT_DIR" config --get remote.origin.url 2>/dev/null || true)"
  if [[ -n "$remote_url" ]]; then
    local parsed_repo
    parsed_repo="$(python3 - "$remote_url" <<'PY'
import re
import sys

remote = sys.argv[1].strip()
match = re.search(r'github\.com[:/](?P<slug>[^/]+/[^/.]+?)(?:\.git)?$', remote)
if match:
    print(match.group("slug"))
PY
)"
    if [[ -n "$parsed_repo" ]]; then
      printf '%s' "$parsed_repo"
      return
    fi
  fi

  printf '%s' 'Yura1686/Observans'
}

write_release_installer() {
  local destination="$1"
  local release_repo="$2"

  python3 - "$ROOT_DIR/install.sh" "$destination" "$release_repo" <<'PY'
from pathlib import Path
import sys

source, destination, repo = sys.argv[1], sys.argv[2], sys.argv[3]
text = Path(source).read_text(encoding="utf-8")
text = text.replace('REPO="${OBSERVANS_REPO:-Yura1686/Observans}"', f'REPO="${{OBSERVANS_REPO:-{repo}}}"')
Path(destination).write_text(text, encoding="utf-8")
PY
  chmod +x "$destination"
}

main() {
  require_cmd cargo
  require_cmd python3
  require_cmd tar
  require_cmd sha256sum
  if ! command -v curl >/dev/null 2>&1 && ! command -v wget >/dev/null 2>&1; then
    echo "curl or wget is required" >&2
    exit 1
  fi

  read_manifest

  if [[ "$TARGET_OS" != "linux" ]]; then
    echo "target $TARGET_ID is not a linux target" >&2
    exit 1
  fi

  mkdir -p "$DIST_DIR" "$WORK_DIR/downloads" "$WORK_DIR/extracted"

  if command -v rustup >/dev/null 2>&1; then
    rustup target add "$RUST_TARGET" >/dev/null
  fi

  export CC_x86_64_unknown_linux_musl="${CC_x86_64_unknown_linux_musl:-musl-gcc}"

  cargo build --release --target "$RUST_TARGET"

  local bundle_dir="$DIST_DIR/$BUNDLE_DIR"
  local runtime_dir="$bundle_dir/_observans_runtime"
  local runtime_bin_dir="$runtime_dir/bin"
  local bundle_ffmpeg_dir="$runtime_dir/ffmpeg/bin"
  local build_meta="$runtime_dir/build_meta.json"
  local binary_src="$ROOT_DIR/target/$RUST_TARGET/release/$BINARY_NAME"
  local archive_path="$DIST_DIR/$ARTIFACT_NAME"
  local checksum_path="$archive_path.sha256"
  local ffmpeg_archive="$WORK_DIR/downloads/$FFMPEG_ASSET"
  local ffmpeg_checksums="$WORK_DIR/downloads/$FFMPEG_CHECKSUMS_ASSET"
  local extracted_dir="$WORK_DIR/extracted/$TARGET_ID"
  local release_repo
  release_repo="$(resolve_release_repository)"

  rm -rf "$bundle_dir" "$extracted_dir"
  mkdir -p "$bundle_ffmpeg_dir" "$runtime_bin_dir" "$extracted_dir"

  cp "$binary_src" "$runtime_bin_dir/$ENTRY_EXECUTABLE"
  chmod +x "$runtime_bin_dir/$ENTRY_EXECUTABLE"
  cp "$ROOT_DIR/RELEASE_README.md" "$bundle_dir/README.md"
  case "$LAUNCHER_KIND" in
    shell)
      write_linux_launcher "$bundle_dir/$DISPLAY_NAME.sh" "_observans_runtime/bin/$ENTRY_EXECUTABLE"
      ;;
    none)
      ;;
    *)
      echo "unsupported launcher kind for $TARGET_ID: $LAUNCHER_KIND" >&2
      exit 1
      ;;
  esac

  download_file "$FFMPEG_BASE_URL/$FFMPEG_ASSET" "$ffmpeg_archive"
  download_file "$FFMPEG_BASE_URL/$FFMPEG_CHECKSUMS_ASSET" "$ffmpeg_checksums"
  local expected_checksum
  expected_checksum="$(resolve_checksum "$ffmpeg_checksums" "$FFMPEG_ASSET" "$FFMPEG_SHA256")"
  printf '%s  %s\n' "$expected_checksum" "$ffmpeg_archive" | sha256sum -c -

  tar -xf "$ffmpeg_archive" -C "$extracted_dir"
  local ffmpeg_path
  ffmpeg_path="$(find "$extracted_dir" -type f -name ffmpeg | head -n 1)"
  if [[ -z "$ffmpeg_path" ]]; then
    echo "ffmpeg executable not found in $FFMPEG_ASSET" >&2
    exit 1
  fi

  cp -a "$(dirname "$ffmpeg_path")/." "$bundle_ffmpeg_dir/"
  chmod +x "$bundle_ffmpeg_dir/ffmpeg"
  write_build_meta "$build_meta"

  rm -f "$archive_path" "$checksum_path"
  tar -C "$DIST_DIR" -czf "$archive_path" "$BUNDLE_DIR"
  printf '%s  %s\n' "$(sha256sum "$archive_path" | awk '{print $1}')" "$ARTIFACT_NAME" > "$checksum_path"

  write_release_installer "$DIST_DIR/install.sh" "$release_repo"
  cp "$ROOT_DIR/uninstall.sh" "$DIST_DIR/uninstall.sh"
  chmod +x "$DIST_DIR/uninstall.sh"

  echo "built $archive_path"
}

main "$@"
