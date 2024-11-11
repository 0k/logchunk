mod action;
mod def;
pub mod log;

use clap::Parser;

pub fn run() -> Result<bool, String> {
    let cli = def::Args::parse();

    // Split log strings upon comma, trim them and flatten all in
    // `logs`, remove empty values
    let logs = cli.log.unwrap_or_else(Vec::new); // Provide an empty Vec if cli.log is None
    let logs = logs
        .iter()
        .flat_map(|log| log.split(',')) // Split each log entry on commas
        .map(str::trim) // Trim whitespace from each resulting entry
        .filter(|s| !s.is_empty()) // Remove empty strings
        .collect::<Vec<&str>>(); // Collect into a Vec<&str>

    // Upon failure, display error message and usage string
    log::setup(cli.verbose, logs, cli.log_time)?;

    if cli.color && cli.no_color {
        return Err("Cannot use both --color and --no-color".to_string());
    }
    if cli.color {
        colored::control::set_override(true);
    }
    if cli.no_color {
        colored::control::set_override(false);
    }

    match &cli.action {
        Some(def::Actions::Next {
            file_name,
            cursor_name,
        }) => action::next::run(file_name, cursor_name),
        Some(def::Actions::Import {
            sqlite_db_path,
            label,
            failed_chunks_folder,
        }) => action::import::run(label, sqlite_db_path, failed_chunks_folder),
        None => Err("Missing action".to_string()),
    }
}
