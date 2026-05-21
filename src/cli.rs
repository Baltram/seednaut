use color_print::cstr;

#[cfg(not(target_os = "windows"))]
const EXAMPLES: &str = cstr!(
    "<bold><underline>Examples:</underline></bold>
  seednaut
  seednaut list /path/to/backup
  echo $MY_MNEMONIC | seednaut list /path/to/backup
  seednaut inspect \"/path/to/other backup\" 1 3
  seednaut verify /path/to/backup
  seednaut help extract
  seednaut extract /path/to/backup --match \"camera\" --export --out ./restore
"
);

#[cfg(target_os = "windows")]
const EXAMPLES: &str = cstr!(
    "<bold><underline>Examples:</underline></bold>
  seednaut
  seednaut list C:\\path\\to\\backup
  echo %MY_MNEMONIC% | seednaut list C:\\path\\to\\backup
  seednaut inspect \"C:\\path\\to\\other backup\" 1 3
  seednaut verify C:\\path\\to\\backup
  seednaut help extract
  seednaut extract C:\\path\\to\\backup --match \"camera\" --export --out .\\restore
"
);

include!("cli_shared.rs");
