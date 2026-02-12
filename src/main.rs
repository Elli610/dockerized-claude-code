use anyhow::{bail, Context, Result};
use clap::{CommandFactory, Parser, Subcommand};
use clap_complete::{generate, Shell};
use colored::Colorize;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::{self, Write};
use std::path::PathBuf;
use std::process::Stdio;
use tokio::process::Command;

const IMAGE_NAME: &str = "claude-code-sandbox";
const DEFAULT_SESSION: &str = "claude";
const CONTAINER_PREFIX: &str = "claude";

#[derive(Parser)]
#[command(name = "claude-sandbox")]
#[command(about = "Run Claude Code in an isolated Docker container")]
#[command(after_help = "Examples:
  claude-sandbox run ./my-project                    Start a new session
  claude-sandbox run ./app -p 3000 -p 8080:8080      Expose ports 3000 and 8080
  claude-sandbox continue ./my-project               Attach to existing session
  claude-sandbox stop all                            Stop all containers

Port formats for -p:
  3000              Map port 3000 to 3000
  8080:3000         Map host 8080 to container 3000
  127.0.0.1:8080:3000  Bind to specific IP")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Start Claude Code with mapped folders
    Run {
        /// Folders to map into the session
        #[arg(required = true)]
        folders: Vec<PathBuf>,
        /// Initial prompt to send to Claude
        #[arg(short = 'm', long)]
        prompt: Option<String>,
        /// Path to a file containing the initial prompt
        #[arg(short = 'f', long)]
        prompt_file: Option<PathBuf>,
        /// Named session (creates new conversation, can be resumed with continue -n)
        #[arg(short, long)]
        name: Option<String>,
        /// Override container name (default: derived from folder names)
        #[arg(long)]
        container: Option<String>,
        /// Memory limit (e.g., "4g")
        #[arg(short, long)]
        memory: Option<String>,
        /// CPU limit (e.g., "2")
        #[arg(long)]
        cpus: Option<String>,
        /// Expose container ports to host. Formats: PORT | HOST:CONTAINER | IP:HOST:CONTAINER
        #[arg(short = 'p', long = "port", value_name = "[HOST:]PORT")]
        ports: Vec<String>,
        /// Additional environment variables (KEY=VALUE)
        #[arg(short, long)]
        env: Vec<String>,
        /// Run in dangerously skip permissions mode
        #[arg(long)]
        dangerously_skip_permissions: bool,
        /// Continue the most recent conversation
        #[arg(short, long)]
        continue_session: bool,
        /// Resume a specific conversation by ID
        #[arg(short, long)]
        resume: Option<String>,
    },
    /// Continue a session by folder path or container name
    Continue {
        /// Folder path or container name to continue
        #[arg(required = false)]
        target: Option<String>,
        /// Named session to resume (omit to continue most recent conversation)
        #[arg(short, long)]
        name: Option<String>,
    },
    /// Resume a specific conversation by ID
    Resume {
        /// Conversation ID to resume
        conversation_id: Option<String>,
        /// Folder path or container name
        #[arg(short, long)]
        target: Option<String>,
    },
    /// Open a shell in a container
    Shell {
        /// Folder path or container name
        target: Option<String>,
    },
    /// Stop a running container
    Stop {
        /// Folder path or container name (or "all" to stop all containers)
        target: Option<String>,
    },
    /// List all Claude sandbox sessions
    List,
    /// Build or rebuild the Docker image
    Build {
        /// Force rebuild without cache
        #[arg(short, long)]
        no_cache: bool,
    },
    /// Reset Claude's persistent state
    Reset {
        /// Skip confirmation prompt
        #[arg(short, long)]
        force: bool,
    },
    /// Show status of a container
    Status {
        /// Folder path or container name
        target: Option<String>,
    },
    /// Generate shell completions
    Completions {
        /// Shell to generate completions for
        #[arg(value_enum)]
        shell: Shell,
    },
}

#[derive(Deserialize)]
struct ContainerInfo {
    #[serde(rename = "State")]
    state: ContainerState,
}

#[derive(Deserialize)]
struct ContainerState {
    #[serde(rename = "Status")]
    status: String,
    #[serde(rename = "Running")]
    running: bool,
}

struct RunConfig {
    folders: Vec<PathBuf>,
    prompt: Option<String>,
    prompt_file: Option<PathBuf>,
    session_name: Option<String>,
    container_override: Option<String>,
    memory: Option<String>,
    cpus: Option<String>,
    ports: Vec<String>,
    env_vars: Vec<String>,
    dangerously_skip_permissions: bool,
    continue_session: bool,
    resume: Option<String>,
}

/// Named sessions registry - maps session names to conversation IDs
#[derive(Serialize, Deserialize, Default)]
struct SessionsRegistry {
    sessions: HashMap<String, String>,
}

/// Folder registry - maps folder paths to container names
#[derive(Serialize, Deserialize, Default)]
struct FolderRegistry {
    /// Maps canonical folder path(s) hash to container name
    folders: HashMap<String, ContainerEntry>,
}

#[derive(Serialize, Deserialize, Clone)]
struct ContainerEntry {
    container_name: String,
    folder_paths: Vec<String>,
    created_at: String,
}

/// Parse and normalize a port mapping string
/// Supports: "8080", "8080:8080", "127.0.0.1:8080:8080"
fn normalize_port_mapping(port: &str) -> Result<String> {
    let parts: Vec<&str> = port.split(':').collect();
    match parts.len() {
        1 => {
            // Just a port number, map to same port on host
            let p: u16 = parts[0].parse().context("Invalid port number")?;
            Ok(format!("{}:{}", p, p))
        }
        2 => {
            // host:container format
            let _host: u16 = parts[0].parse().context("Invalid host port")?;
            let _container: u16 = parts[1].parse().context("Invalid container port")?;
            Ok(port.to_string())
        }
        3 => {
            // ip:host:container format
            let _host: u16 = parts[1].parse().context("Invalid host port")?;
            let _container: u16 = parts[2].parse().context("Invalid container port")?;
            Ok(port.to_string())
        }
        _ => bail!(
            "Invalid port format: {}. Use PORT, HOST:CONTAINER, or IP:HOST:CONTAINER",
            port
        ),
    }
}

fn get_config_dir() -> Result<PathBuf> {
    if let Ok(p) = std::env::var("CLAUDE_SANDBOX_CONFIG") {
        return Ok(PathBuf::from(p));
    }
    dirs::home_dir()
        .map(|h| h.join(".claude-sandbox"))
        .context("Could not determine home directory")
}

fn save_last_session(name: &str) -> Result<()> {
    let config_dir = get_config_dir()?;
    std::fs::create_dir_all(&config_dir)?;
    let path = config_dir.join("last_session");
    std::fs::write(&path, name)?;
    Ok(())
}

fn get_last_session() -> Result<String> {
    let config_dir = get_config_dir()?;
    let path = config_dir.join("last_session");
    if path.exists() {
        let name = std::fs::read_to_string(&path)?.trim().to_string();
        if !name.is_empty() {
            return Ok(name);
        }
    }
    Ok(DEFAULT_SESSION.to_string())
}

/// Sanitize a string for use in container names
fn sanitize_name(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
        .collect::<String>()
        .to_lowercase()
}

/// Derive container name from folder paths
fn derive_container_name(folders: &[PathBuf]) -> Result<String> {
    let names: Vec<String> = folders
        .iter()
        .filter_map(|f| {
            f.canonicalize()
                .ok()
                .and_then(|abs| abs.file_name().and_then(|n| n.to_str()).map(sanitize_name))
        })
        .filter(|n| !n.is_empty())
        .collect();

    if names.is_empty() {
        bail!("Could not derive container name from folders");
    }

    // Combine names, truncate if too long (docker limit is 128 chars)
    let combined = names.join("-");
    let name = format!("{}-{}", CONTAINER_PREFIX, combined);

    // Truncate if too long
    if name.len() > 64 {
        Ok(name[..64].to_string())
    } else {
        Ok(name)
    }
}

/// Get the folder registry path
fn get_folder_registry_path() -> Result<PathBuf> {
    let config_dir = get_config_dir()?;
    Ok(config_dir.join("folder_registry.json"))
}

/// Load the folder registry
fn load_folder_registry() -> Result<FolderRegistry> {
    let path = get_folder_registry_path()?;
    if path.exists() {
        let content = std::fs::read_to_string(&path)?;
        Ok(serde_json::from_str(&content).unwrap_or_default())
    } else {
        Ok(FolderRegistry::default())
    }
}

/// Save the folder registry
fn save_folder_registry(registry: &FolderRegistry) -> Result<()> {
    let path = get_folder_registry_path()?;
    let config_dir = get_config_dir()?;
    std::fs::create_dir_all(&config_dir)?;
    let content = serde_json::to_string_pretty(registry)?;
    std::fs::write(&path, content)?;
    Ok(())
}

/// Create a key for the folder registry from folder paths
fn folder_key(folders: &[PathBuf]) -> Result<String> {
    let mut paths: Vec<String> = folders
        .iter()
        .filter_map(|f| f.canonicalize().ok())
        .map(|p| p.to_string_lossy().to_string())
        .collect();
    paths.sort();
    Ok(paths.join(":"))
}

/// Register a container with its folders
fn register_container(container_name: &str, folders: &[PathBuf]) -> Result<()> {
    let mut registry = load_folder_registry()?;
    let key = folder_key(folders)?;
    let paths: Vec<String> = folders
        .iter()
        .filter_map(|f| f.canonicalize().ok())
        .map(|p| p.to_string_lossy().to_string())
        .collect();

    registry.folders.insert(
        key,
        ContainerEntry {
            container_name: container_name.to_string(),
            folder_paths: paths,
            created_at: chrono::Local::now().to_rfc3339(),
        },
    );
    save_folder_registry(&registry)?;
    Ok(())
}

/// Look up container name by folder path
fn lookup_container_by_folder(folder: &str) -> Result<Option<String>> {
    let registry = load_folder_registry()?;

    // First, try exact path match
    let folder_path = PathBuf::from(folder);
    if let Ok(canonical) = folder_path.canonicalize() {
        let canonical_str = canonical.to_string_lossy().to_string();

        // Check if this folder is part of any registered container
        for entry in registry.folders.values() {
            if entry.folder_paths.contains(&canonical_str) {
                return Ok(Some(entry.container_name.clone()));
            }
            // Also check if folder name matches container suffix
            if let Some(name) = canonical.file_name().and_then(|n| n.to_str()) {
                let expected = format!("{}-{}", CONTAINER_PREFIX, sanitize_name(name));
                if entry.container_name == expected || entry.container_name.starts_with(&expected) {
                    return Ok(Some(entry.container_name.clone()));
                }
            }
        }
    }

    Ok(None)
}

/// Resolve target (folder path or container name) to container name
fn resolve_target_to_container(target: Option<&str>) -> Result<String> {
    match target {
        None => get_last_session(),
        Some(t) => {
            // Check if it's an existing container name
            let path = PathBuf::from(t);
            if path.exists() && path.is_dir() {
                // It's a folder path, look up or derive container name
                if let Some(name) = lookup_container_by_folder(t)? {
                    return Ok(name);
                }
                // Not registered, derive from folder name
                return derive_container_name(&[path]);
            }

            // Check if it looks like a container name (starts with prefix)
            if t.starts_with(CONTAINER_PREFIX) || t.starts_with("claude") {
                return Ok(t.to_string());
            }

            // Try as folder name without path
            if let Some(name) = lookup_container_by_folder(t)? {
                return Ok(name);
            }

            // Assume it's a container name
            Ok(t.to_string())
        }
    }
}

/// Get per-container config directory for isolated state
fn get_container_config_dir(container_name: &str) -> Result<PathBuf> {
    let config_dir = get_config_dir()?;
    Ok(config_dir.join("containers").join(container_name))
}

fn get_sessions_registry_path() -> Result<PathBuf> {
    let config_dir = get_config_dir()?;
    Ok(config_dir.join("named_sessions.json"))
}

fn load_sessions_registry() -> Result<SessionsRegistry> {
    let path = get_sessions_registry_path()?;
    if path.exists() {
        let content = std::fs::read_to_string(&path)?;
        Ok(serde_json::from_str(&content).unwrap_or_default())
    } else {
        Ok(SessionsRegistry::default())
    }
}

fn save_sessions_registry(registry: &SessionsRegistry) -> Result<()> {
    let path = get_sessions_registry_path()?;
    let config_dir = get_config_dir()?;
    std::fs::create_dir_all(&config_dir)?;
    let content = serde_json::to_string_pretty(registry)?;
    std::fs::write(&path, content)?;
    Ok(())
}

fn save_named_session(name: &str, conversation_id: &str) -> Result<()> {
    let mut registry = load_sessions_registry()?;
    registry
        .sessions
        .insert(name.to_string(), conversation_id.to_string());
    save_sessions_registry(&registry)?;
    Ok(())
}

fn get_named_session(name: &str) -> Result<Option<String>> {
    let registry = load_sessions_registry()?;
    Ok(registry.sessions.get(name).cloned())
}

/// Detect the most recent conversation ID by looking at .claude directory
async fn detect_latest_conversation_id(container: &str) -> Result<Option<String>> {
    let output = Command::new("docker")
        .args([
            "exec", container, "bash", "-c",
            "find /home/claude/.claude -name 'conversations' -type d 2>/dev/null | head -1 | xargs -I{} find {} -maxdepth 1 -type d 2>/dev/null | tail -n +2 | xargs -I{} stat --format='%Y %n' {} 2>/dev/null | sort -rn | head -1 | awk '{print $2}' | xargs -I{} basename {}"
        ])
        .output()
        .await?;

    let id = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if id.is_empty() {
        Ok(None)
    } else {
        Ok(Some(id))
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum SessionAction {
    NewSession,
    Continue,
}

fn get_dockerfile_content() -> &'static str {
    r#"FROM debian:bookworm-slim

ENV HOME=/home/claude
ENV LANG=C.UTF-8
ENV LC_ALL=C.UTF-8
ENV TERM=xterm-256color

# Install system dependencies
RUN apt-get update && apt-get install -y \
    curl \
    git \
    ca-certificates \
    build-essential \
    pkg-config \
    libssl-dev \
    xz-utils \
    && rm -rf /var/lib/apt/lists/*

# Create user and directories
RUN useradd -m -s /bin/bash -d /home/claude claude && \
    mkdir -p /home/claude/workspace /home/claude/.claude /home/claude/.config /home/claude/.local/bin && \
    touch /home/claude/.claude.json /home/claude/.claude.json.backup && \
    chown -R claude:claude /home/claude

USER claude
WORKDIR /home/claude

# Set up PATH for all tools
ENV PATH="/home/claude/.local/bin:/home/claude/.cargo/bin:/home/claude/.foundry/bin:${PATH}"
ENV RUSTUP_HOME=/home/claude/.rustup
ENV CARGO_HOME=/home/claude/.cargo
ENV NVM_DIR=/home/claude/.nvm

# Install rustup and Rust (stable toolchain)
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable

# Install Foundry (forge, cast, anvil)
RUN curl -L https://foundry.paradigm.xyz | bash && \
    /home/claude/.foundry/bin/foundryup

# Install nvm and latest Node.js
RUN curl -o- https://raw.githubusercontent.com/nvm-sh/nvm/v0.40.1/install.sh | bash && \
    bash -c "source $NVM_DIR/nvm.sh && nvm install node"

# Create symlinks for node/npm/npx to be accessible without sourcing nvm
RUN bash -c "source $NVM_DIR/nvm.sh && \
    ln -sf \$(which node) /home/claude/.local/bin/node && \
    ln -sf \$(which npm) /home/claude/.local/bin/npm && \
    ln -sf \$(which npx) /home/claude/.local/bin/npx"

# Install claude-code globally and create symlink
RUN bash -c "source $NVM_DIR/nvm.sh && npm install -g @anthropic-ai/claude-code" && \
    bash -c "source $NVM_DIR/nvm.sh && ln -sf \$(which claude) /home/claude/.local/bin/claude"

# Setup bashrc for interactive shells
RUN echo 'export PATH=\"/home/claude/.local/bin:/home/claude/.cargo/bin:/home/claude/.foundry/bin:\$PATH\"' >> /home/claude/.bashrc && \
    echo 'export NVM_DIR=\"\$HOME/.nvm\"' >> /home/claude/.bashrc && \
    echo '[ -s \"\$NVM_DIR/nvm.sh\" ] && . \"\$NVM_DIR/nvm.sh\"' >> /home/claude/.bashrc

# Verify installations
RUN cargo --version && rustc --version && \
    forge --version && \
    node --version && npm --version

WORKDIR /home/claude/workspace

CMD ["tail", "-f", "/dev/null"]
"#
}

/// Resolve a folder path to an absolute path and extract the folder name
fn resolve_folder_path(folder: &PathBuf) -> Result<(PathBuf, String)> {
    let abs = std::fs::canonicalize(folder)
        .with_context(|| format!("Cannot access folder: {}", folder.display()))?;

    let fname = abs
        .file_name()
        .map(|n| n.to_str().unwrap_or("project"))
        .unwrap_or("project")
        .to_string();

    Ok((abs, fname))
}

async fn check_docker() -> Result<()> {
    let status = Command::new("docker")
        .arg("info")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await?;
    if !status.success() {
        bail!("Docker is not running. Please start Docker and try again.");
    }
    Ok(())
}

async fn image_exists() -> Result<bool> {
    let output = Command::new("docker")
        .args(["image", "inspect", IMAGE_NAME])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await?;
    Ok(output.success())
}

async fn container_exists(name: &str) -> Result<bool> {
    let output = Command::new("docker")
        .args(["container", "inspect", name])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await?;
    Ok(output.success())
}

async fn container_running(name: &str) -> Result<bool> {
    let output = Command::new("docker")
        .args(["inspect", "-f", "{{.State.Running}}", name])
        .output()
        .await?;
    Ok(String::from_utf8_lossy(&output.stdout).trim() == "true")
}

async fn build_image(no_cache: bool) -> Result<()> {
    println!("{}", "Building Claude Code sandbox image...".cyan());
    let config_dir = get_config_dir()?;
    std::fs::create_dir_all(&config_dir)?;
    let dockerfile_path = config_dir.join("Dockerfile");
    std::fs::write(&dockerfile_path, get_dockerfile_content())?;
    let mut cmd = Command::new("docker");
    cmd.args(["build", "-t", IMAGE_NAME]);
    if no_cache {
        cmd.arg("--no-cache");
    }
    cmd.args([
        "-f",
        dockerfile_path.to_str().unwrap(),
        config_dir.to_str().unwrap(),
    ]);
    let status = cmd.status().await?;
    if !status.success() {
        bail!("Failed to build Docker image");
    }
    println!("{}", "Image built successfully!".green());
    Ok(())
}

async fn start_container(
    name: &str,
    folders: &[PathBuf],
    memory: Option<&str>,
    cpus: Option<&str>,
    ports: &[String],
    env_vars: &[String],
) -> Result<()> {
    // Per-container directory for isolated conversation history
    let container_config_dir = get_container_config_dir(name)?;
    std::fs::create_dir_all(&container_config_dir)?;

    // Global config directory for shared settings (theme, preferences)
    let global_config_dir = get_config_dir()?;
    std::fs::create_dir_all(&global_config_dir)?;

    let mut args = vec![
        "run".to_string(),
        "-d".to_string(),
        "--name".to_string(),
        name.to_string(),
    ];

    for folder in folders {
        let (abs, fname) = resolve_folder_path(folder)?;
        args.extend([
            "-v".to_string(),
            format!("{}:/home/claude/workspace/{}", abs.display(), fname),
        ]);
    }

    // Mount global .claude directory (for auth, settings, etc.)
    let global_claude_dir = global_config_dir.join(".claude");
    std::fs::create_dir_all(&global_claude_dir)?;
    args.extend([
        "-v".to_string(),
        format!("{}:/home/claude/.claude", global_claude_dir.display()),
    ]);

    // Mount per-container conversations directory (overlay for isolated history)
    let container_conversations = container_config_dir.join("conversations");
    std::fs::create_dir_all(&container_conversations)?;
    args.extend([
        "-v".to_string(),
        format!(
            "{}:/home/claude/.claude/projects",
            container_conversations.display()
        ),
    ]);

    // Mount .claude.json files (GLOBAL - shared settings like theme)
    let claude_json = global_config_dir.join(".claude.json");
    let claude_json_backup = global_config_dir.join(".claude.json.backup");
    if !claude_json.exists() {
        std::fs::write(&claude_json, "{}")?;
    }
    if !claude_json_backup.exists() {
        std::fs::write(&claude_json_backup, "{}")?;
    }
    args.extend([
        "-v".to_string(),
        format!("{}:/home/claude/.claude.json", claude_json.display()),
    ]);
    args.extend([
        "-v".to_string(),
        format!(
            "{}:/home/claude/.claude.json.backup",
            claude_json_backup.display()
        ),
    ]);

    // Mount .config directory (GLOBAL - shared app settings)
    let config_app_dir = global_config_dir.join(".config");
    std::fs::create_dir_all(&config_app_dir)?;
    args.extend([
        "-v".to_string(),
        format!("{}:/home/claude/.config", config_app_dir.display()),
    ]);

    if let Some(m) = memory {
        args.extend(["--memory".to_string(), m.to_string()]);
    }
    if let Some(c) = cpus {
        args.extend(["--cpus".to_string(), c.to_string()]);
    }

    // Add port mappings
    for port in ports {
        let normalized = normalize_port_mapping(port)?;
        args.extend(["-p".to_string(), normalized]);
    }

    args.extend(["-e".to_string(), "ANTHROPIC_API_KEY".to_string()]);
    args.extend(["-e".to_string(), "TERM=xterm-256color".to_string()]);
    for e in env_vars {
        args.extend(["-e".to_string(), e.clone()]);
    }

    args.extend(["--network".to_string(), "bridge".to_string()]);
    args.push(IMAGE_NAME.to_string());

    let output = Command::new("docker").args(&args).output().await?;
    if !output.status.success() {
        bail!(
            "Failed to start container: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    // Wait for container to be ready
    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

    Ok(())
}

async fn exec_claude_interactive(
    name: &str,
    prompt: Option<&str>,
    dangerously_skip_permissions: bool,
    continue_session: bool,
    resume: Option<&str>,
) -> Result<()> {
    let mut args = vec![
        "exec".to_string(),
        "-it".to_string(),
        name.to_string(),
        "claude".to_string(),
    ];

    if dangerously_skip_permissions {
        args.push("--dangerously-skip-permissions".to_string());
    }

    if continue_session {
        args.push("-c".to_string());
    } else if let Some(session) = resume {
        args.push("-r".to_string());
        args.push(session.to_string());
    }

    if let Some(p) = prompt {
        args.push(p.to_string());
    }

    // Use std::process::Command for proper TTY handling
    let status = std::process::Command::new("docker")
        .args(&args)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()?;

    if !status.success() && status.code() != Some(0) {
        // Claude exited, which is fine
    }

    Ok(())
}

fn print_banner(
    container: &str,
    session_name: Option<&str>,
    ports: &[String],
    folders: &[PathBuf],
) {
    println!("\n{}", "═".repeat(70).cyan());
    if let Some(name) = session_name {
        println!(
            "{}  Claude Code running session '{}' in container '{}'",
            "│".cyan(),
            name.green(),
            container.blue()
        );
    } else {
        println!(
            "{}  Claude Code running in container '{}'",
            "│".cyan(),
            container.green()
        );
    }

    // Show mapped folders
    if !folders.is_empty() {
        println!("{}  {}", "│".cyan(), "Mapped folders:".bold());
        for folder in folders {
            if let Ok((abs, fname)) = resolve_folder_path(folder) {
                println!(
                    "{}    {} {} -> /home/claude/workspace/{}",
                    "│".cyan(),
                    "→".green(),
                    abs.display(),
                    fname
                );
            }
        }
    }

    // Show exposed ports
    if !ports.is_empty() {
        println!("{}  {}", "│".cyan(), "Exposed ports:".bold());
        for port in ports {
            let normalized = normalize_port_mapping(port).unwrap_or_else(|_| port.clone());
            println!("{}    {} {}", "│".cyan(), "→".green(), normalized);
        }
    }

    println!(
        "{}  Press {} to exit (container keeps running)",
        "│".cyan(),
        "Ctrl+C".yellow().bold()
    );
    println!("{}", "│".cyan());
    println!("{}  Reconnect with:", "│".cyan());
    let folder_hint = folders
        .first()
        .and_then(|f| f.to_str())
        .unwrap_or(container);
    if let Some(name) = session_name {
        println!(
            "{}    {} - resume this named session",
            "│".cyan(),
            format!("claude-sandbox continue {} -n {}", folder_hint, name).green()
        );
    } else {
        println!(
            "{}    {} - continue last conversation",
            "│".cyan(),
            format!("claude-sandbox continue {}", folder_hint).green()
        );
    }
    println!(
        "{}    {} - resume specific conversation by ID",
        "│".cyan(),
        format!("claude-sandbox resume -t {} <id>", folder_hint).green()
    );
    println!("{}\n", "═".repeat(70).cyan());
}

async fn run_claude(mut config: RunConfig) -> Result<()> {
    check_docker().await?;

    if !image_exists().await? {
        println!("{}", "Image not found, building...".yellow());
        build_image(false).await?;
    }

    // Derive container name from folders if not overridden
    let container_name = match &config.container_override {
        Some(name) => name.clone(),
        None => derive_container_name(&config.folders)?,
    };

    // Check if container already exists and is running
    let container_exists_flag = container_exists(&container_name).await?;
    let container_running_flag = if container_exists_flag {
        container_running(&container_name).await?
    } else {
        false
    };

    // If we have a named session, check if it already exists
    if let Some(ref session_name) = config.session_name {
        if let Some(conversation_id) = get_named_session(session_name)? {
            println!(
                "{}",
                format!(
                    "Session '{}' already exists (conversation: {})",
                    session_name,
                    &conversation_id[..8.min(conversation_id.len())]
                )
                .yellow()
            );
            print!("Overwrite with new session? [y/N]: ");
            io::stdout().flush()?;
            let mut input = String::new();
            io::stdin().read_line(&mut input)?;
            if !input.trim().eq_ignore_ascii_case("y") {
                println!("Use 'continue -n {}' to resume it.", session_name);
                return Ok(());
            }
        }
    }

    // Determine what action to take based on container state
    let action = if container_running_flag {
        // Check if user specified ports - these require container recreation
        if !config.ports.is_empty() {
            println!(
                "{}",
                format!("Container '{}' is already running.", container_name).yellow()
            );
            print!(
                "Recreate with ports {}? [y/N]: ",
                config.ports.join(", ").cyan()
            );
            io::stdout().flush()?;
            let mut input = String::new();
            io::stdin().read_line(&mut input)?;
            if input.trim().eq_ignore_ascii_case("y") {
                SessionAction::NewSession
            } else {
                println!("Attaching without port changes...");
                SessionAction::Continue
            }
        } else {
            // No ports specified, auto-attach
            println!(
                "{}",
                format!(
                    "Container '{}' is already running, attaching...",
                    container_name
                )
                .cyan()
            );
            SessionAction::Continue
        }
    } else {
        SessionAction::NewSession
    };

    match action {
        SessionAction::Continue => {
            // Just continue the existing session - auto-attach
            config.continue_session = true;
        }
        SessionAction::NewSession => {
            // Need to create a new container
            if container_exists_flag {
                // Remove the existing container first
                if container_running_flag {
                    println!(
                        "{}",
                        format!("Stopping existing container '{}'...", container_name).yellow()
                    );
                    Command::new("docker")
                        .args(["stop", &container_name])
                        .status()
                        .await?;
                }
                Command::new("docker")
                    .args(["rm", &container_name])
                    .status()
                    .await?;
            }

            if let Some(ref name) = config.session_name {
                println!(
                    "{}",
                    format!(
                        "Starting new session '{}' in container '{}'...",
                        name, container_name
                    )
                    .cyan()
                );
            } else {
                println!(
                    "{}",
                    format!("Starting new container '{}'...", container_name).cyan()
                );
            }
            println!("{}:", "Mapped folders".bold());
            for folder in &config.folders {
                let (abs, fname) = resolve_folder_path(folder)?;
                println!(
                    "  {} -> /home/claude/workspace/{}",
                    abs.display().to_string().blue(),
                    fname
                );
            }

            if !config.ports.is_empty() {
                println!("{}:", "Exposed ports".bold());
                for port in &config.ports {
                    let normalized = normalize_port_mapping(port)?;
                    println!("  {} {}", "→".green(), normalized);
                }
            }

            start_container(
                &container_name,
                &config.folders,
                config.memory.as_deref(),
                config.cpus.as_deref(),
                &config.ports,
                &config.env_vars,
            )
            .await?;

            // Register the container with its folders
            register_container(&container_name, &config.folders)?;
        }
    }

    // Save the current container as the last used session
    save_last_session(&container_name)?;

    let final_prompt = match (config.prompt, config.prompt_file) {
        (Some(p), _) => Some(p),
        (None, Some(f)) => Some(std::fs::read_to_string(&f)?),
        (None, None) => None,
    };

    print_banner(
        &container_name,
        config.session_name.as_deref(),
        &config.ports,
        &config.folders,
    );

    exec_claude_interactive(
        &container_name,
        final_prompt.as_deref(),
        config.dangerously_skip_permissions,
        config.continue_session,
        config.resume.as_deref(),
    )
    .await?;

    // If this was a named session, detect and save the conversation ID
    if let Some(ref session_name) = config.session_name {
        if let Some(conv_id) = detect_latest_conversation_id(&container_name).await? {
            save_named_session(session_name, &conv_id)?;
            println!(
                "\n{} Session '{}' saved (conversation: {})",
                "✓".green(),
                session_name,
                &conv_id[..8.min(conv_id.len())]
            );
        } else {
            println!(
                "\n{} Could not detect conversation ID for session '{}'",
                "⚠".yellow(),
                session_name
            );
        }
    }

    println!("\n{} Exited Claude session", "✓".green());
    println!("  Container '{}' is still running", container_name);
    // Show how to reconnect
    let folder_hint = config
        .folders
        .first()
        .and_then(|f| f.to_str())
        .unwrap_or(&container_name);
    if let Some(ref name) = config.session_name {
        println!(
            "  Use {} to resume this session",
            format!("claude-sandbox continue {} -n {}", folder_hint, name).yellow()
        );
    } else {
        println!(
            "  Use {} to continue",
            format!("claude-sandbox continue {}", folder_hint).yellow()
        );
    }

    Ok(())
}

async fn continue_session_cmd(container: &str, session_name: Option<&str>) -> Result<()> {
    check_docker().await?;

    if !container_running(container).await? {
        bail!("Container '{container}' is not running. Use 'run' to start it.");
    }

    // Save as last used session
    save_last_session(container)?;

    // If a named session is provided, look up the conversation ID and resume
    if let Some(name) = session_name {
        let conversation_id = get_named_session(name)?.ok_or_else(|| {
            anyhow::anyhow!(
                "Named session '{}' not found. Use 'run -n {}' to create it.",
                name,
                name
            )
        })?;

        println!(
            "{}",
            format!(
                "Resuming session '{}' (conversation: {}) in container '{}'...",
                name,
                &conversation_id[..8.min(conversation_id.len())],
                container
            )
            .cyan()
        );

        exec_claude_interactive(container, None, false, false, Some(&conversation_id)).await?;

        println!("\n{} Exited session '{}'", "✓".green(), name);
    } else {
        println!(
            "{}",
            format!("Continuing last conversation in container '{container}'...").cyan()
        );

        exec_claude_interactive(container, None, false, true, None).await?;

        println!("\n{} Exited Claude session", "✓".green());
    }

    println!("  Container '{container}' is still running");

    Ok(())
}

async fn resume_session_cmd(container: &str, conversation: Option<&str>) -> Result<()> {
    check_docker().await?;

    if !container_running(container).await? {
        bail!("Container '{container}' is not running. Use 'run' to start it.");
    }

    // Save as last used container
    save_last_session(container)?;

    if let Some(c) = conversation {
        println!(
            "{}",
            format!("Resuming conversation '{c}' in container '{container}'...").cyan()
        );
    } else {
        println!(
            "{}",
            format!("Opening conversation picker in container '{container}'...").cyan()
        );
    }

    // If no conversation specified, claude -r will show interactive picker
    exec_claude_interactive(container, None, false, false, conversation.or(Some(""))).await?;

    println!("\n{} Exited Claude session", "✓".green());
    println!("  Container '{container}' is still running");

    Ok(())
}

async fn shell_container(container: &str) -> Result<()> {
    check_docker().await?;
    if !container_running(container).await? {
        bail!("Container '{container}' is not running");
    }
    // Save as last used container
    save_last_session(container)?;
    println!(
        "{}",
        format!("Opening shell in container '{container}'...").cyan()
    );
    std::process::Command::new("docker")
        .args(["exec", "-it", container, "bash"])
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()?;
    Ok(())
}

async fn stop_container(container: &str) -> Result<()> {
    check_docker().await?;
    if !container_exists(container).await? {
        bail!("Container '{container}' does not exist");
    }
    println!("{}", format!("Stopping container '{container}'...").cyan());
    Command::new("docker")
        .args(["stop", container])
        .status()
        .await?;
    Command::new("docker")
        .args(["rm", container])
        .status()
        .await?;
    println!("{} Container stopped and removed", "✓".green());
    Ok(())
}

async fn stop_all_containers() -> Result<()> {
    check_docker().await?;
    println!("{}", "Stopping all Claude sandbox containers...".cyan());

    // Get all containers using our image
    let output = Command::new("docker")
        .args([
            "ps",
            "-a",
            "--filter",
            &format!("ancestor={IMAGE_NAME}"),
            "--format",
            "{{.Names}}",
        ])
        .output()
        .await?;

    let output_str = String::from_utf8_lossy(&output.stdout).to_string();
    let containers: Vec<&str> = output_str.lines().filter(|s| !s.is_empty()).collect();

    if containers.is_empty() {
        println!("No containers to stop.");
        return Ok(());
    }

    for container in &containers {
        println!("  Removing '{}'...", container);
        // Stop if running, then remove
        let _ = Command::new("docker")
            .args(["stop", container])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .await;
        let _ = Command::new("docker")
            .args(["rm", "-f", container])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .await;
    }

    println!(
        "{} Removed {} container(s)",
        "✓".green(),
        containers.len()
    );
    Ok(())
}

async fn list_sessions() -> Result<()> {
    check_docker().await?;
    println!("{}", "Claude sandbox containers:".bold());
    let output = Command::new("docker")
        .args([
            "ps",
            "-a",
            "--filter",
            &format!("ancestor={IMAGE_NAME}"),
            "--format",
            "table {{.Names}}\t{{.Status}}\t{{.Ports}}\t{{.CreatedAt}}",
        ])
        .output()
        .await?;
    print!("{}", String::from_utf8_lossy(&output.stdout));

    // Show the last used container
    if let Ok(last) = get_last_session() {
        println!("\n{}: {}", "Last used container".cyan(), last.green());
    }

    // Show folder mappings
    let folder_registry = load_folder_registry()?;
    if !folder_registry.folders.is_empty() {
        println!("\n{}", "Folder mappings:".bold());
        for entry in folder_registry.folders.values() {
            let folders_str = entry
                .folder_paths
                .iter()
                .map(|p| {
                    PathBuf::from(p)
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or(p)
                        .to_string()
                })
                .collect::<Vec<_>>()
                .join(", ");
            println!(
                "  {} {} [{}]",
                entry.container_name.green(),
                "←".cyan(),
                folders_str.blue()
            );
        }
    }

    // Show named sessions
    let registry = load_sessions_registry()?;
    if !registry.sessions.is_empty() {
        println!("\n{}", "Named sessions:".bold());
        for (name, conv_id) in &registry.sessions {
            println!(
                "  {} -> {}",
                name.green(),
                &conv_id[..8.min(conv_id.len())].blue()
            );
        }
    }
    Ok(())
}

async fn reset_state(force: bool) -> Result<()> {
    let config_dir = get_config_dir()?;
    if !force {
        println!(
            "{}",
            "This will delete all Claude sandbox state and memory.".yellow()
        );
        println!("Config directory: {}", config_dir.display());
        print!("Continue? [y/N] ");
        io::stdout().flush()?;
        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        if !input.trim().eq_ignore_ascii_case("y") {
            println!("Aborted.");
            return Ok(());
        }
    }
    if config_dir.exists() {
        std::fs::remove_dir_all(&config_dir)?;
        println!("{} State reset successfully", "✓".green());
    } else {
        println!("No state to reset.");
    }
    Ok(())
}

async fn status_container(container: &str) -> Result<()> {
    check_docker().await?;
    if !container_exists(container).await? {
        println!("{} Container '{}' does not exist", "✗".red(), container);
        return Ok(());
    }
    let output = Command::new("docker")
        .args(["inspect", container])
        .output()
        .await?;
    let info: Vec<ContainerInfo> = serde_json::from_slice(&output.stdout)?;
    if let Some(i) = info.first() {
        let icon = if i.state.running {
            "●".green()
        } else {
            "○".red()
        };
        println!("{} Container '{}': {}", icon, container, i.state.status);
    }
    Ok(())
}

fn print_completions(shell: Shell) {
    let mut cmd = Cli::command();
    generate(shell, &mut cmd, "claude-sandbox", &mut io::stdout());
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Run {
            folders,
            prompt,
            prompt_file,
            name,
            container,
            memory,
            cpus,
            ports,
            env,
            dangerously_skip_permissions,
            continue_session,
            resume,
        } => {
            run_claude(RunConfig {
                folders,
                prompt,
                prompt_file,
                session_name: name,
                container_override: container,
                memory,
                cpus,
                ports,
                env_vars: env,
                dangerously_skip_permissions,
                continue_session,
                resume,
            })
            .await
        }
        Commands::Continue { target, name } => {
            let container_name = resolve_target_to_container(target.as_deref())?;
            continue_session_cmd(&container_name, name.as_deref()).await
        }
        Commands::Resume {
            conversation_id,
            target,
        } => {
            let container_name = resolve_target_to_container(target.as_deref())?;
            resume_session_cmd(&container_name, conversation_id.as_deref()).await
        }
        Commands::Shell { target } => {
            let container_name = resolve_target_to_container(target.as_deref())?;
            shell_container(&container_name).await
        }
        Commands::Stop { target } => {
            // Handle "all" to stop all containers
            if target.as_deref() == Some("all") {
                stop_all_containers().await
            } else {
                let container_name = resolve_target_to_container(target.as_deref())?;
                stop_container(&container_name).await
            }
        }
        Commands::List => list_sessions().await,
        Commands::Build { no_cache } => build_image(no_cache).await,
        Commands::Reset { force } => reset_state(force).await,
        Commands::Status { target } => {
            let container_name = resolve_target_to_container(target.as_deref())?;
            status_container(&container_name).await
        }
        Commands::Completions { shell } => {
            print_completions(shell);
            Ok(())
        }
    }
}
