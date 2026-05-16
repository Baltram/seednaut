use crate::AppContext;
use crate::cli::SnapshotSelector;
use crate::engine::types::{RawSnapshot, SnapshotInfo};
use anyhow::Result;
use std::collections::BTreeMap;
use std::path::{self, PathBuf};

/// Lists the given snapshots, showing their timestamp, type, name, and location.
pub fn list_snapshots(ctx: &AppContext, selector: &SnapshotSelector) -> Result<()> {
    if ctx.snapshots.is_empty() {
        println!("No snapshots found.");
        return Ok(());
    }

    let snapshots_to_show: Vec<&SnapshotInfo> = if selector.snapshots.is_empty() {
        ctx.snapshots.iter().collect()
    } else {
        ctx.snapshots
            .iter()
            .filter(|s| selector.snapshots.contains(&s.index))
            .collect()
    };

    if snapshots_to_show.is_empty() {
        if !selector.snapshots.is_empty() {
            println!("No snapshots found matching the given indices.");
        }
        return Ok(());
    }

    for s in &snapshots_to_show {
        let ts_str = crate::util::date::format_display(s.timestamp);
        let type_str = match s.raw_snapshot {
            RawSnapshot::App(_) => "App Backup",
            RawSnapshot::File(_) => "File Backup",
        };
        println!("{}) {} - {}: \"{}\"", s.index, ts_str, type_str, s.name);
    }
    println!();

    // Print a tree view of snapshot file locations.

    if !ctx.input_path.is_dir() {
        for s in &snapshots_to_show {
            println!("{} -> {}", s.snapshot_path.display(), s.index);
        }
        return Ok(());
    }

    let tree_root = &ctx.input_path;
    let root_display = tree_root.display().to_string();
    let root_display = root_display.trim_end_matches(path::MAIN_SEPARATOR);
    println!("{}{}", root_display, path::MAIN_SEPARATOR);

    // Group snapshots by repository directory for tree rendering.
    let mut groups: BTreeMap<PathBuf, Vec<(String, u32)>> = BTreeMap::new();
    for s in &snapshots_to_show {
        // The `repo_path` is the directory like ".../a1b2...c3d4" or ".../a1b2...c3d4.sv"
        let repo_path = s.snapshot_path.parent().unwrap_or(&s.snapshot_path);
        let repo_dir = repo_path
            .strip_prefix(tree_root)
            .unwrap_or(repo_path)
            .to_path_buf();
        let snapshot_name = s
            .snapshot_path
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "unknown.snapshot".to_string());

        groups
            .entry(repo_dir)
            .or_default()
            .push((snapshot_name, s.index));
    }

    let mut parent_iter = groups.iter().peekable();
    while let Some((parent, files)) = parent_iter.next() {
        let is_last_parent = parent_iter.peek().is_none();

        // Skip displaying empty parent paths (avoids redundant "/" nodes)
        if parent.as_os_str().is_empty() {
            // Files are at the tree root, display them directly
            let mut file_iter = files.iter().peekable();
            while let Some((name, index)) = file_iter.next() {
                let is_last_file = file_iter.peek().is_none() && is_last_parent;
                let file_prefix = if is_last_file {
                    "└── "
                } else {
                    "├── "
                };
                println!("{}{} -> {}", file_prefix, name, index);
            }
        } else {
            // Display the parent directory and its children
            let parent_prefix = if is_last_parent {
                "└── "
            } else {
                "├── "
            };
            let child_indent = if is_last_parent { "    " } else { "│   " };

            println!(
                "{}{}{}",
                parent_prefix,
                parent.display(),
                path::MAIN_SEPARATOR
            );

            let mut file_iter = files.iter().peekable();
            while let Some((name, index)) = file_iter.next() {
                let is_last_file = file_iter.peek().is_none();
                let file_prefix = if is_last_file {
                    "└── "
                } else {
                    "├── "
                };
                println!("{}{}{} -> {}", child_indent, file_prefix, name, index);
            }
        }
    }

    Ok(())
}
