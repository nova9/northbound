#!/usr/bin/env sh

set -eu

script_directory=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repository_directory=$(dirname "$script_directory")

. "$script_directory/truenorth-release.sh"
read_truenorth_release_paths "$repository_directory"

"$script_directory/download-truenorth.sh" "$release_tag"

# Keep temporary test chains separate from the shared release cache.
test_cache_directory="$repository_directory/.local/truenorth-integration"
mkdir -p "$test_cache_directory"

# Give every run an isolated chain and remove it when the script exits.
test_data_directory=$(mktemp -d "$test_cache_directory/regtest.XXXXXX")
rpc_port=$((20000 + ($$ % 10000)))
p2p_port=$((rpc_port + 1))
node_log="$test_data_directory/truenorthd.log"
daemon_pid=""

cleanup() {
    if [ -n "$daemon_pid" ] && kill -0 "$daemon_pid" 2>/dev/null; then
        "$cli_path" \
            -regtest \
            "-datadir=$test_data_directory" \
            "-rpcport=$rpc_port" \
            stop >/dev/null 2>&1 || kill "$daemon_pid" 2>/dev/null || true

        wait "$daemon_pid" 2>/dev/null || true
    fi

    rm -rf "$test_data_directory"
}

trap cleanup EXIT INT TERM

echo "Starting TrueNorth Core regtest node..."

"$daemon_path" \
    -regtest \
    "-datadir=$test_data_directory" \
    "-rpcport=$rpc_port" \
    "-port=$p2p_port" \
    -server=1 \
    -blockfilterindex=1 \
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

echo "TrueNorth Core is ready on RPC port $rpc_port"

# Keep repeated CLI connection arguments in one visible helper.
run_truenorth_cli() {
    "$cli_path" \
        -regtest \
        "-datadir=$test_data_directory" \
        "-rpcport=$rpc_port" \
        "$@"
}

echo "Creating the integration-test funding wallet..."
run_truenorth_cli createwallet funding >/dev/null

# Let the Rust test use this exact temporary node and funding wallet.
export TRUENORTH_TEST_CLI="$cli_path"
export TRUENORTH_TEST_DATA_DIR="$test_data_directory"
export TRUENORTH_TEST_RPC_PORT="$rpc_port"
export TRUENORTH_TEST_WALLET="funding"

# These tests share a node and funding wallet, so keep mining and spending sequential.
echo "Running Northbound integration tests..."

cd "$repository_directory"

cargo test \
    -p wallet \
    --test truenorth_regtest \
    -- \
    --ignored \
    --nocapture \
    --test-threads=1
