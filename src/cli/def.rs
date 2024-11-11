use clap::{Parser, Subcommand};

/// Verifies expressions against environment variables
#[derive(Parser)]
#[command(author, version, about, long_about=None)]
pub struct Args {
    /// force color mode (defaults to check tty)
    #[arg(long)]
    pub color: bool,

    /// force no-color mode (defaults to check tty)
    #[arg(long)]
    pub no_color: bool,

    /// prepend time to each log line
    #[arg(long)]
    pub log_time: bool,

    /// Turn general verbose logging
    #[arg(short, long, action = clap::ArgAction::Count)]
    pub verbose: u8,

    /// Configure component wise logging
    #[arg(long, short, action = clap::ArgAction::Append)]
    pub log: Option<Vec<String>>,

    #[command(subcommand)]
    pub action: Option<Actions>,
}

#[derive(Subcommand)]
pub enum Actions {
    Next {
        /// Checks constraint on environment

        /// The log filename
        #[clap(name = "FILENAME")]
        file_name: String,

        /// optional cursor name
        #[arg(long, short, default_value = "default")]
        cursor_name: String,
    },
    Import {
        /// Checks constraint on environment

        /// The database filename
        #[clap(name = "SQLITE_DATABASE_PATH")]
        sqlite_db_path: String,

        /// The label associated to the new data
        #[clap(name = "LABEL")]
        label: String,

        /// The failed chunk output if any
        #[clap(name = "FAILED_CHUNKS_FOLDER")]
        failed_chunks_folder: String,
    },
}
