# Claude Sandbox

Run Claude Code in isolated Docker containers with full development environment. **Now supports multiple instances!**

## Features

- **Multiple instances**: Run separate Claude sessions for different projects simultaneously
- **Isolated execution**: Each container has its own Claude state and only accesses folders you explicitly map
- **Auto-attach**: Running `run` on an existing folder automatically attaches to the running container
- **Folder-based naming**: Containers are automatically named based on the folder(s) you open
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
```

## Usage

### Start a new session

```bash
# Run with one folder - creates container "claude-my-project"
claude-sandbox run ./my-project

# Run with multiple folders - creates container "claude-my-project-shared-libs"
claude-sandbox run ./my-project ./shared-libs

# Run with an initial prompt
claude-sandbox run ./project -m "Review the code and suggest improvements"

# Run with a prompt from file
claude-sandbox run ./project -f ./prompts/review.txt
```

### Continue an existing session

```bash
# Continue by folder path (recommended)
claude-sandbox continue ./my-project

# Continue by folder name
claude-sandbox continue my-project

# Continue by container name
claude-sandbox continue claude-my-project

# Continue last used session
claude-sandbox continue
```

### Multiple instances

```bash
# Terminal 1: Start working on project A
claude-sandbox run ./project-a

# Terminal 2: Start working on project B (separate container)
claude-sandbox run ./project-b

# Each project has its own:
# - Container (claude-project-a, claude-project-b)
# - Claude state/memory
# - Conversation history
```

### Named sessions

```bash
# Create a named session for easy resumption
claude-sandbox run ./project -n feature-branch

# Resume by name later
claude-sandbox continue ./project -n feature-branch
```

### Container management

```bash
# List all Claude containers and their folder mappings
claude-sandbox list

# Open shell in a container
claude-sandbox shell ./my-project
claude-sandbox shell                    # uses last session

# Check status
claude-sandbox status ./my-project

# Stop a specific container
claude-sandbox stop ./my-project

# Stop all Claude containers
claude-sandbox stop all
```

### Resource limits

```bash
# Limit memory and CPU
claude-sandbox run ./project --memory 4g --cpus 2
```

### Port mapping

```bash
# Expose ports for web development
claude-sandbox run ./web-app -p 3000 -p 8080:8080
```

### Other commands

```bash
# Build/rebuild the Docker image
claude-sandbox build
claude-sandbox build --no-cache

# Reset all Claude state/memory (all containers)
claude-sandbox reset

# Resume specific conversation by ID
claude-sandbox resume <conversation-id> -t ./my-project
```

## Full CLI Reference

```
claude-sandbox run <FOLDERS>...
    -m, --prompt <PROMPT>           Initial prompt
    -f, --prompt-file <FILE>        File containing initial prompt
    -n, --name <NAME>               Named session (for easy resumption)
        --container <NAME>          Override auto-generated container name
        --memory <MEMORY>           Memory limit (e.g., "4g")
        --cpus <CPUS>               CPU limit (e.g., "2")
    -p, --port <PORT>               Expose ports (can specify multiple)
    -e, --env <KEY=VALUE>           Additional environment variables
        --dangerously-skip-permissions  Skip Claude permission prompts
    -c, --continue-session          Continue most recent conversation
    -r, --resume <ID>               Resume specific conversation by ID

claude-sandbox continue [TARGET]
    TARGET                          Folder path or container name
    -n, --name <NAME>               Resume named session

claude-sandbox resume [CONVERSATION_ID]
    -t, --target <TARGET>           Folder path or container name

claude-sandbox shell [TARGET]       Open bash shell in container
claude-sandbox stop [TARGET]        Stop a container (or "all")
claude-sandbox status [TARGET]      Show container status
claude-sandbox list                 List all containers with folder mappings

claude-sandbox build                Build Docker image
    --no-cache                      Force rebuild without cache
claude-sandbox reset                Reset Claude's persistent state
    -f, --force                     Skip confirmation

claude-sandbox completions <SHELL>  Generate shell completions (bash/zsh/fish)
```

## Container Naming

Containers are automatically named based on the folders you open:

| Command | Container Name |
|---------|---------------|
| `run ./my-project` | `claude-my-project` |
| `run ./project ./lib` | `claude-project-lib` |
| `run ./My-Project` | `claude-my-project` (lowercased) |
| `run ./project --container custom` | `custom` (override) |

## State Management

Claude Sandbox uses a hybrid approach for state:

**Shared globally** (in `~/.claude-sandbox/`):
- `.claude/` - Auth, credentials, and Claude settings
- `.claude.json` - Claude's settings (theme, preferences)
- `.config/` - Application configuration

**Isolated per-container** (in `~/.claude-sandbox/containers/<name>/`):
- `conversations/` - Project-specific conversation history

This means your API key, theme, and settings are shared across all containers, but each project has its own separate conversation history.

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
| `CLAUDE_SANDBOX_CONFIG` | Optional. Custom config directory (default: `~/.claude-sandbox`) |

## Data Storage

```
~/.claude-sandbox/
├── .claude/                  # Global Claude state (auth, settings) - SHARED
├── .claude.json              # Global settings (theme, preferences) - SHARED
├── .claude.json.backup       # Settings backup - SHARED
├── .config/                  # App configuration - SHARED
├── containers/
│   ├── claude-project-a/
│   │   └── conversations/    # Project conversations - ISOLATED
│   └── claude-project-b/
│       └── conversations/    # Project conversations - ISOLATED
├── folder_registry.json      # Maps folders to container names
├── named_sessions.json       # Maps session names to conversation IDs
├── last_session              # Last used container name
└── Dockerfile                # Generated during build
```
