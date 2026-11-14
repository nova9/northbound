#!/usr/bin/env bash

set -euo pipefail

script_directory="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
project_directory="$(cd -- "$script_directory/.." && pwd)"

. "$script_directory/truenorth-release.sh"
read_truenorth_release_paths "$project_directory"

cli="$cli_path"
data_directory="$project_directory/.local/truenorth"

if [[ ! -x "$cli" ]]; then
    echo "Run scripts/download-truenorth.sh first." >&2
    exit 1
fi

"$cli" -regtest "-datadir=$data_directory" -rpcwait createwallet funding
