# covrs

Ingest multi-format code coverage reports into a unified SQLite store. Query, compare, merge, and diff coverage data from a single database — no server required.

## Why

Coverage tools produce reports in different formats (LCOV, Cobertura, etc.) and each has its own tooling. **covrs** normalizes them all into one SQLite database so you can:

- Store multiple reports side by side and compare them
- Merge coverage from parallel test runs
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

The format is auto-detected from the file extension and content. Use `--name` to assign a human-readable report name (defaults to the filename).

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
covrs summary --report my-report
```

```
Report:     my-report
Format:     cobertura
Created:    2025-01-15T11:00:00+00:00
Files:      42
Lines:      1250/1800 (69.4%)
Branches:   300/500 (60.0%)
Functions:  85/100 (85.0%)
```

If `--report` is omitted, the most recent report is used.

### Per-file breakdown

```
covrs files --report my-report
covrs files --sort-by-coverage       # worst-covered files first
```

### Line-level detail

```
covrs lines src/main.rs --report my-report
```

Shows every instrumentable line with its hit count and a ✓/✗ marker.

### Show uncovered lines

```
covrs uncovered src/main.rs
```

Outputs compact line ranges:

```
Uncovered lines in 'src/main.rs':
  15-18, 42, 55-60
  (9 lines)
```

### Merge reports

Combine coverage from multiple test runs by summing hit counts:

```
covrs merge unit-tests --into combined
covrs merge integration-tests --into combined
```

### Delete a report

```
covrs delete old-report
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
Patch coverage for report 'coverage.info':
  Diff adds 45 lines across 3 files
  Of those, 38 are instrumentable, 30 are covered
  Patch coverage: 78.9%
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
  JOIN report r ON r.id = lc.report_id
  WHERE r.name = 'my-report'
  GROUP BY sf.path
  ORDER BY covered * 1.0 / total
"
```

