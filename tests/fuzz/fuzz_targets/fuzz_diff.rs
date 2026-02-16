#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // Diff parser must not panic on any input.
    if let Ok(s) = std::str::from_utf8(data) {
        let _ = covrs::diff::parse_diff(s);
    }
});
