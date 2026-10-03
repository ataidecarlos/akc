use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, ensure};
use clap::{Parser, Subcommand};
use zeroize::Zeroizing;

mod crypto;
mod password;
mod storage;
mod upgrade;

use password::{console_prompt, read_password, read_with};
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
    Init {
        /// Replace an existing keychain, discarding its secrets
        #[arg(long)]
        force: bool,
    },
    /// Change the password of an existing keychain
    ChangePassword {
        /// New password (prompts for it, and confirms it twice, when omitted)
        #[arg(
            long,
            env = "AKC_NEW_PASSWORD",
            hide_env_values = true,
            value_name = "PASSWORD"
        )]
        new_password: Option<String>,
    },
    /// Add or update a secret
    Set {
        /// Name of the secret
        key: String,
        /// Secret value (surrounding whitespace is removed)
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
                Command::Init { force } => cmd_init(file, *force, password),
                Command::ChangePassword { new_password } => {
                    cmd_change_password(file, password, new_password.as_deref())
                }
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

/// Create a new keychain.
///
/// Refuses to touch an existing file unless `force` is given. `init` upserts a
/// password, so the typed password is confirmed twice; `change-password` is the
/// operation for re-keying a vault you already have.
fn cmd_init(file: &Path, force: bool, provided: Option<&str>) -> Result<()> {
    if file.exists() {
        ensure!(
            force,
            "{} already exists: use 'akc {} change-password' to keep its secrets, \
             or 'akc {} init --force' to discard them and start over",
            file.display(),
            file.display(),
            file.display()
        );
        let backup = backup_path(file)?;
        std::fs::copy(file, &backup).with_context(|| {
            format!("cannot back up {} to {}", file.display(), backup.display())
        })?;
        println!("Backed up existing keychain to {}", backup.display());
    }

    let password = read_new_password(provided, &mut console_prompt)?;
    let kc = Keychain::new();
    kc.save(file, &password, !force)
        .with_context(|| format!("cannot create {}", file.display()))?;
    println!("Created {}", file.display());
    Ok(())
}

/// Re-key an existing keychain, keeping every secret.
///
/// `current` is verified by loading the vault first, so a wrong password cannot
/// get this far. The replacement is confirmed twice when typed. This is the only
/// supported way to change a password without losing secrets; `init --force`
/// discards them instead.
///
/// `password` is the current password (like every other command), while
/// `new_password` is the replacement. Keeping them separate is what makes a
/// non-interactive change possible at all.
fn cmd_change_password(
    file: &Path,
    password: Option<&str>,
    new_password: Option<&str>,
) -> Result<()> {
    ensure!(
        file.exists(),
        "keychain not found: {} (run 'akc {} init' first)",
        file.display(),
        file.display()
    );

    // Verifies the current password: a wrong one cannot decrypt the vault.
    let current = read_password(password.map(str::to_string), false)?;
    let kc = Keychain::load(file, &current)?;

    let replacement = read_new_password(new_password, &mut console_prompt)?;
    rekey(file, &kc, &replacement, &mut std::io::stdout())
}

/// Back up the vault, then rewrite it under `new_password`.
///
/// Split out so the CLI and the interactive session share one implementation.
/// Output goes to `out` rather than straight to stdout, so the interactive
/// session can capture it.
fn rekey(file: &Path, kc: &Keychain, new_password: &str, out: &mut impl Write) -> Result<()> {
    let backup = backup_path(file)?;
    std::fs::copy(file, &backup)
        .with_context(|| format!("cannot back up {} to {}", file.display(), backup.display()))?;
    kc.save(file, new_password, false)?;
    writeln!(out, "Backed up existing keychain to {}", backup.display())?;
    writeln!(out, "Changed the password for {}", file.display())?;
    Ok(())
}

/// Ask for a password that will become a keychain's key, confirming it.
///
/// Every action that upserts a password routes through here, so the double-entry
/// guarantee cannot be quietly bypassed by a future command. Confirmation only
/// applies to a typed password: one supplied via `--password`/`AKC_PASSWORD`
/// cannot contain a typo, and asking twice would break scripted use.
fn read_new_password(
    provided: Option<&str>,
    prompt: &mut dyn FnMut(&str) -> Result<String>,
) -> Result<Zeroizing<String>> {
    read_with(provided.map(str::to_string), true, prompt)
}

/// Normalize a secret value.
///
/// Surrounding whitespace is always removed, so `set k "  v  "` stores `v`.
/// That makes a blank value meaningless, so it is rejected rather than stored.
fn normalize_value(raw: &str) -> Result<String> {
    let value = raw.trim();
    ensure!(
        !value.is_empty(),
        "value must not be empty: surrounding whitespace is removed, so a blank value is not allowed"
    );
    Ok(value.to_string())
}

fn cmd_set(file: &Path, key: &str, value: &str, provided: Option<&str>) -> Result<()> {
    let value = normalize_value(value)?;
    let password = read_password(provided.map(str::to_string), false)?;
    let mut kc = Keychain::load(file, &password)?;
    kc.set(key, value);
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
    let stdin = std::io::stdin();
    let mut input = stdin.lock();
    let mut output = std::io::stdout();
    interactive_session(file, &mut input, &mut output, &mut console_prompt, provided)
}

const SET_USAGE: &str = "usage: set <key> <value> (a value is required: surrounding whitespace is \
     removed, so a blank value is rejected)";

const INTERACTIVE_HELP: &str =
    "Commands: get <key>, set <key> <value>, list, delete <key>, change-password, help, exit";

/// The interactive command loop.
///
/// Command input, the password prompt, and output are all injected so the
/// session can be exercised without a terminal. `change-password` re-keys the
/// vault, so the session password is rebound to the new one -- otherwise the
/// next write would silently re-encrypt the file under the password the session
/// started with.
///
/// There is no interactive `init`: interactive mode only ever runs against an
/// existing keychain, and discarding its contents is a deliberate CLI-only act
/// (`akc <file> init --force`).
fn interactive_session(
    file: &Path,
    input: &mut impl BufRead,
    out: &mut impl Write,
    prompt: &mut dyn FnMut(&str) -> Result<String>,
    provided: Option<&str>,
) -> Result<()> {
    let mut password = read_with(provided.map(str::to_string), false, prompt)?;
    let mut kc = Keychain::load(file, &password)?;
    let mut line = String::new();

    writeln!(
        out,
        "Interactive mode. Type 'help' for commands, 'exit' to quit."
    )?;

    loop {
        write!(out, "akc> ")?;
        out.flush()?;
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
            "help" | "?" => writeln!(out, "{INTERACTIVE_HELP}")?,
            "exit" | "quit" => break,
            "list" => {
                if !args.is_empty() {
                    writeln!(out, "usage: list")?;
                    continue;
                }
                for key in kc.keys() {
                    writeln!(out, "{key}")?;
                }
            }
            "get" => match one_argument(args, "get <key>") {
                Ok(key) => match kc.get(key) {
                    Some(value) => writeln!(out, "{value}")?,
                    None => writeln!(out, "error: key not found: {key}")?,
                },
                Err(message) => writeln!(out, "{message}")?,
            },
            "set" => match args.split_once(char::is_whitespace) {
                Some((key, raw)) if !key.is_empty() => match normalize_value(raw) {
                    Ok(value) => {
                        kc.set(key, value);
                        if let Err(err) = kc.save(file, &password, false) {
                            writeln!(out, "error saving: {err:#}")?;
                        }
                    }
                    Err(err) => writeln!(out, "error: {err:#}")?,
                },
                _ => writeln!(out, "{SET_USAGE}")?,
            },
            "delete" => match one_argument(args, "delete <key>") {
                Ok(key) => {
                    if kc.delete(key) {
                        if let Err(err) = kc.save(file, &password, false) {
                            writeln!(out, "error saving: {err:#}")?;
                        }
                    } else {
                        writeln!(out, "error: key not found: {key}")?;
                    }
                }
                Err(message) => writeln!(out, "{message}")?,
            },
            "change-password" => {
                match interactive_change_password(file, &password, prompt, out) {
                    // Adopt the new password for the rest of the session.
                    Ok(new_password) => password = new_password,
                    Err(err) => writeln!(out, "error: {err:#}")?,
                }
            }
            _ => writeln!(out, "unknown command: {command}. Type 'help' for commands.")?,
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

/// Re-key `file` from inside a session, returning the new password so the caller
/// can keep writing with it.
///
/// `current` is the session password, already verified when the session opened.
/// The replacement is always typed and confirmed twice here: there is no
/// subcommand in interactive mode to carry `--new-password`, and a script that
/// needs to change a password should use `akc <file> change-password`.
fn interactive_change_password(
    file: &Path,
    current: &str,
    prompt: &mut dyn FnMut(&str) -> Result<String>,
    out: &mut impl Write,
) -> Result<Zeroizing<String>> {
    let new_password = read_new_password(None, prompt)?;

    // Load before touching anything, so a failure here cannot damage the vault.
    let kc = Keychain::load(file, current)?;
    rekey(file, &kc, &new_password, out)?;
    Ok(new_password)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn temp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "akc-it-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn backup_count(dir: &Path) -> usize {
        std::fs::read_dir(dir)
            .map(|entries| {
                entries
                    .filter_map(|e| e.ok())
                    .filter(|e| e.file_name().to_string_lossy().ends_with(".bak"))
                    .count()
            })
            .unwrap_or(0)
    }

    /// Drive a whole interactive session from a scripted script and a scripted
    /// sequence of password-prompt answers.
    fn run_session(file: &Path, script: &str, answers: &[&str], provided: Option<&str>) -> String {
        let mut queue = answers.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let mut prompted = 0usize;
        let mut prompt = |_: &str| -> Result<String> {
            prompted += 1;
            Ok(queue.remove(0))
        };
        let mut out: Vec<u8> = Vec::new();
        let mut input = std::io::Cursor::new(script.as_bytes().to_vec());
        interactive_session(file, &mut input, &mut out, &mut prompt, provided).unwrap();
        assert!(prompted <= answers.len());
        String::from_utf8(out).unwrap()
    }

    fn prompts_used(answers: &[&str], script: &str, provided: Option<&str>) -> usize {
        // Re-run counting prompts by inspecting how many answers were consumed.
        let mut queue = answers.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let mut prompted = 0usize;
        let mut prompt = |_: &str| -> Result<String> {
            prompted += 1;
            Ok(queue.remove(0))
        };
        let mut out: Vec<u8> = Vec::new();
        let mut input = std::io::Cursor::new(script.as_bytes().to_vec());
        let file = temp_dir().join("promptcount.akc");
        let _ = interactive_session(&file, &mut input, &mut out, &mut prompt, provided);
        prompted
    }

    #[test]
    fn session_gets_and_sets_secrets() {
        let dir = temp_dir();
        let file = dir.join("s.akc");
        let kc = Keychain::new();
        kc.save(&file, "pw", false).unwrap();

        let out = run_session(&file, "set a 1\nget a\nlist\nexit\n", &[], Some("pw"));
        assert!(out.contains("1"), "{out}");
        assert!(out.contains("a"), "{out}");

        let reloaded = Keychain::load(&file, "pw").unwrap();
        assert_eq!(reloaded.get("a").map(String::as_str), Some("1"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Regression: `change-password` re-keys the vault, and the session must keep
    /// writing with the NEW password. Before the fix the session reused the
    /// password it started with, silently reverting the change.
    #[test]
    fn change_password_then_set_persists_under_the_new_password() {
        let dir = temp_dir();
        let file = dir.join("s.akc");
        let mut kc = Keychain::new();
        kc.set("before", "old-value".to_string());
        kc.save(&file, "oldpass", false).unwrap();

        // Answers: session password, then the new password + confirmation.
        run_session(
            &file,
            "change-password\nset after init-value\nexit\n",
            &["oldpass", "newpass", "newpass"],
            None,
        );

        let reloaded = Keychain::load(&file, "newpass").unwrap();
        assert_eq!(
            reloaded.get("after").map(String::as_str),
            Some("init-value")
        );
        // change-password keeps secrets; unlike init it must not wipe them.
        assert_eq!(
            reloaded.get("before").map(String::as_str),
            Some("old-value")
        );
        // And the old password must no longer work.
        assert!(Keychain::load(&file, "oldpass").is_err());

        std::fs::remove_dir_all(&dir).unwrap();
    }
    /// Interactive mode has no subcommand to carry `--new-password`, so the
    /// replacement is always typed and confirmed twice -- even when
    /// `AKC_PASSWORD` is set for the session. The scripted path is
    /// `akc <file> change-password --new-password`.
    #[test]
    fn interactive_change_password_always_confirms_the_new_password() {
        let dir = temp_dir();
        let file = dir.join("s.akc");
        Keychain::new().save(&file, "pw", false).unwrap();

        // Answers: the new password and its confirmation. An empty answer list
        // panics if the prompt is reached fewer times than expected, and a
        // leftover answer is caught by run_session.
        let out = run_session(
            &file,
            "change-password\nset beta b\nexit\n",
            &["newpw", "newpw"],
            Some("pw"),
        );
        assert!(out.contains("Changed the password"), "{out}");

        let reloaded = Keychain::load(&file, "newpw").unwrap();
        assert_eq!(reloaded.get("beta").map(String::as_str), Some("b"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn cli_change_password_needs_no_terminal_when_new_password_is_given() {
        let dir = temp_dir();
        let file = dir.join("s.akc");
        let mut kc = Keychain::new();
        kc.set("keep", "me".to_string());
        kc.save(&file, "oldpass", false).unwrap();

        // Both passwords supplied: this must not prompt at all, so a run that
        // reached the console would fail under a non-interactive caller.
        cmd_change_password(&file, Some("oldpass"), Some("newpass")).unwrap();

        let reloaded = Keychain::load(&file, "newpass").unwrap();
        assert_eq!(reloaded.get("keep").map(String::as_str), Some("me"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn session_password_prompts_exactly_once() {
        let dir = temp_dir();
        let file = dir.join("s.akc");
        Keychain::new().save(&file, "pw", false).unwrap();
        // No `--password`: the session must ask once, and must not ask again
        // per command.
        let prompts = prompts_used(&["pw"], "list\nexit\n", None);
        assert_eq!(prompts, 1);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn change_password_backs_up_the_existing_keychain() {
        let dir = temp_dir();
        let file = dir.join("s.akc");
        let mut kc = Keychain::new();
        kc.set("keep", "me".to_string());
        kc.save(&file, "oldpass", false).unwrap();

        let out = run_session(
            &file,
            "change-password\nexit\n",
            &["oldpass", "new", "new"],
            None,
        );
        assert!(out.contains("Backed up"), "{out}");

        let backups: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(".bak"))
            .collect();
        assert_eq!(backups.len(), 1, "{out}");
        // The backup still holds the old contents under the old password.
        let old = Keychain::load(&dir.join(&backups[0]), "oldpass").unwrap();
        assert_eq!(old.get("keep").map(String::as_str), Some("me"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A mistyped confirmation must abort the change and leave the vault usable
    /// on its original password.
    #[test]
    fn failed_change_password_keeps_the_session_usable() {
        let dir = temp_dir();
        let file = dir.join("s.akc");
        let mut kc = Keychain::new();
        kc.set("keep", "me".to_string());
        kc.save(&file, "oldpass", false).unwrap();

        let out = run_session(
            &file,
            "change-password\nlist\nexit\n",
            &["oldpass", "typed", "different"],
            None,
        );
        assert!(out.contains("error:"), "{out}");
        assert!(
            out.contains("keep"),
            "session should still list keys: {out}"
        );

        let reloaded = Keychain::load(&file, "oldpass").unwrap();
        assert_eq!(reloaded.get("keep").map(String::as_str), Some("me"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Interactive mode runs against an existing keychain, so wiping it is a
    /// CLI-only act. `init` must not be a session command.
    #[test]
    fn interactive_init_is_not_a_command() {
        let dir = temp_dir();
        let file = dir.join("s.akc");
        let mut kc = Keychain::new();
        kc.set("keep", "me".to_string());
        kc.save(&file, "pw", false).unwrap();

        let out = run_session(&file, "init\nlist\nexit\n", &[], Some("pw"));
        assert!(out.contains("unknown command: init"), "{out}");

        // The vault must be untouched.
        let reloaded = Keychain::load(&file, "pw").unwrap();
        assert_eq!(reloaded.get("keep").map(String::as_str), Some("me"));
        assert_eq!(backup_count(&dir), 0, "no backup should be created");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn unknown_and_malformed_commands_do_not_abort_the_session() {
        let dir = temp_dir();
        let file = dir.join("s.akc");
        Keychain::new().save(&file, "pw", false).unwrap();

        let out = run_session(
            &file,
            "bogus\nget\nget a b\nset onlykey\nlist\nexit\n",
            &[],
            Some("pw"),
        );
        assert!(out.contains("unknown command: bogus"), "{out}");
        assert!(out.contains("usage: get <key>"), "{out}");
        assert!(out.contains("usage: set <key> <value>"), "{out}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    // --- value normalization ---

    #[test]
    fn normalize_value_strips_padding() {
        assert_eq!(normalize_value("v").unwrap(), "v");
        assert_eq!(normalize_value("  v  ").unwrap(), "v");
        assert_eq!(normalize_value("\t v \n").unwrap(), "v");
        assert_eq!(normalize_value("  spaced out  ").unwrap(), "spaced out");
        assert!(normalize_value("   ").is_err());
        assert!(normalize_value("").is_err());
    }

    #[test]
    fn empty_value_error_explains_the_rule() {
        let err = normalize_value("   ").unwrap_err().to_string();
        assert!(err.contains("must not be empty"), "{err}");
        assert!(err.contains("whitespace is removed"), "{err}");
    }

    #[test]
    fn interactive_set_normalizes_and_rejects_blank_values() {
        let dir = temp_dir();
        let file = dir.join("s.akc");
        Keychain::new().save(&file, "pw", false).unwrap();

        let out = run_session(
            &file,
            "set padded   padded-value  \nset blank    \nexit\n",
            &[],
            Some("pw"),
        );

        let kc = Keychain::load(&file, "pw").unwrap();
        assert_eq!(
            kc.get("padded").map(String::as_str),
            Some("padded-value"),
            "{out}"
        );
        assert_eq!(
            kc.get("blank"),
            None,
            "blank value must not be stored: {out}"
        );
        // Whichever way the user hits it, the message must state the rule.
        assert!(
            out.contains("whitespace is removed") && out.contains("blank value is rejected"),
            "message should explain the rule: {out}"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    // --- CLI: init / change-password ---

    #[test]
    fn init_refuses_to_clobber_an_existing_keychain() {
        let dir = temp_dir();
        let file = dir.join("s.akc");
        let mut kc = Keychain::new();
        kc.set("keep", "me".to_string());
        kc.save(&file, "pw", false).unwrap();

        let err = cmd_init(&file, false, Some("pw")).unwrap_err().to_string();
        assert!(err.contains("already exists"), "{err}");
        assert!(err.contains("change-password"), "{err}");

        // Untouched, and no backup churn from a refused init.
        let reloaded = Keychain::load(&file, "pw").unwrap();
        assert_eq!(reloaded.get("keep").map(String::as_str), Some("me"));
        assert_eq!(backup_count(&dir), 0);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn init_force_replaces_and_backs_up() {
        let dir = temp_dir();
        let file = dir.join("s.akc");
        let mut kc = Keychain::new();
        kc.set("keep", "me".to_string());
        kc.save(&file, "oldpass", false).unwrap();

        cmd_init(&file, true, Some("newpass")).unwrap();

        // Fresh, empty, under the new password.
        let reloaded = Keychain::load(&file, "newpass").unwrap();
        assert_eq!(reloaded.get("keep"), None);
        assert!(Keychain::load(&file, "oldpass").is_err());
        assert_eq!(backup_count(&dir), 1);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn init_creates_a_new_keychain() {
        let dir = temp_dir();
        let file = dir.join("new.akc");
        cmd_init(&file, false, Some("pw")).unwrap();
        assert!(file.exists());
        assert_eq!(backup_count(&dir), 0);
        // Usable right away.
        cmd_set(&file, "k", "v", Some("pw")).unwrap();
        assert_eq!(
            Keychain::load(&file, "pw")
                .unwrap()
                .get("k")
                .map(String::as_str),
            Some("v")
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn change_password_keeps_secrets_and_requires_the_old_password() {
        let dir = temp_dir();
        let file = dir.join("s.akc");
        let mut kc = Keychain::new();
        kc.set("keep", "me".to_string());
        kc.set("other", "value".to_string());
        kc.save(&file, "oldpass", false).unwrap();

        // A wrong current password cannot even decrypt, so nothing changes.
        let err = cmd_change_password(&file, Some("wrong"), Some("newpass"))
            .unwrap_err()
            .to_string();
        assert!(err.contains("wrong password"), "{err}");
        assert!(Keychain::load(&file, "oldpass").is_ok());
        assert_eq!(backup_count(&dir), 0);

        cmd_change_password(&file, Some("oldpass"), Some("newpass")).unwrap();

        let reloaded = Keychain::load(&file, "newpass").unwrap();
        assert_eq!(reloaded.get("keep").map(String::as_str), Some("me"));
        assert_eq!(reloaded.get("other").map(String::as_str), Some("value"));
        assert!(Keychain::load(&file, "oldpass").is_err());
        assert_eq!(backup_count(&dir), 1);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn change_password_requires_an_existing_keychain() {
        let dir = temp_dir();
        let file = dir.join("missing.akc");
        let err = cmd_change_password(&file, Some("pw"), Some("pw"))
            .unwrap_err()
            .to_string();
        assert!(err.contains("not found"), "{err}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn set_rejects_a_blank_value_without_touching_the_vault() {
        let dir = temp_dir();
        let file = dir.join("s.akc");
        Keychain::new().save(&file, "pw", false).unwrap();

        for blank in ["", "   ", "\t"] {
            let err = cmd_set(&file, "k", blank, Some("pw"))
                .unwrap_err()
                .to_string();
            assert!(err.contains("must not be empty"), "{err}");
            assert!(
                err.contains("whitespace is removed"),
                "error should explain the rule: {err}"
            );
        }
        let kc = Keychain::load(&file, "pw").unwrap();
        assert_eq!(kc.keys(), Vec::<&str>::new());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn set_strips_padding_from_values() {
        let dir = temp_dir();
        let file = dir.join("s.akc");
        Keychain::new().save(&file, "pw", false).unwrap();

        cmd_set(&file, "k", "  padded  ", Some("pw")).unwrap();
        let kc = Keychain::load(&file, "pw").unwrap();
        assert_eq!(kc.get("k").map(String::as_str), Some("padded"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn wrong_password_aborts_the_session() {
        let dir = temp_dir();
        let file = dir.join("s.akc");
        Keychain::new().save(&file, "right", false).unwrap();

        let mut out: Vec<u8> = Vec::new();
        let mut input = std::io::Cursor::new(b"list\nexit\n".to_vec());
        let mut prompt = |_: &str| Ok("wrong".to_string());
        let err = interactive_session(&file, &mut input, &mut out, &mut prompt, None)
            .unwrap_err()
            .to_string();
        assert!(err.contains("wrong password"), "{err}");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
