use std::path::PathBuf;

use anyhow::{Context, Result, ensure};
use clap::{Parser, Subcommand};

mod crypto;
mod password;
mod storage;
mod upgrade;

use password::read_password;
use storage::Keychain;

#[derive(Parser)]
#[command(
    name = "akc",
    version,
    about = "Minimal encrypted keychain (akc): a single portable file holding key/value secrets"
)]
struct Cli {
    /// Password (use only for scripts; prompts interactively when omitted)
    #[arg(long, global = true, value_name = "PASSWORD")]
    password: Option<String>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Create a new empty keychain file
    Init {
        /// Path of the keychain file to create
        file: PathBuf,
    },
    /// Add or update a secret
    Set {
        /// Path of the keychain file
        file: PathBuf,
        /// Name of the secret
        key: String,
        /// Secret value
        value: String,
    },
    /// Print one secret to stdout
    Get {
        /// Path of the keychain file
        file: PathBuf,
        /// Name of the secret
        key: String,
    },
    /// List secret names (values are never shown)
    List {
        /// Path of the keychain file
        file: PathBuf,
    },
    /// Remove a secret
    Delete {
        /// Path of the keychain file
        file: PathBuf,
        /// Name of the secret
        key: String,
    },
    /// Check for and install updates from GitHub releases
    Upgrade {
        /// Force upgrade even if already on latest version
        #[arg(long)]
        force: bool,
        /// Skip confirmation prompt
        #[arg(long, short = 'y')]
        yes: bool,
        /// Specify a particular version to upgrade to (e.g., "1.0.2" or "v1.0.2")
        #[arg(long, value_name = "VERSION")]
        version: Option<String>,
    },
}

fn main() {
    if let Err(err) = run() {
        eprintln!("error: {err:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    let password = cli.password.as_deref();

    match &cli.command {
        Command::Init { file } => cmd_init(file, password),
        Command::Set {
            file,
            key,
            value,
        } => cmd_set(file, key, value, password),
        Command::Get { file, key } => cmd_get(file, key, password),
        Command::List { file } => cmd_list(file, password),
        Command::Delete { file, key } => cmd_delete(file, key, password),
        Command::Upgrade { force, yes, version } => cmd_upgrade(*force, *yes, version.as_deref()),
    }
}

fn cmd_init(file: &std::path::Path, provided: Option<&str>) -> Result<()> {
    let password = read_password(provided.map(str::to_string), true)?;
    let kc = Keychain::new();
    kc.save(file, &password, true)
        .with_context(|| format!("cannot create {}", file.display()))?;
    println!("Created {}", file.display());
    Ok(())
}

fn cmd_set(file: &std::path::Path, key: &str, value: &str, provided: Option<&str>) -> Result<()> {
    let password = read_password(provided.map(str::to_string), false)?;
    let mut kc = Keychain::load(file, &password)?;
    kc.set(key, value.to_string());
    kc.save(file, &password, false)?;
    Ok(())
}

fn cmd_get(file: &std::path::Path, key: &str, provided: Option<&str>) -> Result<()> {
    let password = read_password(provided.map(str::to_string), false)?;
    let kc = Keychain::load(file, &password)?;
    let value = kc.get(key).context(format!("key not found: {key}"))?;
    println!("{value}");
    Ok(())
}

fn cmd_list(file: &std::path::Path, provided: Option<&str>) -> Result<()> {
    let password = read_password(provided.map(str::to_string), false)?;
    let kc = Keychain::load(file, &password)?;
    for key in kc.keys() {
        println!("{key}");
    }
    Ok(())
}

fn cmd_delete(file: &std::path::Path, key: &str, provided: Option<&str>) -> Result<()> {
    let password = read_password(provided.map(str::to_string), false)?;
    let mut kc = Keychain::load(file, &password)?;
    ensure!(kc.delete(key), "key not found: {key}");
    kc.save(file, &password, false)?;
    Ok(())
}

fn cmd_upgrade(force: bool, yes: bool, version: Option<&str>) -> Result<()> {
    upgrade::run(force, yes, version)
}
