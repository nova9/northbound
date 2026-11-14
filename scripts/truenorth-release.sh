#!/usr/bin/env sh

# Read the selected release and set the paths shared by development and tests.
# This only reads configuration; downloading remains explicit in the caller.
read_truenorth_release_paths() {
    release_repository_directory=$1
    truenorth_version=${2:-}

    if [ -z "$truenorth_version" ]; then
        version_file="$release_repository_directory/TRUENORTH_VERSION"

        if [ ! -f "$version_file" ]; then
            echo "Missing TrueNorth version file: $version_file" >&2
            return 1
        fi

        truenorth_version=$(cat "$version_file")
    fi

    version_number=${truenorth_version#v}

    # Release tags must be safe to use as directory names and URL components.
    case "$version_number" in
        '' | *[!0-9A-Za-z._-]* | [!0-9]*)
            echo "Invalid TrueNorth version: $truenorth_version" >&2
            return 1
            ;;
    esac

    release_tag="v$version_number"
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
            echo "Unsupported platform: $system_name $machine_architecture" >&2
            return 1
            ;;
    esac

    cache_directory="$release_repository_directory/.local/truenorth-bin"
    install_directory="$cache_directory/$release_tag/$release_platform"
    daemon_path="$install_directory/truenorthd$executable_extension"
    cli_path="$install_directory/truenorth-cli$executable_extension"
}
