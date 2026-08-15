use anyhow::Result;
use indicatif::ProgressBar;
use std::path::PathBuf;
use std::time::Duration;

use crate::engine::crypto::{DerivedKeys, derive_keys};
use crate::engine::scanner::discover_snapshots;
use crate::engine::types::SnapshotInfo;
use crate::ui::mnemonic::get_mnemonic;

pub struct AppContext {
    pub input_path: PathBuf,
    pub derived_keys: DerivedKeys,
    pub snapshots: Vec<SnapshotInfo>,
}

pub fn init_context(input_path: PathBuf) -> Result<AppContext> {
    if !input_path.exists() {
        anyhow::bail!(
            "The specified input path does not exist: {}",
            input_path.display()
        );
    }

    let mnemonic = get_mnemonic()?;
    let derived_keys = derive_keys(&mnemonic)?;

    let spinner_style = crate::ui::style::spinner();
    let spinner = ProgressBar::new_spinner().with_style(spinner_style);
    spinner.enable_steady_tick(Duration::from_millis(80));
    spinner.set_message("Discovering and decrypting snapshots...");

    let discovery = discover_snapshots(&input_path, &derived_keys)?;
    let (snapshots, failed) = (discovery.snapshots, discovery.failed);

    let msg = if snapshots.is_empty() && failed.is_empty() {
        "No supported SeedVault snapshots found (expected .snapshot or .SeedSnap files)."
            .to_string()
    } else if snapshots.is_empty() {
        "No snapshots could be decrypted. The mnemonic may be incorrect.".to_string()
    } else if !failed.is_empty() {
        format!(
            "{} of {} snapshots could not be decrypted:",
            failed.len(),
            snapshots.len() + failed.len()
        )
    } else {
        format!("Discovered {} snapshot(s).", snapshots.len())
    };
    spinner.finish_with_message(msg);

    if !snapshots.is_empty() && !failed.is_empty() {
        for f in &failed {
            eprintln!("  - {}", f);
        }
    }

    Ok(AppContext {
        input_path,
        derived_keys,
        snapshots,
    })
}
