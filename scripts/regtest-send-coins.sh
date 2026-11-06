#!/usr/bin/env bash

set -euo pipefail

if [[ $# -lt 1 || $# -gt 2 ]]; then
    echo "Usage: $0 <receiving-address> [amount]" >&2
    exit 1
fi

receiving_address="$1"
amount="${2:-10}"
script_directory="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
project_directory="$(cd -- "$script_directory/.." && pwd)"
cli="$project_directory/.local/truenorth-bin/truenorth-cli"
data_directory="$project_directory/.local/truenorth"

if [[ ! -x "$cli" ]]; then
    echo "Run scripts/download-truenorth.sh first." >&2
    exit 1
fi

"$cli" \
    -regtest \
    "-datadir=$data_directory" \
    -rpcwallet=funding \
    -named sendtoaddress \
    address="$receiving_address" \
    amount="$amount" \
    fee_rate=1
