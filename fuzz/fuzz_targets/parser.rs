#![no_main]

use binas_pdf::{OpenOptions, PdfEngine};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = PdfEngine::default().open(data, OpenOptions::default());
});
