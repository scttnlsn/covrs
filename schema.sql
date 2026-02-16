CREATE TABLE IF NOT EXISTS schema_version (
    version INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS report (
    id            INTEGER PRIMARY KEY,
    name          TEXT NOT NULL,
    source_format TEXT NOT NULL,
    source_file   TEXT,
    created_at    TEXT NOT NULL,
    metadata      TEXT,
    UNIQUE(name)
);

CREATE TABLE IF NOT EXISTS source_file (
    id   INTEGER PRIMARY KEY,
    path TEXT NOT NULL,
    UNIQUE(path)
);

-- One row per instrumentable line per report.
-- Presence means the line is instrumentable; hit_count=0 means not executed.
CREATE TABLE IF NOT EXISTS line_coverage (
    report_id      INTEGER NOT NULL REFERENCES report(id) ON DELETE CASCADE,
    source_file_id INTEGER NOT NULL REFERENCES source_file(id),
    line_number    INTEGER NOT NULL,
    hit_count      INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (report_id, source_file_id, line_number)
) WITHOUT ROWID;

-- One row per branch arm per line.
CREATE TABLE IF NOT EXISTS branch_coverage (
    report_id      INTEGER NOT NULL REFERENCES report(id) ON DELETE CASCADE,
    source_file_id INTEGER NOT NULL REFERENCES source_file(id),
    line_number    INTEGER NOT NULL,
    branch_index   INTEGER NOT NULL DEFAULT 0,
    hit_count      INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (report_id, source_file_id, line_number, branch_index)
) WITHOUT ROWID;

-- Function/method-level coverage.
-- Uses a surrogate primary key so that start_line can be NULL (unknown)
-- without causing primary key collisions for functions with the same name.
CREATE TABLE IF NOT EXISTS function_coverage (
    id             INTEGER PRIMARY KEY,
    report_id      INTEGER NOT NULL REFERENCES report(id) ON DELETE CASCADE,
    source_file_id INTEGER NOT NULL REFERENCES source_file(id),
    name           TEXT NOT NULL,
    start_line     INTEGER,
    end_line       INTEGER,
    hit_count      INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX IF NOT EXISTS idx_line_coverage_report
    ON line_coverage(report_id);

CREATE INDEX IF NOT EXISTS idx_branch_coverage_report
    ON branch_coverage(report_id);

CREATE INDEX IF NOT EXISTS idx_function_coverage_report
    ON function_coverage(report_id);

-- Dedup index for function coverage. COALESCE maps NULL start_line to -1
-- so that the uniqueness constraint treats two NULLs as equal.
CREATE UNIQUE INDEX IF NOT EXISTS idx_function_coverage_unique
    ON function_coverage(report_id, source_file_id, name, COALESCE(start_line, -1));
