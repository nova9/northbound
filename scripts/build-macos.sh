#!/bin/sh

set -eu

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
    --package-types macos \
    --no-default-features \
    --features mainnet
