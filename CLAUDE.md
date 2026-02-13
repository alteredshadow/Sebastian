# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Sebastian is a Mythic C2 agent written in Rust that compiles into Linux and macOS executables, shared libraries (.dylib, .so), and static archives. It integrates with Mythic 3.3+ for command and control operations in authorized security testing contexts.

## Architecture

Sebastian has a two-component architecture:

### 1. Go Container Service (`Payload_Type/sebastian/`)
- Integrates with Mythic via RabbitMQ
- Registers commands and handles payload build requests
- Compiles the Rust agent using Cargo with configuration injected via environment variables
- Serves browser scripts for the Mythic UI
- Command definitions in `agentfunctions/*.go` register command metadata (parameters, MITRE mappings, browser scripts)

### 2. Rust Agent (`Payload_Type/sebastian/sebastian/agent_code/`)
- The implant that runs on target systems (Linux/macOS)
- Compiled with build-time configuration injected via `build.rs`
- Can be compiled as executable, shared library, or static archive

**Key Rust modules:**
- `src/main.rs` - Binary entry point
- `src/lib.rs` - Shared library entry point with auto-start via `#[ctor::ctor]`
- `src/commands/` - 68+ command implementations (one file per command)
- `src/profiles/` - C2 profile implementations (http.rs, websocket.rs, dns.rs, tcp.rs, httpx.rs, dynamichttp.rs)
- `src/tasks/` - Task processing and dispatch
- `src/responses/` - Response handling and queuing
- `src/utils/` - Utilities (crypto, files, P2P networking)

**Agent initialization flow:**
1. Initialize C2 profiles (egress and bind)
2. Initialize response channels
3. Initialize P2P networking system
4. Initialize file transfer system
5. Initialize task processing system
6. Start C2 profile communications

## Building

### Go Container
```bash
cd Payload_Type/sebastian
make build          # Build the container binary
make run           # Run pre-built binary
make run_custom    # Build and run with custom config
```

**Environment variables for custom runs:**
- `DEBUG_LEVEL` - Log level (default: "debug")
- `RABBITMQ_HOST` - RabbitMQ host (default: "127.0.0.1")
- `RABBITMQ_PASSWORD` - RabbitMQ password
- `MYTHIC_SERVER_HOST` - Mythic server host (default: "127.0.0.1")
- `MYTHIC_SERVER_GRPC_PORT` - Mythic gRPC port (default: "17444")

### Rust Agent
```bash
cd Payload_Type/sebastian/sebastian/agent_code
cargo build --release --target x86_64-unknown-linux-gnu
cargo build --release --target aarch64-unknown-linux-gnu
```

**Build configuration:** The agent requires environment variables set during build (UUID, AES key, C2 configs). These are normally set by the Go container during Mythic payload builds. For manual builds, check Mythic's Payloads page for the required parameters.

**Cross-compilation targets:**
- Linux: `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`
- macOS: Uses `cargo-zigbuild` with stub .tbd files (see Dockerfile)

**Cargo features:**
- `http`, `websocket`, `tcp`, `dns`, `httpx`, `dynamichttp` - C2 profile selection
- `debug_mode` - Enable debug logging

**Build artifacts:**
- Binary: `target/<triple>/release/sebastian`
- Shared library: `target/<triple>/release/libsebastian.{so,dylib}`

## Docker Build

The Dockerfile uses a multi-stage build:
1. Go builder stage compiles the container service
2. Final stage based on `rust:latest` includes:
   - Cross-compilation toolchains (gcc for Linux ARM64/x86_64)
   - Zig 0.13.0 and cargo-zigbuild for macOS cross-compilation
   - Protobuf compiler for DNS profile
   - macOS SDK stub .tbd files at `/opt/macos-stubs/`

## Adding New Commands

New commands require changes in both components:

**Go side (`agentfunctions/<command>.go`):**
- Define command metadata in `init()` function
- Register with `agentstructs.AllPayloadData.Get("sebastian").AddCommand()`
- Specify parameters, MITRE mappings, browser script path

**Rust side (`agent_code/src/commands/<command>.rs`):**
- Implement command logic as async function
- Parse parameters from JSON
- Return `Result<CommandResult, String>`
- Register in `src/commands/mod.rs`

## Configuration Injection

The Rust agent uses `build.rs` to inject configuration at compile time. Key environment variables:
- `AGENT_UUID` - Unique agent identifier
- `EGRESS_ORDER`, `EGRESS_FAILOVER` - C2 profile failover config
- `C2_<PROFILE>_INITIAL_CONFIG` - Serialized config for each C2 profile
- `DEBUG` - Enable debug statements
- `PROXY_BYPASS` - Proxy bypass configuration
- `SEBASTIAN_CRATE_TYPE` - Output type (bin, cdylib, staticlib)

## C2 Profiles

Six C2 profiles support different network transports:
- **HTTP** - Standard HTTP/HTTPS with cookies, custom headers
- **WebSocket** - Persistent WebSocket connections
- **DynamicHTTP** - Malleable HTTP profile with Jinja2 templating
- **TCP** - Raw TCP for P2P agent linking
- **DNS** - DNS tunneling with protobuf encoding
- **HTTPx** - HTTP profile variant with additional evasion

All profiles support AES-256-CBC encryption with HMAC-SHA256 and RSA-4096 key exchange.

## P2P Networking

Sebastian supports peer-to-peer agent connections:
- **TCP P2P**: Direct TCP connections between agents (`link_tcp`, `unlink_tcp` commands)
- **Webshell P2P**: Indirect communication via webshell (`link_webshell`, `unlink_webshell` commands)
- P2P system handles message routing and connection management in `src/utils/p2p/`

## Development Notes

- This is security research software for authorized penetration testing only
- The agent catches panics in shared library mode to prevent host process crashes
- Shared library mode uses raw pthread with 8MB stack (increased from 512KB for crypto operations)
- Commands run asynchronously via Tokio runtime
- File transfers use chunked streaming to handle large files
- SOCKS5 proxy and reverse port forwarding run as persistent jobs
