+++
title = "sebastian"
chapter = false
weight = 5
+++

## Summary

Sebastian is a fully featured macOS and Linux agent written in Rust. It is a rewrite of Poseidon with the same command set and C2 profile support.

### Highlights

- Cross-platform support for macOS and Linux (x86_64 and ARM64)
- 6 C2 profiles: HTTP, WebSocket, DynamicHTTP, TCP, DNS, HTTPx
- 68 commands covering file operations, process management, persistence, credential access, networking, and more
- AES-256-CBC encryption with RSA-4096 key exchange
- P2P networking via TCP and webshell profiles
- Multiple egress C2 with configurable failover
- SOCKS5 proxy and reverse port forwarding
- Interactive PTY and SSH sessions
- Output formats: executable, shared library (.dylib/.so), static archive

### Build Modes

- **default** - Standard executable (ELF or Mach-O)
- **c-shared** - Shared library (.dylib on macOS, .so on Linux)
- **c-archive** - Static library (.a) packaged as a .zip with header file
