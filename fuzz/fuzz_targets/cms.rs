#![no_main]

use binas_pdf::{OpenOptions, PdfEngine, inspect_signatures};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let data = &data[..data.len().min(4096)];
    let hex = data
        .iter()
        .map(|byte| format!("{byte:02X}"))
        .collect::<String>();
    let placeholder = "9999999999";
    let object = format!(
        "<< /Type /Sig /ByteRange [0 {placeholder} {placeholder} {placeholder}] /Contents <{hex}> >>"
    );
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        "<< /Type /Pages /Kids [] /Count 0 >>".to_owned(),
        object,
    ];
    let mut pdf = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (index, object) in objects.iter().enumerate() {
        offsets.push(pdf.len());
        pdf.extend_from_slice(format!("{} 0 obj\n{object}\nendobj\n", index + 1).as_bytes());
    }
    let xref = pdf.len();
    pdf.extend_from_slice(b"xref\n0 4\n0000000000 65535 f \n");
    for offset in offsets {
        pdf.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    pdf.extend_from_slice(
        format!("trailer\n<< /Size 4 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes(),
    );
    let marker = b"/Contents <";
    let contents = pdf
        .windows(marker.len())
        .position(|value| value == marker)
        .unwrap();
    let gap_start = contents + b"/Contents ".len();
    let gap_end = gap_start
        + pdf[gap_start..]
            .iter()
            .position(|byte| *byte == b'>')
            .unwrap()
        + 1;
    let values = [gap_start, gap_end, pdf.len() - gap_end];
    let mut search = 0;
    for value in values {
        let position = pdf[search..]
            .windows(placeholder.len())
            .position(|window| window == placeholder.as_bytes())
            .unwrap()
            + search;
        pdf[position..position + placeholder.len()]
            .copy_from_slice(format!("{value:010}").as_bytes());
        search = position + placeholder.len();
    }
    if let Ok(document) = PdfEngine::default().open(&pdf, OpenOptions::default()) {
        let _ = inspect_signatures(&document);
    }
});
