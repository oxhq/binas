#![no_main]

use binas_pdf::{OpenOptions, PdfEngine};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let data = &data[..data.len().min(4096)];
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".as_slice(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".as_slice(),
        b"<< /Type /Page /Parent 2 0 R /Contents 4 0 R >>".as_slice(),
    ];
    let mut pdf = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (index, object) in objects.iter().enumerate() {
        offsets.push(pdf.len());
        pdf.extend_from_slice(format!("{} 0 obj\n", index + 1).as_bytes());
        pdf.extend_from_slice(object);
        pdf.extend_from_slice(b"\nendobj\n");
    }
    offsets.push(pdf.len());
    pdf.extend_from_slice(format!("4 0 obj\n<< /Length {} >>\nstream\n", data.len()).as_bytes());
    pdf.extend_from_slice(data);
    pdf.extend_from_slice(b"\nendstream\nendobj\n");
    let xref = pdf.len();
    pdf.extend_from_slice(b"xref\n0 5\n0000000000 65535 f \n");
    for offset in offsets {
        pdf.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    pdf.extend_from_slice(
        format!("trailer\n<< /Size 5 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes(),
    );
    if let Ok(document) = PdfEngine::default().open(&pdf, OpenOptions::default()) {
        let _ = document.query_text_all("fuzz");
    }
});
