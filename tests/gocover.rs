mod common;

#[test]
fn ingest_and_query() {
    let (mut conn, _dir, _) = common::setup_db();

    let input = b"mode: count\n\
        example.com/pkg/main.go:1.1,3.10 2 5\n\
        example.com/pkg/main.go:5.1,6.10 1 0\n";
    let data = covrs::parsers::gocover::parse(input).unwrap();

    covrs::db::insert_coverage(&mut conn, "test-gocover", "gocover", None, &data, false).unwrap();

    let summary = covrs::db::get_summary(&conn).unwrap();
    assert_eq!(summary.total_lines, 5);
    assert_eq!(summary.covered_lines, 3);

    let lines = covrs::db::get_lines(&conn, "example.com/pkg/main.go").unwrap();
    assert_eq!(lines.len(), 5);
    assert_eq!(lines[0].hit_count, 5);
    assert_eq!(lines[3].hit_count, 0); // line 5
}
