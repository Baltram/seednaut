use crate::AppContext;
use crate::cli::ExtractArgs;
use crate::engine::fetcher::{AppChunkFetcher, ChunkFetcher, FileChunkFetcher};
use crate::engine::reassembler;
use crate::engine::types::pb::{calyxos as pb_calyxos, seedvault as pb_seedvault};
use crate::engine::types::{AppChunkId, FileChunkId, RawSnapshot, SnapshotInfo, SnapshotType};
use crate::ui::style;
use crate::util::{path as safe_path, sqlite, tar};

use anyhow::{Context, Result, bail};
use indicatif::{MultiProgress, ProgressBar};
use rayon::prelude::*;
use regex::Regex;
use std::collections::HashMap;
use std::ffi::OsStr;
use std::fs::{self, File};
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use zip::ZipArchive;

/// Converts a millisecond timestamp to FileTime for setting file modification times.
fn ms_to_filetime(ms: i64) -> filetime::FileTime {
    filetime::FileTime::from_unix_time(ms / 1000, (ms % 1000 * 1_000_000) as u32)
}

/// Extracts data from selected snapshots into the output directory,
/// optionally filtering by regex pattern and enabling export mode.
pub fn extract_snapshots(ctx: &AppContext, args: &ExtractArgs) -> Result<()> {
    let snapshots_to_process = ctx
        .snapshots
        .iter()
        .filter(|s| {
            args.selector.snapshots.is_empty() || args.selector.snapshots.contains(&s.index)
        })
        .collect::<Vec<_>>();

    if snapshots_to_process.is_empty() {
        println!("No snapshots selected or found for extraction.");
        return Ok(());
    }

    if args.out_dir.exists() && !args.out_dir.is_dir() {
        bail!(
            "Output path exists but is not a directory: {}",
            args.out_dir.display()
        );
    }

    fs::create_dir_all(&args.out_dir).with_context(|| {
        format!(
            "Failed to create output directory '{}'",
            args.out_dir.display()
        )
    })?;

    println!(
        "Extracting from {} snapshot(s) into {}...",
        snapshots_to_process.len(),
        args.out_dir.display()
    );

    let multi_progress = MultiProgress::new();

    // Compile regex once before processing any snapshots.
    let match_pattern = args
        .pattern_str
        .as_ref()
        .map(|str| regex::Regex::new(str.trim()).context("Invalid regex pattern"))
        .transpose()?;

    let mut all_errors: Vec<String> = Vec::new();

    for s_info in snapshots_to_process {
        let time_str = crate::util::date::format_filename(s_info.timestamp);
        let type_str = match s_info.snapshot_type() {
            SnapshotType::App => "apps",
            SnapshotType::File => "files",
        };
        let dir_name = format!("{}_{}_{}", s_info.index, type_str, time_str);
        let snapshot_out_dir = args.out_dir.join(&dir_name);

        let pb_main = multi_progress.add(ProgressBar::new(1));
        pb_main.set_style(style::snapshot_spinner());
        pb_main.set_message(format!("{} ({})", s_info.index, s_info.name));

        let label = if type_str == "apps" {
            "package"
        } else {
            "file"
        };

        let (succeeded, snap_errors) = match &s_info.raw_snapshot {
            RawSnapshot::App(snapshot) => match extract_app_snapshot(
                ctx,
                s_info,
                snapshot,
                &snapshot_out_dir,
                match_pattern.as_ref(),
                args.export,
                &multi_progress,
            ) {
                Ok(result) => result,
                Err(e) => {
                    pb_main.finish_with_message(format!(
                        "{} ({}) - FAILED (setup error)",
                        s_info.index, s_info.name,
                    ));
                    all_errors.push(format!(
                        "Snapshot {} ({}): {:#}",
                        s_info.index, s_info.name, e,
                    ));
                    continue;
                }
            },
            RawSnapshot::File(snapshot) => match extract_file_snapshot(
                ctx,
                s_info,
                snapshot,
                &snapshot_out_dir,
                match_pattern.as_ref(),
                &multi_progress,
            ) {
                Ok(result) => result,
                Err(e) => {
                    pb_main.finish_with_message(format!(
                        "{} ({}) - FAILED (setup error)",
                        s_info.index, s_info.name,
                    ));
                    all_errors.push(format!(
                        "Snapshot {} ({}): {:#}",
                        s_info.index, s_info.name, e,
                    ));
                    continue;
                }
            },
        };

        let failed_count = snap_errors.len();
        all_errors.extend(snap_errors);

        if succeeded == 0 && failed_count == 0 && match_pattern.is_some() {
            pb_main.finish_with_message(format!("{} ({}) - no matches", s_info.index, s_info.name));
        } else if failed_count > 0 {
            let s_plural = if succeeded == 1 { "" } else { "s" };
            pb_main.finish_with_message(format!(
                "{} ({}) - {} {}{} extracted, {} failed to {}",
                s_info.index, s_info.name, succeeded, label, s_plural, failed_count, dir_name,
            ));
        } else {
            pb_main.finish_with_message(format!(
                "{} ({}) - {} {}{} extracted to {}",
                s_info.index,
                s_info.name,
                succeeded,
                label,
                if succeeded == 1 { "" } else { "s" },
                dir_name,
            ));
        }
    }

    if !all_errors.is_empty() {
        eprintln!(
            "\n{} error(s) occurred during extraction:",
            all_errors.len()
        );
        for err in &all_errors {
            eprintln!("  - {}", err);
        }
        anyhow::bail!("Extraction completed with {} error(s)", all_errors.len());
    }

    println!("\nExtraction complete.");
    Ok(())
}

fn extract_app_snapshot(
    ctx: &AppContext,
    s_info: &SnapshotInfo,
    snapshot: &pb_seedvault::Snapshot,
    snapshot_out_dir: &Path,
    match_pattern: Option<&Regex>,
    export_mode: bool,
    mp: &MultiProgress,
) -> Result<(usize, Vec<String>)> {
    let packages: Vec<_> = snapshot
        .apps
        .iter()
        .filter(|(pkg_name, _)| match_pattern.is_none_or(|p| p.is_match(pkg_name)))
        .collect();

    if packages.is_empty() && match_pattern.is_some() {
        return Ok((0, Vec::new()));
    }

    fs::create_dir_all(snapshot_out_dir)?;
    let json_path = snapshot_out_dir.join("_seedvault_snapshot.json");
    let snapshot_json = crate::util::proto::serialize_app_snapshot(snapshot)?;
    fs::write(&json_path, snapshot_json)
        .with_context(|| format!("Failed to write snapshot metadata to {:?}", json_path))?;

    let blobs = Arc::new(snapshot.blobs.clone());
    let repo_path = s_info.repo_path.clone();
    let app_key = ctx.derived_keys.app_stream_key;

    let pb = mp.add(ProgressBar::new(packages.len() as u64));
    pb.set_style(style::bar_cyan());
    pb.set_prefix(format!("Snapshot {}", s_info.index));

    let errors = Mutex::new(Vec::new());
    let succeeded = AtomicUsize::new(0);

    packages.par_iter().for_each_init(
        || AppChunkFetcher::new(&repo_path, app_key, blobs.clone()).map_err(|e| format!("{e:#}")),
        |fetcher_result, (pkg_name, app_meta)| {
            pb.set_message(format!("{}...", pkg_name));

            let fetcher = match fetcher_result.as_ref() {
                Ok(f) => f,
                Err(msg) => {
                    errors
                        .lock()
                        .unwrap()
                        .push(format!("{}: {}", pkg_name, msg));
                    pb.inc(1);
                    return;
                }
            };

            let result = (|| -> Result<()> {
                safe_path::validate_single_component(pkg_name)?;
                let pkg_name_norm = safe_path::normalize_component(OsStr::new(pkg_name));
                let pkg_dir = snapshot_out_dir.join(&pkg_name_norm);
                fs::create_dir_all(&pkg_dir)?;

                if !app_meta.chunk_ids.is_empty() {
                    let chunk_ids: Vec<AppChunkId> = app_meta
                        .chunk_ids
                        .iter()
                        .map(|bytes| AppChunkId::try_from(bytes.as_slice()))
                        .collect::<Result<_, _>>()?;

                    match pb_seedvault::snapshot::BackupType::try_from(app_meta.r#type) {
                        Ok(pb_seedvault::snapshot::BackupType::Kv) => {
                            if export_mode {
                                let kv_dir = pkg_dir.join("kv");
                                fs::create_dir_all(&kv_dir)?;
                                let out_json = kv_dir.join("export.json");

                                let mut buffer = Vec::new();
                                reassembler::reassemble_data(chunk_ids, fetcher, &mut buffer)?;
                                sqlite::export_sqlite_to_json(&buffer, &out_json)?;
                            } else {
                                let db_path =
                                    pkg_dir.join(format!("{}.db", pkg_name_norm.to_string_lossy()));
                                let mut file = File::create(&db_path)?;
                                reassembler::reassemble_data(chunk_ids, fetcher, &mut file)?;
                            }
                        }
                        Ok(pb_seedvault::snapshot::BackupType::Full) => {
                            if export_mode {
                                let data_dir = pkg_dir.join("full").join("data");
                                let reader =
                                    reassembler::ReassemblingReader::new(fetcher, chunk_ids);
                                tar::safe_extract_tar(reader, &data_dir, pkg_name)?;
                            } else {
                                let tar_path = pkg_dir.join("data.tar");
                                let mut file = File::create(&tar_path)?;
                                reassembler::reassemble_data(chunk_ids, fetcher, &mut file)?;
                            }
                        }
                        _ => bail!("Unknown backup type for package {}", pkg_name),
                    }
                }

                if let Some(apk) = &app_meta.apk {
                    let apk_out_dir = pkg_dir.join("apk");
                    fs::create_dir_all(&apk_out_dir)?;
                    reassembler::reassemble_apk(apk, fetcher, &apk_out_dir)?;
                }

                Ok(())
            })();

            match result {
                Ok(()) => {
                    succeeded.fetch_add(1, Ordering::SeqCst);
                }
                Err(e) => {
                    errors
                        .lock()
                        .unwrap()
                        .push(format!("{}: {:#}", pkg_name, e));
                }
            }
            pb.inc(1);
        },
    );

    pb.finish_and_clear();
    let errors = errors.into_inner().unwrap();
    Ok((succeeded.load(Ordering::SeqCst), errors))
}

struct FileToExtract<'a> {
    relative_path: PathBuf,
    dest_path: PathBuf,
    chunk_ids: &'a [String],
    zip_index: i32,
    last_modified: i64,
}

fn build_file_to_extract<'a>(
    kind: &str,
    name: &str,
    path: &str,
    chunk_ids: &'a [String],
    zip_index: i32,
    last_modified: i64,
) -> Result<FileToExtract<'a>> {
    safe_path::validate_single_component(name)
        .with_context(|| format!("Unsafe {kind} file name '{name}'"))?;

    let parent = if path.is_empty() || Path::new(path) == Path::new(".") {
        PathBuf::new()
    } else {
        safe_path::validate_relative_path(Path::new(path))
            .with_context(|| format!("Unsafe {kind} file parent path '{path}'"))?
    };

    let relative_path = safe_path::normalize_relative_path(&parent.join(name));

    if zip_index < 0 {
        bail!(
            "{} file '{}' has invalid negative zip_index {}",
            kind,
            relative_path.display(),
            zip_index
        );
    }

    if zip_index > 0 && chunk_ids.len() != 1 {
        bail!(
            "{} file '{}' has zip_index {} but {} chunk IDs; zip-backed files must have exactly 1 chunk ID",
            kind,
            relative_path.display(),
            zip_index,
            chunk_ids.len()
        );
    }

    Ok(FileToExtract {
        relative_path,
        dest_path: PathBuf::new(),
        chunk_ids,
        zip_index,
        last_modified,
    })
}

fn extract_file_snapshot(
    ctx: &AppContext,
    s_info: &SnapshotInfo,
    snapshot: &pb_calyxos::BackupSnapshot,
    snapshot_out_dir: &Path,
    match_pattern: Option<&Regex>,
    mp: &MultiProgress,
) -> Result<(usize, Vec<String>)> {
    let repo_path = s_info.repo_path.clone();
    let file_key = ctx.derived_keys.file_stream_key;
    let chunk_id_key = ctx.derived_keys.chunk_id_key;

    let media_files: Result<Vec<FileToExtract>> = snapshot
        .media_files
        .iter()
        .map(|f| {
            build_file_to_extract(
                "media",
                &f.name,
                &f.path,
                &f.chunk_ids,
                f.zip_index,
                f.last_modified,
            )
        })
        .collect();

    let doc_files: Result<Vec<FileToExtract>> = snapshot
        .document_files
        .iter()
        .map(|f| {
            build_file_to_extract(
                "document",
                &f.name,
                &f.path,
                &f.chunk_ids,
                f.zip_index,
                f.last_modified,
            )
        })
        .collect();

    let mut all_files = media_files?;
    all_files.extend(doc_files?);

    let all_files: Vec<FileToExtract> = all_files
        .into_iter()
        .filter(|f| {
            match_pattern
                .as_ref()
                .is_none_or(|p| p.is_match(&f.relative_path.to_string_lossy()))
        })
        .collect();

    if all_files.is_empty() && match_pattern.is_some() {
        return Ok((0, Vec::new()));
    }

    fs::create_dir_all(snapshot_out_dir)?;
    let json_path = snapshot_out_dir.join("_seedvault_snapshot.json");
    let snapshot_json = crate::util::proto::serialize_file_snapshot(snapshot)?;
    fs::write(&json_path, snapshot_json)
        .with_context(|| format!("Failed to write snapshot metadata to {:?}", json_path))?;

    let file_count = all_files.len();

    let pb = mp.add(ProgressBar::new(file_count as u64));
    pb.set_style(style::bar_cyan());
    pb.set_prefix(format!("Snapshot {}", s_info.index));

    // Separate regular files from those stored inside zip chunks.
    // Metadata validation above guarantees that any non-zero zip_index has
    // exactly one chunk ID and is non-negative.
    let mut regular_files = Vec::new();
    let mut zipped_files_by_chunk: HashMap<String, Vec<FileToExtract>> = HashMap::new();

    for file in all_files {
        if file.zip_index > 0 {
            let chunk_id = file.chunk_ids[0].clone();
            zipped_files_by_chunk
                .entry(chunk_id)
                .or_default()
                .push(file);
        } else {
            regular_files.push(file);
        }
    }

    let errors = Mutex::new(Vec::new());
    let succeeded = AtomicUsize::new(0);

    // Resolve output-path collisions (sequential; PathMapper requires &mut self).
    let mut mapper = safe_path::PathMapper::new();
    for file in regular_files.iter_mut() {
        let (dest, renamed) = mapper
            .resolve_entry_path(&file.relative_path, snapshot_out_dir)
            .with_context(|| {
                format!(
                    "Failed to resolve output path for '{}'",
                    file.relative_path.display()
                )
            })?;
        if renamed {
            eprintln!(
                "Warning: Renamed backup file for host compatibility: \
                 original='{}' -> extracted='{}'",
                file.relative_path.display(),
                dest.strip_prefix(snapshot_out_dir)
                    .unwrap_or(&dest)
                    .display(),
            );
        }
        file.dest_path = dest;
    }
    for files in zipped_files_by_chunk.values_mut() {
        for file in files.iter_mut() {
            let (dest, renamed) = mapper
                .resolve_entry_path(&file.relative_path, snapshot_out_dir)
                .with_context(|| {
                    format!(
                        "Failed to resolve output path for '{}'",
                        file.relative_path.display()
                    )
                })?;
            if renamed {
                eprintln!(
                    "Warning: Renamed backup file for host compatibility: \
                     original='{}' -> extracted='{}'",
                    file.relative_path.display(),
                    dest.strip_prefix(snapshot_out_dir)
                        .unwrap_or(&dest)
                        .display(),
                );
            }
            file.dest_path = dest;
        }
    }

    regular_files.par_iter().for_each_init(
        || FileChunkFetcher::new(&repo_path, file_key, chunk_id_key).map_err(|e| format!("{e:#}")),
        |fetcher_result, file| {
            let fetcher = match fetcher_result.as_ref() {
                Ok(f) => f,
                Err(msg) => {
                    errors.lock().unwrap().push(msg.clone());
                    return;
                }
            };
            pb.set_message(file.relative_path.to_string_lossy().into_owned());

            let chunk_ids: Vec<FileChunkId> = match file
                .chunk_ids
                .iter()
                .map(|hex| FileChunkId::from_str(hex))
                .collect::<Result<_, _>>()
            {
                Ok(ids) => ids,
                Err(e) => {
                    errors.lock().unwrap().push(format!(
                        "{}: {:#}",
                        file.relative_path.display(),
                        e
                    ));
                    pb.inc(1);
                    return;
                }
            };

            let out_path = &file.dest_path;

            let result = (|| -> Result<()> {
                if let Some(parent) = out_path.parent() {
                    fs::create_dir_all(parent)?;
                }
                if let Ok(meta) = fs::symlink_metadata(out_path) {
                    if meta.is_dir() {
                        fs::remove_dir_all(out_path)?;
                    } else {
                        let _ = fs::remove_file(out_path);
                    }
                }
                let mut out_file = File::create(out_path)?;
                reassembler::reassemble_data(chunk_ids, fetcher, &mut out_file)?;
                drop(out_file);
                if file.last_modified > 0 {
                    let mtime = ms_to_filetime(file.last_modified);
                    filetime::set_file_mtime(out_path, mtime)?;
                }
                Ok(())
            })();

            match result {
                Ok(()) => {
                    succeeded.fetch_add(1, Ordering::SeqCst);
                }
                Err(e) => {
                    errors.lock().unwrap().push(format!(
                        "{}: {:#}",
                        file.relative_path.display(),
                        e
                    ));
                }
            }
            pb.inc(1);
        },
    );

    // Process zip chunks in parallel (one chunk = one zip archive with multiple files).
    zipped_files_by_chunk.into_par_iter().for_each_init(
        || FileChunkFetcher::new(&repo_path, file_key, chunk_id_key).map_err(|e| format!("{e:#}")),
        |fetcher_result, (zip_chunk_hex, files_in_zip): (String, Vec<FileToExtract>)| {
            let fetcher = match fetcher_result.as_ref() {
                Ok(f) => f,
                Err(msg) => {
                    errors.lock().unwrap().push(msg.clone());
                    return;
                }
            };
            let zip_chunk_id = match FileChunkId::from_str(&zip_chunk_hex) {
                Ok(id) => id,
                Err(e) => {
                    errors
                        .lock()
                        .unwrap()
                        .push(format!("zip chunk {}: {:#}", zip_chunk_hex, e));
                    return;
                }
            };
            let zip_data = match fetcher.fetch_chunk(zip_chunk_id) {
                Ok(data) => data,
                Err(e) => {
                    errors
                        .lock()
                        .unwrap()
                        .push(format!("zip chunk {}: {:#}", zip_chunk_hex, e));
                    return;
                }
            };
            let mut archive = match ZipArchive::new(Cursor::new(zip_data)) {
                Ok(a) => a,
                Err(e) => {
                    errors
                        .lock()
                        .unwrap()
                        .push(format!("zip chunk {}: {:#}", zip_chunk_hex, e));
                    return;
                }
            };

            for file in files_in_zip {
                pb.set_message(file.relative_path.to_string_lossy().into_owned());

                let entry_name = file.zip_index.to_string();
                let mut zip_file = match archive.by_name(&entry_name) {
                    Ok(f) => f,
                    Err(e) => {
                        errors.lock().unwrap().push(format!(
                            "Entry '{}' in zip chunk {}: {:#}",
                            entry_name, zip_chunk_hex, e
                        ));
                        pb.inc(1);
                        continue;
                    }
                };

                let out_path = &file.dest_path;

                let result = (|| -> Result<()> {
                    if let Some(parent) = out_path.parent() {
                        fs::create_dir_all(parent)?;
                    }
                    if let Ok(meta) = fs::symlink_metadata(out_path) {
                        if meta.is_dir() {
                            fs::remove_dir_all(out_path)?;
                        } else {
                            let _ = fs::remove_file(out_path);
                        }
                    }
                    let mut out_file = File::create(out_path)?;
                    std::io::copy(&mut zip_file, &mut out_file)?;
                    drop(out_file);
                    if file.last_modified > 0 {
                        let mtime = ms_to_filetime(file.last_modified);
                        filetime::set_file_mtime(out_path, mtime)?;
                    }
                    Ok(())
                })();

                match result {
                    Ok(()) => {
                        succeeded.fetch_add(1, Ordering::SeqCst);
                    }
                    Err(e) => {
                        errors.lock().unwrap().push(format!(
                            "{}: {:#}",
                            file.relative_path.display(),
                            e
                        ));
                    }
                }
                pb.inc(1);
            }
        },
    );

    pb.finish_and_clear();
    let errors = errors.into_inner().unwrap();
    Ok((succeeded.load(Ordering::SeqCst), errors))
}
