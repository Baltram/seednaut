use anyhow::{Context, Result, anyhow, bail};
use bip39::{Language, Mnemonic};
use inquire::{InquireError, Password, PasswordDisplayMode, validator::Validation};
use std::io::{IsTerminal, Read, Write, stderr, stdin, stdout};
use thiserror::Error;

/// Custom error for more user-friendly mnemonic validation messages.
#[derive(Error, Debug)]
enum MnemonicError {
    #[error("Mnemonic must have 12 words, but it has {0}.")]
    BadWordCount(usize),
    #[error("Mnemonic contains an unknown word (word #{0}).")]
    UnknownWord(usize),
    #[error("{0}")]
    Other(String),
}

impl From<bip39::Error> for MnemonicError {
    fn from(error: bip39::Error) -> Self {
        match error {
            bip39::Error::BadWordCount(count) => MnemonicError::BadWordCount(count),
            bip39::Error::UnknownWord(index) => MnemonicError::UnknownWord(index + 1), // Use 1-based index
            _ => MnemonicError::Other(error.to_string()),
        }
    }
}

/// Parses a case-insensitive user-provided string into a BIP39 Mnemonic.
fn parse_mnemonic(input: &str) -> Result<Mnemonic, MnemonicError> {
    let normalized = input.trim().to_lowercase();
    Mnemonic::parse_in(Language::English, normalized).map_err(Into::into)
}

/// A validator for `inquire` to check if the input is a valid mnemonic phrase.
fn mnemonic_validator(input: &str) -> Result<Validation, Box<dyn std::error::Error + Send + Sync>> {
    match parse_mnemonic(input) {
        Ok(_) => Ok(Validation::Valid),
        Err(e) => Ok(Validation::Invalid(e.to_string().into())),
    }
}

/// Obtains the mnemonic, either from stdin (if not a TTY) or an interactive prompt.
pub fn get_mnemonic() -> Result<Mnemonic> {
    let mnemonic_str = if !stdin().is_terminal() {
        if stderr().is_terminal() {
            eprintln!("Reading mnemonic from stdin...");
        }
        let mut buffer = String::new();
        stdin()
            .read_to_string(&mut buffer)
            .context("Failed to read mnemonic from stdin")?;
        if buffer.trim().is_empty() {
            bail!("No mnemonic found on stdin");
        }
        buffer
    } else {
        // Clear any prompt residue so the mnemonic doesn't leak into terminal output.
        // Only needed on error/interrupt; inquire handles cleanup on success and cancel.
        let clear_prompt = || {
            let _ = write!(stdout(), "\x1b[2K\x1b[1A\x1b[2K");
        };
        match Password::new("Enter your 12-word Seedvault mnemonic phrase:")
            .with_display_mode(PasswordDisplayMode::Full)
            .without_confirmation()
            .with_validator(mnemonic_validator)
            .with_help_message("Paste or type the phrase, separated by spaces.")
            .prompt()
        {
            Ok(s) => s,
            Err(InquireError::OperationInterrupted) => {
                clear_prompt();
                std::process::exit(0);
            }
            Err(InquireError::OperationCanceled) => {
                std::process::exit(0);
            }
            Err(e) => {
                clear_prompt();
                return Err(anyhow::anyhow!(
                    "Failed to read mnemonic from interactive prompt: {}",
                    e
                ));
            }
        }
    };

    parse_mnemonic(&mnemonic_str).map_err(|e| anyhow!("Invalid mnemonic phrase: {}", e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_mnemonic_valid() {
        let phrase = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
        let res = parse_mnemonic(phrase);
        assert!(res.is_ok());
    }

    #[test]
    fn test_parse_mnemonic_invalid_word_count() {
        let phrase = "abandon abandon";
        let res = parse_mnemonic(phrase);
        assert!(matches!(res, Err(MnemonicError::BadWordCount(2))));
    }

    #[test]
    fn test_parse_mnemonic_unknown_word() {
        let phrase = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon foo";
        let res = parse_mnemonic(phrase);
        assert!(matches!(res, Err(MnemonicError::UnknownWord(12))));
    }
}
