mod common;

use covrs::parsers::Parser;

#[test]
fn ingest_and_query() {
    let (mut conn, _dir, _) = common::setup_db();

    let lcov = b"TN:test\nSF:/src/main.rs\nDA:1,5\nDA:2,5\nDA:3,0\nLF:3\nLH:2\nend_of_record\n";
    let data = covrs::parsers::lcov::LcovParser.parse(lcov).unwrap();

    covrs::db::insert_coverage(&mut conn, "test-lcov", "lcov", None, &data, false).unwrap();

    let summary = covrs::db::get_summary(&conn).unwrap();
    assert_eq!(summary.total_lines, 3);
    assert_eq!(summary.covered_lines, 2);

    let lines = covrs::db::get_lines(&conn, "/src/main.rs").unwrap();
    assert_eq!(lines.len(), 3);
    assert_eq!(lines[0].hit_count, 5);
    assert_eq!(lines[2].hit_count, 0);
}
