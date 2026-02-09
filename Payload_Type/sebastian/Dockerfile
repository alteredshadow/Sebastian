FROM golang:1.25 AS go-builder

WORKDIR /Mythic/

COPY ["go.mod", "go.sum", "."]
RUN go mod download

COPY [".", "."]

RUN go build -o /sebastian-container .

# Final image with both Go container binary and Rust toolchain for agent builds
FROM rust:latest

# Install cross-compilation tools for agent builds
RUN apt-get update && apt-get install -y \
    g++-x86-64-linux-gnu \
    libc6-dev-amd64-cross \
    g++-aarch64-linux-gnu \
    libc6-dev-arm64-cross \
    protobuf-compiler \
    && rm -rf /var/lib/apt/lists/*

# Add cross-compilation targets for the Rust agent
RUN rustup target add x86_64-unknown-linux-gnu \
    && rustup target add aarch64-unknown-linux-gnu \
    && rustup target add x86_64-apple-darwin \
    && rustup target add aarch64-apple-darwin

WORKDIR /Mythic/

# Copy the Go container binary
COPY --from=go-builder /sebastian-container /sebastian-container

# Copy all source files (agent code, browserscripts, etc.)
COPY [".", "."]

# Pre-download agent dependencies
RUN cd sebastian/agent_code && cargo fetch

# Create build output directory
RUN mkdir -p /build

CMD ["/sebastian-container"]
