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
    /// List open engram-plan issues
    List,
    /// Learn from a closed issue+PR and close the issue
    Land {
        /// GitHub issue number
        issue: u64,
    },
    /// Show the linked engram issue and PR for the current branch
    Status,
    /// Prune and merge memory files that don't meet the future-looking standard
    Compact,
}
