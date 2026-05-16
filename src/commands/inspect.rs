use crate::AppContext;
use crate::cli::SnapshotSelector;
use crate::engine::types::{
    RawSnapshot, SnapshotInfo, pb::calyxos::BackupSnapshot as FileSnapshot,
    pb::seedvault::Snapshot as AppSnapshot, pb::seedvault::snapshot::BackupType as AppBackupType,
};
use crate::util::path as safe_path;
use anyhow::Result;
use std::path::{Path, PathBuf};

/// Helper to print details for an App backup snapshot.
fn inspect_app_snapshot(app_snapshot: &AppSnapshot) {
    if app_snapshot.apps.is_empty() {
        println!("  This snapshot contains no app backups.");
        return;
    }

    // Deterministic output regardless of HashMap iteration order.
    let mut packages: Vec<_> = app_snapshot.apps.iter().collect();
    packages.sort_by_key(|(name, _)| name.to_lowercase());

    for (pkg_name, app_info) in packages {
        let type_str = match AppBackupType::try_from(app_info.r#type) {
            Ok(AppBackupType::Full) => "(FULL)",
            Ok(AppBackupType::Kv) => "(K/V)",
            _ => "(Unknown)",
        };
        let apk_str = if app_info.apk.is_some() { "[APK]" } else { "" };
        println!("  - {} {} {}", pkg_name, type_str, apk_str);
    }
}

fn join_normalized(parent: &str, name: &str) -> String {
    let parent_path = if parent.is_empty() {
        PathBuf::new()
    } else {
        safe_path::validate_relative_path(Path::new(parent))
            .unwrap_or_else(|_| PathBuf::from(parent))
    };
    parent_path.join(name).to_string_lossy().into_owned()
}

/// Helper to print details for a File backup snapshot.
fn inspect_file_snapshot(file_snapshot: &FileSnapshot) {
    let mut all_files: Vec<String> = Vec::new();

    for file in &file_snapshot.media_files {
        all_files.push(join_normalized(&file.path, &file.name));
    }

    for file in &file_snapshot.document_files {
        all_files.push(join_normalized(&file.path, &file.name));
    }

    if all_files.is_empty() {
        println!("  This snapshot contains no file backups.");
        return;
    }

    // Deterministic output.
    all_files.sort_by_key(|a| a.to_lowercase());

    for path_str in all_files {
        println!("  - {}", path_str);
    }
}

/// Inspects the given snapshots, listing their contained apps or files.
pub fn inspect_snapshots(ctx: &AppContext, selector: &SnapshotSelector) -> Result<()> {
    if ctx.snapshots.is_empty() {
        println!("No snapshots found to inspect.");
        return Ok(());
    }

    let snapshots_to_inspect: Vec<&SnapshotInfo> = if selector.snapshots.is_empty() {
        ctx.snapshots.iter().collect()
    } else {
        ctx.snapshots
            .iter()
            .filter(|s| selector.snapshots.contains(&s.index))
            .collect()
    };

    if snapshots_to_inspect.is_empty() {
        if !selector.snapshots.is_empty() {
            println!("No snapshots found matching the given indices.");
        }
        return Ok(());
    }

    for (i, s) in snapshots_to_inspect.iter().enumerate() {
        if i > 0 {
            println!();
        }
        println!(
            "--- Snapshot {} Contents: \"{}\" ({}) ---",
            s.index,
            s.name,
            crate::util::date::format_display(s.timestamp)
        );

        match &s.raw_snapshot {
            RawSnapshot::App(app_snapshot) => inspect_app_snapshot(app_snapshot),
            RawSnapshot::File(file_snapshot) => inspect_file_snapshot(file_snapshot),
        }
    }

    Ok(())
}
