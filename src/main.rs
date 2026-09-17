use std::path::PathBuf;

use anyhow::{Context, Result, ensure};
use clap::{Args, Parser, Subcommand};

mod crypto;
mod password;
mod storage;

use password::read_password;
use storage::Keychain;

#[derive(Parser)]
#[command(
    name = "akc",
    version,
    about = "Minimal encrypted keychain (akc): a single portable file holding key/value secrets"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Args)]
struct PasswordArg {
    /// Password (use only for scripts; prompts interactively when omitted)
    #[arg(long, value_name = "PASSWORD")]
    password: Option<String>,
}

#[derive(Subcommand)]
enum Command {
    /// Create a new empty keychain file
    Init {
        /// Path of the keychain file to create
        file: PathBuf,
        #[command(flatten)]
        pw: PasswordArg,
    },
    /// Add or update a secret
    Set {
        /// Path of the keychain file
        file: PathBuf,
        /// Name of the secret
        key: String,
        /// Secret value
        value: String,
        #[command(flatten)]
        pw: PasswordArg,
    },
    /// Print one secret to stdout
    Get {
        /// Path of the keychain file
        file: PathBuf,
        /// Name of the secret
        key: String,
        #[command(flatten)]
        pw: PasswordArg,
    },
    /// List secret names (values are never shown)
    List {
        /// Path of the keychain file
        file: PathBuf,
        #[command(flatten)]
        pw: PasswordArg,
    },
    /// Remove a secret
    Delete {
        /// Path of the keychain file
        file: PathBuf,
        /// Name of the secret
        key: String,
        #[command(flatten)]
        pw: PasswordArg,
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
    match &cli.command {
        Command::Init { file, pw } => cmd_init(file, pw.password.as_deref()),
        Command::Set {
            file,
            key,
            value,
            pw,
        } => cmd_set(file, key, value, pw.password.as_deref()),
        Command::Get { file, key, pw } => cmd_get(file, key, pw.password.as_deref()),
        Command::List { file, pw } => cmd_list(file, pw.password.as_deref()),
        Command::Delete { file, key, pw } => cmd_delete(file, key, pw.password.as_deref()),
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
