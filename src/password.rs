use anyhow::{Context, Result, ensure};
use zeroize::Zeroizing;

pub fn read_password(provided: Option<String>, confirm: bool) -> Result<Zeroizing<String>> {
    read_with(provided, confirm, |prompt| {
        rpassword::prompt_password(prompt).context("failed to read password")
    })
}

fn read_with(
    provided: Option<String>,
    confirm: bool,
    mut prompt: impl FnMut(&str) -> Result<String>,
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
        let password = read_with(Some(" pass ".into()), true, |_| panic!()).unwrap();
        assert_eq!(password.as_str(), " pass ");
    }

    #[test]
    fn interactive_confirmation() {
        let mut calls = 0;
        let password = read_with(None, true, |_| {
            calls += 1;
            Ok(" pass ".into())
        })
        .unwrap();
        assert_eq!(password.as_str(), " pass ");
        assert_eq!(calls, 2);
    }

    #[test]
    fn mismatched_confirmation_is_rejected() {
        let mut answers = ["first", "second"].into_iter();
        assert!(read_with(None, true, |_| Ok(answers.next().unwrap().into())).is_err());
    }

    #[test]
    fn empty_and_failed_input_are_rejected() {
        assert!(read_with(Some(String::new()), false, |_| panic!()).is_err());
        assert!(read_with(None, false, |_| anyhow::bail!("no terminal")).is_err());
    }

    #[test]
    fn existing_password_only_prompts_once() {
        let mut calls = 0;
        read_with(None, false, |_| {
            calls += 1;
            Ok("password".into())
        })
        .unwrap();
        assert_eq!(calls, 1);
    }
}
