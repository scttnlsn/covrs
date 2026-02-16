mod common;

use std::io::Write;

/// Test the full `ingest::ingest()` pipeline: read file from disk, auto-detect format, insert.
#[test]
fn ingest_lcov_file_auto_detect() {
    let (mut conn, dir, _) = common::setup_db();

    let lcov_path = dir.path().join("coverage.lcov");
    std::fs::write(
        &lcov_path,
        b"SF:/src/lib.rs\nDA:1,3\nDA:2,0\nend_of_record\n",
    )
    .unwrap();

    let (report_id, format, name) =
        covrs::ingest::ingest(&mut conn, &lcov_path, None, None).unwrap();

    assert!(report_id > 0);
    assert_eq!(format, covrs::detect::Format::Lcov);
    assert_eq!(name, "coverage.lcov");

    let summary = covrs::db::get_summary(&conn, "coverage.lcov").unwrap();
    assert_eq!(summary.total_lines, 2);
    assert_eq!(summary.covered_lines, 1);
}

#[test]
fn ingest_cobertura_file_auto_detect() {
    let (mut conn, dir, _) = common::setup_db();

    // Copy fixture to temp dir with .xml extension
    let fixture = include_bytes!("fixtures/sample_cobertura.xml");
    let xml_path = dir.path().join("coverage.xml");
    let mut f = std::fs::File::create(&xml_path).unwrap();
    f.write_all(fixture).unwrap();

    let (report_id, format, _name) =
        covrs::ingest::ingest(&mut conn, &xml_path, None, None).unwrap();

    assert!(report_id > 0);
    assert_eq!(format, covrs::detect::Format::Cobertura);

    let summary = covrs::db::get_summary(&conn, "coverage.xml").unwrap();
    assert!(summary.total_lines > 0);
    assert!(summary.total_files > 0);
}

#[test]
fn ingest_with_format_override() {
    let (mut conn, dir, _) = common::setup_db();

    // Write lcov content but with a .txt extension (won't auto-detect by extension)
    let lcov_path = dir.path().join("data.txt");
    std::fs::write(
        &lcov_path,
        b"SF:/src/lib.rs\nDA:1,1\nend_of_record\n",
    )
    .unwrap();

    let (_id, format, _name) =
        covrs::ingest::ingest(&mut conn, &lcov_path, Some("lcov"), None).unwrap();

    assert_eq!(format, covrs::detect::Format::Lcov);
}

#[test]
fn ingest_with_custom_report_name() {
    let (mut conn, dir, _) = common::setup_db();

    let lcov_path = dir.path().join("coverage.lcov");
    std::fs::write(
        &lcov_path,
        b"SF:/src/lib.rs\nDA:1,1\nend_of_record\n",
    )
    .unwrap();

    let (_id, _format, name) =
        covrs::ingest::ingest(&mut conn, &lcov_path, None, Some("my-report")).unwrap();

    assert_eq!(name, "my-report");

    let summary = covrs::db::get_summary(&conn, "my-report").unwrap();
    assert_eq!(summary.report_name, "my-report");
}

#[test]
fn ingest_unknown_format_fails() {
    let (mut conn, dir, _) = common::setup_db();

    let path = dir.path().join("random.dat");
    std::fs::write(&path, b"hello world this is not coverage data").unwrap();

    let result = covrs::ingest::ingest(&mut conn, &path, None, None);
    assert!(result.is_err());
}
