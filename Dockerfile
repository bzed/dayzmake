# Stage 1: Build the application
FROM rust:latest AS builder

WORKDIR /usr/src/dayzmake

# --- Build the actual application ---
# Now, copy the rest of the application's source code.
COPY . .

# Build the application in release mode.
# This will be much faster as it uses the cached dependencies from the previous step.
RUN cargo build --release
