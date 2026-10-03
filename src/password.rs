use anyhow::{Context, Result, ensure};
use zeroize::Zeroizing;

/// Read a password from the console, using `provided` if it is set.
///
/// `confirm` only applies when the password is typed interactively: a password
/// supplied via `--password` or `AKC_PASSWORD` has already been entered once by
/// whoever set it, so asking for it twice would break scripted use.
pub fn read_password(provided: Option<String>, confirm: bool) -> Result<Zeroizing<String>> {
    read_with(provided, confirm, &mut console_prompt)
}

/// The real terminal prompt, exposed so callers that take an injectable prompt
/// can still reach the console.
pub(crate) fn console_prompt(prompt: &str) -> Result<String> {
    rpassword::prompt_password(prompt).context("failed to read password")
}

/// Password reader with an injectable prompt, so callers that are not attached
/// to a terminal (notably interactive mode) can drive it in tests.
pub(crate) fn read_with(
    provided: Option<String>,
    confirm: bool,
    prompt: &mut dyn FnMut(&str) -> Result<String>,
) -> Result<Zeroizing<String>> {
    let interactive = provided.is_none();
    let password = Zeroizing::new(match provided {
        Some(password) => password,
        None => prompt("Password: ")?,
    });
    ensure!(!password.is_empty(), "password must not be empty");
    if interactive && confirm {
        let confirmation = Zeroizing::new(prompt("Confirm password: ")?);
        ensure!(*password == *confirmation, "passwords do not match");
    }
    Ok(password)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supplied_password_preserves_whitespace_without_prompting() {
        let mut panic_if_prompted = |_: &str| -> Result<String> { panic!("should not prompt") };
        let password = read_with(Some(" pass ".into()), true, &mut panic_if_prompted).unwrap();
        assert_eq!(password.as_str(), " pass ");
    }

    #[test]
    fn interactive_confirmation() {
        let mut calls = 0;
        let mut prompt = |_: &str| {
            calls += 1;
            Ok(" pass ".into())
        };
        let password = read_with(None, true, &mut prompt).unwrap();
        assert_eq!(password.as_str(), " pass ");
        assert_eq!(calls, 2);
    }

    #[test]
    fn mismatched_confirmation_is_rejected() {
        let mut answers = ["first", "second"].into_iter();
        let mut prompt = |_: &str| Ok(answers.next().unwrap().to_string());
        assert!(read_with(None, true, &mut prompt).is_err());
    }

    #[test]
    fn empty_and_failed_input_are_rejected() {
        let mut panic_if_prompted = |_: &str| -> Result<String> { panic!("should not prompt") };
        assert!(read_with(Some(String::new()), false, &mut panic_if_prompted).is_err());

        let mut failing = |_: &str| anyhow::bail!("no terminal");
        assert!(read_with(None, false, &mut failing).is_err());
    }

    #[test]
    fn existing_password_only_prompts_once() {
        let mut calls = 0;
        let mut prompt = |_: &str| {
            calls += 1;
            Ok("password".into())
        };
        read_with(None, false, &mut prompt).unwrap();
        assert_eq!(calls, 1);
    }

    #[test]
    fn whitespace_only_password_is_accepted() {
        // Trailing/leading spaces are legitimate password material and must not
        // be trimmed away by the reader.
        let mut prompt = |_: &str| Ok("  ".to_string());
        let password = read_with(None, false, &mut prompt).unwrap();
        assert_eq!(password.as_str(), "  ");
    }
}
