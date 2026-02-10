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
    xz-utils \
    && rm -rf /var/lib/apt/lists/*

# Install Zig (used by cargo-zigbuild for macOS cross-compilation)
RUN ZIG_VERSION=0.13.0 && \
    ARCH=$(uname -m) && \
    curl -L "https://ziglang.org/download/${ZIG_VERSION}/zig-linux-${ARCH}-${ZIG_VERSION}.tar.xz" | tar -xJ -C /usr/local && \
    ln -s /usr/local/zig-linux-${ARCH}-${ZIG_VERSION}/zig /usr/local/bin/zig

# Install cargo-zigbuild for macOS cross-compilation
RUN cargo install cargo-zigbuild

# Create macOS SDK stub .tbd files for cross-compilation
# These satisfy the linker; real libraries are on the target macOS system
RUN mkdir -p /opt/macos-stubs/lib \
    /opt/macos-stubs/framework/CoreFoundation.framework \
    /opt/macos-stubs/framework/CoreGraphics.framework \
    /opt/macos-stubs/framework/AppKit.framework \
    /opt/macos-stubs/framework/Security.framework \
    /opt/macos-stubs/framework/IOKit.framework \
    /opt/macos-stubs/framework/IOSurface.framework \
    /opt/macos-stubs/framework/ColorSync.framework && \
    for lib in objc objc.A System.B resolv; do \
        printf -- '--- !tapi-tbd\ntbd-version: 4\ntargets: [ x86_64-macos, arm64-macos ]\ninstall-name: "/usr/lib/lib%s.dylib"\n...\n' "$lib" \
        > "/opt/macos-stubs/lib/lib${lib}.tbd"; \
    done && \
    for fw in CoreFoundation CoreGraphics AppKit Security IOKit IOSurface ColorSync; do \
        printf -- '--- !tapi-tbd\ntbd-version: 4\ntargets: [ x86_64-macos, arm64-macos ]\ninstall-name: "/System/Library/Frameworks/%s.framework/Versions/A/%s"\n...\n' "$fw" "$fw" \
        > "/opt/macos-stubs/framework/${fw}.framework/${fw}.tbd"; \
    done

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
