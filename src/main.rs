use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use colored::Colorize;
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::PathBuf;
use std::process::Stdio;
use tokio::process::Command;

const IMAGE_NAME: &str = "claude-code-sandbox";
const DEFAULT_CONTAINER: &str = "claude";

#[derive(Parser)]
#[command(name = "claude-sandbox")]
#[command(about = "Run Claude Code in an isolated Docker container", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Start Claude Code with mapped folders
    Run {
        /// Folders to map into the container
        #[arg(required = true)]
        folders: Vec<PathBuf>,
        /// Initial prompt to send to Claude
        #[arg(short, long)]
        prompt: Option<String>,
        /// Path to a file containing the initial prompt
        #[arg(short = 'f', long)]
        prompt_file: Option<PathBuf>,
        /// Container name (default: claude)
        #[arg(short, long, default_value = DEFAULT_CONTAINER)]
        name: String,
        /// Memory limit (e.g., "4g")
        #[arg(short, long)]
        memory: Option<String>,
        /// CPU limit (e.g., "2")
        #[arg(long)]
        cpus: Option<String>,
        /// Discord webhook URL for notifications
        #[arg(long, env = "CLAUDE_DISCORD_WEBHOOK")]
        discord_webhook: Option<String>,
        /// Additional environment variables (KEY=VALUE)
        #[arg(short, long)]
        env: Vec<String>,
        /// Run in dangerously skip permissions mode
        #[arg(long)]
        dangerously_skip_permissions: bool,
        /// Continue the most recent conversation
        #[arg(short, long)]
        continue_session: bool,
        /// Resume a specific session by ID or name
        #[arg(short, long)]
        resume: Option<String>,
    },
    /// Continue most recent conversation in container
    Continue {
        /// Container name (default: claude)
        #[arg(short, long, default_value = DEFAULT_CONTAINER)]
        name: String,
    },
    /// Resume a specific session
    Resume {
        /// Session ID or name to resume
        session: Option<String>,
        /// Container name (default: claude)
        #[arg(short, long, default_value = DEFAULT_CONTAINER)]
        name: String,
    },
    /// Open a shell in the Claude container
    Shell {
        /// Container name (default: claude)
        #[arg(short, long, default_value = DEFAULT_CONTAINER)]
        name: String,
    },
    /// Stop a running Claude container
    Stop {
        /// Container name (default: claude)
        #[arg(short, long, default_value = DEFAULT_CONTAINER)]
        name: String,
    },
    /// List all Claude sandbox containers
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
        /// Container name (default: claude)
        #[arg(short, long, default_value = DEFAULT_CONTAINER)]
        name: String,
    },
}

#[derive(Serialize)]
struct DiscordMessage {
    content: String,
    username: String,
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

fn get_config_dir() -> Result<PathBuf> {
    if let Ok(p) = std::env::var("CLAUDE_SANDBOX_CONFIG") {
        return Ok(PathBuf::from(p));
    }
    dirs::home_dir()
        .map(|h| h.join(".claude-sandbox"))
        .context("Could not determine home directory")
}

fn get_dockerfile_content() -> &'static str {
    r#"FROM node:20-slim

ENV HOME=/home/claude
ENV LANG=C.UTF-8
ENV LC_ALL=C.UTF-8
ENV TERM=xterm-256color

RUN apt-get update && apt-get install -y \
    curl \
    git \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

RUN npm install -g @anthropic-ai/claude-code

RUN useradd -m -s /bin/bash -d /home/claude claude && \
    mkdir -p /home/claude/workspace /home/claude/.claude /home/claude/.config && \
    touch /home/claude/.claude.json /home/claude/.claude.json.backup && \
    chown -R claude:claude /home/claude

USER claude
WORKDIR /home/claude/workspace

CMD ["tail", "-f", "/dev/null"]
"#
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

async fn send_discord(webhook: &str, message: &str) -> Result<()> {
    let client = reqwest::Client::new();
    let msg = DiscordMessage {
        content: message.to_string(),
        username: "Claude Sandbox".to_string(),
    };
    client.post(webhook).json(&msg).send().await?;
    Ok(())
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
    env_vars: &[String],
) -> Result<()> {
    let config_dir = get_config_dir()?;
    std::fs::create_dir_all(&config_dir)?;

    let mut args = vec![
        "run".to_string(),
        "-d".to_string(),
        "--name".to_string(),
        name.to_string(),
    ];

    for folder in folders {
        let abs = std::fs::canonicalize(folder)?;
        let fname = abs.file_name().unwrap().to_str().unwrap();
        args.extend([
            "-v".to_string(),
            format!("{}:/home/claude/workspace/{}", abs.display(), fname),
        ]);
    }

    // Mount .claude directory
    let claude_dir = config_dir.join(".claude");
    std::fs::create_dir_all(&claude_dir)?;
    args.extend([
        "-v".to_string(),
        format!("{}:/home/claude/.claude", claude_dir.display()),
    ]);

    // Mount .claude.json files
    let claude_json = config_dir.join(".claude.json");
    let claude_json_backup = config_dir.join(".claude.json.backup");
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

    // Mount .config directory
    let config_app_dir = config_dir.join(".config");
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

fn print_banner(name: &str) {
    println!("\n{}", "═".repeat(70).cyan());
    println!(
        "{}  Claude Code running in container '{}'",
        "│".cyan(),
        name.green()
    );
    println!(
        "{}  Press {} to exit (container keeps running)",
        "│".cyan(),
        "Ctrl+C".yellow().bold()
    );
    println!("{}", "│".cyan());
    println!("{}  Reconnect with:", "│".cyan());
    println!(
        "{}    {} - continue last session",
        "│".cyan(),
        format!("claude-sandbox continue -n {name}").green()
    );
    println!(
        "{}    {} - resume specific session",
        "│".cyan(),
        format!("claude-sandbox resume <session> -n {name}").green()
    );
    println!("{}\n", "═".repeat(70).cyan());
}

async fn run_claude(
    folders: Vec<PathBuf>,
    prompt: Option<String>,
    prompt_file: Option<PathBuf>,
    name: String,
    memory: Option<String>,
    cpus: Option<String>,
    discord_webhook: Option<String>,
    env_vars: Vec<String>,
    dangerously_skip_permissions: bool,
    continue_session: bool,
    resume: Option<String>,
) -> Result<()> {
    check_docker().await?;

    if !image_exists().await? {
        println!("{}", "Image not found, building...".yellow());
        build_image(false).await?;
    }

    let need_new_container = if container_exists(&name).await? {
        if container_running(&name).await? {
            println!("{}", format!("Using existing container '{name}'").cyan());
            false
        } else {
            // Remove stopped container
            Command::new("docker").args(["rm", &name]).status().await?;
            true
        }
    } else {
        true
    };

    if need_new_container {
        println!("{}", "Starting Claude Code sandbox...".cyan());
        println!("{}:", "Mapped folders".bold());
        for folder in &folders {
            let abs = std::fs::canonicalize(folder)?;
            let fname = abs.file_name().unwrap().to_str().unwrap();
            println!(
                "  {} -> /home/claude/workspace/{}",
                abs.display().to_string().blue(),
                fname
            );
        }

        start_container(
            &name,
            &folders,
            memory.as_deref(),
            cpus.as_deref(),
            &env_vars,
        )
        .await?;
    }

    let final_prompt = match (prompt, prompt_file) {
        (Some(p), _) => Some(p),
        (None, Some(f)) => Some(std::fs::read_to_string(&f)?),
        (None, None) => None,
    };

    print_banner(&name);

    if let Some(ref wh) = discord_webhook {
        send_discord(wh, &format!("🚀 Claude sandbox '{name}' started"))
            .await
            .ok();
    }

    exec_claude_interactive(
        &name,
        final_prompt.as_deref(),
        dangerously_skip_permissions,
        continue_session,
        resume.as_deref(),
    )
    .await?;

    println!("\n{} Exited Claude session", "✓".green());
    println!("  Container '{name}' is still running");
    println!(
        "  Use {} to continue",
        format!("claude-sandbox continue -n {name}").yellow()
    );

    if let Some(ref wh) = discord_webhook {
        send_discord(wh, &format!("👋 Detached from Claude sandbox '{name}'"))
            .await
            .ok();
    }

    Ok(())
}

async fn continue_session(name: &str) -> Result<()> {
    check_docker().await?;

    if !container_running(name).await? {
        bail!(
            "Container '{name}' is not running. Use 'run' to start it."
        );
    }

    println!(
        "{}",
        format!("Continuing last session in '{name}'...").cyan()
    );

    exec_claude_interactive(name, None, false, true, None).await?;

    println!("\n{} Exited Claude session", "✓".green());
    println!("  Container '{name}' is still running");

    Ok(())
}

async fn resume_session(name: &str, session: Option<&str>) -> Result<()> {
    check_docker().await?;

    if !container_running(name).await? {
        bail!(
            "Container '{name}' is not running. Use 'run' to start it."
        );
    }

    if let Some(s) = session {
        println!(
            "{}",
            format!("Resuming session '{s}' in '{name}'...").cyan()
        );
    } else {
        println!(
            "{}",
            format!("Opening session picker in '{name}'...").cyan()
        );
    }

    // If no session specified, claude -r will show interactive picker
    exec_claude_interactive(name, None, false, false, session.or(Some(""))).await?;

    println!("\n{} Exited Claude session", "✓".green());
    println!("  Container '{name}' is still running");

    Ok(())
}

async fn shell_container(name: &str) -> Result<()> {
    check_docker().await?;
    if !container_running(name).await? {
        bail!("Container '{name}' is not running");
    }
    println!(
        "{}",
        format!("Opening shell in container '{name}'...").cyan()
    );
    std::process::Command::new("docker")
        .args(["exec", "-it", name, "bash"])
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()?;
    Ok(())
}

async fn stop_container(name: &str) -> Result<()> {
    check_docker().await?;
    if !container_exists(name).await? {
        bail!("Container '{name}' does not exist");
    }
    println!("{}", format!("Stopping container '{name}'...").cyan());
    Command::new("docker").args(["stop", name]).status().await?;
    Command::new("docker").args(["rm", name]).status().await?;
    println!("{} Container stopped and removed", "✓".green());
    Ok(())
}

async fn list_containers() -> Result<()> {
    check_docker().await?;
    println!("{}", "Claude sandbox containers:".bold());
    let output = Command::new("docker")
        .args([
            "ps",
            "-a",
            "--filter",
            &format!("ancestor={IMAGE_NAME}"),
            "--format",
            "table {{.Names}}\t{{.Status}}\t{{.CreatedAt}}",
        ])
        .output()
        .await?;
    print!("{}", String::from_utf8_lossy(&output.stdout));
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
        std::io::stdout().flush()?;
        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;
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

async fn status_container(name: &str) -> Result<()> {
    check_docker().await?;
    if !container_exists(name).await? {
        println!("{} Container '{}' does not exist", "✗".red(), name);
        return Ok(());
    }
    let output = Command::new("docker")
        .args(["inspect", name])
        .output()
        .await?;
    let info: Vec<ContainerInfo> = serde_json::from_slice(&output.stdout)?;
    if let Some(i) = info.first() {
        let icon = if i.state.running {
            "●".green()
        } else {
            "○".red()
        };
        println!("{} Container '{}': {}", icon, name, i.state.status);
    }
    Ok(())
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
            memory,
            cpus,
            discord_webhook,
            env,
            dangerously_skip_permissions,
            continue_session,
            resume,
        } => {
            run_claude(
                folders,
                prompt,
                prompt_file,
                name,
                memory,
                cpus,
                discord_webhook,
                env,
                dangerously_skip_permissions,
                continue_session,
                resume,
            )
            .await
        }
        Commands::Continue { name } => continue_session(&name).await,
        Commands::Resume { session, name } => resume_session(&name, session.as_deref()).await,
        Commands::Shell { name } => shell_container(&name).await,
        Commands::Stop { name } => stop_container(&name).await,
        Commands::List => list_containers().await,
        Commands::Build { no_cache } => build_image(no_cache).await,
        Commands::Reset { force } => reset_state(force).await,
        Commands::Status { name } => status_container(&name).await,
    }
}
