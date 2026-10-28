#!/usr/bin/env python3
"""Battersea's own release steps, which `release.json` lists for `tools/release.py` to run."""

import argparse
import json
import sys
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parent.parent
TYPESCRIPT = ROOT / "typescript"
DEPENDENCIES = ("dependencies", "devDependencies", "peerDependencies", "optionalDependencies")


def read(path: Path) -> Any:
    return json.loads(path.read_text(encoding="utf-8"))


def write(path: Path, value: Any) -> None:
    path.write_text(json.dumps(value, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")


def move(package: dict[str, Any], names: set[str], previous: str, version: str) -> None:
    """Move `package`'s version and its dependencies on the workspace's packages from `previous` to `version`."""
    package["version"] = version
    for kind in DEPENDENCIES:
        for name, requirement in package.get(kind, {}).items():
            if name in names and requirement.lstrip("=^~") == previous:
                package[kind][name] = requirement[: len(requirement) - len(previous)] + version


def npm_version(arguments: argparse.Namespace) -> None:
    """Set the npm workspace, its packages, their dependencies on each other and the lockfile to VERSION."""
    previous, version = arguments.previous, arguments.version
    paths = {"": TYPESCRIPT / "package.json"} | {
        f"packages/{path.parent.name}": path for path in sorted(TYPESCRIPT.glob("packages/*/package.json"))
    }
    manifests = {key: read(path) for key, path in paths.items()}
    stale = [
        str(paths[key].relative_to(ROOT)) for key, manifest in manifests.items() if manifest["version"] != previous
    ]
    if stale:
        sys.exit(f"release steps: {', '.join(stale)} not at {previous}")
    names = {manifest["name"] for key, manifest in manifests.items() if key}
    for key, manifest in manifests.items():
        move(manifest, names, previous, version)
        write(paths[key], manifest)
    lock_path = TYPESCRIPT / "package-lock.json"
    lock = read(lock_path)
    lock["version"] = version
    for key in manifests:
        move(lock["packages"][key], names, previous, version)
    write(lock_path, lock)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(required=True, metavar="COMMAND")
    versioning = commands.add_parser("npm-version", help=npm_version.__doc__)
    versioning.add_argument("previous")
    versioning.add_argument("version")
    versioning.set_defaults(run=npm_version)
    arguments = parser.parse_args()
    arguments.run(arguments)


if __name__ == "__main__":
    main()
