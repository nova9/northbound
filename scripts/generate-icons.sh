#!/bin/sh

set -eu

script_directory=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
wallet_directory=$(dirname "$script_directory")/wallet
source_icon="$wallet_directory/assets/logo.svg"
icons_directory="$wallet_directory/icons"
iconset_directory="$icons_directory/icon.iconset"

for required_command in resvg sips iconutil; do
    if ! command -v "$required_command" >/dev/null 2>&1; then
        echo "Missing required command: $required_command" >&2
        exit 1
    fi
done

if [ ! -f "$source_icon" ]; then
    echo "Source icon not found: $source_icon" >&2
    exit 1
fi

mkdir -p "$iconset_directory"

resvg -w 1024 -h 1024 "$source_icon" "$icons_directory/icon.png"

for size in 16 32 128 256 512; do
    sips -z "$size" "$size" "$icons_directory/icon.png" \
        --out "$iconset_directory/icon_${size}x${size}.png"

    double=$((size * 2))
    sips -z "$double" "$double" "$icons_directory/icon.png" \
        --out "$iconset_directory/icon_${size}x${size}@2x.png"
done

iconutil -c icns "$iconset_directory" -o "$icons_directory/icon.icns"
