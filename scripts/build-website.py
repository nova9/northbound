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
    packages = {
        "macos": f"Northbound-{tag}-macos-arm64.zip",
        "windows": f"Northbound-{tag}-windows-x64.msi",
        "linux": f"Northbound-{tag}-linux-x64.AppImage",
    }
    downloads = {}
    for platform, filename in packages.items():
        download_url = None
        checksum_available = False
        for asset in release["assets"]:
            if asset["name"] == filename:
                download_url = asset["browser_download_url"]
            if asset["name"] == f"{filename}.sha256":
                checksum_available = True
        if not download_url or not checksum_available:
            raise ValueError(f"The release must include {filename} and its SHA-256 checksum.")
        if not download_url.startswith("https://github.com/"):
            raise ValueError("Release links must use https://github.com/.")
        downloads[platform] = download_url

    source_directory = Path(__file__).resolve().parent.parent / "website"
    page = (source_directory / "index.html").read_text()
    replacements = {
        "<!-- release-version -->Latest release<!-- /release-version -->":
            f"{release_label} {html.escape(tag)}",
    }
    for platform, download_url in downloads.items():
        marker = f'data-release-download="{platform}" href="https://github.com/nova9/northbound/releases"'
        replacements[marker] = f'data-release-download="{platform}" href="{html.escape(download_url, quote=True)}"'
    for marker, replacement in replacements.items():
        if page.count(marker) != 1:
            raise ValueError(f"Expected exactly one website release marker: {marker}")
        page = page.replace(marker, replacement)

    # Copy only public files; the repository's README is not a website asset.
    output_directory.mkdir(parents=True, exist_ok=False)
    (output_directory / "index.html").write_text(page)
    shutil.copyfile(source_directory / "styles.css", output_directory / "styles.css")
    shutil.copyfile(source_directory / "carousel.js", output_directory / "carousel.js")
    shutil.copytree(source_directory / "assets", output_directory / "assets")


if __name__ == "__main__":
    if len(sys.argv) != 3:
        sys.exit("Usage: python3 scripts/build-website.py RELEASE_JSON OUTPUT_DIRECTORY")
    try:
        release = json.loads(Path(sys.argv[1]).read_text())
        build_website(release, Path(sys.argv[2]))
    except (KeyError, ValueError, OSError) as error:
        sys.exit(f"Could not build website: {error}")
