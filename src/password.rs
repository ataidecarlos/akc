use anyhow::{Context, Result, ensure};
use zeroize::Zeroizing;

/// Read a password from the console.
///
/// `confirm` asks for it twice, which is what catches a typo before it locks
/// secrets away. A caller that supplies a password programmatically does not
/// call this at all.
pub fn read_password(confirm: bool) -> Result<Zeroizing<String>> {
    read_with(confirm, &mut console_prompt)
}

/// The real terminal prompt, exposed so callers that take an injectable prompt
/// can still reach the console.
pub(crate) fn console_prompt(prompt: &str) -> Result<String> {
    rpassword::prompt_password(prompt).context("failed to read password")
}

/// Password reader with an injectable prompt, so the retry/confirm rules can be
/// tested without a terminal.
pub(crate) fn read_with(
    confirm: bool,
    prompt: &mut dyn FnMut(&str) -> Result<String>,
) -> Result<Zeroizing<String>> {
    let password = Zeroizing::new(prompt("Password: ")?);
    ensure!(!password.is_empty(), "password must not be empty");
    if confirm {
        let confirmation = Zeroizing::new(prompt("Confirm password: ")?);
        ensure!(*password == *confirmation, "passwords do not match");
    }
    Ok(password)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interactive_confirmation() {
        let mut calls = 0;
        let mut prompt = |_: &str| {
            calls += 1;
            Ok(" pass ".into())
        };
        let password = read_with(true, &mut prompt).unwrap();
        assert_eq!(password.as_str(), " pass ");
        assert_eq!(calls, 2);
    }

    #[test]
    fn mismatched_confirmation_is_rejected() {
        let mut answers = ["first", "second"].into_iter();
        let mut prompt = |_: &str| Ok(answers.next().unwrap().to_string());
        assert!(read_with(true, &mut prompt).is_err());
    }

    #[test]
    fn empty_and_failed_input_are_rejected() {
        let mut empty = |_: &str| Ok(String::new());
        assert!(read_with(false, &mut empty).is_err());

        let mut failing = |_: &str| anyhow::bail!("no terminal");
        assert!(read_with(false, &mut failing).is_err());
    }

    #[test]
    fn existing_password_only_prompts_once() {
        let mut calls = 0;
        let mut prompt = |_: &str| {
            calls += 1;
            Ok("password".into())
        };
        read_with(false, &mut prompt).unwrap();
        assert_eq!(calls, 1);
    }

    #[test]
    fn whitespace_only_password_is_accepted() {
        // Trailing/leading spaces are legitimate password material and must not
        // be trimmed away by the reader.
        let mut prompt = |_: &str| Ok("  ".to_string());
        let password = read_with(false, &mut prompt).unwrap();
        assert_eq!(password.as_str(), "  ");
    }
}
