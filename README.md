# Claude Sandbox

Run Claude Code in an isolated Docker container with full development environment.

## Features

- **Isolated execution**: Claude only accesses folders you explicitly map
- **Persistent memory**: Claude's state persists between sessions
- **Background mode**: Run Claude tasks and get Discord notifications when done
- **Full dev environment**: Rust, Node.js (via nvm), Foundry (Solidity)
- **Cross-platform**: Works on macOS, Linux, and Windows

## Installation

### Prerequisites
- Docker installed and running
- Rust toolchain (for building)

### Build from source

```bash
git clone https://github.com/youruser/claude-sandbox.git
cd claude-sandbox

# Build release binary
cargo build --release

# Copy to PATH (Linux/macOS)
sudo cp target/release/claude-sandbox /usr/local/bin/

# Or on Windows, add target/release to your PATH
```

### Set your API key

```bash
# Add to ~/.bashrc, ~/.zshrc, or Windows environment
export ANTHROPIC_API_KEY="your-key-here"

# Optional: Discord webhook for notifications
export CLAUDE_DISCORD_WEBHOOK="https://discord.com/api/webhooks/..."
```

## Usage

### Basic usage

```bash
# Run interactively with one folder
claude-sandbox run ./my-project

# Run with multiple folders
claude-sandbox run ./project ./shared-libs ./configs

# Run with an initial prompt
claude-sandbox run ./project -p "Review the code and suggest improvements"

# Run with a prompt from file
claude-sandbox run ./project -f ./prompts/review.txt
```

### Background execution

```bash
# Start in background
claude-sandbox run ./project -p "Refactor all files to use async/await" --background

# Check status
claude-sandbox status

# View logs
claude-sandbox logs
claude-sandbox logs --follow

# Attach to running container
claude-sandbox attach

# Open a shell to inspect/debug
claude-sandbox shell

# Stop when needed
claude-sandbox stop
```

### Container management

```bash
# List all Claude containers
claude-sandbox list

# Use a custom container name
claude-sandbox run ./project -n my-task --background

# Check specific container
claude-sandbox status -n my-task
claude-sandbox logs -n my-task
claude-sandbox attach -n my-task
```

### Resource limits

```bash
# Limit memory and CPU
claude-sandbox run ./project --memory 4g --cpus 2
```

### Discord notifications

```bash
# Set webhook globally
export CLAUDE_DISCORD_WEBHOOK="https://discord.com/api/webhooks/..."

# Or per-run
claude-sandbox run ./project --background --discord-webhook "https://..."
```

You'll receive notifications when:
- Claude sandbox starts
- Claude sandbox finishes

### Other commands

```bash
# Build/rebuild the Docker image
claude-sandbox build
claude-sandbox build --no-cache

# Reset all Claude state/memory
claude-sandbox reset
```

## Full CLI Reference

```
claude-sandbox run <FOLDERS>...
    -p, --prompt <PROMPT>           Initial prompt
    -f, --prompt-file <FILE>        File containing initial prompt
    -b, --background                Run in background (detached)
    -n, --name <NAME>               Container name [default: claude]
    -m, --memory <MEMORY>           Memory limit (e.g., "4g")
        --cpus <CPUS>               CPU limit (e.g., "2")
        --discord-webhook <URL>     Discord webhook for notifications
    -e, --env <KEY=VALUE>           Additional environment variables
        --dangerously-skip-permissions  Skip Claude permission prompts

claude-sandbox attach [-n <NAME>]   Attach to running container
claude-sandbox shell [-n <NAME>]    Open bash shell in container
claude-sandbox logs [-n <NAME>]     View container logs
    -f, --follow                    Follow log output
    -t, --tail <LINES>              Lines to show [default: 100]

claude-sandbox stop [-n <NAME>]     Stop a running container
claude-sandbox list                 List all Claude containers
claude-sandbox status [-n <NAME>]   Show container status
claude-sandbox build                Build Docker image
    --no-cache                      Force rebuild without cache
claude-sandbox reset                Reset Claude's persistent state
    -f, --force                     Skip confirmation
```

## Installed Tools in Container

| Tool | Description |
|------|-------------|
| **Rust** | Latest stable via rustup, includes rustfmt and clippy |
| **Node.js** | LTS version via nvm |
| **Foundry** | forge, cast, anvil, chisel for Solidity development |
| **Git** | Version control |
| **Claude Code** | The AI coding assistant |

## Configuration

| Environment Variable | Description |
|---------------------|-------------|
| `ANTHROPIC_API_KEY` | Required. Your Anthropic API key |
| `CLAUDE_DISCORD_WEBHOOK` | Optional. Discord webhook URL |
| `CLAUDE_SANDBOX_CONFIG` | Optional. Custom config directory (default: `~/.claude-sandbox`) |
