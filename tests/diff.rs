mod common;

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
    let data = covrs::parsers::lcov::parse(lcov).unwrap();
    covrs::db::insert_coverage(&mut conn, "test", "lcov", None, &data, false).unwrap();

    // Parse the modified_file.diff fixture — adds lines 11, 12, 14 in src/main.rs
    let diff_text = include_str!("fixtures/diffs/modified_file.diff");
    let diff_lines = covrs::diff::parse_diff(diff_text);

    let (covered, total) = {
        let (_, c, t) = covrs::db::diff_coverage(&conn, &diff_lines).unwrap();
        (c, t)
    };
    // Line 11: hit_count=0 (not covered), line 12: hit_count=1 (covered), line 14: hit_count=0
    assert_eq!(total, 3);
    assert_eq!(covered, 1);
}

/// Diff coverage with manually constructed diff lines (lines not in coverage data are ignored).
#[test]
fn diff_coverage_ignores_non_instrumentable_lines() {
    let (mut conn, _dir, _) = common::setup_db();

    let lcov = b"SF:src/main.rs\nDA:1,1\nDA:2,0\nDA:3,1\nDA:4,0\nDA:5,1\nend_of_record\n";
    let data = covrs::parsers::lcov::parse(lcov).unwrap();
    covrs::db::insert_coverage(&mut conn, "test", "lcov", None, &data, false).unwrap();

    // Diff adds lines 2, 3, 4, and 10 (10 is not in coverage data at all)
    let mut diff_lines = std::collections::HashMap::new();
    diff_lines.insert("src/main.rs".to_string(), vec![2, 3, 4, 10]);

    let (covered, total) = {
        let (_, c, t) = covrs::db::diff_coverage(&conn, &diff_lines).unwrap();
        (c, t)
    };
    // Lines 2 (hit=0), 3 (hit=1), 4 (hit=0) are instrumentable. Line 10 is not.
    assert_eq!(total, 3);
    assert_eq!(covered, 1);
}

/// Single report should behave the same.
#[test]
fn diff_coverage_single_report() {
    let (mut conn, _dir, _) = common::setup_db();

    let lcov = b"SF:src/main.rs\nDA:1,1\nDA:2,0\nDA:3,1\nDA:4,0\nDA:5,1\nend_of_record\n";
    let data = covrs::parsers::lcov::parse(lcov).unwrap();
    covrs::db::insert_coverage(&mut conn, "test", "lcov", None, &data, false).unwrap();

    let mut diff_lines = std::collections::HashMap::new();
    diff_lines.insert("src/main.rs".to_string(), vec![2, 3, 4, 10]);

    let (covered, total) = {
        let (_, c, t) = covrs::db::diff_coverage(&conn, &diff_lines).unwrap();
        (c, t)
    };
    // Same result: lines 2 (hit=0), 3 (hit=1), 4 (hit=0) instrumentable, 10 not in data
    assert_eq!(total, 3);
    assert_eq!(covered, 1);
}

/// Aggregates across multiple reports using MAX(hit_count).
/// A line covered in ANY report should count as covered.
#[test]
fn diff_coverage_multiple_reports() {
    let (mut conn, _dir, _) = common::setup_db();

    // Report A: lines 1 covered, 2 not covered, 3 covered
    let lcov_a = b"SF:src/main.rs\nDA:1,1\nDA:2,0\nDA:3,1\nend_of_record\n";
    let data_a = covrs::parsers::lcov::parse(lcov_a).unwrap();
    covrs::db::insert_coverage(&mut conn, "report-a", "lcov", None, &data_a, false).unwrap();

    // Report B: lines 1 not covered, 2 covered, 3 not covered
    let lcov_b = b"SF:src/main.rs\nDA:1,0\nDA:2,1\nDA:3,0\nend_of_record\n";
    let data_b = covrs::parsers::lcov::parse(lcov_b).unwrap();
    covrs::db::insert_coverage(&mut conn, "report-b", "lcov", None, &data_b, false).unwrap();

    let mut diff_lines = std::collections::HashMap::new();
    diff_lines.insert("src/main.rs".to_string(), vec![1, 2, 3]);

    let (covered, total) = {
        let (_, c, t) = covrs::db::diff_coverage(&conn, &diff_lines).unwrap();
        (c, t)
    };
    // MAX across reports: line 1 -> max(1,0)=1, line 2 -> max(0,1)=1, line 3 -> max(1,0)=1
    assert_eq!(total, 3);
    assert_eq!(covered, 3);
}

/// Multiple files across multiple reports.
#[test]
fn diff_coverage_multiple_files() {
    let (mut conn, _dir, _) = common::setup_db();

    // Report A covers file1 but not file2
    let lcov_a = b"SF:src/file1.rs\nDA:1,1\nDA:2,0\nend_of_record\n\
                    SF:src/file2.rs\nDA:1,0\nDA:2,0\nend_of_record\n";
    let data_a = covrs::parsers::lcov::parse(lcov_a).unwrap();
    covrs::db::insert_coverage(&mut conn, "report-a", "lcov", None, &data_a, false).unwrap();

    // Report B covers file2 but not file1
    let lcov_b = b"SF:src/file1.rs\nDA:1,0\nDA:2,0\nend_of_record\n\
                    SF:src/file2.rs\nDA:1,1\nDA:2,1\nend_of_record\n";
    let data_b = covrs::parsers::lcov::parse(lcov_b).unwrap();
    covrs::db::insert_coverage(&mut conn, "report-b", "lcov", None, &data_b, false).unwrap();

    let mut diff_lines = std::collections::HashMap::new();
    diff_lines.insert("src/file1.rs".to_string(), vec![1, 2]);
    diff_lines.insert("src/file2.rs".to_string(), vec![1, 2]);

    let (covered, total) = {
        let (_, c, t) = covrs::db::diff_coverage(&conn, &diff_lines).unwrap();
        (c, t)
    };
    // file1: line 1 -> max(1,0)=1, line 2 -> max(0,0)=0
    // file2: line 1 -> max(0,1)=1, line 2 -> max(0,1)=1
    assert_eq!(total, 4);
    assert_eq!(covered, 3);
}

/// Unknown file contributes 0/0.
#[test]
fn diff_coverage_unknown_file() {
    let (mut conn, _dir, _) = common::setup_db();

    let lcov = b"SF:src/main.rs\nDA:1,1\nend_of_record\n";
    let data = covrs::parsers::lcov::parse(lcov).unwrap();
    covrs::db::insert_coverage(&mut conn, "test", "lcov", None, &data, false).unwrap();

    let mut diff_lines = std::collections::HashMap::new();
    diff_lines.insert("src/other.rs".to_string(), vec![1, 2, 3]);

    let (covered, total) = {
        let (_, c, t) = covrs::db::diff_coverage(&conn, &diff_lines).unwrap();
        (c, t)
    };
    assert_eq!(total, 0);
    assert_eq!(covered, 0);
}

/// Empty diff returns 0/0.
#[test]
fn diff_coverage_empty_diff() {
    let (mut conn, _dir, _) = common::setup_db();

    let lcov = b"SF:src/main.rs\nDA:1,1\nend_of_record\n";
    let data = covrs::parsers::lcov::parse(lcov).unwrap();
    covrs::db::insert_coverage(&mut conn, "test", "lcov", None, &data, false).unwrap();

    let diff_lines = std::collections::HashMap::new();

    let (covered, total) = {
        let (_, c, t) = covrs::db::diff_coverage(&conn, &diff_lines).unwrap();
        (c, t)
    };
    assert_eq!(total, 0);
    assert_eq!(covered, 0);
}
