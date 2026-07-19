use std::io::Write;

use binas_pdf::{
    EncodedImageReplacementRequest, EngineConfig, ImageColorSpace, ImageDecodeParams, ImageFilter,
    ImageMaskPolicy, ImageReplacementRequest, Limits, OpenOptions, PdfEngine, PdfErrorCode,
    PdfFilter, RawFlateImageSamples, decode_filter_chain, list_image_xobjects,
    read_jpeg_xobject_bytes, read_jpx_xobject_bytes, read_raw_flate_image_samples,
};
use flate2::{Compression, write::ZlibEncoder};

fn pdf(mask: bool, signed: bool) -> Vec<u8> {
    pdf_with_image_entries(mask, signed, "")
}

fn pdf_with_image_entries(mask: bool, signed: bool, image_entries: &str) -> Vec<u8> {
    let mask_entry = if mask { " /SMask 6 0 R" } else { "" };
    let mut objects = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /Resources << /XObject << /Im0 5 0 R >> >> /Contents 4 0 R /MediaBox [0 0 20 20] >>".to_vec(),
        stream("", b"q 20 0 0 20 0 0 cm /Im0 Do Q"),
        stream(
            &format!(" /Type /XObject /Subtype /Image /Width 2 /Height 1 /BitsPerComponent 8 /ColorSpace /DeviceRGB{mask_entry}{image_entries}"),
            &[1, 2, 3, 4, 5, 6],
        ),
    ];
    if mask {
        objects.push(stream(
            " /Type /XObject /Subtype /Image /Width 2 /Height 1 /BitsPerComponent 8 /ColorSpace /DeviceGray",
            &[0, 255],
        ));
    }
    if signed {
        objects.push(b"<< /Type /Sig /ByteRange [0 1 2 3] /Contents (x) >>".to_vec());
    }
    classic(&objects)
}

fn stream(entries: &str, bytes: &[u8]) -> Vec<u8> {
    [
        format!("<< /Length {}{entries} >>\nstream\n", bytes.len()).into_bytes(),
        bytes.to_vec(),
        b"\nendstream".to_vec(),
    ]
    .concat()
}

fn classic(objects: &[Vec<u8>]) -> Vec<u8> {
    let mut bytes = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (index, object) in objects.iter().enumerate() {
        offsets.push(bytes.len());
        bytes.extend_from_slice(format!("{} 0 obj\n", index + 1).as_bytes());
        bytes.extend_from_slice(object);
        bytes.extend_from_slice(b"\nendobj\n");
    }
    let xref = bytes.len();
    bytes.extend_from_slice(
        format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
    );
    for offset in offsets {
        bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    bytes.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    bytes
}

fn open(input: &[u8]) -> binas_pdf::PdfDocument {
    PdfEngine::default()
        .open(input, OpenOptions::default())
        .unwrap()
}

fn request(filter: ImageFilter, bytes: Vec<u8>) -> ImageReplacementRequest {
    ImageReplacementRequest {
        object_number: 5,
        object_generation: 0,
        encoded_bytes: bytes,
        width: 2,
        height: 1,
        bits_per_component: 8,
        color_space: ImageColorSpace::DeviceRgb,
        filter,
        decode_params: None,
        mask_policy: ImageMaskPolicy::Reject,
    }
}

#[test]
fn replaces_raw_and_flate_images_at_the_existing_reference() {
    let document = open(&pdf(false, false));
    let raw = document
        .replace_image_xobject(request(ImageFilter::Raw, vec![6, 5, 4, 3, 2, 1]))
        .unwrap();
    assert!(raw.verification.passed);
    assert!(raw.verification.object_reference_preserved);
    assert_eq!(raw.report.object_number, 5);
    assert!(
        raw.bytes
            .windows(b"/Subtype /Image".len())
            .any(|value| value == b"/Subtype /Image")
    );

    let samples = vec![10, 20, 30, 40, 50, 60];
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(&samples).unwrap();
    let encoded = encoder.finish().unwrap();
    let mut flate = request(ImageFilter::Flate, encoded);
    flate.decode_params = Some(ImageDecodeParams {
        predictor: 1,
        colors: 3,
        bits_per_component: 8,
        columns: 2,
    });
    let flate = document.replace_image_xobject(flate).unwrap();
    assert!(flate.verification.encoded_stream_matches);
    assert!(
        flate
            .bytes
            .windows(12)
            .any(|value| value == b"/FlateDecode")
    );
}

#[test]
fn accepts_dimension_checked_jpeg_and_jpx_pass_through() {
    let document = open(&pdf(false, false));
    for (filter, bytes) in [(ImageFilter::Jpeg, jpeg()), (ImageFilter::Jpx, jpx())] {
        let outcome = document
            .replace_image_xobject(request(filter, bytes))
            .unwrap();
        assert!(outcome.verification.passed);
        assert_eq!(outcome.report.filter, filter);
    }
}

#[test]
fn infers_encoded_image_metadata_for_jpeg_and_jpx() {
    let document = open(&pdf(false, false));
    for (filter, bytes) in [(ImageFilter::Jpeg, jpeg()), (ImageFilter::Jpx, jpx())] {
        let outcome = document
            .replace_image_xobject_encoded(EncodedImageReplacementRequest {
                object_number: 5,
                object_generation: 0,
                encoded_bytes: bytes,
                mask_policy: ImageMaskPolicy::Reject,
            })
            .unwrap();
        assert!(outcome.verification.passed);
        assert_eq!(outcome.report.filter, filter);
        assert_eq!((outcome.report.width, outcome.report.height), (2, 1));
    }
}

#[test]
fn reads_exact_dct_image_inventory_entry_as_opaque_jpeg_bytes() {
    let bytes = jpeg();
    let document = jpeg_xobject(2, 1, "/DCTDecode", "", &bytes);
    let entry = list_image_xobjects(&document).unwrap().remove(0);

    assert_eq!(read_jpeg_xobject_bytes(&document, &entry).unwrap(), bytes);

    let mut stale = entry.clone();
    stale.width = 3;
    assert_eq!(
        read_jpeg_xobject_bytes(&document, &stale).unwrap_err().code,
        PdfErrorCode::SelectionNotFound
    );

    let mut non_image = entry;
    non_image.object.object_number = 1;
    assert_eq!(
        read_jpeg_xobject_bytes(&document, &non_image)
            .unwrap_err()
            .code,
        PdfErrorCode::SelectionNotFound
    );
}

#[test]
fn reads_exact_jpx_image_inventory_entry_as_opaque_bytes() {
    let bytes = jpx();
    let document = jpeg_xobject(2, 1, "/JPXDecode", "", &bytes);
    let entry = list_image_xobjects(&document).unwrap().remove(0);

    assert_eq!(read_jpx_xobject_bytes(&document, &entry).unwrap(), bytes);

    let mut stale = entry;
    stale.height = 2;
    assert_eq!(
        read_jpx_xobject_bytes(&document, &stale).unwrap_err().code,
        PdfErrorCode::SelectionNotFound
    );
}

#[test]
fn jpx_xobject_byte_reader_refuses_non_direct_filters_parameters_and_masks() {
    let bytes = jpx();
    for (filter, entries) in [
        ("/DCTDecode", ""),
        ("[/JPXDecode]", ""),
        ("[/ASCII85Decode /JPXDecode]", ""),
        ("/JPXDecode", " /DecodeParms null"),
        ("/JPXDecode", " /Decode [1 0 1 0 1 0]"),
        ("/JPXDecode", " /SMask 1 0 R"),
        ("/JPXDecode", " /Mask [0 1]"),
        ("/JPXDecode", " /ImageMask false"),
        ("/JPXDecode", " /SMaskInData 1"),
    ] {
        let document = jpeg_xobject(2, 1, filter, entries, &bytes);
        let entry = list_image_xobjects(&document).unwrap().remove(0);
        let error = read_jpx_xobject_bytes(&document, &entry).unwrap_err();
        assert_eq!(error.code, PdfErrorCode::UnsupportedFeature);
        assert_eq!(error.object, Some((2, 0)));
    }
}

#[test]
fn jpx_xobject_byte_reader_refuses_invalid_or_inconsistent_jpx_bytes() {
    let valid = jpx();
    let malformed = valid[..valid.len() - 2].to_vec();
    let mut truncated_siz = valid[..valid.len() - 4].to_vec();
    truncated_siz.extend_from_slice(&[0xff, 0xd9]);
    for (bytes, expected) in [
        (malformed, PdfErrorCode::InvalidSyntax),
        (truncated_siz, PdfErrorCode::InvalidSyntax),
        (vec![0], PdfErrorCode::UnsupportedFeature),
    ] {
        let document = jpeg_xobject(2, 1, "/JPXDecode", "", &bytes);
        let entry = list_image_xobjects(&document).unwrap().remove(0);
        let error = read_jpx_xobject_bytes(&document, &entry).unwrap_err();
        assert_eq!(error.code, expected);
        assert_eq!(error.object, Some((2, 0)));
    }

    let document = jpeg_xobject(1, 1, "/JPXDecode", "", &valid);
    let entry = list_image_xobjects(&document).unwrap().remove(0);
    assert_eq!(
        read_jpx_xobject_bytes(&document, &entry).unwrap_err().code,
        PdfErrorCode::InvalidSyntax
    );
}

#[test]
fn reads_exact_raw_flate_image_inventory_entry_as_samples() {
    let expected = vec![10, 20, 30, 40, 50, 60];
    let document = jpeg_xobject(2, 1, "/FlateDecode", "", &flate(&expected));
    let entry = list_image_xobjects(&document).unwrap().remove(0);

    assert_eq!(
        read_raw_flate_image_samples(&document, &entry).unwrap(),
        RawFlateImageSamples {
            object: entry.object,
            width: 2,
            height: 1,
            color_space: ImageColorSpace::DeviceRgb,
            samples: expected,
        }
    );

    let mut stale = entry;
    stale.width = 3;
    assert_eq!(
        read_raw_flate_image_samples(&document, &stale)
            .unwrap_err()
            .code,
        PdfErrorCode::SelectionNotFound
    );
}

#[test]
fn raw_flate_sample_reader_refuses_unsupported_forms_and_length_mismatches() {
    let encoded = flate(&[10, 20, 30, 40, 50, 60]);
    for (filter, entries) in [
        ("/DCTDecode", ""),
        ("[/FlateDecode]", ""),
        ("/FlateDecode", " /DecodeParms null"),
        ("/FlateDecode", " /Decode [1 0 1 0 1 0]"),
        ("/FlateDecode", " /SMask 1 0 R"),
        ("/FlateDecode", " /Mask [0 1]"),
        ("/FlateDecode", " /ImageMask false"),
        ("/FlateDecode", " /SMaskInData 1"),
    ] {
        let document = jpeg_xobject(2, 1, filter, entries, &encoded);
        let entry = list_image_xobjects(&document).unwrap().remove(0);
        let error = read_raw_flate_image_samples(&document, &entry).unwrap_err();
        assert_eq!(error.code, PdfErrorCode::UnsupportedFeature);
        assert_eq!(error.object, Some((2, 0)));
    }

    let document = jpeg_xobject(2, 1, "/FlateDecode", "", &flate(&[10, 20, 30, 40, 50]));
    let entry = list_image_xobjects(&document).unwrap().remove(0);
    let error = read_raw_flate_image_samples(&document, &entry).unwrap_err();
    assert_eq!(error.code, PdfErrorCode::InvalidSyntax);
    assert_eq!(error.object, Some((2, 0)));
}

#[test]
fn jpeg_xobject_byte_reader_refuses_non_direct_filters_parameters_and_masks() {
    let bytes = jpeg();
    for (filter, decode_parms) in [
        ("/FlateDecode", ""),
        ("[/DCTDecode]", ""),
        ("[/ASCII85Decode /DCTDecode]", ""),
        ("/DCTDecode", " /DecodeParms null"),
        ("/DCTDecode", " /Decode [1 0 1 0 1 0]"),
        ("/DCTDecode", " /SMask 1 0 R"),
        ("/DCTDecode", " /Mask [0 1]"),
        ("/DCTDecode", " /ImageMask false"),
        ("/DCTDecode", " /SMaskInData 1"),
    ] {
        let document = jpeg_xobject(2, 1, filter, decode_parms, &bytes);
        let entry = list_image_xobjects(&document).unwrap().remove(0);
        let error = read_jpeg_xobject_bytes(&document, &entry).unwrap_err();
        assert_eq!(error.code, PdfErrorCode::UnsupportedFeature);
        assert_eq!(error.object, Some((2, 0)));
    }
}

#[test]
fn jpeg_xobject_byte_reader_refuses_invalid_or_inconsistent_jpeg_bytes() {
    let valid = jpeg();
    let mut malformed = valid.clone();
    malformed[5] = 1;
    let mut unsupported = valid.clone();
    unsupported[3] = 0xc3;
    let truncated = valid[..valid.len() - 2].to_vec();

    for (bytes, expected) in [
        (malformed, PdfErrorCode::InvalidSyntax),
        (truncated, PdfErrorCode::InvalidSyntax),
        (unsupported, PdfErrorCode::UnsupportedFeature),
    ] {
        let document = jpeg_xobject(2, 1, "/DCTDecode", "", &bytes);
        let entry = list_image_xobjects(&document).unwrap().remove(0);
        let error = read_jpeg_xobject_bytes(&document, &entry).unwrap_err();
        assert_eq!(error.code, expected);
        assert_eq!(error.object, Some((2, 0)));
    }

    let document = jpeg_xobject(1, 1, "/DCTDecode", "", &valid);
    let entry = list_image_xobjects(&document).unwrap().remove(0);
    assert_eq!(
        read_jpeg_xobject_bytes(&document, &entry).unwrap_err().code,
        PdfErrorCode::InvalidSyntax
    );
}

#[test]
fn normalizes_opaque_png_gray_and_rgb_to_flate_samples() {
    let gray_scanlines = [
        0, 10, 20, 30, // None
        1, 12, 13, 8, // Sub
        2, 8, 5, 17, // Up
        3, 20, 30, 15, // Average
        4, 10, 20, 20, // Paeth
    ];
    let gray = open(&pdf(false, false))
        .replace_image_xobject_encoded(EncodedImageReplacementRequest {
            object_number: 5,
            object_generation: 0,
            encoded_bytes: png(3, 5, 8, 0, 0, &gray_scanlines, &[]),
            mask_policy: ImageMaskPolicy::Reject,
        })
        .unwrap();
    assert_eq!(gray.report.filter, ImageFilter::Flate);
    assert_eq!((gray.report.width, gray.report.height), (3, 5));
    assert_eq!(
        decode_filter_chain(
            image_stream(&gray.bytes),
            &[PdfFilter::FlateDecode],
            &[None],
            64,
        )
        .unwrap(),
        [10, 20, 30, 12, 25, 33, 20, 30, 50, 30, 60, 70, 40, 80, 100]
    );

    let rgb = open(&pdf(false, false))
        .replace_image_xobject_encoded(EncodedImageReplacementRequest {
            object_number: 5,
            object_generation: 0,
            encoded_bytes: png(
                2,
                1,
                8,
                2,
                0,
                &[1, 10, 20, 30, 5, 5, 5],
                &[(b"gAMA", &[0, 0, 177, 143])],
            ),
            mask_policy: ImageMaskPolicy::Reject,
        })
        .unwrap();
    assert!(rgb.verification.passed);
    assert_eq!(
        decode_filter_chain(
            image_stream(&rgb.bytes),
            &[PdfFilter::FlateDecode],
            &[None],
            64,
        )
        .unwrap(),
        [10, 20, 30, 15, 25, 35]
    );
}

#[test]
fn normalizes_opaque_indexed_png_to_rgb_samples() {
    let palette = [10, 20, 30, 40, 50, 60, 70, 80, 90];
    let indexed = open(&pdf(false, false))
        .replace_image_xobject_encoded(EncodedImageReplacementRequest {
            object_number: 5,
            object_generation: 0,
            encoded_bytes: png(3, 1, 8, 3, 0, &[0, 2, 0, 1], &[(b"PLTE", &palette)]),
            mask_policy: ImageMaskPolicy::Reject,
        })
        .unwrap();
    assert!(indexed.verification.passed);
    assert_eq!(indexed.report.filter, ImageFilter::Flate);
    assert_eq!(
        decode_filter_chain(
            image_stream(&indexed.bytes),
            &[PdfFilter::FlateDecode],
            &[None],
            64,
        )
        .unwrap(),
        [70, 80, 90, 10, 20, 30, 40, 50, 60]
    );
}

#[test]
fn normalizes_indexed_png_transparency_to_an_owned_soft_mask() {
    let palette = [10, 20, 30, 40, 50, 60, 70, 80, 90];
    let outcome = open(&pdf(false, false))
        .replace_image_xobject_encoded(EncodedImageReplacementRequest {
            object_number: 5,
            object_generation: 0,
            encoded_bytes: png(
                3,
                1,
                8,
                3,
                0,
                &[0, 2, 0, 1],
                &[(b"PLTE", &palette), (b"tRNS", &[0, 128])],
            ),
            mask_policy: ImageMaskPolicy::Reject,
        })
        .unwrap();
    assert!(outcome.verification.passed);
    assert_eq!(
        decode_filter_chain(
            image_stream(&outcome.bytes),
            &[PdfFilter::FlateDecode],
            &[None],
            64,
        )
        .unwrap(),
        [70, 80, 90, 10, 20, 30, 40, 50, 60]
    );
    assert_eq!(
        decode_filter_chain(
            image_stream_at(&outcome.bytes, 1),
            &[PdfFilter::FlateDecode],
            &[None],
            64,
        )
        .unwrap(),
        [255, 0, 128]
    );
    assert_eq!(
        outcome
            .bytes
            .windows(b"/Subtype /Image".len())
            .filter(|value| *value == b"/Subtype /Image")
            .count(),
        2
    );
    assert!(
        outcome
            .bytes
            .windows(b"/SMask 6 0 R".len())
            .any(|value| value == b"/SMask 6 0 R")
    );
    assert!(
        outcome
            .bytes
            .windows(b"/ColorSpace /DeviceGray".len())
            .any(|value| value == b"/ColorSpace /DeviceGray")
    );
}

#[test]
fn rejects_duplicate_and_oversized_indexed_png_palettes() {
    let oversized_palette = vec![0_u8; 257 * 3];
    for encoded_bytes in [
        png(
            1,
            1,
            8,
            3,
            0,
            &[0, 0],
            &[(b"PLTE", &[1, 2, 3]), (b"PLTE", &[4, 5, 6])],
        ),
        png(1, 1, 8, 3, 0, &[0, 0], &[(b"PLTE", &oversized_palette)]),
    ] {
        assert_eq!(
            open(&pdf(false, false))
                .replace_image_xobject_encoded(EncodedImageReplacementRequest {
                    object_number: 5,
                    object_generation: 0,
                    encoded_bytes,
                    mask_policy: ImageMaskPolicy::Reject,
                })
                .unwrap_err()
                .code,
            PdfErrorCode::InvalidSyntax
        );
    }
}

#[test]
fn rejects_unsafe_indexed_png_transparency_layout_and_existing_masks() {
    let palette = [1, 2, 3, 4, 5, 6];
    let mut after_idat = png(1, 1, 8, 3, 0, &[0, 0], &[(b"PLTE", &palette)]);
    let iend = png_chunk(b"IEND", &[]);
    let insertion = after_idat.len() - iend.len();
    after_idat.splice(insertion..insertion, png_chunk(b"tRNS", &[0]));
    for encoded_bytes in [
        png(
            1,
            1,
            8,
            3,
            0,
            &[0, 0],
            &[(b"tRNS", &[0]), (b"PLTE", &palette)],
        ),
        png(
            1,
            1,
            8,
            3,
            0,
            &[0, 0],
            &[(b"PLTE", &palette), (b"tRNS", &[0]), (b"tRNS", &[255])],
        ),
        png(
            1,
            1,
            8,
            3,
            0,
            &[0, 0],
            &[(b"PLTE", &[1, 2, 3]), (b"tRNS", &[0, 255])],
        ),
        png(
            1,
            1,
            8,
            3,
            0,
            &[0, 0],
            &[(b"PLTE", &palette), (b"tRNS", &[])],
        ),
        after_idat,
    ] {
        assert_eq!(
            open(&pdf(false, false))
                .replace_image_xobject_encoded(EncodedImageReplacementRequest {
                    object_number: 5,
                    object_generation: 0,
                    encoded_bytes,
                    mask_policy: ImageMaskPolicy::Reject,
                })
                .unwrap_err()
                .code,
            PdfErrorCode::InvalidSyntax
        );
    }

    assert_eq!(
        open(&pdf(true, false))
            .replace_image_xobject_encoded(EncodedImageReplacementRequest {
                object_number: 5,
                object_generation: 0,
                encoded_bytes: png(
                    1,
                    1,
                    8,
                    3,
                    0,
                    &[0, 0],
                    &[(b"PLTE", &palette), (b"tRNS", &[0])],
                ),
                mask_policy: ImageMaskPolicy::PreserveCompatible,
            })
            .unwrap_err()
            .code,
        PdfErrorCode::UnsupportedFeature
    );
    assert_eq!(
        open(&pdf_with_image_entries(false, false, " /SMaskInData 1"))
            .replace_image_xobject_encoded(EncodedImageReplacementRequest {
                object_number: 5,
                object_generation: 0,
                encoded_bytes: png(
                    1,
                    1,
                    8,
                    3,
                    0,
                    &[0, 0],
                    &[(b"PLTE", &palette), (b"tRNS", &[0])],
                ),
                mask_policy: ImageMaskPolicy::Reject,
            })
            .unwrap_err()
            .code,
        PdfErrorCode::UnsupportedFeature
    );
}

#[test]
fn rejects_unsupported_malformed_and_oversized_png_input() {
    let unsupported = [
        png(1, 1, 8, 6, 0, &[0, 1, 2, 3, 4], &[]),
        png(1, 1, 16, 0, 0, &[0, 0, 1], &[]),
        png(1, 1, 8, 0, 1, &[0, 1], &[]),
        png(1, 1, 8, 0, 0, &[0, 1], &[(b"tRNS", &[0, 0])]),
        png(1, 1, 8, 0, 0, &[0, 1], &[(b"ABCD", &[])]),
        png(1, 1, 8, 0, 0, &[5, 1], &[]),
    ];
    for encoded_bytes in unsupported {
        assert_eq!(
            open(&pdf(false, false))
                .replace_image_xobject_encoded(EncodedImageReplacementRequest {
                    object_number: 5,
                    object_generation: 0,
                    encoded_bytes,
                    mask_policy: ImageMaskPolicy::Reject,
                })
                .unwrap_err()
                .code,
            PdfErrorCode::UnsupportedFeature
        );
    }

    for encoded_bytes in [
        png(1, 1, 8, 3, 0, &[0, 0], &[]),
        png(2, 1, 8, 3, 0, &[0, 0, 1], &[(b"PLTE", &[1, 2, 3])]),
        png(1, 1, 8, 3, 0, &[0, 0], &[(b"PLTE", &[1, 2, 3, 4])]),
    ] {
        assert_eq!(
            open(&pdf(false, false))
                .replace_image_xobject_encoded(EncodedImageReplacementRequest {
                    object_number: 5,
                    object_generation: 0,
                    encoded_bytes,
                    mask_policy: ImageMaskPolicy::Reject,
                })
                .unwrap_err()
                .code,
            PdfErrorCode::InvalidSyntax
        );
    }

    let mut invalid_crc = png(1, 1, 8, 0, 0, &[0, 1], &[]);
    *invalid_crc.last_mut().unwrap() ^= 1;
    assert_eq!(
        open(&pdf(false, false))
            .replace_image_xobject_encoded(EncodedImageReplacementRequest {
                object_number: 5,
                object_generation: 0,
                encoded_bytes: invalid_crc,
                mask_policy: ImageMaskPolicy::Reject,
            })
            .unwrap_err()
            .code,
        PdfErrorCode::InvalidSyntax
    );

    let mut before_ihdr = png(1, 1, 8, 0, 0, &[0, 1], &[]);
    let chunk = png_chunk(b"gAMA", &[0, 0, 177, 143]);
    before_ihdr.splice(8..8, chunk);
    assert_eq!(
        open(&pdf(false, false))
            .replace_image_xobject_encoded(EncodedImageReplacementRequest {
                object_number: 5,
                object_generation: 0,
                encoded_bytes: before_ihdr,
                mask_policy: ImageMaskPolicy::Reject,
            })
            .unwrap_err()
            .code,
        PdfErrorCode::InvalidSyntax
    );

    let limited = PdfEngine::new(EngineConfig {
        limits: Limits {
            max_total_decoded_bytes: 4,
            ..Limits::default()
        },
    })
    .open(&pdf(false, false), OpenOptions::default())
    .unwrap();
    assert_eq!(
        limited
            .replace_image_xobject_encoded(EncodedImageReplacementRequest {
                object_number: 5,
                object_generation: 0,
                encoded_bytes: png(5, 1, 8, 0, 0, &[0, 1, 2, 3, 4, 5], &[]),
                mask_policy: ImageMaskPolicy::Reject,
            })
            .unwrap_err()
            .code,
        PdfErrorCode::ResourceLimit
    );

    let limited_indexed = PdfEngine::new(EngineConfig {
        limits: Limits {
            max_total_decoded_bytes: 6,
            ..Limits::default()
        },
    })
    .open(&pdf(false, false), OpenOptions::default())
    .unwrap();
    assert_eq!(
        limited_indexed
            .replace_image_xobject_encoded(EncodedImageReplacementRequest {
                object_number: 5,
                object_generation: 0,
                encoded_bytes: png(2, 1, 8, 3, 0, &[0, 0, 1], &[(b"PLTE", &[1, 2, 3, 4, 5, 6])],),
                mask_policy: ImageMaskPolicy::Reject,
            })
            .unwrap_err()
            .code,
        PdfErrorCode::ResourceLimit
    );

    let limited_transparent_indexed = PdfEngine::new(EngineConfig {
        limits: Limits {
            max_total_decoded_bytes: 9,
            ..Limits::default()
        },
    })
    .open(&pdf(false, false), OpenOptions::default())
    .unwrap();
    assert_eq!(
        limited_transparent_indexed
            .replace_image_xobject_encoded(EncodedImageReplacementRequest {
                object_number: 5,
                object_generation: 0,
                encoded_bytes: png(
                    2,
                    1,
                    8,
                    3,
                    0,
                    &[0, 0, 1],
                    &[(b"PLTE", &[1, 2, 3, 4, 5, 6]), (b"tRNS", &[0])],
                ),
                mask_policy: ImageMaskPolicy::Reject,
            })
            .unwrap_err()
            .code,
        PdfErrorCode::ResourceLimit
    );
}

#[test]
fn rejects_malformed_layout_and_preserves_only_compatible_masks() {
    let document = open(&pdf(false, false));
    let error = document
        .replace_image_xobject(request(ImageFilter::Raw, vec![1, 2]))
        .unwrap_err();
    assert_eq!(error.code, PdfErrorCode::UnsafeRewrite);

    let masked = open(&pdf(true, false));
    let replacement = request(ImageFilter::Raw, vec![6, 5, 4, 3, 2, 1]);
    assert_eq!(
        masked
            .replace_image_xobject(replacement.clone())
            .unwrap_err()
            .code,
        PdfErrorCode::UnsupportedFeature
    );
    let mut compatible = replacement;
    compatible.mask_policy = ImageMaskPolicy::PreserveCompatible;
    assert!(
        masked
            .replace_image_xobject(compatible)
            .unwrap()
            .verification
            .no_dangling_references
    );
}

#[test]
fn rejects_non_images_and_signed_documents() {
    let document = open(&pdf(false, false));
    let mut wrong_object = request(ImageFilter::Raw, vec![1, 2, 3, 4, 5, 6]);
    wrong_object.object_number = 4;
    assert_eq!(
        document
            .replace_image_xobject(wrong_object)
            .unwrap_err()
            .code,
        PdfErrorCode::SelectionNotFound
    );
    assert_eq!(
        open(&pdf(false, true))
            .replace_image_xobject(request(ImageFilter::Raw, vec![1, 2, 3, 4, 5, 6]))
            .unwrap_err()
            .code,
        PdfErrorCode::UnsafeRewrite
    );
    assert_eq!(
        open(&pdf(false, true))
            .replace_image_xobject_encoded(EncodedImageReplacementRequest {
                object_number: 5,
                object_generation: 0,
                encoded_bytes: jpeg(),
                mask_policy: ImageMaskPolicy::Reject,
            })
            .unwrap_err()
            .code,
        PdfErrorCode::UnsafeRewrite
    );
}

fn jpeg() -> Vec<u8> {
    vec![
        0xff, 0xd8, 0xff, 0xc0, 0x00, 0x11, 0x08, 0x00, 0x01, 0x00, 0x02, 0x03, 0x01, 0x11, 0x00,
        0x02, 0x11, 0x00, 0x03, 0x11, 0x00, 0xff, 0xda, 0x00, 0x0c, 0x03, 0x01, 0x00, 0x02, 0x00,
        0x03, 0x00, 0x00, 0x3f, 0x00, 0x00, 0xff, 0xd9,
    ]
}

fn flate(samples: &[u8]) -> Vec<u8> {
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(samples).unwrap();
    encoder.finish().unwrap()
}

fn jpeg_xobject(
    width: u32,
    height: u32,
    filter: &str,
    decode_parms: &str,
    bytes: &[u8],
) -> binas_pdf::PdfDocument {
    open(&classic(&[
        b"<< /Type /Catalog >>".to_vec(),
        stream(
            &format!(
                " /Type /XObject /Subtype /Image /Width {width} /Height {height} /BitsPerComponent 8 /ColorSpace /DeviceRGB /Filter {filter}{decode_parms}"
            ),
            bytes,
        ),
    ]))
}

fn jpx() -> Vec<u8> {
    let mut bytes = vec![0xff, 0x4f, 0xff, 0x51, 0x00, 0x2f, 0x00, 0x00];
    for value in [2_u32, 1, 0, 0, 2, 1, 0, 0] {
        bytes.extend_from_slice(&value.to_be_bytes());
    }
    bytes.extend_from_slice(&3_u16.to_be_bytes());
    for _ in 0..3 {
        bytes.extend_from_slice(&[7, 1, 1]);
    }
    bytes.extend_from_slice(&[0xff, 0xd9]);
    bytes
}

fn png(
    width: u32,
    height: u32,
    bits: u8,
    color_type: u8,
    interlace: u8,
    scanlines: &[u8],
    chunks: &[(&[u8; 4], &[u8])],
) -> Vec<u8> {
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(scanlines).unwrap();
    let mut output = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[bits, color_type, 0, 0, interlace]);
    output.extend_from_slice(&png_chunk(b"IHDR", &ihdr));
    for (kind, data) in chunks {
        output.extend_from_slice(&png_chunk(kind, data));
    }
    output.extend_from_slice(&png_chunk(b"IDAT", &encoder.finish().unwrap()));
    output.extend_from_slice(&png_chunk(b"IEND", &[]));
    output
}

fn png_chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(data.len() + 12);
    output.extend_from_slice(&u32::try_from(data.len()).unwrap().to_be_bytes());
    output.extend_from_slice(kind);
    output.extend_from_slice(data);
    output.extend_from_slice(&png_crc32(kind, data).to_be_bytes());
    output
}

fn png_crc32(kind: &[u8], data: &[u8]) -> u32 {
    let mut crc = 0xffff_ffff_u32;
    for &byte in kind.iter().chain(data) {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = if crc & 1 == 0 {
                crc >> 1
            } else {
                (crc >> 1) ^ 0xedb8_8320
            };
        }
    }
    !crc
}

fn image_stream(pdf: &[u8]) -> &[u8] {
    image_stream_at(pdf, 0)
}

fn image_stream_at(pdf: &[u8], index: usize) -> &[u8] {
    let mut start = 0;
    let mut image = None;
    for _ in 0..=index {
        let offset = pdf[start..]
            .windows(b"/Subtype /Image".len())
            .position(|bytes| bytes == b"/Subtype /Image")
            .unwrap();
        let found = start + offset;
        image = Some(found);
        start = found + b"/Subtype /Image".len();
    }
    let image = image.unwrap();
    let stream = image
        + pdf[image..]
            .windows(b"stream\n".len())
            .position(|bytes| bytes == b"stream\n")
            .unwrap()
        + b"stream\n".len();
    let end = stream
        + pdf[stream..]
            .windows(b"\nendstream".len())
            .position(|bytes| bytes == b"\nendstream")
            .unwrap();
    &pdf[stream..end]
}
