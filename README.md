# covrs

Ingest multi-format code coverage reports into a unified SQLite store. Query and diff coverage data from a single database — no server required.

## Why

Coverage tools produce reports in different formats (LCOV, Cobertura, etc.) and each has its own tooling. **covrs** normalizes them all into one SQLite database so you can:

- Ingest multiple reports — coverage is automatically unioned across all of them
- Compute patch coverage against a git diff
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

| Format    | Extensions       | Auto-detected |
|-----------|------------------|---------------|
| LCOV      | `.info`, `.lcov` | ✓             |
| Cobertura | `.xml`           | ✓             |

Format detection works by checking file extensions first, then inspecting file content. You can always override with `--format`.

## Usage

### Ingest a coverage file

```
covrs ingest coverage.info
covrs ingest coverage.xml --format cobertura --name my-report
```

The format is auto-detected from the file extension and content. Use `--name` to assign a human-readable report name (defaults to the filename). Use `--overwrite` to replace an existing report with the same name.

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

### Patch coverage (diff coverage)

See what percentage of newly added lines are covered:

```
# From a git diff
covrs diff-coverage --git-diff "main..HEAD"

# From stdin
git diff main | covrs diff-coverage

# With a path prefix (if coverage paths don't match repo paths)
covrs diff-coverage --git-diff "HEAD~1" --path-prefix src
```

```
Patch coverage: 78.9% (30/38 lines covered)

  src/foo.rs  8/12 (66.7%)  missed: 4, 7-9, 15
  src/bar.rs  2/3 (66.7%)   missed: 22

Full repo line coverage: 85.0%
```

Use `--style markdown` to get the output as markdown (e.g. for piping
into other tools):

```
covrs diff-coverage --git-diff "main..HEAD" --style markdown
```

### GitHub PR comment

Post (or update) a patch-coverage comment directly on a pull request by
adding `--comment`:

```
covrs diff-coverage --style markdown --comment
```

This fetches the PR diff via the GitHub API, computes patch coverage, and
posts a comment showing the overall patch coverage percentage, a table of
files with missed lines, and an expandable detail section with the exact
line numbers. All required parameters are read from the standard GitHub
Actions environment variables (`GITHUB_TOKEN`, `GITHUB_REPOSITORY`,
`GITHUB_REF`, `GITHUB_SHA`).

## GitHub Action

covrs is available as a reusable GitHub Action. Run your tests and ingest
coverage first, then add the action to post the patch-coverage comment:

```yaml
- name: Patch coverage
  uses: scttnlsn/covrs@v1
```

#### Inputs

| Input         | Description                                      | Default     |
|---------------|--------------------------------------------------|-------------|
| `token`       | GitHub token for API access                      | `${{ github.token }}` |
| `db`          | Path to the covrs SQLite database                | `.covrs.db` |
| `path-prefix` | Prefix to prepend to diff paths for matching     |             |
| `version`     | covrs version to install (e.g. `0.1.0`)          | latest release |

#### Full example

```yaml
name: CI
on: pull_request

jobs:
  test:
    runs-on: ubuntu-latest
    permissions:
      pull-requests: write
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          components: llvm-tools
      - uses: taiki-e/install-action@cargo-llvm-cov

      # Run tests and generate coverage
      - run: cargo llvm-cov test --lcov --output-path coverage.lcov

      # Install covrs and ingest
      - run: cargo install covrs
      - run: covrs ingest coverage.lcov

      # Post the patch-coverage comment
      - uses: scttnlsn/covrs@v1
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
         SUM(CASE WHEN lc.hit_count > 0 THEN 1 ELSE 0 END) as covered
  FROM line_coverage lc
  JOIN source_file sf ON sf.id = lc.source_file_id
  GROUP BY sf.path
  ORDER BY covered * 1.0 / total
"
```
