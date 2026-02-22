# covrs

Ingest multi-format code coverage reports into a unified SQLite store. Query and diff coverage data from a single database — no server required. Post coverage results as comments on GitHub pull requests with a single command or [reusable Action](#github-action).

Here's a [demo](https://github.com/scttnlsn/covrs/pull/3).

## Why

Coverage tools produce reports in different formats (LCOV, Clover, Cobertura, JaCoCo, etc.) and each has its own tooling. **covrs** normalizes them all into one SQLite database so you can:

- Ingest multiple reports — coverage is automatically unioned across all of them
- Compute diff coverage against a git diff
- Query coverage data with plain SQL if you need to

## Install

From [crates.io](https://crates.io/crates/covrs):

```
cargo install covrs
```

From a local checkout:

```
cargo install --path .
```

## Supported Formats

| Format    | Extensions                  | Auto-detected |
|-----------|------------------------------|---------------|
| LCOV      | `.info`, `.lcov`             | ✓             |
| Clover    | `.xml`                       | ✓             |
| Cobertura | `.xml`                       | ✓             |
| JaCoCo    | `.xml`                       | ✓             |
| Istanbul  | `coverage-final.json`        | ✓             |
| Go        | `.coverprofile`, `.gocov`    | ✓             |

Format detection works by checking file extensions first, then inspecting file content. You can always override with `--format`.

## Usage

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

## GitHub Action

covrs is available as a reusable GitHub Action. It installs covrs,
optionally ingests coverage files, and posts a diff-coverage comment on
the pull request:

```yaml
- name: Coverage report
  uses: scttnlsn/covrs@v0
  with:
    coverage-files: coverage.lcov
    annotate: true
```

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

## Global Options

| Flag   | Description                        | Default    |
|--------|------------------------------------|------------|
| `--db` | Path to the SQLite database file   | `.covrs.db` |

All subcommands accept `--db` to point at a specific database:

```
covrs --db /tmp/ci-coverage.db ingest coverage.xml
covrs --db /tmp/ci-coverage.db summary
```

## Database

covrs uses SQLite with WAL mode for fast concurrent reads. The schema stores:

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
