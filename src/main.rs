use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

use anyhow::{ensure, Context, Result};
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
    #[arg(
        long,
        global = true,
        env = "AKC_PASSWORD",
        hide_env_values = true,
        value_name = "PASSWORD"
    )]
    password: Option<String>,

    /// Path of the keychain file
    keychain: Option<PathBuf>,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Create a new empty keychain file
    Init {},
    /// Add or update a secret
    Set {
        /// Name of the secret
        key: String,
        /// Secret value
        value: String,
    },
    /// Print one secret to stdout
    Get {
        /// Name of the secret
        key: String,
    },
    /// List secret names (values are never shown)
    List {},
    /// Remove a secret
    Delete {
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
        Some(Command::Upgrade {
            force,
            yes,
            version,
        }) => {
            ensure!(
                cli.keychain.is_none(),
                "upgrade does not use a keychain file"
            );
            cmd_upgrade(*force, *yes, version.as_deref())
        }
        Some(command) => {
            let file = cli.keychain.as_deref().context("keychain path required")?;
            match command {
                Command::Init {} => cmd_init(file, password),
                Command::Set { key, value } => cmd_set(file, key, value, password),
                Command::Get { key } => cmd_get(file, key, password),
                Command::List {} => cmd_list(file, password),
                Command::Delete { key } => cmd_delete(file, key, password),
                Command::Upgrade { .. } => unreachable!(),
            }
        }
        None => {
            let file = cli
                .keychain
                .as_deref()
                .context("keychain path required (or use 'akc upgrade')")?;
            cmd_interactive(file, password)
        }
    }
}

fn cmd_init(file: &Path, provided: Option<&str>) -> Result<()> {
    let password = read_password(provided.map(str::to_string), true)?;
    if file.exists() {
        let backup = backup_path(file)?;
        std::fs::copy(file, &backup).with_context(|| {
            format!("cannot back up {} to {}", file.display(), backup.display())
        })?;
        println!("Backed up existing keychain to {}", backup.display());
    }
    let kc = Keychain::new();
    kc.save(file, &password, false)
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

fn backup_path(file: &Path) -> Result<PathBuf> {
    let stem = file
        .file_stem()
        .context("keychain path must include a file name")?
        .to_string_lossy();
    let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S");
    let backup = file.with_file_name(format!("{stem}_{timestamp}.bak"));
    ensure!(
        !backup.exists(),
        "backup file already exists: {}",
        backup.display()
    );
    Ok(backup)
}

fn cmd_interactive(file: &Path, provided: Option<&str>) -> Result<()> {
    ensure!(
        file.exists(),
        "keychain not found: {} (run 'akc {} init' first)",
        file.display(),
        file.display()
    );
    let password = read_password(provided.map(str::to_string), false)?;
    let mut kc = Keychain::load(file, &password)?;
    let stdin = std::io::stdin();
    let mut input = stdin.lock();
    let mut line = String::new();

    println!("Interactive mode. Type 'help' for commands, 'exit' to quit.");
    loop {
        print!("akc> ");
        std::io::stdout().flush()?;
        line.clear();
        if input.read_line(&mut line)? == 0 {
            break;
        }
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let (command, args) = line.split_once(' ').unwrap_or((line, ""));
        let args = args.trim();
        match command {
            "help" | "?" => print_interactive_help(),
            "exit" | "quit" => break,
            "list" => {
                if !args.is_empty() {
                    println!("usage: list");
                    continue;
                }
                for key in kc.keys() {
                    println!("{key}");
                }
            }
            "get" => match one_argument(args, "get <key>") {
                Ok(key) => match kc.get(key) {
                    Some(value) => println!("{value}"),
                    None => println!("error: key not found: {key}"),
                },
                Err(message) => println!("{message}"),
            },
            "set" => match args.split_once(char::is_whitespace) {
                Some((key, value)) if !key.is_empty() && !value.trim().is_empty() => {
                    kc.set(key, value.trim().to_string());
                    if let Err(err) = kc.save(file, &password, false) {
                        println!("error saving: {err:#}");
                    }
                }
                _ => println!("usage: set <key> <value>"),
            },
            "delete" => match one_argument(args, "delete <key>") {
                Ok(key) => {
                    if kc.delete(key) {
                        if let Err(err) = kc.save(file, &password, false) {
                            println!("error saving: {err:#}");
                        }
                    } else {
                        println!("error: key not found: {key}");
                    }
                }
                Err(message) => println!("{message}"),
            },
            "init" => {
                if let Err(err) = interactive_init(file, &mut kc) {
                    println!("error: {err:#}");
                }
            }
            _ => println!("unknown command: {command}. Type 'help' for commands."),
        }
    }
    Ok(())
}

fn one_argument<'a>(args: &'a str, usage: &str) -> Result<&'a str, String> {
    if args.is_empty() || args.contains(char::is_whitespace) {
        Err(format!("usage: {usage}"))
    } else {
        Ok(args)
    }
}

fn interactive_init(file: &Path, kc: &mut Keychain) -> Result<()> {
    let password = read_password(None, true)?;
    let backup = backup_path(file)?;
    std::fs::copy(file, &backup)
        .with_context(|| format!("cannot back up {} to {}", file.display(), backup.display()))?;
    let new_keychain = Keychain::new();
    new_keychain.save(file, &password, false)?;
    *kc = new_keychain;
    println!("Backed up existing keychain to {}", backup.display());
    println!("Created {}", file.display());
    Ok(())
}

fn print_interactive_help() {
    println!("Commands: get <key>, set <key> <value>, list, delete <key>, init, help, exit");
}
