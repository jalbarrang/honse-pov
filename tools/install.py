#!/usr/bin/env python3
"""Install / reinstall honse_pov into a Hachimi Edge (Windows) install.

Copies the release DLL into <game>/hachimi and makes the two config edits the
plugin needs, minimally and idempotently:

  * append "hachimi\\honse_pov.dll" to load_libraries
  * set enable_file_logging to true (without it there is nothing to debug)

The config is patched textually so the diff stays one or two lines; it is
validated with json.loads before being written. The original is backed up to
config.json.bak the first time.

Usage:
    python tools/install.py [--game-dir DIR] [--dll PATH] [--no-logging]

If --game-dir is omitted the usual Steam locations are probed.
"""

from __future__ import annotations

import argparse
import json
import os
import shutil
import sys
from pathlib import Path

DEFAULT_CANDIDATES = [
    r"C:\Program Files (x86)\Steam\steamapps\common\UmamusumePrettyDerby",
    r"C:\Program Files\Steam\steamapps\common\UmamusumePrettyDerby",
]

PLUGIN_ENTRY = r"hachimi\honse_pov.dll"
HACHIMI_DIR = "hachimi"
CONFIG = "config.json"
BACKUP = "config.json.bak"


def find_game_dir() -> Path | None:
    for candidate in DEFAULT_CANDIDATES:
        path = Path(candidate)
        if (path / HACHIMI_DIR / CONFIG).is_file():
            return path
    return None


def patch_config(config_path: Path, enable_logging: bool) -> list[str]:
    """Returns a list of human-readable changes applied."""
    raw = config_path.read_text(encoding="utf-8")
    original = raw
    changes: list[str] = []

    # Validate before touching anything.
    cfg = json.loads(raw)

    backup = config_path.with_name(BACKUP)
    if not backup.exists():
        backup.write_text(raw, encoding="utf-8", newline="")
        changes.append(f"created backup {backup.name}")

    # --- load_libraries -----------------------------------------------------
    libs = list(cfg.get("load_libraries", []))
    if PLUGIN_ENTRY in libs:
        changes.append("load_libraries already contains the plugin")
    else:
        key = '"load_libraries"'
        i = raw.index(key)
        bracket = raw.index("[", raw.index(":", i))
        close = raw.index("]", bracket)

        inner = raw[bracket + 1 : close]
        entries = [e.strip() for e in inner.split(",") if e.strip()]
        entries.append('"' + PLUGIN_ENTRY.replace("\\", "\\\\") + '"')

        raw = raw[: bracket + 1] + "\n    " + ",\n    ".join(entries) + "\n  " + raw[close:]
        changes.append("registered plugin in load_libraries")

    # --- enable_file_logging ------------------------------------------------
    if enable_logging:
        if cfg.get("enable_file_logging") is True:
            changes.append("enable_file_logging already true")
        else:
            anchor = '"enable_file_logging": false'
            if anchor in raw:
                raw = raw.replace(anchor, '"enable_file_logging": true', 1)
                changes.append("enabled enable_file_logging")
            else:
                changes.append("WARNING: could not find enable_file_logging anchor")

    if raw != original:
        check = json.loads(raw)
        if PLUGIN_ENTRY not in check.get("load_libraries", []):
            raise SystemExit("refusing to write: load_libraries check failed")
        if enable_logging and check.get("enable_file_logging") is not True:
            raise SystemExit("refusing to write: enable_file_logging check failed")
        config_path.write_text(raw, encoding="utf-8", newline="")

    return changes


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--game-dir", type=Path, default=None)
    parser.add_argument(
        "--dll",
        type=Path,
        default=Path(__file__).resolve().parent.parent / "target" / "release" / "honse_pov.dll",
    )
    parser.add_argument("--no-logging", action="store_true")
    args = parser.parse_args()

    game_dir = args.game_dir or find_game_dir()
    if game_dir is None:
        print("Could not locate the game install. Pass --game-dir.", file=sys.stderr)
        return 1

    if not args.dll.is_file():
        print(f"Missing build artifact: {args.dll}\nRun: cargo build --release", file=sys.stderr)
        return 1

    hachimi_dir = game_dir / HACHIMI_DIR
    config_path = hachimi_dir / CONFIG
    if not config_path.is_file():
        print(f"No Hachimi config at {config_path}", file=sys.stderr)
        return 1

    target_dll = hachimi_dir / "honse_pov.dll"
    try:
        shutil.copyfile(args.dll, target_dll)
    except PermissionError:
        print(
            f"Cannot write {target_dll}\n"
            "The game is probably running and holding a lock on the DLL. Close it and retry.",
            file=sys.stderr,
        )
        return 1

    print(f"game dir : {game_dir}")
    print(f"plugin   : {target_dll} ({target_dll.stat().st_size} bytes)")

    for change in patch_config(config_path, enable_logging=not args.no_logging):
        print(f"config   : {change}")

    print(f"log file : {game_dir / 'hachimi.log'}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
