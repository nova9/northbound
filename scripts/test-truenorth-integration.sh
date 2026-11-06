#!/usr/bin/env sh

set -eu

script_directory=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repository_directory=$(dirname "$script_directory")

# Keep the node version separate from wallet runtime configuration.
version_file="$repository_directory/wallet/tests/truenorth/VERSION.txt"

if [ ! -f "$version_file" ]; then
    echo "Missing TrueNorth version file: $version_file" >&2
    exit 1
fi

truenorth_version=$(cat "$version_file")

if [ -z "$truenorth_version" ]; then
    echo "TrueNorth version file is empty." >&2
    exit 1
fi

# Select the release archive that can run on this machine.
system_name=$(uname -s)
machine_architecture=$(uname -m)

case "$system_name:$machine_architecture" in
    Darwin:arm64)
        release_platform="macos-arm64"
        archive_extension="tar.gz"
        executable_extension=""
        ;;
    Linux:x86_64 | Linux:amd64)
        release_platform="linux-x86_64"
        archive_extension="tar.gz"
        executable_extension=""
        ;;
    MINGW*:x86_64 | MSYS*:x86_64)
        release_platform="windows-x86_64"
        archive_extension="zip"
        executable_extension=".exe"
        ;;
    *)
        echo "Unsupported test platform: $system_name $machine_architecture" >&2
        exit 1
        ;;
esac

version_number=${truenorth_version#v}
release_tag="v$version_number"
archive_name="truenorth-$release_tag-$release_platform.$archive_extension"
release_url="https://github.com/truenorth-project/truenorth/releases/download/$release_tag/$archive_name"

cache_directory="$repository_directory/.local/truenorth-integration"
archive_path="$cache_directory/$archive_name"
install_directory="$cache_directory/$release_tag/$release_platform"

# Cache downloads so repeated test runs can start without another download.
if ! command -v curl >/dev/null 2>&1; then
    echo "curl is required to download TrueNorth Core." >&2
    exit 1
fi

mkdir -p "$cache_directory"

download_file() {
    source_url=$1
    destination=$2

    if [ -f "$destination" ]; then
        return
    fi

    temporary_file="${destination}.download"
    rm -f "$temporary_file"

    echo "Downloading $(basename "$destination")..."
    curl --fail --location "$source_url" --output "$temporary_file"
    mv "$temporary_file" "$destination"
}

checksum_name="${archive_name}.sha256"
checksum_url="${release_url}.sha256"
checksum_path="$cache_directory/$checksum_name"

download_file "$release_url" "$archive_path"
download_file "$checksum_url" "$checksum_path"

# Never extract or execute an archive that does not match its published hash.
expected_checksum=$(awk 'NR == 1 { print $1 }' "$checksum_path")

if command -v sha256sum >/dev/null 2>&1; then
    actual_checksum=$(sha256sum "$archive_path" | awk '{ print $1 }')
elif command -v shasum >/dev/null 2>&1; then
    actual_checksum=$(shasum -a 256 "$archive_path" | awk '{ print $1 }')
else
    echo "sha256sum or shasum is required to verify the download." >&2
    exit 1
fi

if [ "$actual_checksum" != "$expected_checksum" ]; then
    echo "TrueNorth archive checksum does not match." >&2
    rm -f "$archive_path"
    exit 1
fi

echo "Verified $archive_name"

# Install only the daemon and CLI required by the integration tests.
daemon_path="$install_directory/truenorthd$executable_extension"
cli_path="$install_directory/truenorth-cli$executable_extension"

install_truenorth_binaries() (
    extraction_directory=$(mktemp -d "$cache_directory/extract.XXXXXX")
    trap 'rm -rf "$extraction_directory"' EXIT

    case "$archive_extension" in
        tar.gz)
            tar -xzf "$archive_path" -C "$extraction_directory"
            ;;
        zip)
            if ! command -v unzip >/dev/null 2>&1; then
                echo "unzip is required to extract the Windows archive." >&2
                exit 1
            fi

            unzip -q "$archive_path" -d "$extraction_directory"
            ;;
    esac

    binary_directory=$(find "$extraction_directory" \
        -type f \
        -name "truenorthd$executable_extension" \
        -path '*/bin/*' \
        -exec dirname {} \; \
        -quit)

    if [ -z "$binary_directory" ]; then
        echo "The archive does not contain TrueNorth Core binaries." >&2
        exit 1
    fi

    mkdir -p "$install_directory"
    cp "$binary_directory/truenorthd$executable_extension" "$daemon_path"
    cp "$binary_directory/truenorth-cli$executable_extension" "$cli_path"
    chmod +x "$daemon_path" "$cli_path"
)

if [ ! -f "$daemon_path" ] || [ ! -f "$cli_path" ]; then
    echo "Extracting TrueNorth Core..."
    install_truenorth_binaries
fi

# Give every run an isolated chain and remove it when the script exits.
test_data_directory=$(mktemp -d "$cache_directory/regtest.XXXXXX")
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

echo "Running Northbound integration tests..."

cd "$repository_directory"

cargo test \
    -p wallet \
    --test truenorth_regtest \
    -- \
    --ignored \
    --nocapture
