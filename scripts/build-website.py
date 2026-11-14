"""Prepare the static website with a published release's version and downloads."""

import html
import json
from pathlib import Path
import shutil
import sys


def build_website(release, output_directory):
    if release.get("draft"):
        raise ValueError("The public website requires a published release.")

    tag = release["tag_name"]
    release_label = "Prerelease" if release.get("prerelease") else "Release"
    archive_name = f"Northbound-{tag}-macos-arm64.zip"
    download_url = None
    checksum_available = False
    for asset in release["assets"]:
        if asset["name"] == archive_name:
            download_url = asset["browser_download_url"]
        if asset["name"] == f"{archive_name}.sha256":
            checksum_available = True
    if not download_url or not checksum_available:
        raise ValueError("The release must include its macOS ZIP and SHA-256 checksum.")

    if not download_url.startswith("https://github.com/"):
        raise ValueError("Release links must use https://github.com/.")

    source_directory = Path(__file__).resolve().parent.parent / "website"
    page = (source_directory / "index.html").read_text()
    replacements = {
        "<!-- release-version -->Latest release<!-- /release-version -->":
            f"{release_label} {html.escape(tag)}",
        'data-release-download href="https://github.com/nova9/northbound/releases"':
            f'data-release-download href="{html.escape(download_url, quote=True)}"',
    }
    for marker, replacement in replacements.items():
        if page.count(marker) != 1:
            raise ValueError(f"Expected exactly one website release marker: {marker}")
        page = page.replace(marker, replacement)

    # Copy only public files; the repository's README is not a website asset.
    output_directory.mkdir(parents=True, exist_ok=False)
    (output_directory / "index.html").write_text(page)
    shutil.copyfile(source_directory / "styles.css", output_directory / "styles.css")
    shutil.copytree(source_directory / "assets", output_directory / "assets")


if __name__ == "__main__":
    if len(sys.argv) != 3:
        sys.exit("Usage: python3 scripts/build-website.py RELEASE_JSON OUTPUT_DIRECTORY")
    try:
        release = json.loads(Path(sys.argv[1]).read_text())
        build_website(release, Path(sys.argv[2]))
    except (KeyError, ValueError, OSError) as error:
        sys.exit(f"Could not build website: {error}")
