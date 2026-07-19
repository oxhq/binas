#![no_main]

use binas_pdf::{Limits, ToUnicodeCMap};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(cmap) = ToUnicodeCMap::parse(data, &Limits::default()) {
        let _ = cmap.mapping(data.get(..data.len().min(4)).unwrap_or_default());
    }
});
