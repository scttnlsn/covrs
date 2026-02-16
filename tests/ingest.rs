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
        covrs::ingest::ingest(&mut conn, &lcov_path, None, None, false).unwrap();

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
        covrs::ingest::ingest(&mut conn, &xml_path, None, None, false).unwrap();

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
        covrs::ingest::ingest(&mut conn, &lcov_path, Some("lcov"), None, false).unwrap();

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
        covrs::ingest::ingest(&mut conn, &lcov_path, None, Some("my-report"), false).unwrap();

    assert_eq!(name, "my-report");

    let summary = covrs::db::get_summary(&conn, "my-report").unwrap();
    assert_eq!(summary.report_name, "my-report");
}

#[test]
fn ingest_unknown_format_fails() {
    let (mut conn, dir, _) = common::setup_db();

    let path = dir.path().join("random.dat");
    std::fs::write(&path, b"hello world this is not coverage data").unwrap();

    let result = covrs::ingest::ingest(&mut conn, &path, None, None, false);
    assert!(result.is_err());
}

#[test]
fn ingest_duplicate_name_fails() {
    let (mut conn, dir, _) = common::setup_db();

    let lcov_path = dir.path().join("coverage.lcov");
    std::fs::write(
        &lcov_path,
        b"SF:/src/lib.rs\nDA:1,1\nend_of_record\n",
    )
    .unwrap();

    // First ingest succeeds
    covrs::ingest::ingest(&mut conn, &lcov_path, None, Some("dup"), false).unwrap();

    // Second ingest with same name should fail without --overwrite
    let result = covrs::ingest::ingest(&mut conn, &lcov_path, None, Some("dup"), false);
    assert!(result.is_err());
    let err_msg = format!("{}", result.unwrap_err());
    assert!(err_msg.contains("already exists"), "Error: {}", err_msg);
}

#[test]
fn ingest_overwrite_replaces_report() {
    let (mut conn, dir, _) = common::setup_db();

    let lcov_path = dir.path().join("v1.lcov");
    std::fs::write(
        &lcov_path,
        b"SF:/src/lib.rs\nDA:1,1\nDA:2,0\nend_of_record\n",
    )
    .unwrap();

    covrs::ingest::ingest(&mut conn, &lcov_path, None, Some("report"), false).unwrap();

    let summary = covrs::db::get_summary(&conn, "report").unwrap();
    assert_eq!(summary.total_lines, 2);
    assert_eq!(summary.covered_lines, 1);

    // Now overwrite with different data (3 lines, all covered)
    let lcov_path2 = dir.path().join("v2.lcov");
    std::fs::write(
        &lcov_path2,
        b"SF:/src/lib.rs\nDA:1,5\nDA:2,3\nDA:3,1\nend_of_record\n",
    )
    .unwrap();

    covrs::ingest::ingest(&mut conn, &lcov_path2, None, Some("report"), true).unwrap();

    let summary = covrs::db::get_summary(&conn, "report").unwrap();
    assert_eq!(summary.total_lines, 3);
    assert_eq!(summary.covered_lines, 3);
}

#[test]
fn ingest_empty_coverage_file() {
    let (mut conn, dir, _) = common::setup_db();

    let lcov_path = dir.path().join("empty.lcov");
    std::fs::write(&lcov_path, b"TN:test\n").unwrap();

    // Should succeed (with a warning to stderr) but produce a report with 0 files
    let (report_id, _format, _name) =
        covrs::ingest::ingest(&mut conn, &lcov_path, None, Some("empty"), false).unwrap();
    assert!(report_id > 0);

    let summary = covrs::db::get_summary(&conn, "empty").unwrap();
    assert_eq!(summary.total_files, 0);
    assert_eq!(summary.total_lines, 0);
}
