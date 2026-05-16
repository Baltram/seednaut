use indicatif::ProgressStyle;

/// Creates a spinner progress style with green spinner characters.
pub fn spinner() -> ProgressStyle {
    ProgressStyle::with_template("{spinner:.green} {wide_msg}")
        .expect("Invalid progress template")
        .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"])
}

/// Creates a spinner style specifically for snapshot operations.
pub fn snapshot_spinner() -> ProgressStyle {
    ProgressStyle::default_bar()
        .template("{spinner:.green} Snapshot {msg}")
        .expect("Invalid progress template")
}

/// Creates a cyan progress bar with prefix, position, and message.
pub fn bar_cyan() -> ProgressStyle {
    ProgressStyle::default_bar()
        .template("  {prefix:.bold} [{bar:40.cyan/blue}] {pos}/{len} {wide_msg}")
        .expect("Invalid progress template")
        .progress_chars("-> ")
}

/// Creates a cyan progress bar with elapsed time, percentage, and spinner.
pub fn bar_cyan_elapsed() -> ProgressStyle {
    ProgressStyle::default_bar()
        .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} ({percent}%) - {msg}")
        .expect("Invalid progress template")
        .progress_chars("-> ")
}

/// Creates a yellow/red progress bar with prefix and position.
pub fn bar_yellow() -> ProgressStyle {
    ProgressStyle::default_bar()
        .template("  {prefix:.bold} [{bar:40.yellow/red}] {pos}/{len} {msg}")
        .expect("Invalid progress template")
        .progress_chars("-> ")
}
