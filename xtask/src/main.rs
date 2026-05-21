use clap::{CommandFactory, Parser, Subcommand};
use std::path::PathBuf;

mod cli {
    const EXAMPLES: &str = "\
seednaut

seednaut list /path/to/backup

echo $MY_MNEMONIC | seednaut list /path/to/backup

seednaut inspect \"/path/to/other backup\" 1 3

seednaut verify /path/to/backup

seednaut extract /path/to/backup --match \"camera\" --export --out ./restore
";

    include!("../../src/cli_shared.rs");
}

#[derive(Parser)]
#[command(name = "xtask")]
struct Xtask {
    #[command(subcommand)]
    command: SubCommand,
}

#[derive(Subcommand)]
enum SubCommand {
    /// Generate protobuf bindings
    Proto,
    /// Generate man pages in man/
    Man,
}

fn cmd_proto() -> Result<(), Box<dyn std::error::Error>> {
    let out_dir = PathBuf::from("src/proto/generated");

    std::fs::create_dir_all(&out_dir)?;

    let mut config = prost_build::Config::new();

    config.disable_comments(&["."]);
    config.file_descriptor_set_path(out_dir.join("descriptor.bin"));
    config.out_dir(&out_dir);

    config.compile_protos(
        &[
            "proto/snapshot.proto",
            "proto/backup_snapshot.proto",
            "proto/backup_media_file.proto",
            "proto/backup_document_file.proto",
        ],
        &["proto"],
    )?;

    println!("Generated protobuf bindings.");

    Ok(())
}

fn seednaut_version() -> &'static str {
    let cargo_toml = include_str!("../../Cargo.toml");
    for line in cargo_toml.lines() {
        let line = line.trim();
        if line.starts_with("version = ") {
            return line
                .trim_start_matches("version = ")
                .trim_matches('"')
                .trim_matches('\'');
        }
    }
    "0.0.0"
}

fn cmd_man() -> Result<(), Box<dyn std::error::Error>> {
    let cmd = cli::Cli::command()
        .name("seednaut")
        .version(seednaut_version());

    let man_dir = PathBuf::from("man");
    std::fs::create_dir_all(&man_dir)?;

    if man_dir.is_dir() {
        for entry in std::fs::read_dir(&man_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().map_or(false, |ext| ext == "1") {
                std::fs::remove_file(&path)?;
            }
        }
    }

    clap_mangen::generate_to(cmd, &man_dir)?;

    // Rename EXTRA section heading to EXAMPLES
    for entry in std::fs::read_dir(&man_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().map_or(false, |ext| ext == "1") {
            let content = std::fs::read_to_string(&path)?;
            let content = content.replace(".SH EXTRA\n", ".SH EXAMPLES\n");
            std::fs::write(&path, content)?;
        }
    }

    println!("Generated man pages in man/");

    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let xtask = Xtask::parse();

    match xtask.command {
        SubCommand::Proto => cmd_proto(),
        SubCommand::Man => cmd_man(),
    }
}
