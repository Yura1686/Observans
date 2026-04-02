"""
tools/sync_installers.py — keep install.sh aligned with release_manifest.json.
"""

from __future__ import annotations

import argparse
import json
from dataclasses import dataclass
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent
MANIFEST_PATH = ROOT / "tools" / "release_manifest.json"
INSTALL_SH = ROOT / "install.sh"


@dataclass(frozen=True)
class LinuxRelease:
    artifact_name: str
    ffmpeg_relative_path: str


def load_linux_release() -> LinuxRelease:
    payload = json.loads(MANIFEST_PATH.read_text(encoding="utf-8"))
    cfg = payload["targets"]["linux-x64"]
    return LinuxRelease(
        artifact_name=cfg["artifact_name"],
        ffmpeg_relative_path="ffmpeg/bin/ffmpeg",
    )


def render_manifest_block(target: LinuxRelease) -> str:
    start = "# BEGIN AUTO-GENERATED: release_manifest\n"
    end = "# END AUTO-GENERATED: release_manifest\n"
    return (
        start
        + f'ARTIFACT_LINUX_X64="{target.artifact_name}"\n'
        + f'FFMPEG_BIN_REL="{target.ffmpeg_relative_path}"\n'
        + end
    )


def replace_manifest_block(text: str, replacement: str) -> str:
    start = "# BEGIN AUTO-GENERATED: release_manifest\n"
    end = "# END AUTO-GENERATED: release_manifest\n"
    start_index = text.find(start)
    end_index = text.find(end)
    if start_index == -1 or end_index == -1 or end_index < start_index:
        raise ValueError("install.sh is missing manifest sync markers")
    end_index += len(end)
    return text[:start_index] + replacement + text[end_index:]


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--check", action="store_true", help="verify install.sh without rewriting it")
    args = parser.parse_args()

    target = load_linux_release()
    replacement = render_manifest_block(target)
    current = INSTALL_SH.read_text(encoding="utf-8")
    synced = replace_manifest_block(current, replacement)

    if args.check:
        if current != synced:
            raise SystemExit("install.sh is out of sync with tools/release_manifest.json")
        print("OK: install.sh is in sync")
        return

    INSTALL_SH.write_text(synced, encoding="utf-8")
    print("OK: install.sh synced with release_manifest.json")


if __name__ == "__main__":
    main()

