#!/bin/sh

# Get the latest git tag
latest_tag=$(git describe --tags --abbrev=0)

# Build the Docker images
docker buildx build --platform=linux/amd64 -t addrindexrs:amd64 . &
docker buildx build --platform=linux/arm64 -t addrindexrs:arm64 . &

# Wait for both build commands to finish
wait

# Create a container from the amd64 image and copy the binary
docker create --name temp_amd64 addrindexrs:amd64
docker cp temp_amd64:/home/user/target/release/addrindexrs ./dist/addrindexrs-"$latest_tag"-x86_64-linux
docker rm temp_amd64

# Create a container from the arm64 image and copy the binary
docker create --name temp_arm64 addrindexrs:arm64
docker cp temp_arm64:/home/user/target/release/addrindexrs ./dist/addrindexrs-"$latest_tag"-arm64-linux
docker rm temp_arm64

# Remove Docker images
docker rmi addrindexrs:amd64 addrindexrs:arm64

echo "Binaries copied to the host file system."
