mod common;

use covrs::parsers::Parser;

#[test]
fn ingest_and_query() {
    let (mut conn, _dir, _) = common::setup_db();

    let xml = include_bytes!("fixtures/coverage.xml");
    let data = covrs::parsers::cobertura::CoberturaParser.parse(xml).unwrap();

    let report_id = covrs::db::insert_coverage(
        &mut conn,
        "test-cobertura",
        "cobertura",
        Some("coverage.xml"),
        &data,
    )
    .unwrap();
    assert!(report_id > 0);

    // Summary
    let summary = covrs::db::get_summary(&conn, "test-cobertura").unwrap();
    assert_eq!(summary.report_name, "test-cobertura");
    assert_eq!(summary.source_format, "cobertura");
    assert!(summary.total_files > 0);
    assert!(summary.total_lines > 0);
    assert!(summary.covered_lines > 0);
    assert!(summary.covered_lines <= summary.total_lines);

    // File summaries
    let files = covrs::db::get_file_summaries(&conn, "test-cobertura").unwrap();
    assert!(!files.is_empty());

    // Reports list
    let reports = covrs::db::list_reports(&conn).unwrap();
    assert_eq!(reports.len(), 1);
    assert_eq!(reports[0].0, "test-cobertura");
}
