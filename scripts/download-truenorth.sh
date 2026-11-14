#!/usr/bin/env sh

set -eu

if [ "$#" -gt 1 ]; then
    echo "Usage: $0 [version] (defaults to TRUENORTH_VERSION)" >&2
    exit 1
fi

script_directory=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repository_directory=$(dirname "$script_directory")

. "$script_directory/truenorth-release.sh"
read_truenorth_release_paths "$repository_directory" "${1:-}"

archive_name="truenorth-$release_tag-$release_platform.$archive_extension"
release_url="https://github.com/truenorth-project/truenorth/releases/download/$release_tag/$archive_name"
archive_path="$cache_directory/$archive_name"

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

# Install only the daemon and CLI used by development and integration tests.

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

    if [ -z "$binary_directory" ] || [ ! -f "$binary_directory/truenorth-cli$executable_extension" ]; then
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

echo "Installed TrueNorth binaries in $install_directory"
