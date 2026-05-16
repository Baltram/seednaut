use anyhow::{Context, Result};
use filetime::{FileTime, set_file_mtime};
use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};
use tar::{Archive, EntryType};

use super::path::PathMapper;

/// Extracts a tar archive into a directory.
///
/// Regular files and directories are extracted with their modification time
/// preserved. Symlinks with safe relative targets are extracted on Unix; they
/// are silently skipped on Windows. Hardlinks and special files are skipped
/// with a warning. Entry paths must stay relative: paths containing `..`,
/// absolute roots, or Windows prefixes are rejected and skipped with a warning.
///
/// If an entry starts with `apps/<package_name>/`, that prefix is stripped so
/// files are written directly under `out_dir`.
///
/// Existing destination files may be overwritten.
pub fn safe_extract_tar(reader: impl io::Read, out_dir: &Path, package_name: &str) -> Result<()> {
    fs::create_dir_all(out_dir).context("Failed to create output directory for tar extraction")?;

    let mut archive = Archive::new(reader);
    let strip_prefix = Path::new("apps").join(package_name);

    let mut dir_mtimes: Vec<(PathBuf, u64)> = Vec::new();
    let mut mapper = PathMapper::new();

    for entry_result in archive.entries()? {
        let mut entry = entry_result.context("Failed to read tar entry")?;
        let entry_path = entry
            .path()
            .context("Failed to get path for tar entry")?
            .into_owned();

        let clean_path = entry_path
            .strip_prefix(&strip_prefix)
            .unwrap_or(&entry_path);

        if clean_path.as_os_str().is_empty() || clean_path == Path::new(".") {
            continue;
        }

        let dest_path = match mapper.resolve_entry_path(clean_path, out_dir) {
            Ok((p, renamed)) => {
                if renamed {
                    eprintln!(
                        "Warning: Renamed archive entry for host compatibility: \
                         original='{}' -> extracted='{}'",
                        clean_path.display(),
                        p.strip_prefix(out_dir).unwrap_or(&p).display(),
                    );
                }
                p
            }
            Err(e) => {
                eprintln!(
                    "Warning: Skipping tar entry '{}': {:#}",
                    entry_path.display(),
                    e
                );
                continue;
            }
        };

        match entry.header().entry_type() {
            EntryType::Regular => {
                if let Ok(meta) = fs::symlink_metadata(&dest_path) {
                    if meta.is_dir() {
                        fs::remove_dir_all(&dest_path).with_context(|| {
                            format!(
                                "Failed to remove existing directory to make room for '{}'",
                                dest_path.display()
                            )
                        })?;
                    } else {
                        let _ = fs::remove_file(&dest_path);
                    }
                }
                if let Some(parent) = dest_path.parent() {
                    fs::create_dir_all(parent).with_context(|| {
                        format!(
                            "Failed to create parent directory for '{}'",
                            dest_path.display()
                        )
                    })?;
                }
                let mut out = File::create(&dest_path)
                    .with_context(|| format!("Failed to create file '{}'", dest_path.display()))?;
                io::copy(&mut entry, &mut out).with_context(|| {
                    format!("Failed to write tar entry to '{}'", dest_path.display())
                })?;
                drop(out);
                if let Ok(mtime) = entry.header().mtime()
                    && mtime > 0
                {
                    set_file_mtime(&dest_path, FileTime::from_unix_time(mtime as i64, 0))
                        .with_context(|| {
                            format!("Failed to set mtime on '{}'", dest_path.display())
                        })?;
                }
            }
            EntryType::Directory => {
                fs::create_dir_all(&dest_path).with_context(|| {
                    format!("Failed to create directory '{}'", dest_path.display())
                })?;
                if let Ok(mtime) = entry.header().mtime()
                    && mtime > 0
                {
                    dir_mtimes.push((dest_path, mtime));
                }
            }
            EntryType::Symlink => {
                let link_target = entry
                    .header()
                    .link_name()
                    .context("Failed to read symlink target")?
                    .and_then(|cow| {
                        if cow.as_os_str().is_empty() {
                            None
                        } else {
                            Some(cow.into_owned())
                        }
                    });
                let Some(link_target) = link_target else {
                    eprintln!(
                        "Warning: Skipping symlink '{}' with empty target",
                        entry_path.display()
                    );
                    continue;
                };
                match super::path::validate_relative_path(&link_target) {
                    Ok(_) => {
                        #[cfg(unix)]
                        {
                            let _ = fs::remove_file(&dest_path);
                            let _ = fs::remove_dir_all(&dest_path);
                            if let Some(parent) = dest_path.parent() {
                                fs::create_dir_all(parent).with_context(|| {
                                    format!(
                                        "Failed to create parent directory for '{}'",
                                        dest_path.display()
                                    )
                                })?;
                            }
                            std::os::unix::fs::symlink(&link_target, &dest_path).with_context(
                                || {
                                    format!(
                                        "Failed to create symlink '{}' -> '{}'",
                                        dest_path.display(),
                                        link_target.display()
                                    )
                                },
                            )?;
                        }
                    }
                    Err(e) => {
                        eprintln!(
                            "Warning: Skipping symlink '{}' with unsafe target '{}': {:#}",
                            entry_path.display(),
                            link_target.display(),
                            e
                        );
                    }
                }
            }
            _ => {
                eprintln!(
                    "Warning: Skipping unsupported tar entry type '{:?}' for '{}'",
                    entry.header().entry_type(),
                    entry_path.display()
                );
            }
        }
    }

    dir_mtimes.sort_by_key(|(p, _)| std::cmp::Reverse(p.components().count()));

    for (dir_path, mtime) in dir_mtimes {
        set_file_mtime(&dir_path, FileTime::from_unix_time(mtime as i64, 0))
            .with_context(|| format!("Failed to set mtime on '{}'", dir_path.display()))?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    // ── PathMapper::resolve_entry_path unit tests ──────────────────────────

    #[test]
    fn test_resolve_path_normal() {
        let out_dir = PathBuf::from("/tmp/out");
        let mut mapper = PathMapper::new();
        let (result, renamed) = mapper
            .resolve_entry_path(Path::new("foo.txt"), &out_dir)
            .unwrap();
        assert!(!renamed);
        assert_eq!(result, out_dir.join("foo.txt"));
    }

    #[test]
    fn test_resolve_path_traversal_rejected() {
        let out_dir = PathBuf::from("/tmp/out");
        let mut mapper = PathMapper::new();
        let result = mapper.resolve_entry_path(Path::new("../../etc/passwd"), &out_dir);
        assert!(result.is_err());
    }

    #[test]
    fn test_resolve_path_absolute_rejected() {
        let out_dir = PathBuf::from("/tmp/out");
        let mut mapper = PathMapper::new();
        let result = mapper.resolve_entry_path(Path::new("/etc/passwd"), &out_dir);
        assert!(result.is_err());
    }

    #[test]
    fn test_resolve_path_parent_dir_in_middle_rejected() {
        let out_dir = PathBuf::from("/tmp/out");
        let mut mapper = PathMapper::new();
        let result = mapper.resolve_entry_path(Path::new("good/../evil"), &out_dir);
        assert!(result.is_err());
    }

    #[test]
    fn test_resolve_path_empty_rejected() {
        let out_dir = PathBuf::from("/tmp/out");
        let mut mapper = PathMapper::new();
        let result = mapper.resolve_entry_path(Path::new(""), &out_dir);
        assert!(result.is_err());
    }

    // ── safe_extract_tar integration tests ────────────────────────────────

    /// Builds an in-memory tar archive with the given entries.
    ///
    /// Each entry is a tuple of `(path, contents, entry_type)` where
    /// `entry_type` is `"file"`, `"dir"`, or `"symlink"`. For symlink entries
    /// `contents` should be the link target path as a string.
    fn build_tar(entries: &[(&str, &[u8], &str)]) -> Vec<u8> {
        let mut buf = Vec::new();
        {
            let mut builder = tar::Builder::new(&mut buf);
            for &(path, contents, kind) in entries {
                let mut header = tar::Header::new_gnu();
                match kind {
                    "file" => {
                        header.set_size(contents.len() as u64);
                        header.set_mode(0o644);
                        header.set_entry_type(EntryType::Regular);
                        builder
                            .append_data(&mut header, Path::new(path), contents)
                            .unwrap();
                    }
                    "dir" => {
                        header.set_size(0);
                        header.set_mode(0o755);
                        header.set_entry_type(EntryType::Directory);
                        builder
                            .append_data(&mut header, Path::new(path), b"" as &[u8])
                            .unwrap();
                    }
                    "symlink" => {
                        let target = std::str::from_utf8(contents).unwrap();
                        header.set_size(0);
                        header.set_entry_type(EntryType::Symlink);
                        header.set_link_name(Path::new(target)).unwrap();
                        builder
                            .append_data(&mut header, Path::new(path), b"" as &[u8])
                            .unwrap();
                    }
                    _ => panic!("unknown entry kind"),
                }
            }
            builder.finish().unwrap();
        }
        buf
    }

    #[test]
    fn test_safe_extract_tar_normal_file() {
        let tmp = tempfile::tempdir().unwrap();
        let out_dir = tmp.path().join("out");
        let tar_data = build_tar(&[("apps/com.example/foo.txt", b"hello", "file")]);

        safe_extract_tar(tar_data.as_slice(), &out_dir, "com.example").unwrap();

        let extracted = out_dir.join("foo.txt");
        assert!(extracted.is_file());
        assert_eq!(fs::read_to_string(&extracted).unwrap(), "hello");
    }

    #[test]
    fn test_safe_extract_tar_nested_dirs() {
        let tmp = tempfile::tempdir().unwrap();
        let out_dir = tmp.path().join("out");
        let tar_data = build_tar(&[("apps/com.example/sub/dir/deep.txt", b"nested", "file")]);

        safe_extract_tar(tar_data.as_slice(), &out_dir, "com.example").unwrap();

        let extracted = out_dir.join("sub/dir/deep.txt");
        assert!(extracted.is_file());
        assert_eq!(fs::read_to_string(&extracted).unwrap(), "nested");
    }

    #[test]
    #[cfg(unix)]
    fn test_safe_extract_tar_harmless_symlink_extracted() {
        let tmp = tempfile::tempdir().unwrap();
        let out_dir = tmp.path().join("out");
        let tar_data = build_tar(&[
            ("apps/com.example/real.txt", b"real", "file"),
            ("apps/com.example/link.txt", b"real.txt", "symlink"),
        ]);

        safe_extract_tar(tar_data.as_slice(), &out_dir, "com.example").unwrap();

        assert!(out_dir.join("real.txt").is_file());
        let link = out_dir.join("link.txt");
        assert!(link.is_symlink());
        assert_eq!(fs::read_link(&link).unwrap(), PathBuf::from("real.txt"));
    }

    #[test]
    fn test_safe_extract_tar_absolute_target_symlink_skipped() {
        let tmp = tempfile::tempdir().unwrap();
        let out_dir = tmp.path().join("out");
        let tar_data = build_tar(&[("apps/com.example/badlink.txt", b"/etc/passwd", "symlink")]);

        safe_extract_tar(tar_data.as_slice(), &out_dir, "com.example").unwrap();

        assert!(!out_dir.join("badlink.txt").exists());
    }

    #[test]
    fn test_safe_extract_tar_parent_dir_target_symlink_skipped() {
        let tmp = tempfile::tempdir().unwrap();
        let out_dir = tmp.path().join("out");
        let tar_data = build_tar(&[(
            "apps/com.example/badlink.txt",
            b"../../etc/passwd",
            "symlink",
        )]);

        safe_extract_tar(tar_data.as_slice(), &out_dir, "com.example").unwrap();

        assert!(!out_dir.join("badlink.txt").exists());
    }

    #[test]
    fn test_safe_extract_tar_directory_created() {
        let tmp = tempfile::tempdir().unwrap();
        let out_dir = tmp.path().join("out");
        let tar_data = build_tar(&[
            ("apps/com.example/subdir/", b"", "dir"),
            ("apps/com.example/subdir/file.txt", b"inside", "file"),
        ]);

        safe_extract_tar(tar_data.as_slice(), &out_dir, "com.example").unwrap();

        assert!(out_dir.join("subdir").is_dir());
        assert!(out_dir.join("subdir/file.txt").is_file());
    }

    #[test]
    fn test_safe_extract_tar_file_overwrites_existing_directory() {
        let tmp = tempfile::tempdir().unwrap();
        let out_dir = tmp.path().join("out");

        // First pass: extract a directory at "foo"
        let tar_data1 = build_tar(&[
            ("apps/com.example/foo/", b"", "dir"),
            ("apps/com.example/foo/inside.txt", b"first", "file"),
        ]);
        safe_extract_tar(tar_data1.as_slice(), &out_dir, "com.example").unwrap();
        assert!(out_dir.join("foo").is_dir());
        assert_eq!(
            fs::read_to_string(out_dir.join("foo/inside.txt")).unwrap(),
            "first"
        );

        // Second pass: extract a regular file named "foo" — should remove the dir
        let tar_data2 = build_tar(&[("apps/com.example/foo", b"replaced", "file")]);
        safe_extract_tar(tar_data2.as_slice(), &out_dir, "com.example").unwrap();
        assert!(out_dir.join("foo").is_file());
        assert_eq!(fs::read_to_string(out_dir.join("foo")).unwrap(), "replaced");
    }

    #[test]
    fn test_safe_extract_tar_preserves_mtime() {
        let tmp = tempfile::tempdir().unwrap();
        let out_dir = tmp.path().join("out");

        let mut buf = Vec::new();
        {
            let mut builder = tar::Builder::new(&mut buf);

            let mut file_hdr = tar::Header::new_gnu();
            file_hdr.set_size(5);
            file_hdr.set_entry_type(EntryType::Regular);
            file_hdr.set_mtime(1_620_000_000);
            builder
                .append_data(
                    &mut file_hdr,
                    Path::new("apps/com.example/hello.txt"),
                    b"hello" as &[u8],
                )
                .unwrap();

            let mut dir_hdr = tar::Header::new_gnu();
            dir_hdr.set_size(0);
            dir_hdr.set_entry_type(EntryType::Directory);
            dir_hdr.set_mtime(1_620_000_001);
            builder
                .append_data(
                    &mut dir_hdr,
                    Path::new("apps/com.example/mydir/"),
                    b"" as &[u8],
                )
                .unwrap();

            builder.finish().unwrap();
        }

        safe_extract_tar(buf.as_slice(), &out_dir, "com.example").unwrap();

        let file_meta = fs::metadata(out_dir.join("hello.txt")).unwrap();
        assert_eq!(
            FileTime::from_last_modification_time(&file_meta).unix_seconds(),
            1_620_000_000
        );

        let dir_meta = fs::metadata(out_dir.join("mydir")).unwrap();
        assert_eq!(
            FileTime::from_last_modification_time(&dir_meta).unix_seconds(),
            1_620_000_001
        );
    }
}
