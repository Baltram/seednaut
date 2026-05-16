use anyhow::{Result, bail};
use std::collections::HashSet;
use std::ffi::OsStr;
use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};

/// Validates that a candidate relative path contains only safe components.
///
/// Accepts `Normal` and `CurDir` components; rejects `ParentDir`, `RootDir`,
/// and `Prefix` (Windows). Returns a cleaned `PathBuf` with only `Normal` parts.
pub fn validate_relative_path(rel: &Path) -> Result<PathBuf> {
    let mut out = PathBuf::new();
    let mut has_components = false;

    for c in rel.components() {
        match c {
            Component::Normal(part) => {
                out.push(part);
                has_components = true;
            }
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                bail!("Unsafe path component '{:?}' in '{}'", c, rel.display());
            }
        }
    }

    if !has_components {
        bail!("Path '{}' resolved to empty relative path", rel.display());
    }

    Ok(out)
}

/// Validates that `name` is a single safe filename component.
///
/// Rejects empty strings, names containing `/` or `\` (portably), `..`,
/// absolute paths, and anything that resolves to more than one path component
/// on the host OS.
pub fn validate_single_component(name: &str) -> Result<()> {
    if name.contains('/') || name.contains('\\') {
        bail!("Name must not contain path separators: '{}'", name);
    }

    let mut components = Path::new(name).components();
    match (components.next(), components.next()) {
        (Some(Component::Normal(part)), None) if !part.is_empty() => Ok(()),
        _ => bail!("Name is not a single safe filename component: '{}'", name),
    }
}

#[cfg(windows)]
const HOST_INVALID_CHARS: &[char] = &['<', '>', ':', '"', '/', '\\', '|', '?', '*'];

#[cfg(windows)]
const DOS_RESERVED: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// Normalizes a single path component for the host filesystem.
///
/// On Windows this percent-encodes `%` itself (as `%25`, so mapping stays
/// reversible), then encodes invalid characters (`< > : " / \\ | ? *`),
/// control characters (`\x00..\x1F`), trailing dots/spaces, and reserved DOS
/// names (`CON`, `PRN`, `NUL`, `COM1`–`COM9`, `LPT1`–`LPT9`). On Unix this
/// is the identity function.
pub fn normalize_component(component: &OsStr) -> OsString {
    #[cfg(not(windows))]
    {
        component.to_os_string()
    }

    #[cfg(windows)]
    {
        let mut s = component.to_string_lossy().into_owned();

        // 1. Encode invalid/control characters.  % is encoded first (as %25)
        //    so that a literal "%3A" in the archive does not collide with a
        //    ":" encoded as "%3A".
        let mut encoded = String::with_capacity(s.len());
        for ch in s.chars() {
            if (ch as u32) < 0x20 || ch == '%' || HOST_INVALID_CHARS.contains(&ch) {
                push_percent_encoded(&mut encoded, ch);
            } else {
                encoded.push(ch);
            }
        }
        s = encoded;

        // 2. Trailing dots/spaces — iterate backward so "foo. ." stays correct
        let mut trailing = String::new();
        while s.ends_with(['.', ' ']) {
            let ch = s.pop().unwrap();
            let mut enc = String::new();
            push_percent_encoded(&mut enc, ch);
            trailing.insert_str(0, &enc);
        }
        s.push_str(&trailing);

        // 3. Reserved DOS names (case-insensitive; with or without extension)
        let stem = s.split('.').next().unwrap_or(&s);
        if stem.len() <= 4 {
            let upper = stem.to_uppercase();
            if DOS_RESERVED.contains(&upper.as_str()) {
                // percent-encode first character of the stem
                let first = s.remove(0);
                let mut enc = String::new();
                push_percent_encoded(&mut enc, first);
                s.insert_str(0, &enc);
            }
        }

        OsString::from(s)
    }
}

#[cfg(windows)]
fn push_percent_encoded(out: &mut String, ch: char) {
    use std::fmt::Write;
    let mut buf = [0u8; 4];
    for byte in ch.encode_utf8(&mut buf).as_bytes() {
        write!(out, "%{byte:02X}").unwrap();
    }
}

/// Applies [`normalize_component`] to every component of a (pre-validated)
/// relative path.
pub fn normalize_relative_path(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in path.components() {
        if let Component::Normal(part) = c {
            out.push(normalize_component(part));
        }
    }
    out
}

/// Returns a collision-detection key for the given relative path.
///
/// On Windows the key is lowercased so that `FOO.txt` and `foo.txt` collide.
/// On Unix it is the identity function.
fn collision_key(path: &Path) -> PathBuf {
    #[cfg(windows)]
    {
        PathBuf::from(path.to_string_lossy().to_lowercase())
    }
    #[cfg(not(windows))]
    {
        path.to_path_buf()
    }
}

/// Tracks archive-internal output path collisions and resolves them
/// deterministically. Non-UTF-8 path components are normalized lossy
/// (via [`to_string_lossy`]) on Windows.
///
/// All paths passed to [`resolve_entry_path`] are reserved in the collision
/// namespace regardless of archive entry type (file, directory, symlink).
pub struct PathMapper {
    used_paths: HashSet<PathBuf>,
}

impl Default for PathMapper {
    fn default() -> Self {
        Self::new()
    }
}

impl PathMapper {
    pub fn new() -> Self {
        Self {
            used_paths: HashSet::new(),
        }
    }

    /// Validates, normalizes, and resolves an archive entry's relative path
    /// against `out_dir`. Archive-internal collisions (after host
    /// normalization) are resolved with ` (2)`, ` (3)`, etc. suffixes.
    ///
    /// Returns `(dest_path, was_renamed)`.
    pub fn resolve_entry_path(
        &mut self,
        clean_path: &Path,
        out_dir: &Path,
    ) -> Result<(PathBuf, bool)> {
        let validated = validate_relative_path(clean_path)?;
        let normalized = normalize_relative_path(&validated);
        let key = collision_key(&normalized);

        let (dest, renamed) = if self.used_paths.contains(&key) {
            let unique = uniquify_path(&normalized, out_dir, &self.used_paths);
            let unique_rel = unique.strip_prefix(out_dir).unwrap();
            self.used_paths.insert(collision_key(unique_rel));
            (unique, true)
        } else {
            self.used_paths.insert(key);
            (out_dir.join(&normalized), false)
        };

        Ok((dest, renamed))
    }
}

fn uniquify_path(normalized: &Path, out_dir: &Path, used: &HashSet<PathBuf>) -> PathBuf {
    let parent = normalized.parent().unwrap_or(Path::new("."));
    let stem = normalized
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("file");
    let ext = normalized.extension().and_then(|s| s.to_str());

    for n in 2u32.. {
        let candidate_rel: PathBuf = match ext {
            Some(e) => parent.join(format!("{} ({}).{}", stem, n, e)),
            None => parent.join(format!("{} ({})", stem, n)),
        };
        let key = collision_key(&candidate_rel);
        if !used.contains(&key) {
            return out_dir.join(&candidate_rel);
        }
    }
    unreachable!();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_relative_path_normal() {
        let result = validate_relative_path(Path::new("foo/bar/baz.txt"));
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), PathBuf::from("foo/bar/baz.txt"));
    }

    #[test]
    fn test_validate_relative_path_root_dir() {
        let result = validate_relative_path(Path::new("/etc/passwd"));
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_relative_path_parent_dir() {
        let result = validate_relative_path(Path::new("../escape"));
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_relative_path_empty() {
        let result = validate_relative_path(Path::new(""));
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_single_component_valid() {
        assert!(validate_single_component("com.example.app").is_ok());
        assert!(validate_single_component("BASE_SPLIT").is_ok());
        assert!(validate_single_component("config.armeabi_v7a").is_ok());
    }

    #[test]
    fn test_validate_single_component_empty() {
        assert!(validate_single_component("").is_err());
    }

    #[test]
    fn test_validate_single_component_parent_dir() {
        assert!(validate_single_component("..").is_err());
    }

    #[test]
    fn test_validate_single_component_absolute() {
        assert!(validate_single_component("/etc/passwd").is_err());
    }

    #[test]
    fn test_validate_single_component_with_separator() {
        assert!(validate_single_component("foo/bar").is_err());
    }

    #[test]
    fn test_validate_single_component_with_backslash() {
        assert!(validate_single_component("foo\\bar").is_err());
    }

    #[test]
    fn test_normalize_component_invalid_chars() {
        let result = normalize_component(OsStr::new("foo:bar"));
        #[cfg(windows)]
        assert_eq!(result, "foo%3Abar");
        #[cfg(not(windows))]
        assert_eq!(result, "foo:bar");
    }

    #[test]
    fn test_normalize_component_trailing_dot() {
        let result = normalize_component(OsStr::new("foo."));
        #[cfg(windows)]
        assert_eq!(result, "foo%2E");
        #[cfg(not(windows))]
        assert_eq!(result, "foo.");
    }

    #[test]
    fn test_normalize_component_trailing_space() {
        let result = normalize_component(OsStr::new("foo "));
        #[cfg(windows)]
        assert_eq!(result, "foo%20");
        #[cfg(not(windows))]
        assert_eq!(result, "foo ");
    }

    #[test]
    fn test_normalize_component_reserved_name() {
        let result = normalize_component(OsStr::new("CON.txt"));
        #[cfg(windows)]
        assert!(
            result.to_str().unwrap().contains('%'),
            "expected percent-encoded reserved name, got '{result:?}'"
        );
        #[cfg(not(windows))]
        assert_eq!(result, "CON.txt");
    }

    #[test]
    fn test_normalize_component_preserves_valid() {
        let result = normalize_component(OsStr::new("normal_file.txt"));
        assert_eq!(result, "normal_file.txt");
    }

    #[test]
    fn test_normalize_component_percent_escape() {
        // A literal "%3A" must not collide with ":" encoded as "%3A".
        let result = normalize_component(OsStr::new("foo%3Abar"));
        #[cfg(windows)]
        assert_eq!(result, "foo%253Abar");
        #[cfg(not(windows))]
        assert_eq!(result, "foo%3Abar");
    }

    #[test]
    fn test_normalize_component_preserves_non_ascii() {
        // Non-ASCII characters that aren't in the invalid set pass through.
        let result = normalize_component(OsStr::new("café"));
        assert_eq!(result, "café");
    }

    #[test]
    fn test_normalize_relative_path() {
        let result = normalize_relative_path(Path::new("foo:dir/normal.txt"));
        #[cfg(windows)]
        assert_eq!(result, PathBuf::from("foo%3Adir/normal.txt"));
        #[cfg(not(windows))]
        assert_eq!(result, PathBuf::from("foo:dir/normal.txt"));
    }

    #[test]
    fn test_path_mapper_collision() {
        let tmp = Path::new("/tmp");
        let mut m = PathMapper::new();
        let (p1, renamed1) = m.resolve_entry_path(Path::new("foo.txt"), tmp).unwrap();
        assert!(!renamed1);
        let (p2, renamed2) = m.resolve_entry_path(Path::new("foo.txt"), tmp).unwrap();
        assert!(renamed2);
        assert_ne!(p1, p2);
        assert!(p2.to_string_lossy().contains("(2)"));
    }

    #[test]
    fn test_path_mapper_different_dirs_no_collision() {
        let tmp = Path::new("/tmp");
        let mut m = PathMapper::new();
        let (p1, r1) = m
            .resolve_entry_path(Path::new("dir1/foo.txt"), tmp)
            .unwrap();
        let (p2, r2) = m
            .resolve_entry_path(Path::new("dir2/foo.txt"), tmp)
            .unwrap();
        assert!(!r1);
        assert!(!r2);
        assert_ne!(p1, p2);
    }

    #[test]
    fn test_path_mapper_triple_collision() {
        let tmp = Path::new("/tmp");
        let mut m = PathMapper::new();
        let (_, r1) = m.resolve_entry_path(Path::new("foo.txt"), tmp).unwrap();
        let (_, r2) = m.resolve_entry_path(Path::new("foo.txt"), tmp).unwrap();
        let (p3, r3) = m.resolve_entry_path(Path::new("foo.txt"), tmp).unwrap();
        assert!(!r1);
        assert!(r2);
        assert!(r3);
        assert!(p3.to_string_lossy().contains("(3)"));
    }
}
