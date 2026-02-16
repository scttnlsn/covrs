mod common;

use covrs::parsers::Parser;

/// End-to-end: parse a real diff file, ingest coverage, compute diff coverage.
#[test]
fn diff_coverage_end_to_end() {
    let (mut conn, _dir, _) = common::setup_db();

    // Ingest coverage that covers src/main.rs lines 1-15
    let lcov = b"\
SF:src/main.rs\n\
DA:1,1\n\
DA:2,1\n\
DA:3,1\n\
DA:4,0\n\
DA:5,1\n\
DA:6,1\n\
DA:7,1\n\
DA:8,0\n\
DA:9,1\n\
DA:10,1\n\
DA:11,0\n\
DA:12,1\n\
DA:13,1\n\
DA:14,0\n\
DA:15,1\n\
end_of_record\n";
    let data = covrs::parsers::lcov::LcovParser.parse(lcov).unwrap();
    covrs::db::insert_coverage(&mut conn, "test", "lcov", None, &data).unwrap();

    // Parse the modified_file.diff fixture — adds lines 11, 12, 14 in src/main.rs
    let diff_text = include_str!("fixtures/diffs/modified_file.diff");
    let diff_lines = covrs::diff::parse_diff(diff_text);

    let (covered, total) = covrs::db::diff_coverage(&conn, "test", &diff_lines).unwrap();
    // Line 11: hit_count=0 (not covered), line 12: hit_count=1 (covered), line 14: hit_count=0
    assert_eq!(total, 3);
    assert_eq!(covered, 1);
}

/// Diff coverage with manually constructed diff lines (lines not in coverage data are ignored).
#[test]
fn diff_coverage_ignores_non_instrumentable_lines() {
    let (mut conn, _dir, _) = common::setup_db();

    let lcov = b"SF:src/main.rs\nDA:1,1\nDA:2,0\nDA:3,1\nDA:4,0\nDA:5,1\nend_of_record\n";
    let data = covrs::parsers::lcov::LcovParser.parse(lcov).unwrap();
    covrs::db::insert_coverage(&mut conn, "test", "lcov", None, &data).unwrap();

    // Diff adds lines 2, 3, 4, and 10 (10 is not in coverage data at all)
    let mut diff_lines = std::collections::HashMap::new();
    diff_lines.insert("src/main.rs".to_string(), vec![2, 3, 4, 10]);

    let (covered, total) = covrs::db::diff_coverage(&conn, "test", &diff_lines).unwrap();
    // Lines 2 (hit=0), 3 (hit=1), 4 (hit=0) are instrumentable. Line 10 is not.
    assert_eq!(total, 3);
    assert_eq!(covered, 1);
}

/// Diff referencing a file not in coverage data should contribute 0/0.
#[test]
fn diff_coverage_unknown_file() {
    let (mut conn, _dir, _) = common::setup_db();

    let lcov = b"SF:src/main.rs\nDA:1,1\nend_of_record\n";
    let data = covrs::parsers::lcov::LcovParser.parse(lcov).unwrap();
    covrs::db::insert_coverage(&mut conn, "test", "lcov", None, &data).unwrap();

    let mut diff_lines = std::collections::HashMap::new();
    diff_lines.insert("src/other.rs".to_string(), vec![1, 2, 3]);

    let (covered, total) = covrs::db::diff_coverage(&conn, "test", &diff_lines).unwrap();
    assert_eq!(total, 0);
    assert_eq!(covered, 0);
}
