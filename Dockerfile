# Use the official Rust image as the base image for amd64
FROM rust:1.42-slim-buster

# Install necessary dependencies
RUN apt-get update && \
    apt-get install -y clang cmake libsnappy-dev ca-certificates

# Create a new user and set the working directory
RUN adduser --disabled-login --system --shell /bin/false --uid 1000 user
USER user
WORKDIR /home/user

# Copy the source code into the container
COPY --chown=user:user ./ /home/user

# Build the project for amd64
RUN cargo build --release
