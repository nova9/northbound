#!/usr/bin/env bash

set -euo pipefail

script_directory="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
project_directory="$(cd -- "$script_directory/.." && pwd)"
cli="$project_directory/.local/truenorth-bin/truenorth-cli"
data_directory="$project_directory/.local/truenorth"

blocks="${1:-101}"

if [[ ! -x "$cli" ]]; then
    echo "Run scripts/download-truenorth.sh first." >&2
    exit 1
fi

mining_address="$("$cli" -regtest "-datadir=$data_directory" -rpcwallet=funding getnewaddress)"

"$cli" -regtest "-datadir=$data_directory" generatetoaddress "$blocks" "$mining_address" >/dev/null
echo "Generated spendable regtest coins in the funding wallet."
