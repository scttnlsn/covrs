mod common;

use covrs::model::{CoverageData, FileCoverage, FunctionCoverage, LineCoverage};
use covrs::parsers::Parser;

#[test]
fn delete_report() {
    let (mut conn, _dir, _) = common::setup_db();

    let lcov = b"SF:/src/lib.rs\nDA:1,1\nend_of_record\n";
    let data = covrs::parsers::lcov::LcovParser.parse(lcov).unwrap();
    covrs::db::insert_coverage(&mut conn, "to-delete", "lcov", None, &data).unwrap();

    assert_eq!(covrs::db::list_reports(&conn).unwrap().len(), 1);
    covrs::db::delete_report(&mut conn, "to-delete").unwrap();
    assert_eq!(covrs::db::list_reports(&conn).unwrap().len(), 0);
}

#[test]
fn delete_nonexistent_report_fails() {
    let (mut conn, _dir, _) = common::setup_db();

    let result = covrs::db::delete_report(&mut conn, "does-not-exist");
    assert!(result.is_err());
}

#[test]
fn duplicate_report_name_fails() {
    let (mut conn, _dir, _) = common::setup_db();

    let lcov = b"SF:/src/lib.rs\nDA:1,1\nend_of_record\n";
    let data = covrs::parsers::lcov::LcovParser.parse(lcov).unwrap();

    covrs::db::insert_coverage(&mut conn, "dupe", "lcov", None, &data).unwrap();
    let result = covrs::db::insert_coverage(&mut conn, "dupe", "lcov", None, &data);
    assert!(result.is_err());
}

#[test]
fn function_coverage_null_start_line_dedup() {
    // Two functions with the same name but different start_lines (one NULL, one not)
    // should be stored as separate entries. Same name + same NULL start_line should
    // be deduplicated via upsert.
    let (mut conn, _dir, _) = common::setup_db();

    let mut data = CoverageData::new();
    let mut file = FileCoverage::new("/src/lib.rs".to_string());
    file.lines.push(LineCoverage { line_number: 1, hit_count: 1 });

    // Two functions: same name, one with start_line, one without
    file.functions.push(FunctionCoverage {
        name: "init".to_string(),
        start_line: Some(10),
        end_line: None,
        hit_count: 3,
    });
    file.functions.push(FunctionCoverage {
        name: "init".to_string(),
        start_line: None,
        end_line: None,
        hit_count: 5,
    });
    data.files.push(file);

    covrs::db::insert_coverage(&mut conn, "test-fn", "lcov", None, &data).unwrap();

    let summary = covrs::db::get_summary(&conn, "test-fn").unwrap();
    // Both functions should be stored (different start_lines)
    assert_eq!(summary.total_functions, 2);
    assert_eq!(summary.covered_functions, 2);
}

#[test]
fn get_summary_nonexistent_report_fails() {
    let (conn, _dir, _) = common::setup_db();

    let result = covrs::db::get_summary(&conn, "nope");
    assert!(result.is_err());
}

#[test]
fn latest_report_name() {
    let (mut conn, _dir, _) = common::setup_db();

    assert!(covrs::db::get_latest_report_name(&conn).unwrap().is_none());

    let lcov = b"SF:/src/lib.rs\nDA:1,1\nend_of_record\n";
    let data = covrs::parsers::lcov::LcovParser.parse(lcov).unwrap();
    covrs::db::insert_coverage(&mut conn, "first", "lcov", None, &data).unwrap();
    covrs::db::insert_coverage(&mut conn, "second", "lcov", None, &data).unwrap();

    let latest = covrs::db::get_latest_report_name(&conn).unwrap().unwrap();
    assert_eq!(latest, "second");
}
