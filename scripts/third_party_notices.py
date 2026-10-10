"""Collect licenses for the locked Windows dependency graph, including fonts.

Run from any directory after cargo fetch. Files missing from a published crate
are read from its upstream GitHub source commit and cached under artifacts/.
Only the generated notice file belongs in the public source/package.
"""
import json
import os
from pathlib import Path
import re
import subprocess
from urllib.error import HTTPError
from urllib.request import urlopen

ROOT = Path(__file__).resolve().parents[1]
CACHE = ROOT / "artifacts" / "upstream-licenses"
OUTPUT = ROOT / "THIRD_PARTY_NOTICES.txt"


def upstream_licenses(package, directory):
    vcs_path = directory / ".cargo_vcs_info.json"
    repository = (package.get("repository") or "").rstrip("/")
    match = re.match(r"https://github\.com/([^/]+/[^/]+)", repository)
    if not match or not vcs_path.is_file():
        raise RuntimeError(f"No exact upstream source for {package['name']}")
    repository = match.group(1).removesuffix(".git")
    commit = json.loads(vcs_path.read_text(encoding="utf-8"))["git"]["sha1"]
    result = []
    for filename in ["LICENSE-APACHE", "LICENSE-MIT", "LICENSE", "LICENSE.md", "LICENSE.txt", "COPYING", "NOTICE"]:
        url = f"https://raw.githubusercontent.com/{repository}/{commit}/{filename}"
        cache = CACHE / repository.replace("/", "_") / commit / filename
        absent = cache.with_name(cache.name + ".absent")
        if absent.is_file():
            continue
        if not cache.is_file():
            try:
                with urlopen(url, timeout=20) as response:
                    content = response.read().decode("utf-8")
            except HTTPError as error:
                if error.code == 404:
                    absent.parent.mkdir(parents=True, exist_ok=True)
                    absent.touch()
                    continue
                raise
            cache.parent.mkdir(parents=True, exist_ok=True)
            cache.write_text(content, encoding="utf-8")
        result.append((url, cache.read_text(encoding="utf-8")))
    if not result:
        raise RuntimeError(f"Missing license text for {package['name']}")
    return result


def main():
    raw = subprocess.check_output(
        [os.environ.get("CARGO", "cargo"), "metadata", "--locked", "--format-version", "1",
         "--filter-platform", "x86_64-pc-windows-msvc"], cwd=ROOT, text=True, encoding="utf-8")
    metadata = json.loads(raw)
    packages = sorted((p for p in metadata["packages"] if p.get("source")), key=lambda p: (p["name"], p["version"]))
    parts = ["Diskvale - third-party licenses and notices\n\n"
             "Generated from Cargo.lock for x86_64-pc-windows-msvc. Includes the\n"
             "resolved application, build and test dependencies; not every listed\n"
             "crate is linked into the final executable. Original Diskvale code\n"
             "uses the MIT license in LICENSE. Dependencies and embedded fonts\n"
             "retain the terms reproduced below.\n\n"
             "SOURCE INDEX\n"]
    for package in packages:
        parts.append(f"{package['name']} {package['version']} | {package.get('license') or 'see license text'} | {package.get('repository') or 'https://crates.io/crates/' + package['name']}\n")
    notices = {}
    package_references = []
    for package in packages:
        directory = Path(package["manifest_path"]).parent
        files = sorted(path for path in directory.iterdir() if path.is_file()
                       and path.name.lower().startswith(("license", "copying", "notice")))
        for folder in directory.iterdir():
            if folder.is_dir() and folder.name.lower() in {"license", "licenses"}:
                files.extend(sorted(path for path in folder.rglob("*") if path.is_file()))
        declared = package.get("license_file")
        if declared:
            path = directory / declared
            if path.is_file() and path not in files:
                files.append(path)
        texts = [(path.relative_to(directory).as_posix(), path.read_text(encoding="utf-8", errors="strict")) for path in files]
        if not texts:
            texts = upstream_licenses(package, directory)
        # egui's bundled fonts carry separate copyright and license texts.
        fonts = directory / "fonts"
        if fonts.is_dir():
            texts.extend(("fonts/" + path.name, path.read_text(encoding="utf-8")) for path in sorted(fonts.glob("*.txt")))
        references = [f"\n{package['name']} {package['version']}\n"]
        for source, text in texts:
            normalized = "\n".join(line.rstrip() for line in text.splitlines()).strip()
            if normalized not in notices:
                notices[normalized] = len(notices) + 1
            references.append(f"  NOTICE-{notices[normalized]:03d}: {source}\n")
        package_references.extend(references)
    parts.append("\nLICENSE REFERENCES\nIdentical license texts are reproduced once; each package lists its notices.\n")
    parts.extend(package_references)
    for text, identifier in notices.items():
        parts.append(f"\n{'=' * 78}\nNOTICE-{identifier:03d}\n\n{text}\n")
    OUTPUT.write_text("".join(parts), encoding="utf-8", newline="\n")
    print(f"Wrote notices for {len(packages)} locked dependency packages, including embedded fonts.")


if __name__ == "__main__":
    main()
