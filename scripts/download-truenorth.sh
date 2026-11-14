#!/usr/bin/env bash

set -euo pipefail

if [[ $# -ne 0 ]]; then
    echo "Usage: $0 (update TRUENORTH_VERSION to select a release)" >&2
    exit 1
fi

script_directory="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
project_directory="$(cd -- "$script_directory/.." && pwd)"
version_file="$project_directory/TRUENORTH_VERSION"

if [[ ! -f "$version_file" ]]; then
    echo "Missing TrueNorth version file: $version_file" >&2
    exit 1
fi

truenorth_version="$(cat "$version_file")"

if [[ -z "$truenorth_version" ]]; then
    echo "TrueNorth version file is empty." >&2
    exit 1
fi

system_name="$(uname -s)"
machine_architecture="$(uname -m)"

case "$system_name:$machine_architecture" in
    Darwin:arm64) release_platform="macos-arm64" ;;
    Linux:x86_64 | Linux:amd64) release_platform="linux-x86_64" ;;
    *)
        echo "Unsupported development platform: $system_name $machine_architecture" >&2
        exit 1
        ;;
esac

release_tag="v${truenorth_version#v}"
archive_name="truenorth-$release_tag-$release_platform.tar.gz"
archive_url="https://github.com/truenorth-project/truenorth/releases/download/$release_tag/$archive_name"
install_directory="$project_directory/.local/truenorth-bin"
temporary_directory="$(mktemp -d)"

trap 'rm -rf "$temporary_directory"' EXIT

echo "Downloading $archive_name..."
curl --fail --location "$archive_url" --output "$temporary_directory/truenorth.tar.gz"
tar -xzf "$temporary_directory/truenorth.tar.gz" -C "$temporary_directory"

binary_directory="$(find "$temporary_directory" -type f -name truenorthd -path '*/bin/*' -exec dirname {} \; -quit)"

if [[ -z "$binary_directory" || ! -x "$binary_directory/truenorth-cli" ]]; then
    echo "The archive does not contain bin/truenorthd and bin/truenorth-cli." >&2
    exit 1
fi

mkdir -p "$install_directory"
cp "$binary_directory/truenorthd" "$install_directory/truenorthd"
cp "$binary_directory/truenorth-cli" "$install_directory/truenorth-cli"
chmod +x "$install_directory/truenorthd" "$install_directory/truenorth-cli"

echo "Installed TrueNorth binaries in $install_directory"
