# Seednaut
Inspect, verify and extract files from [`Seedvault`](https://github.com/seedvault-app/seedvault) backups.

<a href="https://github.com/Baltram/seednaut/actions"><img src="https://img.shields.io/github/actions/workflow/status/Baltram/seednaut/ci.yml" alt="Build Status"></a>
<a href="https://crates.io/crates/seednaut"><img src="https://img.shields.io/crates/v/seednaut.svg" alt="Crates.io"></a>
<img src="https://img.shields.io/crates/l/seednaut.svg" alt="License">
<p>
  <a href="#quick-start">Quick Start</a> •
  <a href="#cli-examples">CLI Examples</a> •
  <a href="#development-notes">Development notes</a>
</p>

---

Various Android distributions like CalyxOS, GrapheneOS, iodéOS, LineageOS and /e/OS integrate Seedvault as their main backup mechanism. There are currently no official tools for inspecting such backups off-device. To help fill that gap:

**Seednaut** is a desktop command-line and terminal UI utility that lets you

* Verify that backups are intact
* Explore backup contents before relying on them
* Recover specific files without a full device restore
* Unpack app backup data (.tar, .db) into accessible formats

It runs purely offline and supports current Seedvault backup formats (v2 app backup and v0 file backup formats). For older backups you can try some of the [`other 3rd party tools`](https://github.com/seedvault-app/seedvault#third-party-tools).

![Animated Seednaut demo](./assets/demo.svg)

---

## Quick start

**Prerequisites:**

Seedvault backups are usually stored in a directory named `.SeedVaultAndroidBackup`. Depending on your backup configuration you can copy it

* from a phone
* from a USB drive
* from Nextcloud/WebDAV storage

Note: some file managers hide directories beginning with `.` by default.

You will also need the 12-word mnemonic phrase that was used to create the backup.

### 1. Get Seednaut

Choose one of the following methods:

#### A) pkgsrc (NetBSD, SmartOS, Linux, etc.)

If you use `pkgin`:

```bash
pkgin install seednaut
```

Or via `pkg_add`:

```bash
pkg_add seednaut
```

Build from source using `pkgsrc`:

```bash
cd /usr/pkgsrc/sysutils/seednaut  # Adjust pkgsrc path as needed
make install
```

#### B) Install from `crates.io` (Rust >=1.88)

```bash
cargo install seednaut
```

#### C) Build from source (Rust >=1.88)

```bash
git clone https://github.com/Baltram/seednaut && cd seednaut
cargo build --release
```

#### D) Download a prebuilt binary

Go to the [`Releases`](https://github.com/Baltram/seednaut/releases) page and download the file matching your operating system:

| Platform            | File match                  |
| ------------------- | --------------------------- |
| Linux (Intel/AMD)   | `x86_64-unknown-linux-musl` |
| Windows (64-bit)    | `x86_64-pc-windows-msvc`    |
| macOS Apple Silicon | `aarch64-apple-darwin`      |
| macOS Intel         | `x86_64-apple-darwin`       |

### 2. Run Seednaut

Running Seednaut without arguments starts the interactive TUI mode. It guides you through the steps of locating the backup, entering your mnemonic and performing the inspection/verification/extraction. Depending on your installation method, enter `seednaut` or `./seednaut` in a shell.

On Windows, you can simply double click `seednaut.exe` or drag&drop your backup onto it.

## CLI Examples

Seednaut can also be used directly via the following commands.

List snapshots:
```bash
seednaut list /path/to/.SeedVaultAndroidBackup
```

Inspect snapshot contents:
```bash
# All snapshots
seednaut inspect /path/to/.SeedVaultAndroidBackup

# Only specific snapshots (e.g. indices 1 and 3 from the output of the list command)
seednaut inspect /path/to/.SeedVaultAndroidBackup 1 3
```

Verify cryptographic integrity of snapshots:
```bash
seednaut verify /path/to/.SeedVaultAndroidBackup
```

Extract files:
```bash
# Basic extraction
seednaut extract /path/to/backup --out ./recovered

# Also unpack app backup data or, in case of K/V app backup, convert to JSON
seednaut extract /path/to/backup --export --out ./recovered

# Use regex to selectively extract matching package names and file paths
seednaut extract /path/to/backup --match "IMG_2026.*\.jpg$" --out ./recovered
```

Pipe mnemonic via stdin:
```bash
echo "$MY_MNEMONIC" | seednaut verify /path/to/backup
```

Run `seednaut help` or `seednaut help <command>` for full CLI documentation.

## Development notes

Generated man pages are available under `man/` and can be regenerated with `cargo run -p xtask -- man`. Similarly, protobuf bindings can be regenerated with `cargo run -p xtask -- proto`.

AI coding tools were used during development. Generated code, architecture, crate selection were manually reviewed and iterated on extensively before release.

Please note that, while developed with security in mind, Seednaut has not undergone professional security auditing and is provided on a best-effort basis.

## License

Licensed under either of

- Apache License, Version 2.0
- MIT license

at your option.