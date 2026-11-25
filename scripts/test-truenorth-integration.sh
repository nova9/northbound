#!/usr/bin/env sh

set -eu

verbose=${VERBOSE:-0}
target=""
test_filter=""
usage="Usage: $0 [-v|--verbose] [all|workspace|wallet|indexer|server] [test_filter]"

for argument in "$@"; do
    case "$argument" in
        -v | --verbose)
            verbose=1
            ;;
        --workspace)
            if [ -z "$target" ]; then
                target=all
            else
                echo "$usage" >&2
                exit 1
            fi
            ;;
        -*)
            echo "$usage" >&2
            exit 1
            ;;
        *)
            if [ -z "$target" ]; then
                target=$argument
            elif [ -z "$test_filter" ]; then
                test_filter=$argument
            else
                echo "$usage" >&2
                exit 1
            fi
            ;;
    esac
done

target=${target:-all}
case "$target" in
    wallet | indexer | server | all | workspace | --workspace) ;;
    *) echo "$usage" >&2; exit 1 ;;
esac
if [ "$target" = workspace ] || [ "$target" = --workspace ]; then
    target=all
fi
if [ "$target" = all ] && [ -n "$test_filter" ]; then
    echo "Cannot combine whole-workspace run with a test filter; select one crate." >&2
    exit 1
fi

script_directory=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repository_directory=$(dirname "$script_directory")

. "$script_directory/truenorth-release.sh"
read_truenorth_release_paths "$repository_directory"

"$script_directory/download-truenorth.sh" "$release_tag"

# Keep temporary test chains separate from the shared release cache.
test_cache_directory="$repository_directory/.local/truenorth-integration"
mkdir -p "$test_cache_directory"

# Give every run an isolated chain and remove it when the script exits.
test_data_directory=""
rpc_port=$((20000 + ($$ % 10000)))
p2p_port=$((rpc_port + 1))
daemon_pid=""
completed_suites=""

cleanup() {
    if [ -n "$daemon_pid" ] && kill -0 "$daemon_pid" 2>/dev/null; then
        "$cli_path" \
            -regtest \
            "-datadir=$test_data_directory" \
            "-rpcport=$rpc_port" \
            stop >/dev/null 2>&1 || kill "$daemon_pid" 2>/dev/null || true

        wait "$daemon_pid" 2>/dev/null || true
    fi

    if [ -n "$test_data_directory" ]; then
        rm -rf "$test_data_directory"
    fi
}

trap cleanup EXIT INT TERM

run_integration_tests() {
    test_package=$1
    transaction_index=$2
    suite_label="$test_package (txindex=$transaction_index)"
    test_data_directory=$(mktemp -d "$test_cache_directory/regtest.XXXXXX")
    node_log="$test_data_directory/truenorthd.log"

    printf '\n==> %s\n' "$suite_label"

    # Indexed API reads must work without the daemon's historical scan index.
    block_filter_index=1
    if [ "$test_package" = server ]; then
        block_filter_index=0
    fi

    "$daemon_path" \
        -regtest \
        "-datadir=$test_data_directory" \
        "-rpcport=$rpc_port" \
        "-port=$p2p_port" \
        -server=1 \
        "-txindex=$transaction_index" \
        "-blockfilterindex=$block_filter_index" \
        -fallbackfee=0.00001000 \
        >"$node_log" 2>&1 &

    daemon_pid=$!

    # Wait for RPC readiness instead of assuming the node starts immediately.
    attempt=0

    until "$cli_path" \
        -regtest \
        "-datadir=$test_data_directory" \
        "-rpcport=$rpc_port" \
        getblockchaininfo >/dev/null 2>&1
    do
        if ! kill -0 "$daemon_pid" 2>/dev/null; then
            echo "TrueNorth Core stopped before becoming ready." >&2
            cat "$node_log" >&2
            exit 1
        fi

        attempt=$((attempt + 1))

        if [ "$attempt" -ge 120 ]; then
            echo "Timed out waiting for TrueNorth Core." >&2
            cat "$node_log" >&2
            exit 1
        fi

        sleep 0.25
    done

    if [ "$verbose" = 1 ]; then
        echo "TrueNorth Core is ready on RPC port $rpc_port"
    fi

    # Keep repeated CLI connection arguments in one visible helper.
    run_truenorth_cli() {
        "$cli_path" \
            -regtest \
            "-datadir=$test_data_directory" \
            "-rpcport=$rpc_port" \
            "$@"
    }

    run_truenorth_cli createwallet funding >/dev/null

    # Let the Rust test use this exact temporary node and funding wallet.
    export TRUENORTH_TEST_CLI="$cli_path"
    export TRUENORTH_TEST_DATA_DIR="$test_data_directory"
    export TRUENORTH_TEST_RPC_PORT="$rpc_port"
    export TRUENORTH_TEST_WALLET="funding"
    export TRUENORTH_TEST_ISOLATED="1"

    # These tests share a node and funding wallet, so keep mining and spending sequential.
    cd "$repository_directory"

    set -- --ignored --test-threads=1
    if [ "$verbose" = 1 ]; then
        set -- "$@" --nocapture
    fi
    if [ -n "$test_filter" ]; then
        set -- "$@" "$test_filter"
    fi

    if [ "$verbose" = 1 ]; then
        cargo test \
            -p "$test_package" \
            --test truenorth_regtest \
            -- \
            "$@"
    else
        cargo test \
            -q \
            -p "$test_package" \
            --test truenorth_regtest \
            -- \
            "$@"
    fi

    if [ -z "$completed_suites" ]; then
        completed_suites="$suite_label"
    else
        completed_suites="$completed_suites, $suite_label"
    fi

    cleanup
    daemon_pid=""
    test_data_directory=""
}

run_package() {
    package=$1
    run_integration_tests "$package" 0
    if [ "$package" = server ]; then
        run_integration_tests "$package" 1
    fi
}

if [ "$target" = all ]; then
    run_package wallet
    run_package indexer
    run_package server
else
    run_package "$target"
fi

printf '\nPassed: %s\n' "$completed_suites"
