#!/bin/bash

# This script builds the dayzmake binary using a containerized build process
# with podman. It builds an image from the Dockerfile, then extracts the
# compiled binary to the host system.

set -e # Exit immediately if a command exits with a non-zero status.

IMAGE_NAME="localhost/dayzmake"
CONTAINER_NAME="dayzmake-build-container"
OUTPUT_DIR="$(pwd)/target/podman"
BINARY_NAME="dayzmake"

echo "Building container image: ${IMAGE_NAME}"
podman build -t "${IMAGE_NAME}" .

echo "Creating container to copy artifact..."
# Create a container from the built image. The --name flag is important to reference it later.
# If a container with the same name exists, it will be replaced.
podman create --name "${CONTAINER_NAME}" --replace "${IMAGE_NAME}"

mkdir -p "${OUTPUT_DIR}"

echo "Copying binary from container to ${OUTPUT_DIR}/${BINARY_NAME}"
podman cp "${CONTAINER_NAME}:/usr/src/dayzmake/target/release/${BINARY_NAME}" "${OUTPUT_DIR}/${BINARY_NAME}"

echo "Cleaning up container..."
podman rm "${CONTAINER_NAME}"

echo "Build complete! The binary is available at: ${OUTPUT_DIR}/${BINARY_NAME}"