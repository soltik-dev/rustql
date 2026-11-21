#!/usr/bin/env python3
"""Release helper: keeps the version in Cargo.toml, Cargo.lock, CHANGELOG.md
and the AppStream metainfo in step.

    scripts/release.py prepare 0.9.0 [--date 2026-12-01]
        Sets the version everywhere and turns the Unreleased section of the
        changelog into the notes of that version (also in the metainfo).
    scripts/release.py state
        Whether the current version is a prepared release (used by CI). Fails
        if the files disagree.
    scripts/release.py notes 0.9.0
        The changelog section of a version, as Markdown.
    scripts/release.py manifest 0.9.0 DIR
        Writes DIR/SHA256SUMS and DIR/release.json for the files in DIR.

Only the standard library: it runs on any CI image and locally.
"""

import argparse
import datetime
import hashlib
import html
import json
import re
import sys
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CARGO_TOML = ROOT / "Cargo.toml"
CARGO_LOCK = ROOT / "Cargo.lock"
CHANGELOG = ROOT / "CHANGELOG.md"
METAINFO = ROOT / "packaging/linux/com.rustql.RustQL.metainfo.xml"
REPO = "https://github.com/soltik-dev/rustql"

VERSION_RE = re.compile(r"\d+\.\d+\.\d+")
SECTION_RE = re.compile(r"^## \[(?P<version>[^\]]+)\](?: - (?P<date>\d{4}-\d{2}-\d{2}))?\s*$", re.M)
METAINFO_RELEASE_RE = re.compile(r'<release version="(?P<version>[^"]+)" date="(?P<date>[^"]+)"')


class ReleaseError(Exception):
    pass


# ── Reading the versions ──────────────────────────────────────────────────────


def cargo_version() -> str:
    return tomllib.loads(CARGO_TOML.read_text())["package"]["version"]


def lock_version() -> str:
    lock = tomllib.loads(CARGO_LOCK.read_text())
    return next(p["version"] for p in lock["package"] if p["name"] == "rustql")


def changelog_sections(text: str) -> list[dict]:
    """Sections of the changelog in order: version, date and body."""
    matches = list(SECTION_RE.finditer(text))
    sections = []
    for i, m in enumerate(matches):
        end = matches[i + 1].start() if i + 1 < len(matches) else len(text)
        body = text[m.end():end]
        # Link references at the end of the file are not part of the last section
        body = re.split(r"^\[[^\]]+\]: ", body, maxsplit=1, flags=re.M)[0]
        sections.append({"version": m["version"], "date": m["date"], "body": body.strip()})
    return sections


def changelog_latest() -> dict | None:
    """The newest released section (not Unreleased)."""
    return next((s for s in changelog_sections(CHANGELOG.read_text()) if s["version"] != "Unreleased"), None)


def metainfo_latest() -> dict | None:
    m = METAINFO_RELEASE_RE.search(METAINFO.read_text())
    return m and {"version": m["version"], "date": m["date"]}


def state() -> dict:
    """Whether the version in Cargo.toml is a prepared release. Raises if the
    files only half agree (a release prepared by hand and left incomplete)."""
    version = cargo_version()
    changelog = changelog_latest()
    metainfo = metainfo_latest()
    found = {
        "Cargo.lock": lock_version(),
        "CHANGELOG.md": changelog and changelog["version"],
        "metainfo": metainfo and metainfo["version"],
    }
    if found["Cargo.lock"] != version:
        raise ReleaseError(f"Cargo.lock has {found['Cargo.lock']}, Cargo.toml has {version}")
    prepared = [name for name in ("CHANGELOG.md", "metainfo") if found[name] == version]
    if len(prepared) == 1:
        missing = "metainfo" if prepared == ["CHANGELOG.md"] else "CHANGELOG.md"
        raise ReleaseError(f"{version} is in {prepared[0]} but not in {missing} (found {found[missing]})")
    if prepared and changelog["date"] != metainfo["date"]:
        raise ReleaseError(f"release dates differ: CHANGELOG {changelog['date']}, metainfo {metainfo['date']}")
    return {"version": version, "prepared": bool(prepared), "date": changelog["date"] if prepared else None}


def notes(version: str) -> str:
    for section in changelog_sections(CHANGELOG.read_text()):
        if section["version"] == version:
            return section["body"]
    raise ReleaseError(f"{version} is not in CHANGELOG.md")


# ── prepare ───────────────────────────────────────────────────────────────────


def markdown_to_appstream(md: str, indent: str) -> str:
    """The subset of Markdown the changelog uses (paragraphs, ### headings,
    `-` lists with wrapped lines) as an AppStream description."""
    out, items, para = [], [], []

    def text(lines):
        joined = " ".join(l.strip() for l in lines)
        return html.escape(re.sub(r"`([^`]*)`", r"\1", joined), quote=False)

    def flush():
        if para:
            out.append(f"<p>{text(para)}</p>")
            para.clear()
        if items:
            out.append("<ul>")
            out.extend(f"  <li>{text(item)}</li>" for item in items)
            out.append("</ul>")
            items.clear()

    for line in md.splitlines():
        if not line.strip():
            flush()
        elif line.startswith("### "):
            flush()
            out.append(f"<p>{text([line[4:]])}:</p>")
        elif line.startswith("- "):
            if para:
                flush()
            items.append([line[2:]])
        elif items and line.startswith("  "):
            items[-1].append(line)
        else:
            if items:
                flush()
            para.append(line)
    flush()
    return "\n".join(indent + l for l in out)


def prepare(version: str, date: str) -> None:
    if not VERSION_RE.fullmatch(version):
        raise ReleaseError(f"{version}: expected X.Y.Z (no pre-release suffix: RPM and MSI cannot order them)")
    current = state()
    if current["prepared"] and current["version"] == version:
        raise ReleaseError(f"{version} is already prepared")

    # CHANGELOG: Unreleased becomes the version, and a new empty Unreleased goes on top
    text = CHANGELOG.read_text()
    sections = changelog_sections(text)
    if any(s["version"] == version for s in sections):
        raise ReleaseError(f"{version} is already in CHANGELOG.md")
    unreleased = next((s for s in sections if s["version"] == "Unreleased"), None)
    if not unreleased or not unreleased["body"]:
        raise ReleaseError("CHANGELOG.md has no Unreleased notes to release")
    text = text.replace("## [Unreleased]", f"## [Unreleased]\n\n## [{version}] - {date}", 1)
    previous = next((s["version"] for s in sections if s["version"] != "Unreleased"), None)
    links = re.compile(r"^\[Unreleased\]: .*$", re.M)
    new_links = f"[Unreleased]: {REPO}/compare/v{version}...HEAD\n" + (
        f"[{version}]: {REPO}/compare/v{previous}...v{version}" if previous else f"[{version}]: {REPO}/releases/tag/v{version}"
    )
    text = links.sub(new_links, text, count=1) if links.search(text) else text.rstrip() + "\n\n" + new_links + "\n"
    CHANGELOG.write_text(text)

    # Cargo.toml and Cargo.lock
    toml = CARGO_TOML.read_text()
    CARGO_TOML.write_text(re.sub(r'(?m)^version = "[^"]+"', f'version = "{version}"', toml, count=1))
    lock = CARGO_LOCK.read_text()
    CARGO_LOCK.write_text(re.sub(r'(name = "rustql"\nversion = )"[^"]+"', rf'\1"{version}"', lock, count=1))

    # metainfo: the newest release goes first
    meta = METAINFO.read_text()
    m = re.search(r"^(?P<indent>[ \t]*)<releases>[ \t]*\n", meta, re.M)
    if not m:
        raise ReleaseError("no <releases> element in the metainfo")
    ind = m["indent"]
    release = (
        f'{ind}  <release version="{version}" date="{date}">\n'
        f"{ind}    <description>\n"
        f"{markdown_to_appstream(unreleased['body'], ind + '      ')}\n"
        f"{ind}    </description>\n"
        f'{ind}    <url type="details">{REPO}/releases/tag/v{version}</url>\n'
        f"{ind}  </release>\n"
    )
    METAINFO.write_text(meta[: m.end()] + release + meta[m.end():])

    after = state()
    assert after["prepared"] and after["version"] == version, after


# ── manifest ──────────────────────────────────────────────────────────────────

# File name → what it is, for the website and the in-app update notice
ASSET_KINDS = [
    (r"-setup\.exe$", "windows", "x86_64", "installer"),
    (r"-windows-x86_64\.zip$", "windows", "x86_64", "portable"),
    (r"-macos-universal\.dmg$", "macos", "universal", "disk-image"),
    (r"-x86_64\.AppImage$", "linux", "x86_64", "appimage"),
    (r"_amd64\.deb$", "linux", "x86_64", "deb"),
    (r"-linux-x86_64\.tar\.gz$", "linux", "x86_64", "tarball"),
    (r"-vendored\.tar\.xz$", "source", None, "source"),
]


def manifest(version: str, directory: Path) -> None:
    files = sorted(p for p in directory.iterdir() if p.is_file() and p.name not in ("SHA256SUMS", "release.json"))
    if not files:
        raise ReleaseError(f"no files in {directory}")
    sums, assets = [], []
    for path in files:
        digest = hashlib.sha256(path.read_bytes()).hexdigest()
        sums.append(f"{digest}  {path.name}")
        kind = next(((os_, arch, k) for pattern, os_, arch, k in ASSET_KINDS if re.search(pattern, path.name)), None)
        if kind is None:
            raise ReleaseError(f"unknown release file {path.name}: add it to ASSET_KINDS")
        assets.append({
            "name": path.name,
            "os": kind[0],
            "arch": kind[1],
            "kind": kind[2],
            "size": path.stat().st_size,
            "sha256": digest,
            "url": f"{REPO}/releases/download/v{version}/{path.name}",
        })
    (directory / "SHA256SUMS").write_text("\n".join(sums) + "\n")
    current = state()
    if current["version"] != version:
        raise ReleaseError(f"Cargo.toml is at {current['version']}, not {version}")
    release = {
        "version": version,
        "date": current["date"],
        "url": f"{REPO}/releases/tag/v{version}",
        # A manual test run of an unprepared version has no notes yet
        "notes": notes(version) if current["prepared"] else "",
        "assets": assets,
    }
    (directory / "release.json").write_text(json.dumps(release, indent=2, ensure_ascii=False) + "\n")


# ── Command line ──────────────────────────────────────────────────────────────


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest="command", required=True)
    p = sub.add_parser("prepare")
    p.add_argument("version")
    p.add_argument("--date", default=datetime.date.today().isoformat())
    sub.add_parser("state")
    p = sub.add_parser("notes")
    p.add_argument("version")
    p = sub.add_parser("manifest")
    p.add_argument("version")
    p.add_argument("directory", type=Path)
    args = parser.parse_args()
    try:
        if args.command == "prepare":
            prepare(args.version, args.date)
            print(f"Prepared {args.version}: review the diff and open a PR")
        elif args.command == "state":
            s = state()
            print(f"version={s['version']}")
            print(f"prepared={'true' if s['prepared'] else 'false'}")
        elif args.command == "notes":
            print(notes(args.version))
        elif args.command == "manifest":
            manifest(args.version, args.directory)
    except ReleaseError as e:
        print(f"error: {e}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
