use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "engram", about = "Plan-based agentic development helper")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Initialize engram in this repository
    Init,
    /// Create a GitHub issue as a plan
    Plan {
        /// Issue title
        title: String,
        /// Issue body
        #[arg(long)]
        body: Option<String>,
    },
    /// Synthesize learnings from a closed issue+PR into memory
    Learn {
        /// GitHub issue number
        issue: u64,
    },
    /// Check that all engram dependencies are installed and configured
    Doctor,
}
