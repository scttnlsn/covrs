use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

use covrs::diff::{DiffSource, GitDiff, GitHubDiff, StdinDiff};
use covrs::{cli, db};

/// covrs — Multi-format code coverage ingestion into a unified SQLite store.
#[derive(Parser)]
#[command(name = "covrs", version, about)]
struct Cli {
    /// Path to the SQLite database (default: ./.covrs.db)
    #[arg(long, global = true, default_value = ".covrs.db")]
    db: PathBuf,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Ingest a coverage file into the database.
    Ingest {
        /// Path to the coverage file.
        file: PathBuf,

        /// Override format detection (cobertura, jacoco, lcov).
        #[arg(long)]
        format: Option<String>,

        /// Name for this report (default: filename).
        #[arg(long)]
        name: Option<String>,

        /// Overwrite existing report with the same name.
        #[arg(long)]
        overwrite: bool,

        /// Project root for making coverage paths relative.
        /// Absolute paths from coverage files are stripped to be relative
        /// to this directory. Defaults to the current working directory.
        #[arg(long)]
        root: Option<PathBuf>,
    },

    /// Show a coverage summary across all reports.
    Summary,

    /// List all reports in the database.
    Reports,

    /// List per-file coverage across all reports.
    Files {
        /// Sort by coverage rate ascending (show worst files first).
        #[arg(long)]
        sort_by_coverage: bool,
    },

    /// Show line-level coverage for a source file.
    Lines {
        /// The source file path (as stored in the coverage data).
        source_file: String,

        /// Show only uncovered lines as compact ranges.
        #[arg(long)]
        uncovered: bool,
    },

    /// Compute coverage for lines in a git diff (diff coverage).
    ///
    /// By default, reads a diff from stdin or via --git-diff and prints a
    /// plain-text coverage summary to stdout. Use --style to control the
    /// output format.
    ///
    /// With --comment, posts (or updates) the output as a comment on a
    /// GitHub pull request. The diff is fetched from the GitHub API and
    /// the PR number, repo, and SHA are detected from standard GitHub
    /// Actions environment variables (GITHUB_TOKEN, GITHUB_REF,
    /// GITHUB_REPOSITORY, GITHUB_SHA).
    DiffCoverage {
        /// Git diff arguments, e.g. "HEAD~1" or "main..HEAD".
        /// If omitted, reads a unified diff from stdin.
        /// Ignored when --comment is used.
        #[arg(long)]
        git_diff: Option<String>,

        /// Optional path prefix to prepend to diff paths for matching
        /// against coverage data paths.
        #[arg(long)]
        path_prefix: Option<String>,

        /// Output format.
        #[arg(long, value_enum, default_value_t = cli::Style::Text)]
        style: cli::Style,

        /// Post results as a comment on a GitHub pull request.
        /// The diff is fetched via the GitHub API and all required
        /// parameters are read from the environment (GITHUB_TOKEN,
        /// GITHUB_REPOSITORY, GITHUB_REF, GITHUB_SHA).
        #[arg(long)]
        comment: bool,
    },
}

fn main() -> Result<()> {
    let args = Cli::parse();

    let mut conn = db::open(&args.db).context("Failed to open database")?;
    db::init_schema(&conn).context("Failed to initialize schema")?;

    match args.command {
        Commands::Ingest {
            file,
            format,
            name,
            overwrite,
            root,
        } => {
            let out = cli::cmd_ingest(
                &mut conn,
                &file,
                format.as_deref(),
                name.as_deref(),
                overwrite,
                root.as_deref(),
            )?;
            print!("{out}");
        }
        Commands::Summary => print!("{}", cli::cmd_summary(&conn)?),
        Commands::Reports => print!("{}", cli::cmd_reports(&conn)?),
        Commands::Files { sort_by_coverage } => {
            print!("{}", cli::cmd_files(&conn, sort_by_coverage)?)
        }
        Commands::Lines {
            source_file,
            uncovered,
        } => print!("{}", cli::cmd_lines(&conn, &source_file, uncovered)?),
        Commands::DiffCoverage {
            git_diff,
            path_prefix,
            style,
            comment,
        } => run_diff_coverage(&conn, git_diff, path_prefix, style, comment)?,
    }

    Ok(())
}

/// Orchestrates I/O (stdin / git / GitHub) then delegates to [`cli::cmd_diff_coverage`].
fn run_diff_coverage(
    conn: &rusqlite::Connection,
    git_diff: Option<String>,
    path_prefix: Option<String>,
    style: cli::Style,
    comment: bool,
) -> Result<()> {
    if comment {
        // GitHub mode: fetch diff from the API and post results as a PR comment
        let gh = GitHubDiff::from_env()?;
        let diff_text = gh.fetch_diff()?;
        let output =
            cli::cmd_diff_coverage(conn, &diff_text, path_prefix.as_deref(), &style, gh.sha())?;
        gh.post_comment(&output)?;
    } else {
        // Local mode: read diff from git or stdin
        let source: Box<dyn DiffSource> = if let Some(args) = git_diff {
            Box::new(GitDiff { args })
        } else {
            Box::new(StdinDiff)
        };
        let diff_text = source.fetch_diff()?;
        let output =
            cli::cmd_diff_coverage(conn, &diff_text, path_prefix.as_deref(), &style, None)?;
        print!("{output}");
    }

    Ok(())
}
