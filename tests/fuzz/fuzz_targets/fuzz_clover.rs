#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // Parser must not panic on any input.
    let _ = covrs::parsers::clover::parse(data);
});
