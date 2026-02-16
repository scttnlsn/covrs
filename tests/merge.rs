mod common;

use covrs::model::{CoverageData, FileCoverage, FunctionCoverage, LineCoverage};
use covrs::parsers::Parser;

#[test]
fn merge_sums_hit_counts() {
    let (mut conn, _dir, _) = common::setup_db();

    let lcov_a = b"SF:/src/lib.rs\nDA:1,3\nDA:2,0\nDA:3,1\nend_of_record\n";
    let data_a = covrs::parsers::lcov::LcovParser.parse(lcov_a).unwrap();
    covrs::db::insert_coverage(&mut conn, "run-a", "lcov", None, &data_a).unwrap();

    let lcov_b = b"SF:/src/lib.rs\nDA:1,2\nDA:2,1\nDA:3,0\nend_of_record\n";
    let data_b = covrs::parsers::lcov::LcovParser.parse(lcov_b).unwrap();
    covrs::db::insert_coverage(&mut conn, "run-b", "lcov", None, &data_b).unwrap();

    covrs::db::merge_reports(&mut conn, "run-b", "merged").unwrap();
    covrs::db::merge_reports(&mut conn, "run-a", "merged").unwrap();

    let summary = covrs::db::get_summary(&conn, "merged").unwrap();
    assert_eq!(summary.total_lines, 3);
    assert_eq!(summary.covered_lines, 3); // all lines now covered

    let lines = covrs::db::get_lines(&conn, "merged", "/src/lib.rs").unwrap();
    assert_eq!(lines[0].hit_count, 5); // 3 + 2
    assert_eq!(lines[1].hit_count, 1); // 0 + 1
    assert_eq!(lines[2].hit_count, 1); // 1 + 0
}

#[test]
fn merge_function_coverage_with_null_start_line() {
    let (mut conn, _dir, _) = common::setup_db();

    // Report A: function "process" with no start_line, hit 2 times
    let mut data_a = CoverageData::new();
    let mut file_a = FileCoverage::new("/src/lib.rs".to_string());
    file_a.lines.push(LineCoverage { line_number: 1, hit_count: 1 });
    file_a.functions.push(FunctionCoverage {
        name: "process".to_string(),
        start_line: None,
        end_line: None,
        hit_count: 2,
    });
    data_a.files.push(file_a);
    covrs::db::insert_coverage(&mut conn, "fn-a", "lcov", None, &data_a).unwrap();

    // Report B: same function, hit 3 times
    let mut data_b = CoverageData::new();
    let mut file_b = FileCoverage::new("/src/lib.rs".to_string());
    file_b.lines.push(LineCoverage { line_number: 1, hit_count: 1 });
    file_b.functions.push(FunctionCoverage {
        name: "process".to_string(),
        start_line: None,
        end_line: None,
        hit_count: 3,
    });
    data_b.files.push(file_b);
    covrs::db::insert_coverage(&mut conn, "fn-b", "lcov", None, &data_b).unwrap();

    // Merge both into "fn-merged"
    covrs::db::merge_reports(&mut conn, "fn-a", "fn-merged").unwrap();
    covrs::db::merge_reports(&mut conn, "fn-b", "fn-merged").unwrap();

    let summary = covrs::db::get_summary(&conn, "fn-merged").unwrap();
    // Should be 1 merged function with hit_count = 2 + 3 = 5
    assert_eq!(summary.total_functions, 1);
    assert_eq!(summary.covered_functions, 1);
}
