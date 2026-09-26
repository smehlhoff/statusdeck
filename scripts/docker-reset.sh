#!/usr/bin/env bash

set -Eeuo pipefail

if ! command -v docker >/dev/null 2>&1; then
    echo "Error: Docker is not installed or is not available in PATH." >&2
    exit 1
fi

if ! docker info >/dev/null 2>&1; then
    echo "Error: Cannot connect to the Docker daemon." >&2
    exit 1
fi

if [[ "${1:-}" != "--force" ]]; then
    echo "WARNING: This will permanently delete:"
    echo "  - All Docker containers"
    echo "  - All Docker images"
    echo "  - All Docker volumes"
    echo "  - All unused Docker networks"
    echo "  - All Docker build cache"
    echo
    read -r -p "Type 'DELETE' to continue: " confirmation

    if [[ "$confirmation" != "DELETE" ]]; then
        echo "Aborted."
        exit 0
    fi
fi

echo "Removing all containers..."
container_ids="$(docker ps -aq)"
if [[ -n "$container_ids" ]]; then
    docker rm --force $container_ids
fi

echo "Removing all images..."
image_ids="$(docker images -aq)"
if [[ -n "$image_ids" ]]; then
    docker image rm --force $image_ids
fi

echo "Removing all volumes..."
volume_names="$(docker volume ls -q)"
if [[ -n "$volume_names" ]]; then
    docker volume rm --force $volume_names
fi

echo "Pruning unused networks and remaining Docker resources..."
docker system prune --all --force --volumes

echo "Removing build cache..."
docker builder prune --all --force

if docker buildx version >/dev/null 2>&1; then
    echo "Removing Buildx cache..."
    docker buildx prune --all --force
fi

echo "Docker has been reset."
