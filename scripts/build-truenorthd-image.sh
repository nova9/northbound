#!/usr/bin/env sh

set -eu

if [ "$#" -ne 0 ]; then
    echo "This script takes no arguments. Update server/devops/truenorthd/VERSION.txt instead." >&2
    exit 1
fi

script_directory=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repository_directory=$(dirname "$script_directory")
dockerfile="$repository_directory/server/devops/truenorthd/Dockerfile"
build_context="$repository_directory/server/devops"
version_file="$repository_directory/server/devops/truenorthd/VERSION.txt"

DOCKERHUB_REPOSITORY="${DOCKERHUB_REPOSITORY:-}"
APP_SLUG="${APP_SLUG:-truenorthd}"
DOCKER_PLATFORMS="${DOCKER_PLATFORMS:-linux/amd64}"

if [ -z "$DOCKERHUB_REPOSITORY" ]; then
    echo "Set DOCKERHUB_REPOSITORY to the Docker Hub namespace and repository." >&2
    exit 1
fi

IMAGE_VERSION=$(cat "$version_file")
IMAGE_VERSION="${IMAGE_VERSION#v}"

if [ -z "$IMAGE_VERSION" ]; then
    echo "The TrueNorth VERSION.txt file is empty." >&2
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
