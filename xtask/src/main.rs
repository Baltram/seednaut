use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
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
