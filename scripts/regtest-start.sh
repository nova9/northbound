#!/usr/bin/env bash

set -euo pipefail

script_directory="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
project_directory="$(cd -- "$script_directory/.." && pwd)"

. "$script_directory/truenorth-release.sh"
read_truenorth_release_paths "$project_directory"

data_directory="$project_directory/.local/truenorth"
daemon="$daemon_path"

if [[ ! -x "$daemon" ]]; then
    echo "Run scripts/download-truenorth.sh first." >&2
    exit 1
fi

mkdir -p "$data_directory"

"$daemon" -regtest "-datadir=$data_directory" -daemon -blockfilterindex=1
