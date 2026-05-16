use directories::UserDirs;
use inquire::{
    CustomUserError, Text,
    autocompletion::{Autocomplete, Replacement},
    ui::{Color, RenderConfig, StyleSheet},
    validator::Validation,
};
use std::fs;
use std::path::{self, PathBuf};

/// Number of items to show in the autocomplete dropdown
const AUTOCOMPLETE_PAGE_SIZE: usize = 7;

/// Ensures a path string ends with the platform's path separator
fn ensure_trailing_separator(mut s: String) -> String {
    if !s.ends_with(path::MAIN_SEPARATOR) {
        s.push(path::MAIN_SEPARATOR);
    }
    s
}

/// Cleans user input to handle Drag & Drop behavior (stripping quotes)
/// and whitespace.
fn clean_path(input: &str) -> String {
    input
        .trim()
        .trim_matches(&['\'', '"'][..])
        .trim()
        .to_string()
}

/// Expands a leading `~` to the user's home directory.
/// If no `UserDirs` is provided, it attempts to create one.
/// Only expands exactly `~`, `~/...`, or `~\\...` (Windows).
/// Returns the expanded path or the original input unchanged.
fn expand_home(input: &str, user_dirs: Option<&UserDirs>) -> PathBuf {
    let home = user_dirs
        .map(|d| d.home_dir().to_path_buf())
        .or_else(|| UserDirs::new().map(|d| d.home_dir().to_path_buf()));

    let Some(home) = home else {
        return PathBuf::from(input);
    };

    if input == "~" {
        return home;
    }
    if let Some(rest) = input.strip_prefix("~/") {
        return home.join(rest);
    }
    if let Some(rest) = input.strip_prefix("~\\") {
        return home.join(rest);
    }

    PathBuf::from(input)
}

/// A robust autocompleter for file system paths.
/// Suggests common user directories when input is empty, and
/// scans the filesystem for partial matches.
#[derive(Clone)]
struct FileSystemCompleter {
    user_dirs: Option<UserDirs>,
}

impl FileSystemCompleter {
    fn new() -> Self {
        Self {
            user_dirs: UserDirs::new(),
        }
    }

    /// Determines the search directory and filename prefix based on user input.
    /// Returns (base_directory, filename_prefix) for filtering suggestions.
    fn get_search_context(&self, clean_input: &str) -> (PathBuf, String) {
        let expanded_path = expand_home(clean_input, self.user_dirs.as_ref());

        if clean_input.ends_with(path::MAIN_SEPARATOR) || clean_input.is_empty() {
            if clean_input.is_empty() {
                (PathBuf::from("."), String::new())
            } else {
                (expanded_path, String::new())
            }
        } else {
            let parent = expanded_path
                .parent()
                .map(|p| {
                    if p.as_os_str().is_empty() {
                        PathBuf::from(".")
                    } else {
                        p.to_path_buf()
                    }
                })
                .unwrap_or_else(|| PathBuf::from("."));

            let file_stem = expanded_path
                .file_name()
                .map(|f| f.to_string_lossy().to_string())
                .unwrap_or_default();

            (parent, file_stem)
        }
    }
}

impl Autocomplete for FileSystemCompleter {
    fn get_suggestions(&mut self, input: &str) -> Result<Vec<String>, CustomUserError> {
        let clean = clean_path(input);

        if clean.is_empty() {
            let mut suggestions = Vec::new();

            if let Some(dirs) = &self.user_dirs {
                // Prefer common drop locations for non-technical users.
                if let Some(p) = dirs.desktop_dir() {
                    suggestions.push(ensure_trailing_separator(p.display().to_string()));
                }
                if let Some(p) = dirs.document_dir() {
                    suggestions.push(ensure_trailing_separator(p.display().to_string()));
                }
                if let Some(p) = dirs.download_dir() {
                    suggestions.push(ensure_trailing_separator(p.display().to_string()));
                }
                suggestions.push(ensure_trailing_separator(
                    dirs.home_dir().display().to_string(),
                ));
            }
            suggestions.push(String::from("."));
            return Ok(suggestions);
        }

        let (base_dir, prefix) = self.get_search_context(&clean);
        let mut suggestions = Vec::new();

        if let Ok(entries) = fs::read_dir(base_dir) {
            for entry in entries.flatten() {
                let path = entry.path();

                if !path.is_dir() {
                    continue;
                }

                let name = entry.file_name().to_string_lossy().to_string();

                if name.to_lowercase().starts_with(&prefix.to_lowercase()) {
                    let suggestion =
                        if clean.contains(path::MAIN_SEPARATOR) || clean.starts_with("~") {
                            path.to_string_lossy().to_string()
                        } else {
                            name
                        };

                    suggestions.push(ensure_trailing_separator(suggestion));
                }
            }
        }

        suggestions.sort();
        Ok(suggestions)
    }

    fn get_completion(
        &mut self,
        input: &str,
        highlighted_suggestion: Option<String>,
    ) -> Result<Replacement, CustomUserError> {
        if let Some(s) = highlighted_suggestion {
            return Ok(Replacement::Some(s));
        }

        // Auto-complete if there is exactly one match.
        let suggestions = self.get_suggestions(input)?;
        if suggestions.len() == 1 {
            return Ok(Replacement::Some(suggestions[0].clone()));
        }

        Ok(Replacement::None)
    }
}

/// A Generic helper to prompt for a path with standardized styling and behavior
fn prompt_path(
    message: &str,
    help_msg: &str,
    must_exist: bool,
    must_be_dir_if_exists: bool,
) -> Result<PathBuf, inquire::InquireError> {
    let user_dirs = UserDirs::new();

    let validator = |input: &str| {
        let cleaned = clean_path(input);
        if cleaned.is_empty() {
            return Ok(Validation::Invalid("Please enter a path.".into()));
        }

        let path = expand_home(&cleaned, user_dirs.as_ref());

        if must_exist && !path.exists() {
            return Ok(Validation::Invalid("This path does not exist.".into()));
        }

        if must_be_dir_if_exists && path.exists() && !path.is_dir() {
            return Ok(Validation::Invalid("This path is not a folder.".into()));
        }

        if !must_exist
            && let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
            && !parent.exists()
        {
            return Ok(Validation::Invalid(
                "The parent folder for this path doesn't exist.".into(),
            ));
        }

        Ok(Validation::Valid)
    };

    let completer = FileSystemCompleter::new();

    let option_style = StyleSheet::new().with_fg(Color::DarkCyan);
    let selected_option_style = StyleSheet::new().with_fg(Color::LightCyan);
    let render_config = RenderConfig::default()
        .with_option(option_style)
        .with_selected_option(Some(selected_option_style));

    let input = Text::new(message)
        .with_placeholder("Type, paste, or drag & drop a path")
        .with_help_message(help_msg)
        .with_autocomplete(completer)
        .with_validator(validator)
        .with_page_size(AUTOCOMPLETE_PAGE_SIZE)
        .with_render_config(render_config)
        .prompt()?;

    Ok(expand_home(&clean_path(&input), user_dirs.as_ref()))
}

/// Helper for getting backup source
/// Ensures the path exists (can be a file or directory).
pub fn get_backup_input_path() -> Result<PathBuf, inquire::InquireError> {
    prompt_path(
        "Enter input path\n>",
        "↑↓ and <tab> to select suggestion",
        true,
        false,
    )
}

/// Helper for getting extraction trget
/// Allows non-existent directories (as long as parent exists) so we can create them.
pub fn get_target_output_path() -> Result<PathBuf, inquire::InquireError> {
    prompt_path(
        "Enter output path\n>",
        "↑↓ and <tab> to select suggestion",
        false,
        true,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ensure_trailing_separator() {
        assert_eq!(
            ensure_trailing_separator("foo".to_string()),
            format!("foo{}", path::MAIN_SEPARATOR)
        );
        assert_eq!(
            ensure_trailing_separator(format!("foo{}", path::MAIN_SEPARATOR)),
            format!("foo{}", path::MAIN_SEPARATOR)
        );
    }

    #[test]
    fn test_clean_path() {
        assert_eq!(clean_path("  /tmp/foo  "), "/tmp/foo");
        assert_eq!(clean_path("' /tmp/foo '"), "/tmp/foo");
        assert_eq!(clean_path("\"/tmp/foo\""), "/tmp/foo");
    }

    #[test]
    fn test_expand_home_bare_tilde() {
        let home = UserDirs::new().unwrap().home_dir().to_path_buf();
        let result = expand_home("~", None);
        assert_eq!(result, home);
    }

    #[test]
    fn test_expand_home_tilde_slash() {
        let home = UserDirs::new().unwrap().home_dir().to_path_buf();
        let result = expand_home("~/foo", None);
        assert_eq!(result, home.join("foo"));
    }

    #[test]
    fn test_expand_home_does_not_expand_tilde_username() {
        let result = expand_home("~otheruser", None);
        assert!(result.as_path() == std::path::Path::new("~otheruser"));
    }
}
