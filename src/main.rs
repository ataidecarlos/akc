use std::io::{IsTerminal, Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, ensure};
use clap::{ArgGroup, CommandFactory, Parser};
use zeroize::Zeroizing;

mod config;
mod crypto;
mod password;
mod storage;
mod upgrade;

use config::{Config, default_key_file, read_key_file};
use storage::{Keychain, Lock};

#[derive(Parser)]
#[command(
    name = "akc",
    version,
    about = "Minimal encrypted keychain (akc): fetch a secret from a single portable file",
    group(ArgGroup::new("command").args(["init", "set", "delete", "list", "rotate", "update"]))
)]
struct Cli {
    /// Secret name(s). With no command flag their values are printed.
    #[arg(value_name = "NAME")]
    names: Vec<String>,

    /// Path of the keychain file (defaults to the config file)
    #[arg(long, global = true, value_name = "PATH")]
    keychain: Option<PathBuf>,

    /// Path of the file holding the master key
    #[arg(long, global = true, value_name = "PATH")]
    key_file: Option<PathBuf>,

    /// Master key as a literal value; prefer --key-file so it stays out of argv
    #[arg(
        long,
        global = true,
        env = "AKC_PASSWORD",
        hide_env_values = true,
        value_name = "PASSWORD"
    )]
    password: Option<String>,

    /// Create an empty keychain
    #[arg(long)]
    init: bool,

    /// Store NAME, overwriting it if present; reads the value from stdin when
    /// no VALUE is given
    // `Option<Option<_>>` is what distinguishes `--set` (read stdin) from
    // `--set ""` (an empty value, which is rejected). Keep that out of the
    // help text: it is an implementation note, not a user-facing one.
    #[arg(long, value_name = "VALUE", num_args = 0..=1)]
    set: Option<Option<String>>,

    /// Remove NAME
    #[arg(long, alias = "drop", alias = "remove", alias = "rm")]
    delete: bool,

    /// List secret names
    #[arg(long)]
    list: bool,

    /// Re-key the keychain under a new master key, keeping every secret
    #[arg(long)]
    rotate: bool,

    /// New master key file, for --rotate and --init
    #[arg(long, value_name = "PATH")]
    new_key_file: Option<PathBuf>,

    /// New master key as a literal value, for --rotate and --init
    #[arg(
        long,
        env = "AKC_NEW_PASSWORD",
        hide_env_values = true,
        value_name = "PASSWORD"
    )]
    new_password: Option<String>,

    /// Print output as JSON
    #[arg(long)]
    json: bool,

    /// Check for and install updates
    #[arg(long)]
    update: bool,

    /// With --update or --init: proceed even though it discards something
    #[arg(long)]
    force: bool,

    /// With --update or --delete: skip the confirmation prompt
    #[arg(long, short = 'y')]
    yes: bool,

    /// With --update: install a specific version
    #[arg(long, value_name = "VERSION")]
    to: Option<String>,
}

/// Errors that carry their own process exit code, so a caller can branch
/// without parsing the message.
#[derive(Debug)]
struct NotFound(String);

impl std::fmt::Display for NotFound {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for NotFound {}

#[derive(Debug)]
struct Usage(String);

impl std::fmt::Display for Usage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Usage {}

fn not_found(message: impl Into<String>) -> anyhow::Error {
    anyhow::Error::new(NotFound(message.into()))
}

fn usage(message: impl Into<String>) -> anyhow::Error {
    anyhow::Error::new(Usage(message.into()))
}

fn main() {
    let cli = Cli::parse();

    // A bare `akc` should show help, not scold.
    if cli.names.is_empty()
        && !cli.init
        && cli.set.is_none()
        && !cli.delete
        && !cli.list
        && !cli.rotate
        && !cli.update
    {
        let _ = Cli::command().print_help();
        println!();
        std::process::exit(2);
    }

    let json = cli.json;
    if let Err(err) = run(cli) {
        if json {
            eprintln!("{}", serde_json::json!({ "error": format!("{err:#}") }));
        } else {
            eprintln!("error: {err:#}");
        }
        std::process::exit(exit_code(&err));
    }
}

/// 0 ok, 1 general, 2 bad arguments, 3 wrong key or corrupt vault, 4 not found.
fn exit_code(err: &anyhow::Error) -> i32 {
    if err.downcast_ref::<crypto::AuthError>().is_some() {
        3
    } else if err.downcast_ref::<NotFound>().is_some() {
        4
    } else if err.downcast_ref::<Usage>().is_some() {
        2
    } else {
        1
    }
}

fn run(cli: Cli) -> Result<()> {
    let config = Config::load()?;

    // A flag that belongs to another command is an error, not a silent no-op.
    if cli.to.is_some() && !cli.update {
        return Err(usage("--to only applies to --update"));
    }
    if cli.yes && !(cli.update || cli.delete) {
        return Err(usage("--yes only applies to --update or --delete"));
    }
    if (cli.new_password.is_some() || cli.new_key_file.is_some()) && !(cli.init || cli.rotate) {
        return Err(usage(
            "--new-password and --new-key-file only apply to --init or --rotate",
        ));
    }
    if cli.force && !(cli.init || cli.update) {
        return Err(usage("--force only applies to --init or --update"));
    }
    if cli.json && (cli.init || cli.rotate || cli.update || cli.set.is_some() || cli.delete) {
        return Err(usage(
            "--json only applies to reading: `akc NAME...` or `akc --list`",
        ));
    }

    if cli.update {
        if !cli.names.is_empty() {
            return Err(usage("--update does not take secret names"));
        }
        return upgrade::run(cli.force, cli.yes, cli.to.as_deref());
    }

    let file = cli.keychain.clone().or(config.keychain.clone()).context(
        "no keychain configured: set `keychain` in the akc config file, or pass --keychain",
    )?;

    let key_file = cli
        .key_file
        .clone()
        .or_else(|| std::env::var_os("AKC_KEY_FILE").map(PathBuf::from));

    // `--init` only ever sets a key, so it must not be made to open the vault
    // first -- that would prompt for a key that does not exist yet.
    if cli.init {
        if !cli.names.is_empty() {
            return Err(usage("--init does not take secret names"));
        }
        // Deliberately no fallback to --password / AKC_PASSWORD: those unlock
        // an existing vault, and a key exported for reading must not silently
        // become the master key of a newly initialised one.
        let replacement = resolve_new_key(
            cli.new_password.as_deref(),
            cli.new_key_file
                .as_deref()
                .or(key_file.as_deref())
                .or(config.key_file.as_deref()),
        )?;
        return cmd_init(&file, cli.force, &replacement);
    }

    let password = resolve_key(cli.password.as_deref(), key_file.as_deref(), &config)?;

    if cli.rotate {
        if !cli.names.is_empty() {
            return Err(usage("--rotate does not take secret names"));
        }
        let replacement =
            resolve_new_key(cli.new_password.as_deref(), cli.new_key_file.as_deref())?;
        return cmd_rotate(&file, &password, &replacement);
    }
    if cli.list {
        return cmd_list(&file, &password, &cli.names, cli.json);
    }
    if let Some(value) = cli.set.as_ref() {
        let [name] = cli.names.as_slice() else {
            return Err(usage(
                "--set takes exactly one NAME: put the name before the flag, e.g. akc NAME --set VALUE",
            ));
        };
        return cmd_set(&file, name, value.as_deref(), &password);
    }
    if cli.delete {
        return cmd_delete(&file, &password, &cli.names, cli.yes);
    }
    cmd_get(&file, &password, &cli.names, cli.json)
}

/// Resolve the master key for opening an existing keychain.
///
/// Order: a literal password, then a key file, then the console. The key file
/// is what lets an agent fetch a secret with no prompt and no environment
/// setup, so it is checked before the console is considered at all.
fn resolve_key(
    password: Option<&str>,
    key_file: Option<&Path>,
    config: &Config,
) -> Result<Zeroizing<String>> {
    if let Some(password) = password {
        ensure!(!password.is_empty(), "password must not be empty");
        return Ok(Zeroizing::new(password.to_string()));
    }

    // A named key file must exist. The default one is optional, so that a
    // machine with nothing configured falls through to the prompt rather than
    // failing with a confusing message about a file the user never mentioned.
    if let Some(path) = key_file.or(config.key_file.as_deref()) {
        ensure!(
            path.exists(),
            "key file not found: {} (set `key_file` in the akc config file, or pass --key-file)",
            path.display()
        );
        return read_key_file(path);
    }
    if let Some(path) = default_key_file()
        && path.exists()
    {
        return read_key_file(&path);
    }

    password::read_password(None, false)
}

/// Resolve the replacement key for `--init` and `--rotate`.
///
/// A typed replacement is confirmed twice, because a typo here locks the
/// secrets away behind a key nobody has. One supplied by flag, file or
/// environment is used as-is: it cannot contain a typo, and asking twice would
/// break scripted use.
fn resolve_new_key(password: Option<&str>, key_file: Option<&Path>) -> Result<Zeroizing<String>> {
    if let Some(password) = password {
        ensure!(!password.is_empty(), "new password must not be empty");
        return Ok(Zeroizing::new(password.to_string()));
    }
    if let Some(path) = key_file {
        ensure!(path.exists(), "new key file not found: {}", path.display());
        return read_key_file(path);
    }
    password::read_password(None, true)
}

/// Normalize a secret value.
///
/// Surrounding whitespace is always removed, so `--set "  v  "` stores `v`.
/// That makes a blank value meaningless, so it is rejected rather than stored.
fn normalize_value(raw: &str) -> Result<String> {
    let value = raw.trim();
    ensure!(
        !value.is_empty(),
        "value must not be empty: surrounding whitespace is removed, so a blank value is not allowed"
    );
    Ok(value.to_string())
}

/// Read a secret value without putting it on the command line.
///
/// A pipe is the agent path. A terminal gets a hidden prompt instead, because a
/// value typed at a visible prompt would sit in the scrollback.
fn read_value() -> Result<String> {
    if std::io::stdin().is_terminal() {
        return rpassword::prompt_password("Value: ").context("failed to read the value");
    }
    let mut raw = String::new();
    std::io::stdin()
        .read_to_string(&mut raw)
        .context("failed to read the value from stdin")?;
    Ok(raw)
}

/// Apply `f` to the vault and save it, holding the vault's write lock.
///
/// The vault is reloaded inside the lock, so a write another process made since
/// this one last read it is preserved rather than clobbered. `f` returning an
/// error aborts before the save, leaving the file untouched.
fn mutate(
    file: &Path,
    password: &str,
    f: impl FnOnce(&mut Keychain) -> Result<()>,
) -> Result<Keychain> {
    let _lock = Lock::acquire(file)?;
    let mut kc = Keychain::load(file, password)?;
    f(&mut kc)?;
    kc.save(file, password, false)?;
    Ok(kc)
}

/// Create an empty keychain.
///
/// Refuses to touch an existing file unless `force` is given, so a mistyped
/// path cannot quietly replace a vault you already had. `--rotate` is the
/// operation for re-keying one you want to keep.
fn cmd_init(file: &Path, force: bool, password: &str) -> Result<()> {
    let _lock = Lock::acquire(file)?;
    if file.exists() {
        ensure!(
            force,
            "{} already exists: use --rotate to keep its secrets, \
             or --init --force to discard them and start over",
            file.display()
        );
        let backup = backup_path(file)?;
        std::fs::copy(file, &backup).with_context(|| {
            format!("cannot back up {} to {}", file.display(), backup.display())
        })?;
        println!("Backed up existing keychain to {}", backup.display());
    }
    Keychain::new().save(file, password, !force)?;
    println!("Created {}", file.display());
    Ok(())
}

/// Re-key the keychain, keeping every secret.
///
/// The vault is one blob, so this is a single re-encrypt rather than a loop
/// over secrets -- it costs the same whether the vault holds one key or a
/// thousand. A wrong current key cannot decrypt, so it fails before anything is
/// written, and the old file is backed up first.
///
/// Note what this cannot do: if the vault is committed to git, the old key
/// still opens every commit made before the rotation.
fn cmd_rotate(file: &Path, current: &str, replacement: &str) -> Result<()> {
    if !file.exists() {
        return Err(not_found(format!("keychain not found: {}", file.display())));
    }
    let _lock = Lock::acquire(file)?;
    let kc = Keychain::load(file, current)?;
    rekey(file, &kc, replacement, &mut std::io::stdout())
}

/// Back up the vault, then rewrite it under `new_password`.
fn rekey(file: &Path, kc: &Keychain, new_password: &str, out: &mut impl Write) -> Result<()> {
    let backup = backup_path(file)?;
    std::fs::copy(file, &backup)
        .with_context(|| format!("cannot back up {} to {}", file.display(), backup.display()))?;
    kc.save(file, new_password, false)?;
    writeln!(out, "Backed up existing keychain to {}", backup.display())?;
    writeln!(out, "Rotated the master key for {}", file.display())?;
    Ok(())
}

/// Store a secret. `value` is `None` when the flag was given with nothing after
/// it, which is the signal to read the secret from stdin instead of argv.
fn cmd_set(file: &Path, name: &str, value: Option<&str>, password: &str) -> Result<()> {
    let raw = Zeroizing::new(match value {
        Some(value) => value.to_string(),
        None => read_value()?,
    });
    let value = Zeroizing::new(normalize_value(&raw)?);
    mutate(file, password, |kc| {
        kc.set(name, value.to_string());
        Ok(())
    })?;
    Ok(())
}

fn cmd_get(file: &Path, password: &str, names: &[String], json: bool) -> Result<()> {
    if names.is_empty() {
        return Err(usage("no secret name given"));
    }
    let kc = Keychain::load(file, password)?;

    // Every name is resolved before anything is printed, so a missing one
    // cannot leave half a result on stdout for a caller to misread.
    let mut values = Vec::with_capacity(names.len());
    for name in names {
        let value = kc
            .get(name)
            .ok_or_else(|| not_found(format!("key not found: {name}")))?;
        values.push((name.as_str(), value.as_str()));
    }

    if json {
        let map: serde_json::Map<String, serde_json::Value> = values
            .into_iter()
            .map(|(k, v)| (k.to_string(), serde_json::Value::String(v.to_string())))
            .collect();
        println!("{}", serde_json::to_string_pretty(&map)?);
    } else {
        for (_, value) in values {
            println!("{value}");
        }
    }
    Ok(())
}

fn cmd_list(file: &Path, password: &str, names: &[String], json: bool) -> Result<()> {
    let kc = Keychain::load(file, password)?;
    let keys: Vec<&str> = if names.is_empty() {
        kc.keys()
    } else {
        // Naming keys asks "which of these exist", so a missing one is simply
        // absent rather than an error.
        names
            .iter()
            .filter(|name| kc.get(name).is_some())
            .map(String::as_str)
            .collect()
    };

    if json {
        println!("{}", serde_json::to_string_pretty(&keys)?);
    } else {
        for key in keys {
            println!("{key}");
        }
    }
    Ok(())
}

/// Remove one or more secrets. All or nothing: if any name is missing, nothing
/// is written.
///
/// On a terminal this asks first: unlike `--init` there is no `--force` guard
/// and unlike `--rotate` there is no backup, so a typo would be final. A
/// non-interactive caller (an agent, a script) is not made to hang on a prompt;
/// `--yes` skips it explicitly.
fn cmd_delete(file: &Path, password: &str, names: &[String], yes: bool) -> Result<()> {
    if names.is_empty() {
        return Err(usage("no secret name given"));
    }
    if !yes && std::io::stdin().is_terminal() {
        print!(
            "Delete {} from {}? [y/N] ",
            names.join(", "),
            file.display()
        );
        std::io::stdout().flush()?;
        let mut answer = String::new();
        std::io::stdin().read_line(&mut answer)?;
        if !answer.trim().eq_ignore_ascii_case("y") {
            println!("Cancelled.");
            return Ok(());
        }
    }
    mutate(file, password, |kc| {
        for name in names {
            if !kc.delete(name) {
                return Err(not_found(format!("key not found: {name}")));
            }
        }
        Ok(())
    })?;
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Unique per call. A timestamp alone is not enough: two tests can start
    /// inside the same clock tick, and sharing a directory means one test
    /// reads another's vault.
    fn temp_dir() -> PathBuf {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "akc-it-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
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

    // --- set / get / list / delete ---

    #[test]
    fn set_then_get_round_trips() {
        let dir = temp_dir();
        let file = dir.join("s.akc");
        cmd_init(&file, false, "pw").unwrap();

        cmd_set(&file, "api_key", Some("sk-123"), "pw").unwrap();
        cmd_get(&file, "pw", &["api_key".to_string()], false).unwrap();

        let kc = Keychain::load(&file, "pw").unwrap();
        assert_eq!(kc.get("api_key").map(String::as_str), Some("sk-123"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn set_strips_padding_from_values() {
        let dir = temp_dir();
        let file = dir.join("s.akc");
        cmd_init(&file, false, "pw").unwrap();

        cmd_set(&file, "k", Some("  padded  "), "pw").unwrap();
        let kc = Keychain::load(&file, "pw").unwrap();
        assert_eq!(kc.get("k").map(String::as_str), Some("padded"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn set_overwrites_an_existing_secret() {
        let dir = temp_dir();
        let file = dir.join("s.akc");
        cmd_init(&file, false, "pw").unwrap();

        cmd_set(&file, "k", Some("first"), "pw").unwrap();
        cmd_set(&file, "k", Some("second"), "pw").unwrap();

        let kc = Keychain::load(&file, "pw").unwrap();
        assert_eq!(kc.get("k").map(String::as_str), Some("second"));
        assert_eq!(kc.keys(), vec!["k"], "must not duplicate the name");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn set_rejects_a_blank_value_without_touching_the_vault() {
        let dir = temp_dir();
        let file = dir.join("s.akc");
        cmd_init(&file, false, "pw").unwrap();

        // A value of "   " is whitespace, not the empty-string signal that
        // means "read stdin", so it must be rejected rather than prompt.
        let err = cmd_set(&file, "k", Some("   "), "pw").unwrap_err().to_string();
        assert!(err.contains("must not be empty"), "{err}");

        let kc = Keychain::load(&file, "pw").unwrap();
        assert!(kc.keys().is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn get_reads_several_names_in_one_open() {
        let dir = temp_dir();
        let file = dir.join("s.akc");
        cmd_init(&file, false, "pw").unwrap();
        cmd_set(&file, "a", Some("1"), "pw").unwrap();
        cmd_set(&file, "b", Some("2"), "pw").unwrap();

        let names = vec!["a".to_string(), "b".to_string()];
        cmd_get(&file, "pw", &names, false).unwrap();
        cmd_get(&file, "pw", &names, true).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// One missing name must abort the whole read rather than print the names
    /// that did resolve, so a caller never sees a partial result.
    #[test]
    fn get_with_a_missing_name_prints_nothing() {
        let dir = temp_dir();
        let file = dir.join("s.akc");
        cmd_init(&file, false, "pw").unwrap();
        cmd_set(&file, "present", Some("v"), "pw").unwrap();

        let names = vec!["present".to_string(), "absent".to_string()];
        let err = cmd_get(&file, "pw", &names, false).unwrap_err().to_string();
        assert!(err.contains("key not found: absent"), "{err}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn get_with_no_names_is_an_error() {
        let dir = temp_dir();
        let file = dir.join("s.akc");
        cmd_init(&file, false, "pw").unwrap();
        assert!(cmd_get(&file, "pw", &[], false).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn list_shows_names_and_filters_when_given_them() {
        let dir = temp_dir();
        let file = dir.join("s.akc");
        cmd_init(&file, false, "pw").unwrap();
        cmd_set(&file, "alpha", Some("1"), "pw").unwrap();
        cmd_set(&file, "beta", Some("2"), "pw").unwrap();

        // Listing is a read: naming a missing key is not an error.
        let names = vec!["alpha".to_string(), "absent".to_string()];
        cmd_list(&file, "pw", &names, false).unwrap();
        cmd_list(&file, "pw", &[], true).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn delete_removes_secrets() {
        let dir = temp_dir();
        let file = dir.join("s.akc");
        cmd_init(&file, false, "pw").unwrap();
        cmd_set(&file, "a", Some("1"), "pw").unwrap();
        cmd_set(&file, "b", Some("2"), "pw").unwrap();

        cmd_delete(&file, "pw", &["a".to_string()], true).unwrap();
        let kc = Keychain::load(&file, "pw").unwrap();
        assert_eq!(kc.keys(), vec!["b"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn delete_of_a_missing_name_removes_nothing() {
        let dir = temp_dir();
        let file = dir.join("s.akc");
        cmd_init(&file, false, "pw").unwrap();
        cmd_set(&file, "keep", Some("me"), "pw").unwrap();

        let names = vec!["keep".to_string(), "absent".to_string()];
        let err = cmd_delete(&file, "pw", &names, true)
            .unwrap_err()
            .to_string();
        assert!(err.contains("key not found: absent"), "{err}");

        // All or nothing: the name that did exist must still be there.
        let kc = Keychain::load(&file, "pw").unwrap();
        assert_eq!(kc.get("keep").map(String::as_str), Some("me"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    // --- init ---

    #[test]
    fn init_creates_a_new_keychain() {
        let dir = temp_dir();
        let file = dir.join("new.akc");
        cmd_init(&file, false, "pw").unwrap();
        assert!(file.exists());
        assert_eq!(backup_count(&dir), 0);

        cmd_set(&file, "k", Some("v"), "pw").unwrap();
        let kc = Keychain::load(&file, "pw").unwrap();
        assert_eq!(kc.get("k").map(String::as_str), Some("v"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn init_refuses_to_clobber_an_existing_keychain() {
        let dir = temp_dir();
        let file = dir.join("s.akc");
        cmd_init(&file, false, "pw").unwrap();
        cmd_set(&file, "keep", Some("me"), "pw").unwrap();

        let err = cmd_init(&file, false, "pw").unwrap_err().to_string();
        assert!(err.contains("already exists"), "{err}");
        assert!(err.contains("--rotate"), "{err}");

        let kc = Keychain::load(&file, "pw").unwrap();
        assert_eq!(kc.get("keep").map(String::as_str), Some("me"));
        assert_eq!(backup_count(&dir), 0, "a refused init must not back up");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn init_force_replaces_and_backs_up() {
        let dir = temp_dir();
        let file = dir.join("s.akc");
        cmd_init(&file, false, "oldpass").unwrap();
        cmd_set(&file, "keep", Some("me"), "oldpass").unwrap();

        cmd_init(&file, true, "newpass").unwrap();

        let reloaded = Keychain::load(&file, "newpass").unwrap();
        assert_eq!(reloaded.get("keep"), None, "force discards the secrets");
        assert!(Keychain::load(&file, "oldpass").is_err());
        assert_eq!(backup_count(&dir), 1);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    // --- rotate ---

    #[test]
    fn rotate_keeps_every_secret() {
        let dir = temp_dir();
        let file = dir.join("s.akc");
        cmd_init(&file, false, "oldpass").unwrap();
        cmd_set(&file, "keep", Some("me"), "oldpass").unwrap();
        cmd_set(&file, "other", Some("value"), "oldpass").unwrap();

        cmd_rotate(&file, "oldpass", "newpass").unwrap();

        let reloaded = Keychain::load(&file, "newpass").unwrap();
        assert_eq!(reloaded.get("keep").map(String::as_str), Some("me"));
        assert_eq!(reloaded.get("other").map(String::as_str), Some("value"));
        assert!(Keychain::load(&file, "oldpass").is_err());
        assert_eq!(backup_count(&dir), 1);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A wrong current key cannot decrypt, so it must fail before anything is
    /// written or backed up.
    #[test]
    fn rotate_with_a_wrong_key_changes_nothing() {
        let dir = temp_dir();
        let file = dir.join("s.akc");
        cmd_init(&file, false, "oldpass").unwrap();
        cmd_set(&file, "keep", Some("me"), "oldpass").unwrap();

        let err = cmd_rotate(&file, "wrong", "newpass").unwrap_err().to_string();
        assert!(err.contains("wrong password"), "{err}");

        assert!(Keychain::load(&file, "oldpass").is_ok());
        assert_eq!(backup_count(&dir), 0);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn rotate_requires_an_existing_keychain() {
        let dir = temp_dir();
        let err = cmd_rotate(&dir.join("missing.akc"), "pw", "pw")
            .unwrap_err()
            .to_string();
        assert!(err.contains("not found"), "{err}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    // --- key resolution ---

    #[test]
    fn a_literal_password_wins() {
        let key = resolve_key(Some("literal"), None, &Config::default()).unwrap();
        assert_eq!(key.as_str(), "literal");
    }

    #[test]
    fn an_empty_literal_password_is_rejected() {
        assert!(resolve_key(Some(""), None, &Config::default()).is_err());
    }

    /// A named key file must exist: silently falling back to a prompt would
    /// leave an agent hanging on a file the user believed was configured.
    #[test]
    fn a_missing_named_key_file_is_an_error() {
        let err = resolve_key(None, Some(Path::new("/nonexistent/key")), &Config::default())
            .unwrap_err()
            .to_string();
        assert!(err.contains("key file not found"), "{err}");
    }

    #[test]
    fn a_key_file_is_read_from_the_config() {
        let dir = temp_dir();
        let key_path = dir.join("key");
        std::fs::write(&key_path, "from-file\n").unwrap();

        let config = Config {
            keychain: None,
            key_file: Some(key_path),
        };
        let key = resolve_key(None, None, &config).unwrap();
        assert_eq!(key.as_str(), "from-file");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_new_key_file_must_exist() {
        let err = resolve_new_key(None, Some(Path::new("/nonexistent/key")))
            .unwrap_err()
            .to_string();
        assert!(err.contains("new key file not found"), "{err}");
    }

    // --- concurrent writers ---

    /// Regression: the vault is one blob that every write rewrites wholesale, so
    /// two writers that both read before either saved would clobber each other
    /// and silently lose a secret. Before the write lock, ten concurrent `set`
    /// calls left one key standing.
    #[test]
    fn concurrent_writers_do_not_lose_secrets() {
        let dir = temp_dir();
        let file = dir.join("s.akc");
        cmd_init(&file, false, "pw").unwrap();

        const WRITERS: usize = 6;
        std::thread::scope(|scope| {
            for i in 0..WRITERS {
                let file = &file;
                scope.spawn(move || {
                    mutate(file, "pw", |kc| {
                        kc.set(&format!("key{i}"), format!("value{i}"));
                        Ok(())
                    })
                    .unwrap();
                });
            }
        });

        let kc = Keychain::load(&file, "pw").unwrap();
        assert_eq!(kc.keys().len(), WRITERS, "lost writes: {:?}", kc.keys());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// `mutate` reloads inside the lock, so a write made after the caller last
    /// read the vault is preserved rather than overwritten by the stale copy.
    #[test]
    fn mutate_reloads_before_writing() {
        let dir = temp_dir();
        let file = dir.join("s.akc");
        cmd_init(&file, false, "pw").unwrap();

        // What a long-lived caller holds: state read before the other write.
        let stale = Keychain::load(&file, "pw").unwrap();
        assert!(stale.keys().is_empty());

        cmd_set(&file, "theirs", Some("other"), "pw").unwrap();

        mutate(&file, "pw", |kc| {
            kc.set("mine", "value".to_string());
            Ok(())
        })
        .unwrap();

        let reloaded = Keychain::load(&file, "pw").unwrap();
        assert_eq!(reloaded.get("mine").map(String::as_str), Some("value"));
        assert_eq!(reloaded.get("theirs").map(String::as_str), Some("other"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A rejected mutation must not rewrite the vault.
    #[test]
    fn a_rejected_mutation_leaves_the_vault_alone() {
        let dir = temp_dir();
        let file = dir.join("s.akc");
        cmd_init(&file, false, "pw").unwrap();
        cmd_set(&file, "keep", Some("me"), "pw").unwrap();
        let before = std::fs::read(&file).unwrap();

        let err = mutate(&file, "pw", |kc| {
            kc.set("added", "x".to_string());
            anyhow::bail!("rejected")
        })
        .err()
        .expect("the mutation should have failed")
        .to_string();
        assert!(err.contains("rejected"), "{err}");

        assert_eq!(std::fs::read(&file).unwrap(), before, "file was rewritten");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    // --- exit codes ---

    #[test]
    fn exit_codes_are_distinct() {
        assert_eq!(exit_code(&anyhow::Error::new(crypto::AuthError)), 3);
        assert_eq!(exit_code(&not_found("key not found: x")), 4);
        assert_eq!(exit_code(&usage("bad flag")), 2);
        assert_eq!(exit_code(&anyhow::anyhow!("other")), 1);
    }
}
