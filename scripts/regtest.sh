#!/usr/bin/env sh

set -eu

usage() {
    echo "Usage: $0 start|stop|create-wallet <wallet-name>|generate-coins <wallet-name> [blocks]|send-coins <wallet-name> <address> [amount]" >&2
    exit 1
}

command=${1:-}
if [ "$#" -eq 0 ]; then
    usage
fi
shift

case "$command" in
    start | stop)
        [ "$#" -eq 0 ] || usage
        ;;
    create-wallet)
        [ "$#" -eq 1 ] || usage
        [ -n "$1" ] || usage
        ;;
    generate-coins)
        [ "$#" -ge 1 ] && [ "$#" -le 2 ] || usage
        [ -n "$1" ] || usage
        ;;
    send-coins)
        [ "$#" -ge 2 ] && [ "$#" -le 3 ] || usage
        [ -n "$1" ] && [ -n "$2" ] || usage
        ;;
    *) usage ;;
esac

script_directory=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repository_directory=$(dirname "$script_directory")

. "$script_directory/truenorth-release.sh"
read_truenorth_release_paths "$repository_directory"

data_directory="$repository_directory/.local/truenorth"
required_binary="$cli_path"
if [ "$command" = start ]; then
    required_binary="$daemon_path"
fi

if [ ! -x "$required_binary" ]; then
    echo "Run scripts/download-truenorth.sh first." >&2
    exit 1
fi

case "$command" in
    start)
        mkdir -p "$data_directory"
        # Wait for initialization so a failed background node cannot look started.
        if ! "$daemon_path" -regtest "-datadir=$data_directory" -daemonwait -blockfilterindex=1; then
            echo "TrueNorth Core failed to start. See $data_directory/regtest/debug.log." >&2
            exit 1
        fi
        ;;
    stop)
        "$cli_path" -regtest "-datadir=$data_directory" stop
        ;;
    create-wallet)
        wallet_name=$1
        "$cli_path" -regtest "-datadir=$data_directory" -rpcwait createwallet "$wallet_name"
        ;;
    generate-coins)
        wallet_name=$1
        blocks=${2:-101}
        mining_address=$("$cli_path" -regtest "-datadir=$data_directory" "-rpcwallet=$wallet_name" getnewaddress)
        "$cli_path" -regtest "-datadir=$data_directory" generatetoaddress "$blocks" "$mining_address" >/dev/null
        echo "Generated $blocks regtest blocks for wallet '$wallet_name'."
        ;;
    send-coins)
        wallet_name=$1
        receiving_address=$2
        amount=${3:-10}
        "$cli_path" \
            -regtest \
            "-datadir=$data_directory" \
            "-rpcwallet=$wallet_name" \
            -named sendtoaddress \
            "address=$receiving_address" \
            "amount=$amount" \
            fee_rate=1
        ;;
esac
