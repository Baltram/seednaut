use anyhow::{Context, Result};
use clap::{CommandFactory, Parser};
use inquire::{InquireError, validator::Validation};
use regex::Regex;
use std::io::{IsTerminal, stdin, stdout};
use std::path::PathBuf;

mod app;
mod cli;
mod commands;
mod engine;
mod proto;
mod ui;
mod util;

use crate::app::AppContext;
use crate::app::init_context;
use crate::cli::{Cli, Command, SnapshotSelector};
use crate::commands::extract::extract_snapshots;
use crate::commands::inspect::inspect_snapshots;
use crate::commands::list::list_snapshots;
use crate::commands::verify::verify_snapshots;
use crate::engine::types::RawSnapshot;
use crate::ui::path::{get_backup_input_path, get_target_output_path};

fn is_reserved_cli_word(arg: &str) -> bool {
    matches!(
        arg,
        "list" | "inspect" | "verify" | "extract" | "help" | "-h" | "--help"
    )
}

fn interactive_tty_available() -> bool {
    stdin().is_terminal() && stdout().is_terminal()
}

fn prompt_result<T>(result: std::result::Result<T, InquireError>) -> Result<Option<T>> {
    match result {
        Ok(value) => Ok(Some(value)),
        Err(InquireError::OperationCanceled) => Ok(None),
        Err(InquireError::OperationInterrupted) => {
            crate::ui::cleanup_terminal();
            Ok(None)
        }
        Err(e) => Err(e.into()),
    }
}

fn regex_filter_validator(
    input: &str,
) -> std::result::Result<Validation, Box<dyn std::error::Error + Send + Sync>> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Ok(Validation::Valid);
    }

    match Regex::new(trimmed) {
        Ok(_) => Ok(Validation::Valid),
        Err(e) => Ok(Validation::Invalid(format!("Invalid regex: {e}").into())),
    }
}

/// Runs the application in command-line mode based on parsed arguments.
fn run_command_mode(cli: Cli) -> Result<()> {
    let input_path = match &cli.command {
        Command::List(s) => s.path.clone(),
        Command::Inspect(s) => s.path.clone(),
        Command::Verify(s) => s.path.clone(),
        Command::Extract(args) => args.selector.path.clone(),
    };

    let context = init_context(input_path)?;

    if context.snapshots.is_empty() {
        return Ok(());
    }

    match cli.command {
        Command::List(selector) => list_snapshots(&context, &selector),
        Command::Inspect(selector) => inspect_snapshots(&context, &selector),
        Command::Verify(selector) => verify_snapshots(&context, &selector),
        Command::Extract(args) => extract_snapshots(&context, &args),
    }
}

/// Runs the application in interactive mode, prompting the user for all inputs.
/// Optionally accepts an initial path to skip the first directory prompt.
fn run_interactive_mode(initial_path: Option<PathBuf>) -> Result<()> {
    let input_path = match initial_path {
        Some(p) => {
            println!("Using input path from argument: {}", p.display());
            p
        }
        None => match prompt_result(get_backup_input_path())? {
            Some(p) => p,
            None => return Ok(()),
        },
    };

    let context = match init_context(input_path) {
        Ok(ctx) => {
            if ctx.snapshots.is_empty() {
                return Ok(());
            }
            ctx
        }
        Err(e) => {
            eprintln!("\nError during initialization: {}", e);
            eprintln!("Please check your mnemonic and the input path.");
            return Ok(());
        }
    };

    // 3. Main interactive loop
    loop {
        let choices = vec![
            "List all snapshots",
            "Inspect snapshot(s)",
            "Verify snapshot(s)",
            "Extract from snapshot(s)",
            "Exit",
        ];
        let Some(choice) =
            prompt_result(inquire::Select::new("What would you like to do?", choices).prompt())?
        else {
            break;
        };

        let action_ran;

        let action_result: Result<()> = match choice {
            "List all snapshots" => {
                action_ran = true;
                list_snapshots(
                    &context,
                    &SnapshotSelector {
                        path: context.input_path.clone(),
                        snapshots: vec![],
                    },
                )
            }
            "Inspect snapshot(s)" => {
                let Some(selector) = prompt_for_snapshot_selection(&context, "inspect")? else {
                    continue;
                };
                action_ran = true;
                inspect_snapshots(&context, &selector)
            }
            "Verify snapshot(s)" => {
                let Some(selector) = prompt_for_snapshot_selection(&context, "verify")? else {
                    continue;
                };
                action_ran = true;
                verify_snapshots(&context, &selector)
            }
            "Extract from snapshot(s)" => {
                let Some(selector) = prompt_for_snapshot_selection(&context, "extract from")?
                else {
                    continue;
                };
                let Some(out_dir) = prompt_result(get_target_output_path())? else {
                    continue;
                };
                let Some(pattern_input) = prompt_result(
                    inquire::Text::new(
                        "Enter a regex to filter files/packages (optional, press Enter to skip):",
                    )
                    .with_help_message("Leave empty to extract everything. Esc to go back.")
                    .with_validator(regex_filter_validator)
                    .prompt(),
                )?
                else {
                    continue;
                };
                let pattern_str = match pattern_input.trim() {
                    "" => None,
                    s => Some(s.to_string()),
                };
                let has_app_snapshots = if selector.snapshots.is_empty() {
                    context
                        .snapshots
                        .iter()
                        .any(|s| matches!(s.raw_snapshot, RawSnapshot::App(_)))
                } else {
                    context.snapshots.iter().any(|s| {
                        selector.snapshots.contains(&s.index)
                            && matches!(s.raw_snapshot, RawSnapshot::App(_))
                    })
                };

                let export = if has_app_snapshots {
                    let Some(export) = prompt_result(
                        inquire::Confirm::new(
                            "Enable export mode? (unpack .tar, convert .db to .json)",
                        )
                        .with_default(false)
                        .prompt(),
                    )?
                    else {
                        continue;
                    };
                    export
                } else {
                    false
                };

                let args = crate::cli::ExtractArgs {
                    selector,
                    out_dir,
                    pattern_str,
                    export,
                };
                action_ran = true;
                extract_snapshots(&context, &args)
            }
            "Exit" => break,
            _ => unreachable!(),
        };

        if let Err(e) = action_result {
            eprintln!("\nError: {e:#}");
        }

        if action_ran {
            crate::ui::wait_for_key_press();
        }
    }

    Ok(())
}

/// Prompts the user to select one or more snapshots.
fn prompt_for_snapshot_selection(
    context: &AppContext,
    verb: &str,
) -> Result<Option<SnapshotSelector>> {
    const ALL_SNAPSHOTS_OPTION: &str = "[All snapshots]";

    let option_pairs: Vec<(String, u32)> = context
        .snapshots
        .iter()
        .map(|s| {
            let ts_str = crate::util::date::format_display(s.timestamp);
            let s_type = match s.raw_snapshot {
                RawSnapshot::App(_) => "App",
                RawSnapshot::File(_) => "File",
            };
            (
                format!("{}) [{}] {} \"{}\"", s.index, s_type, ts_str, s.name),
                s.index,
            )
        })
        .collect();

    let mut options: Vec<String> = vec![ALL_SNAPSHOTS_OPTION.to_string()];
    options.extend(option_pairs.iter().map(|(label, _)| label.clone()));

    let Some(selected_options) = prompt_result(
        inquire::MultiSelect::new(
            &format!(
                "Select snapshot(s) to {}: (space to toggle, enter to confirm)",
                verb
            ),
            options,
        )
        .prompt(),
    )?
    else {
        return Ok(None);
    };

    if selected_options.is_empty() {
        println!("No snapshots selected.");
        Ok(None)
    } else {
        let indices: Vec<u32> = if selected_options
            .iter()
            .any(|selected| selected == ALL_SNAPSHOTS_OPTION)
        {
            context.snapshots.iter().map(|s| s.index).collect()
        } else {
            selected_options
                .iter()
                .map(|selected| {
                    option_pairs
                        .iter()
                        .find(|(label, _)| label == selected)
                        .map(|(_, index)| *index)
                        .with_context(|| {
                            format!(
                                "Internal error: could not resolve selected snapshot '{}'",
                                selected
                            )
                        })
                })
                .collect::<Result<_>>()?
        };
        Ok(Some(SnapshotSelector {
            path: context.input_path.clone(),
            snapshots: indices,
        }))
    }
}

/// The main entrypoint of the application.
fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();

    // Enter interactive mode when:
    // 1. No arguments.
    if args.len() == 1 {
        if !interactive_tty_available() {
            Cli::command().print_help()?;
            println!();
            anyhow::bail!(
                "No command supplied. Interactive mode requires an interactive terminal."
            );
        }
        return run_interactive_mode(None);
    }

    // 2. Single non-flag argument that is not a known command: treat it as the input path.
    if args.len() == 2 {
        let arg = &args[1];
        if !arg.starts_with('-') && !is_reserved_cli_word(arg) {
            if !interactive_tty_available() {
                anyhow::bail!(
                    "A lone path argument starts interactive mode, which requires an interactive terminal.\n\
                     Use a subcommand instead, for example:\n\
                       {} list {}\n\
                       {} inspect {}\n\
                       {} verify {}\n\
                       {} extract {} --out <DIR>",
                    args[0],
                    arg,
                    args[0],
                    arg,
                    args[0],
                    arg,
                    args[0],
                    arg
                );
            }
            return run_interactive_mode(Some(PathBuf::from(arg)));
        }
    }

    // Standard CLI parsing
    let cli = Cli::parse();
    run_command_mode(cli)?;

    Ok(())
}
