#!/usr/bin/env sh

set -eu

script_directory=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repository_directory=$(dirname "$script_directory")

DOCKERHUB_REPOSITORY="${DOCKERHUB_REPOSITORY:-}"
APP_SLUG="${APP_SLUG:-northbound-server}"
DOCKER_PLATFORMS="${DOCKER_PLATFORMS:-linux/amd64}"

if [ -z "$DOCKERHUB_REPOSITORY" ]; then
    echo "Set DOCKERHUB_REPOSITORY to the Docker Hub namespace and repository." >&2
    exit 1
fi

PACKAGE_VERSION=$(awk '
    /^version[[:space:]]*=/ {
        version = $0
        sub(/^[^=]*=[[:space:]]*"/, "", version)
        sub(/".*$/, "", version)
        print version
        exit
    }
' "$repository_directory/server/Cargo.toml")

if [ -z "$PACKAGE_VERSION" ]; then
    echo "Could not read the server version from server/Cargo.toml." >&2
    exit 1
fi

IMAGE_VERSION="${1:-$PACKAGE_VERSION}"
IMAGE_VERSION="${IMAGE_VERSION#v}"

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
    --file "$repository_directory/server/Dockerfile" \
    --platform "$DOCKER_PLATFORMS" \
    --tag "$VERSION_IMAGE" \
    --push \
    "$repository_directory"

echo "Published ${VERSION_IMAGE}"
