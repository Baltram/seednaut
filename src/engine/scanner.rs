use crate::engine::crypto::{DerivedKeys, create_streaming_aead};
use crate::engine::decrypt::{RepoFileType, decrypt_and_decompress};
use crate::engine::types::{RawSnapshot, SnapshotInfo, pb};
use anyhow::Result;
use prost::Message;
use regex::Regex;
use std::path::Path;
use walkdir::WalkDir;

const APP_REPO_PATTERN: &str = r"^[a-f0-9]{64}$";
const FILE_REPO_PATTERN: &str = r"^[a-f0-9]{16}\.sv$";

/// Result of snapshot discovery, including any failures encountered.
pub struct DiscoveryResult {
    pub snapshots: Vec<SnapshotInfo>,
    pub failed: Vec<String>,
}

/// Scans the input path for app and file snapshots, decrypts them, and returns them sorted by time.
pub fn discover_snapshots(
    input_path: &Path,
    derived_keys: &DerivedKeys,
) -> Result<DiscoveryResult> {
    let app_repo_regex = Regex::new(APP_REPO_PATTERN).expect("Invalid app repo regex");
    let file_repo_regex = Regex::new(FILE_REPO_PATTERN).expect("Invalid file repo regex");

    let app_aead = create_streaming_aead(derived_keys.app_stream_key)?;
    let file_aead = create_streaming_aead(derived_keys.file_stream_key)?;

    let mut found_snapshots = Vec::new();
    let mut failures = Vec::new();

    for entry in WalkDir::new(input_path).into_iter().filter_map(|e| e.ok()) {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }

        let file_name = match path.file_name().and_then(|n| n.to_str()) {
            Some(name) => name,
            None => continue,
        };

        let Some(parent_dir) = path.parent() else {
            continue;
        };
        let Some(parent_dir_name) = parent_dir
            .file_name()
            .and_then(|n| n.to_str())
            .filter(|s| !s.is_empty())
        else {
            continue;
        };

        // Identify snapshot files by extension + parent directory naming convention.
        if file_name.ends_with(".snapshot") && app_repo_regex.is_match(parent_dir_name) {
            match decrypt_and_decompress(path, &*app_aead, RepoFileType::AppSnapshot) {
                Ok(decrypted_bytes) => {
                    match pb::seedvault::Snapshot::decode(decrypted_bytes.as_slice()) {
                        Ok(snapshot) => {
                            let info = SnapshotInfo {
                                index: 0,
                                // App snapshots use `token` as the backup timestamp.
                                timestamp: snapshot.token,
                                name: snapshot.name.clone(),
                                snapshot_path: path.to_path_buf(),
                                repo_path: parent_dir.to_path_buf(),
                                raw_snapshot: RawSnapshot::App(snapshot),
                            };
                            found_snapshots.push(info);
                        }
                        Err(e) => failures.push(format!("{}: {}", path.display(), e)),
                    }
                }
                Err(e) => failures.push(format!("{}: {}", path.display(), e)),
            }
        } else if file_name.ends_with(".SeedSnap")
            && file_repo_regex.is_match(parent_dir_name)
            && let Some(timestamp_str) = path.file_stem().and_then(|s| s.to_str())
            && let Ok(timestamp) = timestamp_str.parse::<u64>()
        {
            let file_type = RepoFileType::FileSnapshot { timestamp };
            match decrypt_and_decompress(path, &*file_aead, file_type) {
                Ok(decrypted_bytes) => {
                    match pb::calyxos::BackupSnapshot::decode(decrypted_bytes.as_slice()) {
                        Ok(snapshot) => {
                            let info = SnapshotInfo {
                                index: 0,
                                // File snapshots use `time_start` as the backup timestamp.
                                timestamp: snapshot.time_start as u64,
                                name: snapshot.name.clone(),
                                snapshot_path: path.to_path_buf(),
                                repo_path: parent_dir.to_path_buf(),
                                raw_snapshot: RawSnapshot::File(snapshot),
                            };
                            found_snapshots.push(info);
                        }
                        Err(e) => failures.push(format!("{}: {}", path.display(), e)),
                    }
                }
                Err(e) => failures.push(format!("{}: {}", path.display(), e)),
            }
        }
    }

    // Most recent first.
    found_snapshots.sort_by_key(|s| std::cmp::Reverse(s.timestamp));

    for (i, s) in found_snapshots.iter_mut().enumerate() {
        s.index = (i + 1) as u32;
    }

    Ok(DiscoveryResult {
        snapshots: found_snapshots,
        failed: failures,
    })
}
