#!/usr/bin/env sh

set -eu

if [ "$#" -ne 0 ]; then
    echo "Usage: $0 (builds for the host OS)" >&2
    exit 1
fi

case "$(uname -s)" in
    Linux) package_type="appimage" ;;
    Darwin) package_type="macos" ;;
    *)
        echo "Unsupported wallet build OS. On Windows, use scripts/build-windows.ps1." >&2
        exit 1
        ;;
esac

script_directory=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repository_directory=$(dirname "$script_directory")
dx="$HOME/.cargo/bin/dx"

if [ ! -x "$dx" ]; then
    echo "Dioxus CLI not found or not executable: $dx" >&2
    exit 1
fi

cd "$repository_directory"

"$dx" bundle \
    --desktop \
    --release \
    --package wallet \
    --package-types "$package_type" \
    --no-default-features \
    --features mainnet
