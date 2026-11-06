#!/usr/bin/env bash

set -euo pipefail

if [[ $# -ne 1 ]]; then
    echo "Usage: $0 <TrueNorth release archive URL>" >&2
    exit 1
fi

archive_url="$1"
script_directory="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
project_directory="$(cd -- "$script_directory/.." && pwd)"
install_directory="$project_directory/.local/truenorth-bin"
temporary_directory="$(mktemp -d)"

trap 'rm -rf "$temporary_directory"' EXIT

echo "Downloading TrueNorth..."
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
