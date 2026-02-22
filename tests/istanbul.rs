mod common;

#[test]
fn ingest_and_query() {
    let (mut conn, _dir, _) = common::setup_db();

    let input = br#"{
        "/src/app.js": {
            "statementMap": {
                "0": { "start": { "line": 1, "column": 0 }, "end": { "line": 1, "column": 30 } },
                "1": { "start": { "line": 2, "column": 0 }, "end": { "line": 2, "column": 20 } },
                "2": { "start": { "line": 3, "column": 0 }, "end": { "line": 3, "column": 15 } }
            },
            "s": { "0": 5, "1": 3, "2": 0 },
            "branchMap": {},
            "b": {},
            "fnMap": {},
            "f": {}
        }
    }"#;
    let data = covrs::parsers::istanbul::parse(input).unwrap();

    covrs::db::insert_coverage(&mut conn, "test-istanbul", "istanbul", None, &data, false).unwrap();

    let summary = covrs::db::get_summary(&conn).unwrap();
    assert_eq!(summary.total_lines, 3);
    assert_eq!(summary.covered_lines, 2);

    let lines = covrs::db::get_lines(&conn, "/src/app.js").unwrap();
    assert_eq!(lines.len(), 3);
    assert_eq!(lines[0].hit_count, 5); // line 1
    assert_eq!(lines[2].hit_count, 0); // line 3
}

#[test]
fn ingest_auto_detect() {
    let (mut conn, dir, _) = common::setup_db();

    let fixture = include_bytes!("fixtures/sample_istanbul.json");
    let json_path = dir.path().join("coverage-final.json");
    std::fs::write(&json_path, fixture).unwrap();

    let (report_id, format, _name) =
        covrs::ingest::ingest(&mut conn, &json_path, None, None, false, None).unwrap();

    assert!(report_id > 0);
    assert_eq!(format, covrs::parsers::Format::Istanbul);

    let summary = covrs::db::get_summary(&conn).unwrap();
    assert!(summary.total_lines > 0);
    assert!(summary.total_files > 0);
}
