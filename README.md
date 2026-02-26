# covrs

[![CI](https://github.com/scttnlsn/covrs/actions/workflows/ci.yml/badge.svg)](https://github.com/scttnlsn/covrs/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/covrs.svg)](https://crates.io/crates/covrs)
[![MIT licensed](https://img.shields.io/crates/l/covrs.svg)](https://github.com/scttnlsn/covrs/blob/main/LICENSE)

Code coverage ingestion and reporting.
Diff coverage, PR comments, and line annotations for your CI — add this step to your GitHub actions workflow:

```yaml
# ...run your tests and generate a coverage file...

- name: Coverage report
  uses: scttnlsn/covrs@v0
  with:
    coverage-files: coverage.lcov  # coverage output from your tests
    annotate: true
```

**[Demo](https://github.com/scttnlsn/covrs/pull/3)**

Supports many common coverage formats, normalizes them into a single SQLite database, and unions coverage across multiple reports automatically. Much of what Codecov/Coveralls offers — no server required.

## GitHub Action

The action installs covrs, ingests your coverage files, and posts a diff-coverage comment on the pull request.

> **Important:** Your job needs these permissions:
>
> ```yaml
> permissions:
>   pull-requests: write   # required for PR comments
>   checks: write          # required for line annotations (annotate: true)
> ```

#### Inputs

| Input            | Description                                      | Default     |
|------------------|--------------------------------------------------|-------------|
| `token`          | GitHub token for API access                      | `${{ github.token }}` |
| `coverage-files` | Coverage file(s) to ingest (space or newline separated) | *required* |
| `db`             | Path to the covrs SQLite database                | `.covrs.db` |
| `root`           | Project root for making coverage paths relative  | current directory |
| `path-prefix`    | Prefix to prepend to diff paths for matching     |             |
| `annotate`       | Add line annotations to a check run for uncovered lines | `false` |
| `version`        | covrs version to install (e.g. `0.1.0`)          | latest release |

#### Full example

```yaml
name: CI
on: pull_request

jobs:
  test:
    runs-on: ubuntu-latest
    permissions:
      pull-requests: write
      checks: write          # required for annotate
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          components: llvm-tools
      - uses: taiki-e/install-action@cargo-llvm-cov

      # Run tests and generate coverage
      - run: cargo llvm-cov test --lcov --output-path coverage.lcov

      # Ingest, post comment, and annotate uncovered lines
      - uses: scttnlsn/covrs@v0
        with:
          coverage-files: coverage.lcov
          annotate: true
```

## Supported Formats

| Format    | Extensions                  | `--format` value | Auto-detected |
|-----------|------------------------------|------------------|---------------|
| LCOV      | `.info`, `.lcov`             | `lcov`           | yes           |
| Clover    | `.xml`                       | `clover`         | yes           |
| Cobertura | `.xml`                       | `cobertura`      | yes           |
| JaCoCo    | `.xml`                       | `jacoco`         | yes           |
| Istanbul  | `coverage-final.json`        | `istanbul`       | yes           |
| Go        | `.coverprofile`, `.gocov`    | `gocover`        | yes           |

Format detection works by checking file extensions first, then inspecting file content. You can always override with `--format <value>`.

## CLI Usage

covrs can also be used as a standalone CLI tool (see [Install](#install)). Run `covrs --help` or `covrs <subcommand> --help` to see all available flags and options.

### Ingest a coverage file

```
covrs ingest coverage.info
covrs ingest coverage.xml --format cobertura --name my-report
```

The format is auto-detected from the file extension and content. Use `--name` to assign a human-readable report name (defaults to the filename). Use `--overwrite` to replace an existing report with the same name.

Absolute paths from coverage files (e.g., `/home/user/project/src/main.rs`) are automatically made relative to the current working directory during ingestion. Use `--root` to specify a different project root:

```
covrs ingest coverage.info --root /path/to/project
```

Ingesting multiple files builds up a combined view — all queries automatically union coverage across every report in the database. A line is considered covered if *any* report has a hit for it.

### List reports

```
covrs reports
```

```
NAME                           FORMAT          CREATED
----------------------------------------------------------------------
coverage.info                  lcov            2025-01-15T10:30:00+00:00
my-report                      cobertura       2025-01-15T11:00:00+00:00
```

### View a summary

```
covrs summary
```

```
Files:      42
Lines:      1250/1800 (69.4%)
Branches:   300/500 (60.0%)
Functions:  85/100 (85.0%)
```

### Per-file breakdown

```
covrs files
covrs files --sort-by-coverage       # worst-covered files first
```

### Line-level detail

```
covrs lines src/main.rs
```

Shows every instrumentable line with its hit count and a ✓/✗ marker.

Use `--uncovered` to show only uncovered lines as compact ranges:

```
covrs lines src/main.rs --uncovered
```

```
Uncovered lines in 'src/main.rs':
  15-18, 42, 55-60
  (9 lines)
```

### Diff coverage

See what percentage of newly added lines are covered:

```
# From a git diff
covrs diff-coverage --git-diff "main..HEAD"

# From stdin
git diff main | covrs diff-coverage

# With a path prefix (if coverage paths use a different relative root)
covrs diff-coverage --git-diff "HEAD~1" --path-prefix src
```

```
Diff coverage: 78.9% (30/38 lines covered)

  src/foo.rs  8/12 (66.7%)  missed: 4, 7-9, 15
  src/bar.rs  2/3 (66.7%)   missed: 22

Full project coverage: 85.0%
```

> **Note:** Diff coverage reports on line coverage only. Branch and function coverage are available via the `summary` and `files` commands.

Use `--style markdown` to get the output as markdown (e.g. for piping
into other tools):

```
covrs diff-coverage --git-diff "main..HEAD" --style markdown
```

### GitHub PR comment

Post (or update) a diff-coverage comment directly on a pull request by
adding `--comment`:

```
covrs diff-coverage --style markdown --comment
```

This fetches the PR diff via the GitHub API, computes diff coverage, and
posts a comment showing the overall diff coverage percentage, a table of
files with missed lines, and an expandable detail section with the exact
line numbers. All required parameters are read from the standard GitHub
Actions environment variables (`GITHUB_TOKEN`, `GITHUB_REPOSITORY`,
`GITHUB_REF`, `GITHUB_SHA`).

### GitHub line annotations

Add inline annotations to uncovered lines on a pull request using
`--annotate`:

```
covrs diff-coverage --annotate
```

This creates a GitHub check run named "covrs" with warning-level
annotations on every uncovered line in the diff. The annotations appear
inline in the "Files changed" tab of the pull request. Can be combined
with `--comment` to post both a summary comment and line annotations:

```
covrs diff-coverage --style markdown --comment --annotate
```

The same environment variables are required as `--comment`. The check run
finishes with a `neutral` conclusion so it never blocks merges.
Annotations are submitted in batches of 50 (the GitHub API limit per
request).

### Global options

| Flag   | Description                        | Default    |
|--------|------------------------------------|------------|
| `--db` | Path to the SQLite database file   | `.covrs.db` |

All subcommands accept `--db` to point at a specific database:

```
covrs --db /tmp/ci-coverage.db ingest coverage.xml
covrs --db /tmp/ci-coverage.db summary
```

## Database

covrs uses SQLite with WAL mode for fast concurrent reads. The schema (see [`schema.sql`](schema.sql)) stores:

- **report** — metadata for each ingested report
- **source_file** — deduplicated source file paths
- **line_coverage** — per-line hit counts (one row per instrumentable line per report)
- **branch_coverage** — per-branch-arm hit counts
- **function_coverage** — function/method-level hit counts

You can query the database directly:

```sql
sqlite3 .covrs.db "
  SELECT sf.path, COUNT(*) as total,
         SUM(CASE WHEN lc.max_hits > 0 THEN 1 ELSE 0 END) as covered
  FROM (
      SELECT source_file_id, line_number, MAX(hit_count) as max_hits
      FROM line_coverage
      GROUP BY source_file_id, line_number
  ) lc
  JOIN source_file sf ON sf.id = lc.source_file_id
  GROUP BY sf.path
  ORDER BY covered * 1.0 / total
"
```

## Install

Prebuilt binaries for Linux (x86_64, ARM64) and macOS (Intel, Apple Silicon) are published with each [GitHub release](https://github.com/scttnlsn/covrs/releases). Windows is not currently supported.

From [crates.io](https://crates.io/crates/covrs):

```
cargo install covrs
```

From a local checkout:

```
cargo install --path .
```
