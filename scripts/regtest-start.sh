#!/usr/bin/env bash

set -euo pipefail

script_directory="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
project_directory="$(cd -- "$script_directory/.." && pwd)"
binary_directory="$project_directory/.local/truenorth-bin"
data_directory="$project_directory/.local/truenorth"
daemon="$binary_directory/truenorthd"

if [[ ! -x "$daemon" ]]; then
    echo "Run scripts/download-truenorth.sh first." >&2
    exit 1
fi

mkdir -p "$data_directory"

"$daemon" -regtest "-datadir=$data_directory" -daemon -blockfilterindex=1
