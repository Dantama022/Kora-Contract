#![no_main]

// libFuzzer target for secondary-market buy and cancel settlement invariants.
libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    kora_fuzz::targets::secondary_market::run(data);
});