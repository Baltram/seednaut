use crate::AppContext;
use crate::cli::SnapshotSelector;
use crate::engine::fetcher::{AppChunkFetcher, ChunkFetcher, FileChunkFetcher};
use crate::engine::types::pb::{calyxos as pb_calyxos, seedvault as pb_seedvault};
use crate::engine::types::{AppChunkId, FileChunkId, RawSnapshot, SnapshotInfo};
use crate::ui::style;
use crate::util::path as safe_path;

use anyhow::{Context, Result};
use indicatif::{MultiProgress, ProgressBar};
use rayon::prelude::*;
use std::path::Path;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Verifies all chunks in the given snapshots are readable and cryptographically valid.
/// Uses a Rayon thread pool for parallelism across files/packages.
///
/// Note: verification checks integrity of individual chunks; it does not validate
/// semantic correctness of the snapshot data.
pub fn verify_snapshots(ctx: &AppContext, selector: &SnapshotSelector) -> Result<()> {
    let snapshots_to_process = ctx
        .snapshots
        .iter()
        .filter(|s| selector.snapshots.is_empty() || selector.snapshots.contains(&s.index))
        .collect::<Vec<_>>();

    if snapshots_to_process.is_empty() {
        println!("No snapshots selected or found for verification.");
        return Ok(());
    }

    println!("Verifying {} snapshot(s)...", snapshots_to_process.len());
    let multi_progress = MultiProgress::new();
    let mut total_errors = 0;

    for s_info in snapshots_to_process {
        let pb = multi_progress.add(ProgressBar::new(1));
        pb.set_style(style::bar_cyan_elapsed());
        pb.set_message(format!("Snapshot {} ({})", s_info.index, s_info.name));

        let errors = match &s_info.raw_snapshot {
            RawSnapshot::App(snapshot) => {
                verify_app_snapshot(ctx, s_info, snapshot, &multi_progress)?
            }
            RawSnapshot::File(snapshot) => {
                verify_file_snapshot(ctx, s_info, snapshot, &multi_progress)?
            }
        };

        if errors > 0 {
            total_errors += errors;
            pb.finish_with_message(format!(
                "Snapshot {} ({}) - FAILED with {} errors",
                s_info.index, s_info.name, errors
            ));
        } else {
            pb.finish_with_message(format!("Snapshot {} ({}) - OK", s_info.index, s_info.name));
        }
    }

    if total_errors > 0 {
        println!(
            "\nVerification finished with {} total errors.",
            total_errors
        );
        anyhow::bail!("Verification failed with {} errors", total_errors);
    } else {
        println!("\nVerification successful. All chunks are valid.");
    }

    Ok(())
}

fn collect_app_chunk_ids(
    app_meta: &pb_seedvault::snapshot::App,
) -> anyhow::Result<Vec<AppChunkId>> {
    let mut all_chunks = Vec::new();

    for bytes in &app_meta.chunk_ids {
        all_chunks
            .push(AppChunkId::try_from(bytes.as_slice()).context("Invalid app data chunk ID")?);
    }

    if let Some(apk) = &app_meta.apk {
        for split in &apk.splits {
            for bytes in &split.chunk_ids {
                all_chunks.push(AppChunkId::try_from(bytes.as_slice()).with_context(|| {
                    format!("Invalid APK split chunk ID in split '{}'", split.name)
                })?);
            }
        }
    }

    Ok(all_chunks)
}

fn verify_app_snapshot(
    ctx: &AppContext,
    s_info: &SnapshotInfo,
    snapshot: &pb_seedvault::Snapshot,
    mp: &MultiProgress,
) -> Result<usize> {
    let blobs = Arc::new(snapshot.blobs.clone());
    let repo_path = s_info.repo_path.clone();
    let app_key = ctx.derived_keys.app_stream_key;

    let packages: Vec<_> = snapshot.apps.iter().collect();
    let error_count = AtomicUsize::new(0);
    let pb = mp.add(ProgressBar::new(packages.len() as u64));
    pb.set_style(style::bar_yellow());
    pb.set_prefix(format!("Snapshot {}", s_info.index));

    let icon_fetcher = AppChunkFetcher::new(&repo_path, app_key, blobs.clone())
        .context("Failed to initialize app chunk fetcher for icon verification")?;

    for bytes in &snapshot.icon_chunk_ids {
        let chunk_id = match AppChunkId::try_from(bytes.as_slice()) {
            Ok(id) => id,
            Err(e) => {
                error_count.fetch_add(1, Ordering::SeqCst);
                pb.println(format!("  - ERROR: Invalid icon chunk ID: {e}"));
                continue;
            }
        };

        if let Err(e) = icon_fetcher.fetch_chunk(chunk_id) {
            error_count.fetch_add(1, Ordering::SeqCst);
            pb.println(format!(
                "  - ERROR: Verification failed for snapshot icon chunk {}: {e:#}",
                chunk_id
            ));
        }
    }

    packages.par_iter().for_each_init(
        || AppChunkFetcher::new(&repo_path, app_key, blobs.clone()).map_err(|e| format!("{e:#}")),
        |fetcher_result, (pkg_name, app_meta)| {
            pb.set_message(pkg_name.to_string());

            let fetcher = match fetcher_result.as_ref() {
                Ok(fetcher) => fetcher,
                Err(msg) => {
                    error_count.fetch_add(1, Ordering::SeqCst);
                    pb.println(format!(
                        "  - ERROR: Failed to initialize app chunk fetcher for {}: {}",
                        pkg_name, msg
                    ));
                    pb.inc(1);
                    return;
                }
            };

            let all_chunks = match collect_app_chunk_ids(app_meta) {
                Ok(ids) => ids,
                Err(e) => {
                    error_count.fetch_add(1, Ordering::SeqCst);
                    pb.println(format!(
                        "  - ERROR: Verification failed for {}: {:#}",
                        pkg_name, e
                    ));
                    pb.inc(1);
                    return;
                }
            };

            for chunk_id in all_chunks {
                if fetcher.fetch_chunk(chunk_id).is_err() {
                    error_count.fetch_add(1, Ordering::SeqCst);
                    pb.println(format!("  - ERROR: Verification failed for {}", pkg_name));
                    pb.inc(1);
                    return;
                }
            }

            pb.inc(1);
        },
    );

    pb.finish_and_clear();
    Ok(error_count.load(Ordering::SeqCst))
}

fn format_snapshot_file_path(path: &str, name: &str) -> String {
    if path.is_empty() {
        name.to_string()
    } else {
        let parent = safe_path::validate_relative_path(Path::new(path))
            .unwrap_or_else(|_| PathBuf::from(path));
        parent.join(name).to_string_lossy().into_owned()
    }
}

fn verify_file_snapshot(
    ctx: &AppContext,
    s_info: &SnapshotInfo,
    snapshot: &pb_calyxos::BackupSnapshot,
    mp: &MultiProgress,
) -> Result<usize> {
    let repo_path = s_info.repo_path.clone();
    let file_key = ctx.derived_keys.file_stream_key;
    let chunk_id_key = ctx.derived_keys.chunk_id_key;

    let files: Vec<_> = snapshot
        .media_files
        .iter()
        .map(|f| (format_snapshot_file_path(&f.path, &f.name), &f.chunk_ids))
        .chain(
            snapshot
                .document_files
                .iter()
                .map(|f| (format_snapshot_file_path(&f.path, &f.name), &f.chunk_ids)),
        )
        .collect();

    let error_count = AtomicUsize::new(0);
    let pb = mp.add(ProgressBar::new(files.len() as u64));
    pb.set_style(style::bar_yellow());
    pb.set_prefix(format!("Snapshot {}", s_info.index));

    files.par_iter().for_each_init(
        || FileChunkFetcher::new(&repo_path, file_key, chunk_id_key).map_err(|e| format!("{e:#}")),
        |fetcher_result, (full_path, chunk_ids_hex)| {
            pb.set_message(full_path.clone());
            let mut has_error = false;

            let fetcher = match fetcher_result.as_ref() {
                Ok(fetcher) => fetcher,
                Err(msg) => {
                    error_count.fetch_add(1, Ordering::SeqCst);
                    pb.println(format!(
                        "  - ERROR: Failed to initialize file chunk fetcher for {}: {}",
                        full_path, msg
                    ));
                    pb.inc(1);
                    return;
                }
            };

            for chunk_hex in chunk_ids_hex.iter() {
                let chunk_id = match FileChunkId::from_str(chunk_hex) {
                    Ok(id) => id,
                    Err(_) => {
                        has_error = true;
                        break;
                    }
                };
                if fetcher.fetch_chunk(chunk_id).is_err() {
                    has_error = true;
                    break;
                }
            }

            if has_error {
                error_count.fetch_add(1, Ordering::SeqCst);
                pb.println(format!("  - ERROR: Verification failed for {}", full_path));
            }

            pb.inc(1);
        },
    );

    pb.finish_and_clear();
    Ok(error_count.load(Ordering::SeqCst))
}
