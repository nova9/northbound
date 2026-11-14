#!/usr/bin/env sh

set -eu

usage() {
    echo "Usage: $0 server [version] | truenorthd" >&2
    exit 1
}

image_target=${1:-}
case "$image_target" in
    server) [ "$#" -le 2 ] || usage ;;
    truenorthd) [ "$#" -eq 1 ] || usage ;;
    *) usage ;;
esac

script_directory=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repository_directory=$(dirname "$script_directory")

DOCKERHUB_REPOSITORY="${DOCKERHUB_REPOSITORY:-}"
DOCKER_PLATFORMS="${DOCKER_PLATFORMS:-linux/amd64}"

if [ -z "$DOCKERHUB_REPOSITORY" ]; then
    echo "Set DOCKERHUB_REPOSITORY to the Docker Hub namespace and repository." >&2
    exit 1
fi

case "$image_target" in
    server)
        dockerfile="$repository_directory/server/Dockerfile"
        build_context="$repository_directory"
        APP_SLUG="${APP_SLUG:-northbound-server}"
        version_manifest="$repository_directory/server/Cargo.toml"
        version_section="[package]"

        if awk '
            /^version[.]workspace[[:space:]]*=[[:space:]]*true/ { inherited = 1 }
            END { exit !inherited }
        ' "$version_manifest"; then
            version_manifest="$repository_directory/Cargo.toml"
            version_section="[workspace.package]"
        fi

        PACKAGE_VERSION=$(awk -v section="$version_section" '
            /^\[/ { in_section = ($0 == section) }
            in_section && /^version[[:space:]]*=/ {
                version = $0
                sub(/^[^=]*=[[:space:]]*"/, "", version)
                sub(/".*$/, "", version)
                print version
                exit
            }
        ' "$version_manifest")

        if [ -z "$PACKAGE_VERSION" ]; then
            echo "Could not read the server version from $version_manifest." >&2
            exit 1
        fi

        IMAGE_VERSION="${2:-$PACKAGE_VERSION}"
        ;;
    truenorthd)
        dockerfile="$repository_directory/server/devops/truenorthd/Dockerfile"
        build_context="$repository_directory/server/devops"
        APP_SLUG="${APP_SLUG:-truenorthd}"
        IMAGE_VERSION=$(cat "$repository_directory/server/devops/truenorthd/VERSION.txt")
        ;;
esac

IMAGE_VERSION="${IMAGE_VERSION#v}"
if [ -z "$IMAGE_VERSION" ]; then
    echo "The selected image version is empty." >&2
    exit 1
fi

if ! command -v docker >/dev/null 2>&1; then
    echo "Docker is not installed or is not available on PATH." >&2
    exit 1
fi

if ! docker buildx version >/dev/null 2>&1; then
    echo "Docker Buildx is not installed or is not available." >&2
    exit 1
fi

VERSION_IMAGE="docker.io/${DOCKERHUB_REPOSITORY}:${APP_SLUG}-v${IMAGE_VERSION}"

echo "Building ${VERSION_IMAGE} for ${DOCKER_PLATFORMS}"

docker buildx build \
    --file "$dockerfile" \
    --platform "$DOCKER_PLATFORMS" \
    --tag "$VERSION_IMAGE" \
    --push \
    "$build_context"

echo "Published ${VERSION_IMAGE}"
