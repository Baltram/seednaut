pub mod mnemonic;
pub mod path;
pub mod style;

use console::Term;
use std::io::{Write, stdout};

/// Clears terminal content from the cursor position down and prints a newline.
///
/// This prevents inquire prompt artifacts (autocomplete suggestions, help text)
/// from leaking into the next shell prompt after Ctrl+C interrupts a prompt.
pub fn cleanup_terminal() {
    // \x1b[0J clears from cursor to end of screen (same as crossterm's Clear(ClearType::FromCursorDown))
    let mut out = stdout();
    let _ = write!(out, "\x1b[0J");
    let _ = out.flush();
    println!();
}

/// Waits for the user to press any key before continuing.
///
/// This is best-effort UI behavior; terminal errors are ignored so the program
/// does not panic after an otherwise successful operation.
pub fn wait_for_key_press() {
    let term = Term::stdout();

    if term
        .write_line("\n--- Operation complete. Press any key to continue ---")
        .is_err()
    {
        return;
    }

    let _ = term.read_key();
    let _ = term.clear_last_lines(1);
}
