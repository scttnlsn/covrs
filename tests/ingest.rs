mod common;

use std::io::Write;
use std::path::Path;

use covrs::parsers::Format;

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
        covrs::ingest::ingest(&mut conn, &lcov_path, None, None, false, None).unwrap();

    assert!(report_id > 0);
    assert_eq!(format, Format::Lcov);
    assert_eq!(name, "coverage.lcov");

    let summary = covrs::db::get_summary(&conn).unwrap();
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
        covrs::ingest::ingest(&mut conn, &xml_path, None, None, false, None).unwrap();

    assert!(report_id > 0);
    assert_eq!(format, Format::Cobertura);

    let summary = covrs::db::get_summary(&conn).unwrap();
    assert!(summary.total_lines > 0);
    assert!(summary.total_files > 0);
}

#[test]
fn ingest_with_format_override() {
    let (mut conn, dir, _) = common::setup_db();

    // Write lcov content but with a .txt extension (won't auto-detect by extension)
    let lcov_path = dir.path().join("data.txt");
    std::fs::write(&lcov_path, b"SF:/src/lib.rs\nDA:1,1\nend_of_record\n").unwrap();

    let (_id, format, _name) =
        covrs::ingest::ingest(&mut conn, &lcov_path, Some("lcov"), None, false, None).unwrap();

    assert_eq!(format, Format::Lcov);
}

#[test]
fn ingest_with_custom_report_name() {
    let (mut conn, dir, _) = common::setup_db();

    let lcov_path = dir.path().join("coverage.lcov");
    std::fs::write(&lcov_path, b"SF:/src/lib.rs\nDA:1,1\nend_of_record\n").unwrap();

    let (_id, _format, name) =
        covrs::ingest::ingest(&mut conn, &lcov_path, None, Some("my-report"), false, None).unwrap();

    assert_eq!(name, "my-report");

    let reports = covrs::db::list_reports(&conn).unwrap();
    assert_eq!(reports[0].name, "my-report");
}

#[test]
fn ingest_unknown_format_fails() {
    let (mut conn, dir, _) = common::setup_db();

    let path = dir.path().join("random.dat");
    std::fs::write(&path, b"hello world this is not coverage data").unwrap();

    let result = covrs::ingest::ingest(&mut conn, &path, None, None, false, None);
    assert!(result.is_err());
}

#[test]
fn ingest_duplicate_name_fails() {
    let (mut conn, dir, _) = common::setup_db();

    let lcov_path = dir.path().join("coverage.lcov");
    std::fs::write(&lcov_path, b"SF:/src/lib.rs\nDA:1,1\nend_of_record\n").unwrap();

    // First ingest succeeds
    covrs::ingest::ingest(&mut conn, &lcov_path, None, Some("dup"), false, None).unwrap();

    // Second ingest with same name should fail without --overwrite
    let result = covrs::ingest::ingest(&mut conn, &lcov_path, None, Some("dup"), false, None);
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

    covrs::ingest::ingest(&mut conn, &lcov_path, None, Some("report"), false, None).unwrap();

    let summary = covrs::db::get_summary(&conn).unwrap();
    assert_eq!(summary.total_lines, 2);
    assert_eq!(summary.covered_lines, 1);

    // Now overwrite with different data (3 lines, all covered)
    let lcov_path2 = dir.path().join("v2.lcov");
    std::fs::write(
        &lcov_path2,
        b"SF:/src/lib.rs\nDA:1,5\nDA:2,3\nDA:3,1\nend_of_record\n",
    )
    .unwrap();

    covrs::ingest::ingest(&mut conn, &lcov_path2, None, Some("report"), true, None).unwrap();

    let summary = covrs::db::get_summary(&conn).unwrap();
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
        covrs::ingest::ingest(&mut conn, &lcov_path, None, Some("empty"), false, None).unwrap();
    assert!(report_id > 0);

    // Verify the report was created even though it has no coverage data
    let reports = covrs::db::list_reports(&conn).unwrap();
    assert_eq!(reports.len(), 1);
    assert_eq!(reports[0].name, "empty");
}

// ── Path normalization tests ───────────────────────────────────────────────

#[test]
fn ingest_strips_absolute_paths_with_root() {
    let (mut conn, dir, _) = common::setup_db();

    let lcov_path = dir.path().join("coverage.lcov");
    std::fs::write(
        &lcov_path,
        b"SF:/home/user/project/src/main.rs\nDA:1,5\nDA:2,0\nend_of_record\n\
          SF:/home/user/project/src/lib.rs\nDA:1,3\nend_of_record\n",
    )
    .unwrap();

    let root = Path::new("/home/user/project");
    covrs::ingest::ingest(&mut conn, &lcov_path, None, Some("test"), false, Some(root)).unwrap();

    let files = covrs::db::get_file_summaries(&conn).unwrap();
    let paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
    assert!(paths.contains(&"src/main.rs"), "paths: {paths:?}");
    assert!(paths.contains(&"src/lib.rs"), "paths: {paths:?}");
}

#[test]
fn ingest_leaves_relative_paths_unchanged() {
    let (mut conn, dir, _) = common::setup_db();

    let lcov_path = dir.path().join("coverage.lcov");
    std::fs::write(&lcov_path, b"SF:src/main.rs\nDA:1,5\nend_of_record\n").unwrap();

    let root = Path::new("/home/user/project");
    covrs::ingest::ingest(&mut conn, &lcov_path, None, Some("test"), false, Some(root)).unwrap();

    let files = covrs::db::get_file_summaries(&conn).unwrap();
    assert_eq!(files[0].path, "src/main.rs");
}

#[test]
fn ingest_leaves_absolute_paths_outside_root_unchanged() {
    let (mut conn, dir, _) = common::setup_db();

    let lcov_path = dir.path().join("coverage.lcov");
    std::fs::write(
        &lcov_path,
        b"SF:/other/place/lib.rs\nDA:1,1\nend_of_record\n",
    )
    .unwrap();

    let root = Path::new("/home/user/project");
    covrs::ingest::ingest(&mut conn, &lcov_path, None, Some("test"), false, Some(root)).unwrap();

    let files = covrs::db::get_file_summaries(&conn).unwrap();
    assert_eq!(files[0].path, "/other/place/lib.rs");
}

#[test]
fn ingest_no_root_skips_normalization() {
    let (mut conn, dir, _) = common::setup_db();

    let lcov_path = dir.path().join("coverage.lcov");
    std::fs::write(
        &lcov_path,
        b"SF:/absolute/path/main.rs\nDA:1,1\nend_of_record\n",
    )
    .unwrap();

    covrs::ingest::ingest(&mut conn, &lcov_path, None, Some("test"), false, None).unwrap();

    let files = covrs::db::get_file_summaries(&conn).unwrap();
    assert_eq!(files[0].path, "/absolute/path/main.rs");
}

#[test]
fn ingest_root_strips_cobertura_absolute_paths() {
    let (mut conn, dir, _) = common::setup_db();

    // Copy fixture (has <source>/home/user/project/src</source> + filename="main.py")
    let fixture = include_bytes!("fixtures/sample_cobertura.xml");
    let xml_path = dir.path().join("coverage.xml");
    let mut f = std::fs::File::create(&xml_path).unwrap();
    f.write_all(fixture).unwrap();

    let root = Path::new("/home/user/project");
    covrs::ingest::ingest(&mut conn, &xml_path, None, Some("test"), false, Some(root)).unwrap();

    let files = covrs::db::get_file_summaries(&conn).unwrap();
    let paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
    // /home/user/project/src/main.py → src/main.py
    assert!(paths.contains(&"src/main.py"), "paths: {paths:?}");
    assert!(paths.contains(&"src/util.py"), "paths: {paths:?}");
}
