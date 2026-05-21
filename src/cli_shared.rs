use clap::{ArgAction, Args, Parser, Subcommand};
use std::path::PathBuf;

/// A tool to inspect, verify and extract Seedvault backups.
///
/// The 12-word mnemonic phrase is required to decrypt the backup. It can be
/// provided through an interactive prompt or piped via stdin.
///
/// Supports version 2 app backups and version 0 file backups.
///
/// Interactive mode:
///   Run without arguments or with a single path argument to enter interactive mode.
#[derive(Parser, Debug)]
#[command(author, version, about)]
#[command(disable_help_flag = true)]
#[command(after_help = EXAMPLES)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,

    /// Print help
    #[arg(long, short, action = ArgAction::HelpLong)]
    pub help: Option<bool>,
}

#[derive(Subcommand, Debug, Clone)]
pub enum Command {
    /// List available snapshots
    #[command(arg_required_else_help = true)]
    List(SnapshotSelector),
    /// Inspect the contents of snapshots (packages, files)
    #[command(arg_required_else_help = true)]
    Inspect(SnapshotSelector),
    /// Verify the integrity of all chunks in snapshots
    #[command(arg_required_else_help = true)]
    Verify(SnapshotSelector),
    /// Extract files and data from snapshots
    #[command(arg_required_else_help = true)]
    Extract(ExtractArgs),
}

/// Arguments for selecting one or more snapshots.
#[derive(Args, Debug, Clone)]
pub struct SnapshotSelector {
    /// The input path (snapshot file or directory)
    pub path: PathBuf,

    /// Indices of snapshots to target. If omitted, all snapshots are targeted.
    #[arg(required = false)]
    pub snapshots: Vec<u32>,
}

/// Arguments for the 'extract' command.
#[derive(Args, Debug, Clone)]
pub struct ExtractArgs {
    #[command(flatten)]
    pub selector: SnapshotSelector,

    /// Output directory for extracted files
    #[arg(short, long = "out")]
    pub out_dir: PathBuf,

    /// Regex to filter packages or file paths for extraction
    #[arg(short = 'm', long = "match")]
    pub pattern_str: Option<String>,

    /// Unpack archives (.tar) and convert databases (.db) to a readable format
    #[arg(short, long)]
    pub export: bool,
}
