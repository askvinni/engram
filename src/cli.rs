use clap::{Parser, Subcommand};

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputMode {
    Human,
    Agent,
    Json,
}

#[derive(Parser)]
#[command(name = "engram", about = "Plan-based agentic development helper")]
pub struct Cli {
    /// Emit compact, stable, machine-readable output (no human chrome)
    #[arg(long, global = true)]
    pub agent: bool,

    /// Emit structured JSON output for jq-style consumers; takes precedence over --agent
    #[arg(long, global = true)]
    pub json: bool,

    #[command(subcommand)]
    pub command: Commands,
}

impl Cli {
    #[allow(dead_code)]
    pub fn output_mode(&self) -> OutputMode {
        if self.json {
            OutputMode::Json
        } else if self.agent {
            OutputMode::Agent
        } else {
            OutputMode::Human
        }
    }
}

#[derive(Subcommand)]
pub enum Commands {
    /// Initialize engram in this repository
    Init,
    /// Plan subcommands (create, list, learn, land, status)
    Plan {
        #[command(subcommand)]
        subcommand: PlanCommands,
    },
    /// Search memory files using full-text search
    Search {
        /// Query string (FTS5 syntax supported)
        query: String,
    },
    /// Check that all engram dependencies are installed and configured
    Doctor,
    /// Prune and merge memory files that don't meet the future-looking standard
    Compact,
    /// Coordinate multiple related plans toward a shared goal
    Objective {
        #[command(subcommand)]
        subcommand: ObjectiveCommands,
    },
    /// Read a memory file by path or tag
    Read {
        /// Exact path (e.g. patterns/foo.md) or filename stem to look up
        permalink: String,
    },
    /// kata mirror maintenance
    Kata {
        #[command(subcommand)]
        subcommand: KataCommands,
    },
    /// Write a memory file directly, without an issue, PR, or Claude synthesis
    Capture {
        /// Memory category: patterns, tripwires, architecture, or testing
        #[arg(long)]
        category: String,
        /// Title for the memory file's frontmatter
        #[arg(long)]
        title: String,
        /// Filename stem; derived from the title if omitted
        #[arg(long)]
        slug: Option<String>,
        /// Condition under which an agent should load this file (repeatable)
        #[arg(long = "read-when")]
        read_when: Vec<String>,
        /// Tripwire as `ACTION::WARNING` (repeatable)
        #[arg(long = "tripwire", value_parser = crate::capture::parse_tripwire)]
        tripwires: Vec<crate::memory::Tripwire>,
        /// Markdown body; read from stdin if omitted
        #[arg(long)]
        body: Option<String>,
        /// Mark as a mid-session observation (`origin: session`) for later consolidation by `compact`
        #[arg(long)]
        session: bool,
    },
    /// Start work on plan issues (or an objective's unblocked nodes): claim each
    /// kata mirror, push a branch, open a draft PR, create a kwt worktree and
    /// launch Claude in its tmux session
    Dispatch {
        /// engram-plan and/or engram-objective issue numbers; an objective
        /// dispatches its unblocked nodes that already have a plan issue
        #[arg(required = true)]
        issues: Vec<u64>,
        #[command(flatten)]
        opts: crate::dispatch::DispatchOptions,
    },
    /// Memory file maintenance
    Memory {
        #[command(subcommand)]
        subcommand: MemoryCommands,
    },
}

#[derive(Subcommand)]
pub enum MemoryCommands {
    /// Rewrite all memory files into OKF v0.2 typed frontmatter
    Migrate,
}

#[derive(Subcommand)]
pub enum KataCommands {
    /// Reconcile drift between a GitHub issue and its linked kata mirror
    Sync {
        /// GitHub issue number to sync
        #[arg(long, conflicts_with = "all")]
        issue: Option<u64>,
        /// Sync every linked issue pair
        #[arg(long, conflicts_with = "issue")]
        all: bool,
    },
}

#[derive(Subcommand)]
pub enum PlanCommands {
    /// Create a GitHub issue as a plan
    New {
        /// Issue title
        title: String,
        /// Issue body
        #[arg(long)]
        body: Option<String>,
        /// Planning conversation to attach as a comment for richer learn synthesis
        #[arg(long)]
        conversation: Option<String>,
        /// Jira key for the kata mirror's `jira` metadata (e.g. ABC-123); otherwise read from a `Jira:` body line or the title
        #[arg(long, value_parser = crate::kata::parse_jira_arg)]
        jira: Option<String>,
        /// Change type for the kata mirror's `type` metadata (feat, fix, chore, ...); otherwise read from a `Type:` body line or a `feat:`-style title
        #[arg(long = "type", value_parser = crate::kata::parse_type_arg)]
        kind: Option<String>,
        /// Skip all kata calls for this invocation, even if kata is available
        #[arg(long)]
        no_kata: bool,
    },
    /// List open engram-plan issues
    List,
    /// Synthesize learnings from a closed issue+PR into memory
    Learn {
        /// GitHub issue number
        issue: Option<u64>,
        /// Process all closed plan issues that have not yet been learned
        #[arg(long, conflicts_with = "issue")]
        all: bool,
        /// Skip all kata calls for this invocation, even if kata is available
        #[arg(long)]
        no_kata: bool,
    },
    /// Learn from a closed issue+PR, close the issue, and delete the local branch
    Land {
        /// GitHub issue number
        issue: u64,
        /// Skip all kata calls for this invocation, even if kata is available
        #[arg(long)]
        no_kata: bool,
    },
    /// Show the linked engram issue and PR for the current branch
    Status,
}

#[derive(Subcommand)]
pub enum ObjectiveCommands {
    /// Create a new objective with a roadmap of nodes
    New {
        /// Objective title
        title: String,
        /// Body with ## Goal and ## Roadmap (bullet list) sections
        #[arg(long)]
        body: String,
        /// Jira key for the kata mirror's `jira` metadata (e.g. ABC-123); otherwise read from a `Jira:` body line or the title
        #[arg(long, value_parser = crate::kata::parse_jira_arg)]
        jira: Option<String>,
        /// Change type for the kata mirror's `type` metadata (feat, fix, chore, ...); otherwise read from a `Type:` body line or a `feat:`-style title
        #[arg(long = "type", value_parser = crate::kata::parse_type_arg)]
        kind: Option<String>,
        /// Skip all kata calls for this invocation, even if kata is available
        #[arg(long)]
        no_kata: bool,
    },
    /// List open objective issues
    List,
    /// View an objective and its roadmap
    View {
        /// Objective issue number
        number: u64,
    },
    /// Create a plan issue for a specific objective node
    Plan {
        /// Objective issue number
        number: u64,
        /// Node ID (e.g. 1.1)
        #[arg(
            long,
            conflicts_with = "all_unblocked",
            required_unless_present = "all_unblocked"
        )]
        node: Option<String>,
        /// Create plan issues for all pending nodes whose dependencies are all done
        #[arg(long, conflicts_with = "node")]
        all_unblocked: bool,
        /// Plan body; generated by Claude if omitted
        #[arg(long)]
        body: Option<String>,
        /// Jira key for the kata mirror's `jira` metadata (e.g. ABC-123); otherwise read from a `Jira:` body line or the title
        #[arg(long, value_parser = crate::kata::parse_jira_arg)]
        jira: Option<String>,
        /// Change type for the kata mirror's `type` metadata (feat, fix, chore, ...); otherwise read from a `Type:` body line or a `feat:`-style title
        #[arg(long = "type", value_parser = crate::kata::parse_type_arg)]
        kind: Option<String>,
        /// Skip all kata calls for this invocation, even if kata is available
        #[arg(long)]
        no_kata: bool,
    },
    /// Synthesize learnings from all plans in an objective and close it when done
    Land {
        /// Objective issue number
        number: u64,
    },
    /// Append a new node to an existing objective
    Append {
        /// Objective issue number
        number: u64,
        /// Node ID (e.g. 1.3)
        #[arg(long)]
        id: String,
        /// Node description
        #[arg(long)]
        description: String,
        /// Comma-separated node IDs this node depends on (e.g. 1.1,1.2)
        #[arg(long)]
        depends: Option<String>,
    },
}
