//! `--answers env:NAME` reads the answers JSON from an environment variable,
//! so a container needs no file for them. Anything else is a file path.

use std::path::PathBuf;

use crate::{CliError, CliResult};

/// Resolve an `--answers` argument to a readable file path.
///
/// A value shaped `env:NAME` is read from the environment variable `NAME`
/// and staged into a temp file `run_start` can open like any other answers
/// file; anything else is returned unchanged, as a plain path, exactly as
/// today.
pub(crate) fn resolve(raw: PathBuf) -> CliResult<PathBuf> {
    let Some(name) = raw.to_str().and_then(|text| text.strip_prefix("env:")) else {
        return Ok(raw);
    };
    let value = std::env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            CliError::usage(format!(
                "--answers names the environment variable `{name}`, which is unset or empty"
            ))
        })?;
    let dir = std::env::temp_dir().join(format!("greentic-sorx-answers-{}", std::process::id()));
    std::fs::create_dir_all(&dir)
        .map_err(|err| CliError::usage(format!("cannot stage answers: {err}")))?;
    let path = dir.join("answers.json");
    std::fs::write(&path, value)
        .map_err(|err| CliError::usage(format!("cannot stage answers: {err}")))?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plain_path_is_returned_unchanged() {
        let path = PathBuf::from("/tmp/answers.json");
        assert_eq!(resolve(path.clone()).expect("plain path"), path);
    }

    #[test]
    fn an_unset_variable_names_itself_in_the_error() {
        // SAFETY: test-only; no other thread in this process reads or writes
        // this variable name.
        unsafe {
            std::env::remove_var("SORX_ANSWERS_SOURCE_TEST_UNSET");
        }
        let err = resolve(PathBuf::from("env:SORX_ANSWERS_SOURCE_TEST_UNSET"))
            .expect_err("unset variable must be refused");
        assert!(
            err.message.contains("SORX_ANSWERS_SOURCE_TEST_UNSET"),
            "{}",
            err.message
        );
    }

    #[test]
    fn an_empty_variable_is_treated_as_unset() {
        // SAFETY: test-only; see above.
        unsafe {
            std::env::set_var("SORX_ANSWERS_SOURCE_TEST_EMPTY", "   ");
        }
        let err = resolve(PathBuf::from("env:SORX_ANSWERS_SOURCE_TEST_EMPTY"))
            .expect_err("blank variable must be refused");
        assert!(
            err.message.contains("SORX_ANSWERS_SOURCE_TEST_EMPTY"),
            "{}",
            err.message
        );
        // SAFETY: test-only; see above.
        unsafe {
            std::env::remove_var("SORX_ANSWERS_SOURCE_TEST_EMPTY");
        }
    }

    #[test]
    fn a_set_variable_is_staged_to_a_readable_file() {
        // SAFETY: test-only; see above.
        unsafe {
            std::env::set_var("SORX_ANSWERS_SOURCE_TEST_SET", "{\"ok\":true}");
        }
        let path = resolve(PathBuf::from("env:SORX_ANSWERS_SOURCE_TEST_SET")).expect("staged");
        let staged = std::fs::read_to_string(&path).expect("read staged answers");
        assert_eq!(staged, "{\"ok\":true}");
        // SAFETY: test-only; see above.
        unsafe {
            std::env::remove_var("SORX_ANSWERS_SOURCE_TEST_SET");
        }
    }
}
