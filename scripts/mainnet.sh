#!/usr/bin/env sh

set -eu

usage() {
    echo "Usage: $0 start|stop|tip|peers|presync" >&2
    exit 1
}

[ "$#" -eq 1 ] || usage
command=$1
case "$command" in
    start | stop | tip | peers | presync) ;;
    *) usage ;;
esac

script_directory=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repository_directory=$(dirname "$script_directory")

. "$script_directory/truenorth-release.sh"
read_truenorth_release_paths "$repository_directory"

data_directory="$repository_directory/.local/truenorth/mainnet"
required_binary="$cli_path"
if [ "$command" = start ]; then
    required_binary="$daemon_path"
fi

if [ "$command" != presync ] && [ ! -x "$required_binary" ]; then
    echo "Run scripts/download-truenorth.sh first." >&2
    exit 1
fi

case "$command" in
    start)
        mkdir -p "$data_directory"
        # The wallet server needs historical blocks; keep RPC on loopback.
        if ! "$daemon_path" -chain=main "-datadir=$data_directory" \
            -server=1 -prune=0 -rpcbind=127.0.0.1 -rpcallowip=127.0.0.1 -daemonwait; then
            echo "TrueNorth Core failed to start. See $data_directory/debug.log." >&2
            exit 1
        fi
        ;;
    stop)
        "$cli_path" -chain=main "-datadir=$data_directory" stop
        ;;
    tip)
        "$cli_path" -chain=main "-datadir=$data_directory" getblockchaininfo
        ;;
    peers)
        "$cli_path" -chain=main "-datadir=$data_directory" getconnectioncount
        ;;
    presync)
        log_path="$data_directory/debug.log"
        if [ ! -r "$log_path" ]; then
            echo "Cannot read the mainnet node log: $log_path" >&2
            exit 1
        fi
        # RPC height can stay at zero during header pre-sync. Keep timestamps
        # visible because a stopped node's log is not a live status report.
        printf '%s\n' "Latest logged sync progress (may be stale if the node is stopped):"
        awk '
            /Using data directory / {
                latest = ""
                have_peer_height = 0
                header_redownload = 0
            }
            /Pre-synchronizing blockheaders|Pre-synchronization reached|Re-downloading blockheaders|UpdateTip:/ {
                latest = $0
                header_redownload = ($0 ~ /Re-downloading blockheaders/)
            }
            /peer connected:.*blocks=[0-9]+/ {
                for (field = 1; field <= NF; field++) {
                    if ($field !~ /^blocks=/) continue
                    height = $field
                    sub(/^blocks=/, "", height)
                    sub(/,$/, "", height)
                    if (height !~ /^[0-9]+$/) continue
                    if (!have_peer_height || height + 0 >= peer_height) {
                        peer_height = height + 0
                        peer_timestamp = $1
                        have_peer_height = 1
                    }
                }
            }
            END {
                if (latest != "") print latest
                else print "No sync progress recorded for the latest startup."
                if (header_redownload)
                    print "The re-download target is a pre-sync checkpoint, not the network tip."
                if (have_peer_height)
                    printf "Highest peer-advertised height (logged): %.0f (last seen %s)\n", peer_height, peer_timestamp
                else print "No peer-advertised height recorded for the latest startup."
            }
        ' "$log_path"
        ;;
esac
