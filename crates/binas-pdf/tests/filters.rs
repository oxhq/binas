use binas_pdf::{DecodeParams, PdfErrorCode, PdfFilter, decode_filter_chain};

fn decode(input: &[u8], filter: PdfFilter, limit: usize) -> Result<Vec<u8>, binas_pdf::PdfError> {
    decode_filter_chain(input, &[filter], &[None], limit)
}

#[test]
fn decodes_filter_vectors_and_chains_in_order() {
    assert_eq!(
        decode(b"48656c6c6f>", PdfFilter::ASCIIHexDecode, 5).unwrap(),
        b"Hello"
    );
    assert_eq!(
        decode(b"87cURDZ~>", PdfFilter::ASCII85Decode, 5).unwrap(),
        b"Hello"
    );
    assert_eq!(
        decode(&[252, b'A', 128], PdfFilter::RunLengthDecode, 5).unwrap(),
        b"AAAAA"
    );

    let flate_hex = b"789cf348cdc9c90700058c01f5>";
    assert_eq!(
        decode_filter_chain(
            flate_hex,
            &[PdfFilter::ASCIIHexDecode, PdfFilter::FlateDecode],
            &[None, None],
            64,
        )
        .unwrap(),
        b"Hello"
    );
}

#[test]
fn decodes_lzw_and_predictors() {
    // Codes 256(clear), 65, 66, 258("AB"), 257(EOD), packed MSB-first at 9 bits.
    assert_eq!(
        decode(&[128, 16, 72, 80, 40, 8], PdfFilter::LzwDecode, 4).unwrap(),
        b"ABAB"
    );

    let params = DecodeParams {
        predictor: 12,
        columns: 3,
        ..DecodeParams::default()
    };
    // Two PNG Up rows: [1,2,3], then deltas [3,3,3] => [4,5,6].
    assert_eq!(
        decode_filter_chain(
            &[
                0x78, 0x9c, 0x63, 0x62, 0x64, 0x62, 0x66, 0x62, 0x66, 0x66, 0x06, 0x00, 0x00, 0x54,
                0x00, 0x14
            ],
            &[PdfFilter::FlateDecode],
            &[Some(params)],
            32,
        )
        .unwrap(),
        [1, 2, 3, 4, 5, 6]
    );

    for early_change in [0, 1] {
        let encoded = literal_lzw(300, early_change);
        let params = DecodeParams {
            early_change,
            ..DecodeParams::default()
        };
        assert_eq!(
            decode_filter_chain(&encoded, &[PdfFilter::LzwDecode], &[Some(params)], 300,).unwrap(),
            vec![0; 300]
        );
    }
}

fn literal_lzw(count: usize, early_change: u8) -> Vec<u8> {
    let mut output = Vec::new();
    let mut pending = 0_u32;
    let mut pending_bits = 0_u8;
    let mut next_code = 258_usize;
    let mut width = 9_u8;
    let mut previous = false;

    for code in std::iter::once(256)
        .chain(std::iter::repeat_n(0, count))
        .chain(std::iter::once(257))
    {
        pending = (pending << width) | code;
        pending_bits += width;
        while pending_bits >= 8 {
            pending_bits -= 8;
            output.push((pending >> pending_bits) as u8);
            pending &= (1 << pending_bits) - 1;
        }
        if code < 256 {
            if previous && next_code < 4096 {
                next_code += 1;
                if width < 12 && next_code >= (1 << width) - usize::from(early_change) {
                    width += 1;
                }
            }
            previous = true;
        }
    }
    if pending_bits > 0 {
        output.push((pending << (8 - pending_bits)) as u8);
    }
    output
}

#[test]
fn rejects_malformed_input_parameters_and_expansion() {
    for (input, filter) in [
        (&b"0g>"[..], PdfFilter::ASCIIHexDecode),
        (&b"!~>"[..], PdfFilter::ASCII85Decode),
        (&b"!!!!!"[..], PdfFilter::ASCII85Decode),
        (&[2, 1][..], PdfFilter::RunLengthDecode),
        (&[0xff, 0xff][..], PdfFilter::LzwDecode),
    ] {
        assert_eq!(
            decode(input, filter, 64).unwrap_err().code,
            PdfErrorCode::InvalidSyntax
        );
    }

    let mismatch = decode_filter_chain(b"", &[PdfFilter::FlateDecode], &[], 1).unwrap_err();
    assert_eq!(mismatch.code, PdfErrorCode::InvalidSyntax);

    let bad_params = DecodeParams {
        early_change: 2,
        ..DecodeParams::default()
    };
    assert_eq!(
        decode_filter_chain(b"", &[PdfFilter::LzwDecode], &[Some(bad_params)], 1)
            .unwrap_err()
            .code,
        PdfErrorCode::InvalidSyntax
    );

    assert_eq!(
        decode(&[129, b'A', 128], PdfFilter::RunLengthDecode, 1)
            .unwrap_err()
            .code,
        PdfErrorCode::ResourceLimit
    );
    assert_eq!(
        decode(
            &[
                0x78, 0x9c, 0xf3, 0x48, 0xcd, 0xc9, 0xc9, 0x07, 0x00, 0x05, 0x8c, 0x01, 0xf5,
            ],
            PdfFilter::FlateDecode,
            4,
        )
        .unwrap_err()
        .code,
        PdfErrorCode::ResourceLimit
    );
}

#[test]
fn unsupported_filters_fail_closed() {
    let error = decode(b"jpeg", PdfFilter::Unsupported("DCTDecode".into()), 16).unwrap_err();
    assert_eq!(error.code, PdfErrorCode::UnsupportedFeature);
}
