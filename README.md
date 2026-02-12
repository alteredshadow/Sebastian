# Sebastian

<p align="center">
  <img alt="Sebastian Logo" src="Payload_Type/sebastian/sebastian/agentfunctions/sebastian.svg" height="30%" width="30%">
</p>

Sebastian is a Rust agent that compiles into Linux and macOS executables, shared libraries, and static archives. It is a full rewrite of [Poseidon](https://github.com/MythicAgents/poseidon) in Rust, targeting Mythic 3.3+.

## Features

- **Supported OS**: macOS, Linux
- **Architectures**: x86_64, ARM64
- **Output formats**: ELF, Mach-O, .dylib, .so, static archive (.a)
- **6 C2 profiles**: HTTP, WebSocket, DynamicHTTP, TCP (P2P), DNS, HTTPx
- **68 commands** including shell execution, file operations, process management, SOCKS5 proxy, reverse port forwarding, interactive PTY, SSH, keylogging, screenshots, clipboard monitoring, XPC, and more
- **Encryption**: AES-256-CBC with HMAC-SHA256, RSA-4096 key exchange
- **P2P networking**: TCP and webshell-based peer-to-peer connections
- **Multiple egress**: Configurable C2 profile priority with failover rotation

## Installation

Install into a running Mythic instance using `mythic-cli`:

```bash
# Install from GitHub
sudo ./mythic-cli install github https://github.com/user/sebastian

# Install a specific branch
sudo ./mythic-cli install github https://github.com/user/sebastian branchname

# Install from a local directory
sudo ./mythic-cli install folder /path/to/sebastian
```

Then start the agent container:

```bash
sudo ./mythic-cli start sebastian
```

## Architecture

Sebastian has two components:

1. **Mythic Container** (`Payload_Type/sebastian/`) - A Go service that integrates with Mythic via RabbitMQ. It registers commands, handles payload build requests (compiling the Rust agent with Cargo), and serves browser scripts.

2. **Agent Code** (`Payload_Type/sebastian/sebastian/agent_code/`) - The Rust implant that runs on target systems. Compiled with configuration injected at build time via environment variables and `build.rs`.

## Commands

| Command | Description | OS |
|---------|-------------|-----|
| `cat` | Read file contents | All |
| `cd` | Change directory | All |
| `chmod` | Change file permissions | All |
| `clipboard` | Get clipboard contents | macOS |
| `clipboard_monitor` | Monitor clipboard changes | macOS |
| `config` | View agent configuration | All |
| `cp` | Copy files | All |
| `curl` | Make HTTP requests | All |
| `curl_env_set/get/clear` | Manage curl environment config | All |
| `download` | Download a file from target | All |
| `download_bulk` | Download multiple files | All |
| `drives` | List mounted drives | All |
| `execute_library` | Load and run a shared library | All |
| `exit` | Exit the agent | All |
| `getenv` | Get environment variables | All |
| `getuser` | Get current user info | All |
| `head` | Read first N lines of a file | All |
| `ifconfig` | List network interfaces | All |
| `jobkill` | Kill a running job | All |
| `jobs` | List running jobs | All |
| `jsimport` | Load a JXA script | macOS |
| `jsimport_call` | Call a loaded JXA function | macOS |
| `jxa` | Execute JXA code | macOS |
| `keylog` | Keylog users as root | Linux |
| `keys` | Interact with the keyring | Linux |
| `kill` | Kill a process | All |
| `libinject` | Inject a library into a process | macOS |
| `link_tcp` | Link to a P2P TCP agent | All |
| `link_webshell` | Link to a webshell agent | All |
| `list_entitlements` | List process entitlements | macOS |
| `listtasks` | List task ports | macOS |
| `ls` | List directory contents | All |
| `lsopen` | Open app via LaunchServices | macOS |
| `mkdir` | Create a directory | All |
| `mv` | Move/rename files | All |
| `persist_launchd` | Persist via launch agent/daemon | macOS |
| `persist_loginitem` | Persist via login items | macOS |
| `portscan` | Scan for open ports | All |
| `print_c2` | Print C2 configuration | All |
| `print_p2p` | Print P2P connections | All |
| `prompt` | Prompt user for credentials | macOS |
| `ps` | List processes | All |
| `pty` | Open interactive PTY | All |
| `pwd` | Print working directory | All |
| `rm` | Remove files | All |
| `rpfwd` | Reverse port forward | All |
| `run` | Execute a binary | All |
| `screencapture` | Take a screenshot | macOS |
| `setenv` | Set environment variable | All |
| `shell` | Execute shell command | All |
| `shell_config` | Configure default shell | All |
| `sleep` | Set sleep interval/jitter | All |
| `socks` | Start/stop SOCKS5 proxy | All |
| `ssh` | Interactive SSH session | All |
| `sshauth` | SSH command/SCP across hosts | All |
| `sudo` | Privilege escalation | macOS |
| `tail` | Read last N lines of a file | All |
| `tcc_check` | Check TCC permissions | macOS |
| `test_password` | Test user credentials | macOS |
| `triagedirectory` | Find interesting files | All |
| `unlink_tcp` | Unlink TCP P2P connection | All |
| `unlink_webshell` | Unlink webshell connection | All |
| `unsetenv` | Unset environment variable | All |
| `update_c2` | Update C2 config at runtime | All |
| `upload` | Upload a file to target | All |
| `xpc_*` | XPC service interaction (7 commands) | macOS |
| `caffeinate` | Prevent system sleep | macOS |

## Building Outside of Mythic

To build the agent outside of Mythic, you need the Rust toolchain installed. Set the required environment variables (UUID, C2 configs, etc.) and run:

```bash
cd Payload_Type/sebastian/sebastian/agent_code
cargo build --release --target x86_64-unknown-linux-gnu
```

To get the required build parameters (UUID, AES key, C2 config), kick off a build within Mythic and check the Payloads page for the details.

## Icon

Sebastian the lobster.
