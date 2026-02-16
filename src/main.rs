use std::path::PathBuf;
use std::process::Command;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

use covrs::{db, diff, ingest};

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

        /// Override format detection (cobertura, lcov).
        #[arg(long)]
        format: Option<String>,

        /// Name for this report (default: filename).
        #[arg(long)]
        name: Option<String>,

        /// Overwrite existing report with the same name.
        #[arg(long)]
        overwrite: bool,
    },

    /// Show a summary of a report.
    Summary {
        /// Report name to summarize. If omitted, summarizes the most recent.
        #[arg(long)]
        report: Option<String>,
    },

    /// List all reports in the database.
    Reports,

    /// List per-file coverage for a report.
    Files {
        /// Report name. If omitted, uses the most recent.
        #[arg(long)]
        report: Option<String>,

        /// Sort by coverage rate ascending (show worst files first).
        #[arg(long)]
        sort_by_coverage: bool,
    },

    /// Show line-level coverage for a source file.
    Lines {
        /// The source file path (as stored in the coverage data).
        source_file: String,

        /// Report name.
        #[arg(long)]
        report: Option<String>,
    },

    /// Show only uncovered lines for a source file.
    Uncovered {
        /// The source file path.
        source_file: String,

        /// Report name.
        #[arg(long)]
        report: Option<String>,
    },

    /// Merge one report into another (summing hit counts).
    Merge {
        /// Source report name.
        source: String,

        /// Target report name (will be created if it doesn't exist).
        #[arg(long)]
        into: String,
    },

    /// Delete a report from the database.
    Delete {
        /// Report name to delete.
        name: String,
    },

    /// Compute coverage for lines in a git diff (patch coverage).
    DiffCoverage {
        /// Report name.
        #[arg(long)]
        report: Option<String>,

        /// Git diff arguments, e.g. "HEAD~1" or "main..HEAD".
        /// If omitted, reads a unified diff from stdin.
        #[arg(long)]
        git_diff: Option<String>,

        /// Optional path prefix to prepend to diff paths for matching
        /// against coverage data paths.
        #[arg(long)]
        path_prefix: Option<String>,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    let mut conn = db::open(&cli.db).context("Failed to open database")?;
    db::init_schema(&conn).context("Failed to initialize schema")?;

    match cli.command {
        Commands::Ingest { file, format, name, overwrite } => {
            cmd_ingest(&mut conn, &file, format.as_deref(), name.as_deref(), overwrite)
        }
        Commands::Summary { report } => cmd_summary(&conn, report.as_deref()),
        Commands::Reports => cmd_reports(&conn),
        Commands::Files {
            report,
            sort_by_coverage,
        } => cmd_files(&conn, report.as_deref(), sort_by_coverage),
        Commands::Lines {
            source_file,
            report,
        } => cmd_lines(&conn, &source_file, report.as_deref()),
        Commands::Uncovered {
            source_file,
            report,
        } => cmd_uncovered(&conn, &source_file, report.as_deref()),
        Commands::Merge { source, into } => cmd_merge(&mut conn, &source, &into),
        Commands::Delete { name } => cmd_delete(&mut conn, &name),
        Commands::DiffCoverage {
            report,
            git_diff,
            path_prefix,
        } => cmd_diff_coverage(&conn, report.as_deref(), git_diff.as_deref(), path_prefix.as_deref()),
    }
}

fn resolve_report_name(conn: &rusqlite::Connection, name: Option<&str>) -> Result<String> {
    match name {
        Some(n) => Ok(n.to_string()),
        None => db::get_latest_report_name(conn)?
            .ok_or_else(|| anyhow::anyhow!("No reports found in database")),
    }
}

fn cmd_ingest(
    conn: &mut rusqlite::Connection,
    file: &std::path::Path,
    format: Option<&str>,
    name: Option<&str>,
    overwrite: bool,
) -> Result<()> {
    let (report_id, detected_format, actual_name) = ingest::ingest(conn, file, format, name, overwrite)?;
    println!(
        "Ingested {} as format '{}' → report id {} (name: '{}')",
        file.display(),
        detected_format,
        report_id,
        actual_name,
    );
    Ok(())
}

fn cmd_summary(conn: &rusqlite::Connection, report: Option<&str>) -> Result<()> {
    let name = resolve_report_name(conn, report)?;
    let summary = db::get_summary(conn, &name)?;

    println!("Report:     {}", summary.report_name);
    println!("Format:     {}", summary.source_format);
    println!("Created:    {}", summary.created_at);
    println!("Files:      {}", summary.total_files);
    println!(
        "Lines:      {}/{} ({:.1}%)",
        summary.covered_lines,
        summary.total_lines,
        summary.line_rate() * 100.0
    );
    if summary.total_branches > 0 {
        println!(
            "Branches:   {}/{} ({:.1}%)",
            summary.covered_branches,
            summary.total_branches,
            summary.branch_rate() * 100.0
        );
    }
    if summary.total_functions > 0 {
        println!(
            "Functions:  {}/{} ({:.1}%)",
            summary.covered_functions,
            summary.total_functions,
            summary.function_rate() * 100.0
        );
    }
    Ok(())
}

fn cmd_reports(conn: &rusqlite::Connection) -> Result<()> {
    let reports = db::list_reports(conn)?;
    if reports.is_empty() {
        println!("No reports in database.");
        return Ok(());
    }
    println!("{:<30} {:<15} CREATED", "NAME", "FORMAT");
    println!("{}", "-".repeat(70));
    for (name, format, created) in &reports {
        println!("{:<30} {:<15} {}", name, format, created);
    }
    Ok(())
}

fn cmd_files(
    conn: &rusqlite::Connection,
    report: Option<&str>,
    sort_by_coverage: bool,
) -> Result<()> {
    let name = resolve_report_name(conn, report)?;
    let mut files = db::get_file_summaries(conn, &name)?;

    if sort_by_coverage {
        files.sort_by(|a, b| a.line_rate().total_cmp(&b.line_rate()));
    }

    println!(
        "{:<60} {:>8} {:>8} {:>8}",
        "FILE", "LINES", "COVERED", "RATE"
    );
    println!("{}", "-".repeat(88));

    for f in &files {
        println!(
            "{:<60} {:>8} {:>8} {:>7.1}%",
            f.path,
            f.total_lines,
            f.covered_lines,
            f.line_rate() * 100.0
        );
    }

    Ok(())
}

fn cmd_lines(
    conn: &rusqlite::Connection,
    source_file: &str,
    report: Option<&str>,
) -> Result<()> {
    let name = resolve_report_name(conn, report)?;
    let lines = db::get_lines(conn, &name, source_file)?;

    if lines.is_empty() {
        println!("No coverage data for '{}'", source_file);
        return Ok(());
    }

    println!("{:>6}  {:>10}", "LINE", "HITS");
    println!("{}", "-".repeat(18));
    for line in &lines {
        let marker = if line.hit_count > 0 { "✓" } else { "✗" };
        println!(
            "{:>6}  {:>10}  {}",
            line.line_number, line.hit_count, marker
        );
    }
    Ok(())
}

fn cmd_uncovered(
    conn: &rusqlite::Connection,
    source_file: &str,
    report: Option<&str>,
) -> Result<()> {
    let name = resolve_report_name(conn, report)?;
    let lines = db::get_lines(conn, &name, source_file)?;

    let uncovered: Vec<_> = lines.iter().filter(|l| l.hit_count == 0).collect();

    if uncovered.is_empty() {
        println!("All instrumentable lines are covered in '{}'", source_file);
        return Ok(());
    }

    println!("Uncovered lines in '{}':", source_file);
    // Group into ranges for compact display
    let mut ranges: Vec<String> = Vec::new();
    let mut start: Option<u32> = None;
    let mut end: Option<u32> = None;

    for line in &uncovered {
        match (start, end) {
            (Some(_), Some(e)) if line.line_number == e + 1 => {
                end = Some(line.line_number);
            }
            (Some(s), Some(e)) => {
                if s == e {
                    ranges.push(format!("{}", s));
                } else {
                    ranges.push(format!("{}-{}", s, e));
                }
                start = Some(line.line_number);
                end = Some(line.line_number);
            }
            _ => {
                start = Some(line.line_number);
                end = Some(line.line_number);
            }
        }
    }
    if let (Some(s), Some(e)) = (start, end) {
        if s == e {
            ranges.push(format!("{}", s));
        } else {
            ranges.push(format!("{}-{}", s, e));
        }
    }

    println!("  {}", ranges.join(", "));
    println!("  ({} lines)", uncovered.len());
    Ok(())
}

fn cmd_merge(conn: &mut rusqlite::Connection, source: &str, target: &str) -> Result<()> {
    let target_id = db::merge_reports(conn, source, target)?;
    println!(
        "Merged '{}' into '{}' (report id {})",
        source, target, target_id
    );
    Ok(())
}

fn cmd_delete(conn: &mut rusqlite::Connection, name: &str) -> Result<()> {
    db::delete_report(conn, name)?;
    println!("Deleted report '{}'", name);
    Ok(())
}

fn cmd_diff_coverage(
    conn: &rusqlite::Connection,
    report: Option<&str>,
    git_diff: Option<&str>,
    path_prefix: Option<&str>,
) -> Result<()> {
    use std::collections::HashMap;
    use std::io::Read;

    let name = resolve_report_name(conn, report)?;

    // Get the diff text
    let diff_text = if let Some(diff_arg) = git_diff {
        // Run `git diff <args...>` to get the diff
        let diff_args: Vec<&str> = diff_arg.split_whitespace().collect();
        let output = Command::new("git")
            .arg("diff")
            .args(&diff_args)
            .output()
            .context("Failed to run git diff")?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("git diff failed: {}", stderr);
        }
        String::from_utf8(output.stdout).context("git diff output not valid UTF-8")?
    } else {
        // Read from stdin
        let mut buf = String::new();
        std::io::stdin()
            .read_to_string(&mut buf)
            .context("Failed to read diff from stdin")?;
        buf
    };

    let mut diff_lines = diff::parse_diff(&diff_text);

    // Apply path prefix if provided
    if let Some(prefix) = path_prefix {
        let prefix = prefix.trim_end_matches('/');
        let mut prefixed: HashMap<String, Vec<u32>> = HashMap::new();
        for (path, lines) in diff_lines {
            prefixed.insert(format!("{}/{}", prefix, path), lines);
        }
        diff_lines = prefixed;
    }

    if diff_lines.is_empty() {
        println!("No added lines found in diff.");
        return Ok(());
    }

    let total_diff_lines: usize = diff_lines.values().map(|v| v.len()).sum();
    let (covered, total) = db::diff_coverage(conn, &name, &diff_lines)?;

    println!("Patch coverage for report '{}':", name);
    println!("  Diff adds {} lines across {} files", total_diff_lines, diff_lines.len());
    println!(
        "  Of those, {} are instrumentable, {} are covered",
        total, covered
    );
    if total > 0 {
        let rate = covered as f64 / total as f64 * 100.0;
        println!("  Patch coverage: {:.1}%", rate);
    } else {
        println!("  No instrumentable lines in diff (nothing to cover)");
    }
    Ok(())
}
