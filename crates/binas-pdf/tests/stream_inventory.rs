use binas_pdf::{
    DecodeParams, ImageColorSpace, ImageXObjectInventoryEntry, OpenOptions, PdfEngine,
    PdfErrorCode, PdfFilter, StreamFilterMetadata, StreamObjectRef, list_image_xobjects,
    list_streams,
};

fn pdf(objects: &[&str]) -> Vec<u8> {
    let mut bytes = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::with_capacity(objects.len());
    for (index, object) in objects.iter().enumerate() {
        offsets.push(bytes.len());
        bytes.extend_from_slice(format!("{} 0 obj\n{object}\nendobj\n", index + 1).as_bytes());
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

fn open(objects: &[&str]) -> binas_pdf::PdfDocument {
    PdfEngine::default()
        .open(&pdf(objects), OpenOptions::default())
        .unwrap()
}

#[test]
fn inventories_direct_and_filtered_stream_metadata() {
    let document = open(&[
        "<< /Type /Catalog >>",
        "<< /Length 3 >>\nstream\nraw\nendstream",
        "<< /Length 7 /Filter [/ASCII85Decode /FlateDecode] /DecodeParms [null << /Predictor 12 /Columns 4 >>] >>\nstream\nencoded\nendstream",
    ]);

    let streams = list_streams(&document).unwrap();
    assert_eq!(streams.len(), 2);
    assert_eq!(
        streams[0].object,
        StreamObjectRef {
            object_number: 2,
            object_generation: 0,
        }
    );
    assert_eq!(streams[0].encoded_length, 3);
    assert!(streams[0].filter_chain.is_empty());
    assert!(!streams[0].image_xobject);

    assert_eq!(streams[1].encoded_length, 7);
    assert_eq!(streams[1].filter_chain.len(), 2);
    assert_eq!(streams[1].filter_chain[0].filter, PdfFilter::ASCII85Decode);
    assert_eq!(streams[1].filter_chain[0].decode_params, None);
    assert_eq!(streams[1].filter_chain[1].filter, PdfFilter::FlateDecode);
    assert_eq!(
        streams[1].filter_chain[1].decode_params,
        Some(DecodeParams {
            predictor: 12,
            colors: 1,
            bits_per_component: 8,
            columns: 4,
            early_change: 1,
        })
    );
}

#[test]
fn inventories_direct_image_xobject_metadata_without_exposing_bytes() {
    let document = open(&[
        "<< /Type /Catalog >>",
        "<< /Type /XObject /Subtype /Image /Width 2 /Height 1 /ColorSpace /DeviceRGB /Length 1 >>\nstream\nA\nendstream",
    ]);

    assert_eq!(
        list_image_xobjects(&document).unwrap(),
        vec![ImageXObjectInventoryEntry {
            object: StreamObjectRef {
                object_number: 2,
                object_generation: 0,
            },
            width: 2,
            height: 1,
            filter_chain: Vec::new(),
            color_space: Some(ImageColorSpace::DeviceRgb),
        }]
    );
}

#[test]
fn inventories_filtered_image_xobject_metadata() {
    let document = open(&[
        "<< /Type /Catalog >>",
        "<< /Type /XObject /Subtype /Image /Width 2 /Height 1 /ColorSpace /DeviceRGB /Filter /FlateDecode /DecodeParms << /Predictor 12 /Colors 3 /BitsPerComponent 8 /Columns 2 >> /Length 1 >>\nstream\nA\nendstream",
    ]);

    let images = list_image_xobjects(&document).unwrap();
    assert_eq!(images.len(), 1);
    assert_eq!(
        images[0].filter_chain,
        vec![StreamFilterMetadata {
            filter: PdfFilter::FlateDecode,
            decode_params: Some(DecodeParams {
                predictor: 12,
                colors: 3,
                bits_per_component: 8,
                columns: 2,
                early_change: 1,
            }),
        }]
    );
}

#[test]
fn refuses_malformed_image_xobject_metadata() {
    for image in [
        "<< /Type /XObject /Subtype /Image /Width 0 /Height 1 /Length 1 >>\nstream\nA\nendstream",
        "<< /Type /XObject /Subtype /Image /Width 1 /Height 1 /ColorSpace 7 /Length 1 >>\nstream\nA\nendstream",
        "<< /Type /XObject /Subtype /Image /Width 1 /Height 1 /Filter [/FlateDecode 7] /Length 1 >>\nstream\nA\nendstream",
    ] {
        let error = list_image_xobjects(&open(&["<< /Type /Catalog >>", image])).unwrap_err();
        assert_eq!(error.code, PdfErrorCode::InvalidSyntax);
        assert_eq!(error.object, Some((2, 0)));
    }
}

#[test]
fn refuses_malformed_stream_metadata() {
    for stream in [
        "<< /Length 1 /Filter [/FlateDecode 7] >>\nstream\nA\nendstream",
        "<< /Length 1 /Filter [/FlateDecode /ASCII85Decode] /DecodeParms << >> >>\nstream\nA\nendstream",
        "<< /Length 1 /Filter /FlateDecode /DecodeParms << /Predictor /bad >> >>\nstream\nA\nendstream",
    ] {
        let error = list_streams(&open(&["<< /Type /Catalog >>", stream])).unwrap_err();
        assert_eq!(error.code, PdfErrorCode::InvalidSyntax);
    }
}
