//! Original generated resource-census fixtures, Apache-2.0. No external assets.
use std::collections::BTreeMap;

use binas_core::{
    BudgetLimits, CancellationToken, OperationContext, ResourceBudget, RevisionId, SnapshotId,
};
use binas_pdf::preflight::{
    ImagePreflightResult, ImagePreflightScope, ImageResolutionProfile, ImageResolutionStatus,
    inspect_document_images,
};
use binas_pdf::{
    OpenOptions, PdfEngine, PdfErrorCode, PdfPageTextEditRequest, PdfResourceKind, PdfRevision,
    PdfRevisionRequest, PdfSharedEditMode, SurgicalTextEditRequest,
};

fn context() -> OperationContext {
    OperationContext::new(
        ResourceBudget::new(BudgetLimits {
            max_live_bytes: 32 * 1024 * 1024,
            max_work_units: 64 * 1024 * 1024,
        }),
        CancellationToken::new(),
    )
}

fn stream(data: &[u8], entries: &str) -> Vec<u8> {
    let mut bytes = format!("<< /Length {} {entries} >>\nstream\n", data.len()).into_bytes();
    bytes.extend_from_slice(data);
    bytes.extend_from_slice(b"\nendstream");
    bytes
}

fn objects() -> BTreeMap<u32, Vec<u8>> {
    BTreeMap::from([
        (1,b"<< /Type /Catalog /Pages 2 0 R >>".to_vec()),
        (2,b"<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 /MediaBox [0 0 200 200] /Resources << /Font << /F1 10 0 R >> /XObject << /Outer 7 0 R /Unused 9 0 R >> >> >>".to_vec()),
        (3,b"<< /Type /Page /Parent 2 0 R /Contents 5 0 R >>".to_vec()),
        (4,b"<< /Type /Page /Parent 2 0 R /Contents 6 0 R >>".to_vec()),
        (5,stream(b"BT /F1 12 Tf (a) Tj ET q 1 0 0 1 10 20 cm /Outer Do Q q 2 0 0 2 30 40 cm /Outer Do Q","")),
        (6,stream(b"BT /F1 12 Tf (b) Tj ET","")),
        (7,stream(b"/Inner Do", "/Type /XObject /Subtype /Form /BBox [0 0 10 10] /Matrix [1 0 0 1 3 4] /Resources << /XObject << /Inner 8 0 R >> >>")),
        (8,stream(b"BT /F1 9 Tf (c) Tj ET /Pixel Do", "/Type /XObject /Subtype /Form /BBox [0 0 10 10] /Matrix [1 0 0 1 5 6] /Resources << /Font << /F1 11 0 R >> /XObject << /Pixel 9 0 R >> >>")),
        (9,stream(&[0x40], "/Type /XObject /Subtype /Image /Width 1 /Height 1 /BitsPerComponent 8 /ColorSpace /DeviceGray")),
        (10,b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_vec()),
        (11,b"<< /Type /Font /Subtype /Type1 /BaseFont /Courier >>".to_vec()),
    ])
}

fn appearance_and_mask_objects() -> BTreeMap<u32, Vec<u8>> {
    let mut objects = objects();
    objects.insert(
        3,
        b"<< /Type /Page /Parent 2 0 R /Contents 5 0 R /Annots [13 0 R] >>".to_vec(),
    );
    objects.insert(12,stream(b"/Pixel Do","/Type /XObject /Subtype /Form /BBox [0 0 10 10] /Resources << /XObject << /Pixel 9 0 R >> >>"));
    objects.insert(13,b"<< /Type /Annot /Subtype /Stamp /Rect [0 0 10 10] /P 3 0 R /AS /On /AP << /N << /On 12 0 R /Off 14 0 R >> >> >>".to_vec());
    objects.insert(14,stream(b"BT /F1 8 Tf (off) Tj ET","/Type /XObject /Subtype /Form /BBox [0 0 10 10] /Resources << /Font << /F1 11 0 R >> >>"));
    objects.insert(9,stream(&[0x40],"/Type /XObject /Subtype /Image /Width 1 /Height 1 /BitsPerComponent 8 /ColorSpace /DeviceGray /SMask 15 0 R"));
    objects.insert(15,stream(&[0x80],"/Type /XObject /Subtype /Image /Width 1 /Height 1 /BitsPerComponent 8 /ColorSpace /DeviceGray"));
    objects
}

fn optional_content_objects() -> BTreeMap<u32, Vec<u8>> {
    let mut objects = objects();
    objects.insert(1,b"<< /Type /Catalog /Pages 2 0 R /OCProperties << /OCGs [12 0 R] /D << /Order [12 0 R] /OFF [12 0 R] >> >> >>".to_vec());
    objects.insert(3,b"<< /Type /Page /Parent 2 0 R /Contents 5 0 R /Resources << /Properties << /Layer 12 0 R >> /XObject << /Outer 7 0 R >> >> >>".to_vec());
    objects.insert(5, stream(b"/OC /Layer BDC /Outer Do EMC", ""));
    objects.insert(12, b"<< /Type /OCG /Name (hidden layer) >>".to_vec());
    objects
}

fn type3_objects() -> BTreeMap<u32, Vec<u8>> {
    let mut objects = objects();
    objects.insert(5, stream(b"BT /F1 12 Tf (A) Tj ET", ""));
    objects.insert(10,b"<< /Type /Font /Subtype /Type3 /FontBBox [0 0 600 700] /FontMatrix [0.001 0 0 0.001 0 0] /CharProcs << /A 12 0 R >> /Encoding << /Type /Encoding /Differences [65 /A] >> /FirstChar 65 /LastChar 65 /Widths [600] /Resources << /XObject << /Pixel 9 0 R >> >> >>".to_vec());
    objects.insert(
        12,
        stream(b"600 0 0 0 600 700 d1 q 600 0 0 700 0 0 cm /Pixel Do Q", ""),
    );
    objects
}

#[test]
fn original_appearance_mask_and_hidden_layer_fixtures_open() {
    for objects in [
        appearance_and_mask_objects(),
        optional_content_objects(),
        type3_objects(),
    ] {
        let bytes = pdf(objects);
        PdfEngine::default()
            .open(&bytes, OpenOptions::default())
            .unwrap();
    }
}

fn pdf(objects: BTreeMap<u32, Vec<u8>>) -> Vec<u8> {
    let size = objects.keys().last().unwrap() + 1;
    let mut bytes = b"%PDF-1.7\n".to_vec();
    let mut offsets = BTreeMap::new();
    for (number, object) in objects {
        offsets.insert(number, bytes.len());
        bytes.extend_from_slice(format!("{number} 0 obj\n").as_bytes());
        bytes.extend(object);
        bytes.extend_from_slice(b"\nendobj\n");
    }
    let xref = bytes.len();
    bytes.extend_from_slice(format!("xref\n0 {size}\n0000000000 65535 f \n").as_bytes());
    for number in 1..size {
        bytes.extend_from_slice(
            match offsets.get(&number) {
                Some(offset) => format!("{offset:010} 00000 n \n"),
                None => "0000000000 00000 f \n".into(),
            }
            .as_bytes(),
        );
    }
    bytes.extend_from_slice(
        format!("trailer\n<< /Size {size} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes(),
    );
    bytes
}

#[test]
fn original_nested_fixture_opens_and_has_distinct_page_and_form_font_contexts() {
    let bytes = pdf(objects());
    let document = PdfEngine::default()
        .open(&bytes, OpenOptions::default())
        .unwrap();
    let text = document.extract_page_text_spans(0).unwrap();
    assert!(text.warnings.is_empty());
    assert_eq!(
        text.spans
            .iter()
            .map(|span| span.text.as_str())
            .collect::<Vec<_>>(),
        ["a", "c", "c"]
    );
    assert_eq!(
        text.spans[1].geometry.user_matrix,
        [1., 0., 0., 1., 18., 30.]
    );
    assert_eq!(
        text.spans[2].geometry.user_matrix,
        [2., 0., 0., 2., 46., 60.]
    );
    context().checkpoint().unwrap();
}

#[test]
fn census_retains_each_nested_placement_and_resolves_shadowed_fonts() {
    let document = PdfEngine::default()
        .open_owned(pdf(objects()), OpenOptions::default())
        .unwrap();
    let usage = document.page_resource_usage(0, &context()).unwrap();
    assert!(usage.complete(), "{:?}", usage.gaps);
    let images = usage
        .uses
        .iter()
        .filter(|item| item.kind == PdfResourceKind::Image)
        .collect::<Vec<_>>();
    assert_eq!(images.len(), 2);
    for ((image, matrix), dpi) in images
        .iter()
        .zip([[1., 0., 0., 1., 18., 30.], [2., 0., 0., 2., 46., 60.]])
        .zip([[72., 72.], [36., 36.]])
    {
        assert_eq!(image.target.unwrap().number, 9);
        assert_eq!(image.stream.number, 8);
        assert_eq!(image.page.number, 3);
        assert_eq!(image.page_index, 0);
        assert_eq!(image.ctm, Some(matrix));
        assert_eq!(image.image_dpi, Some(dpi));
        assert_eq!(
            image
                .invocation_path
                .iter()
                .map(|reference| reference.number)
                .collect::<Vec<_>>(),
            [7, 8]
        );
        assert!(!image.appearance);
        assert!(!image.conditional);
    }
    let fonts = usage
        .uses
        .iter()
        .filter(|item| item.kind == PdfResourceKind::Font)
        .map(|item| item.target.unwrap().number)
        .collect::<Vec<_>>();
    assert!(fonts.contains(&10));
    assert_eq!(fonts.iter().filter(|font| **font == 11).count(), 2);
    assert!(!usage.uses.iter().any(|item| item.name == b"Unused"));
    assert!(
        !usage
            .scanned_streams
            .iter()
            .any(|reference| reference.number == 6 || reference.number == 9)
    );
}

#[test]
fn contents_parts_share_graphics_state_pending_operands_and_q_restoration() {
    let mut source = objects();
    source.insert(
        3,
        b"<< /Type /Page /Parent 2 0 R /Contents [5 0 R 12 0 R 13 0 R] /Resources << /Font << /F1 10 0 R /F2 11 0 R >> /XObject << /Unused 9 0 R >> >> >>".to_vec(),
    );
    source.insert(
        5,
        stream(
            b"BT /F1 12 Tf (before) Tj ET q 2 0 0 3 10 20 cm BT /F2 9 Tf (first)",
            "",
        ),
    );
    source.insert(
        12,
        stream(
            b"Tj ET /Unused Do Q BT (after) Tj ET /Unused Do q 4 0 0 5 0 0 cm",
            "",
        ),
    );
    source.insert(13, stream(b"/Unused Do Q", ""));
    let document = PdfEngine::default()
        .open_owned(pdf(source), OpenOptions::default())
        .unwrap();
    let usage = document.page_resource_usage(0, &context()).unwrap();
    assert!(usage.complete(), "{:?}", usage.gaps);
    let text = usage
        .uses
        .iter()
        .find(|item| item.encoded_text.as_deref() == Some(b"first"))
        .unwrap();
    assert_eq!(text.kind, PdfResourceKind::Font);
    assert_eq!(text.target.unwrap().number, 11);
    for restored in [b"before".as_slice(), b"after"] {
        assert!(usage.uses.iter().any(|item| {
            item.encoded_text.as_deref() == Some(restored)
                && item.target.is_some_and(|font| font.number == 10)
        }));
    }
    let images = usage
        .uses
        .iter()
        .filter(|item| item.kind == PdfResourceKind::Image)
        .collect::<Vec<_>>();
    assert_eq!(images.len(), 3);
    for (image, (matrix, dpi)) in images.iter().zip([
        ([2., 0., 0., 3., 10., 20.], [36., 24.]),
        ([1., 0., 0., 1., 0., 0.], [72., 72.]),
        ([4., 0., 0., 5., 0., 0.], [18., 14.4]),
    ]) {
        assert_eq!(image.ctm, Some(matrix));
        assert_eq!(image.image_dpi, Some(dpi));
    }
}

#[test]
fn forms_inherit_the_active_font_identity_before_resolving_a_local_font_name() {
    let mut source = objects();
    source.insert(5, stream(b"BT /F1 12 Tf (page) Tj ET /Outer Do", ""));
    source.insert(
        7,
        stream(
            b"BT (inherited) Tj /F1 9 Tf (local) Tj ET",
            "/Type /XObject /Subtype /Form /BBox [0 0 10 10] /Resources << /Font << /F1 11 0 R >> >>",
        ),
    );
    let document = PdfEngine::default()
        .open_owned(pdf(source), OpenOptions::default())
        .unwrap();
    let usage = document.page_resource_usage(0, &context()).unwrap();
    assert!(usage.complete(), "{:?}", usage.gaps);
    let painted_fonts = usage
        .uses
        .iter()
        .filter_map(|item| {
            item.encoded_text.as_ref().map(|text| {
                assert_eq!(item.kind, PdfResourceKind::Font);
                (text.as_slice(), item.target.unwrap().number)
            })
        })
        .collect::<Vec<_>>();
    assert_eq!(
        painted_fonts,
        [(b"page".as_slice(), 10), (b"inherited", 10), (b"local", 11)]
    );
}

#[test]
fn inherited_font_text_records_the_optional_form_invocation_instead_of_the_tf_site() {
    let mut source = objects();
    source.insert(
        5,
        stream(
            b"BT /F1 12 Tf ET /Outer Do /Outer Do BT (outside) Tj ET",
            "",
        ),
    );
    source.insert(
        7,
        stream(
            b"BT (inherited) Tj ET",
            "/Type /XObject /Subtype /Form /BBox [0 0 10 10] /Matrix [2 0 0 3 4 5] /OC 12 0 R /Resources << /Font << /F1 11 0 R >> >>",
        ),
    );
    source.insert(12, b"<< /Type /OCG /Name (conditional Form) >>".to_vec());
    let document = PdfEngine::default()
        .open_owned(pdf(source), OpenOptions::default())
        .unwrap();
    let usage = document.page_resource_usage(0, &context()).unwrap();
    assert!(usage.complete(), "{:?}", usage.gaps);
    let inherited = usage
        .uses
        .iter()
        .filter(|item| item.encoded_text.as_deref() == Some(b"inherited"))
        .collect::<Vec<_>>();
    assert_eq!(inherited.len(), 2);
    for text in inherited {
        assert_eq!(text.target.unwrap().number, 10);
        assert_eq!(text.stream.number, 7);
        assert!(text.conditional);
        assert!(!text.appearance);
        assert_eq!(text.ctm, Some([2., 0., 0., 3., 4., 5.]));
        assert_eq!(
            text.invocation_path
                .iter()
                .map(|reference| reference.number)
                .collect::<Vec<_>>(),
            [7]
        );
    }
    let outside = usage
        .uses
        .iter()
        .find(|item| item.encoded_text.as_deref() == Some(b"outside"))
        .unwrap();
    assert_eq!(outside.target.unwrap().number, 10);
    assert_eq!(outside.stream.number, 5);
    assert!(!outside.conditional);
    assert!(outside.invocation_path.is_empty());
    assert_eq!(outside.ctm, Some([1., 0., 0., 1., 0., 0.]));
}

#[test]
fn image_color_spaces_resolve_named_resources_and_report_missing_or_invalid_values() {
    let mut source = objects();
    source.insert(5, stream(b"/Unused Do", ""));
    source.insert(
        3,
        b"<< /Type /Page /Parent 2 0 R /Contents 5 0 R /Resources << /XObject << /Unused 9 0 R >> /ColorSpace << /CS0 12 0 R >> >> >>".to_vec(),
    );
    source.insert(
        9,
        stream(
            &[0x40],
            "/Type /XObject /Subtype /Image /Width 1 /Height 1 /BitsPerComponent 8 /ColorSpace /CS0",
        ),
    );
    source.insert(12, b"[/CalGray << /WhitePoint [1 1 1] >>]".to_vec());
    let document = PdfEngine::default()
        .open_owned(pdf(source), OpenOptions::default())
        .unwrap();
    let usage = document.page_resource_usage(0, &context()).unwrap();
    assert!(usage.complete(), "{:?}", usage.gaps);
    assert!(usage.uses.iter().any(|item| {
        item.kind == PdfResourceKind::ColorSpace
            && item.target.is_some_and(|target| target.number == 12)
            && item.name == b"CS0"
    }));

    for color in [
        "",
        "/ColorSpace /CS0",
        "/ColorSpace 42",
        "/ColorSpace 99 0 R",
        "/ColorSpace []",
        "/ColorSpace null",
    ] {
        let mut source = objects();
        source.insert(5, stream(b"/Unused Do", ""));
        source.insert(
            9,
            stream(
                &[0x40],
                &format!(
                    "/Type /XObject /Subtype /Image /Width 1 /Height 1 /BitsPerComponent 8 {color}"
                ),
            ),
        );
        let document = PdfEngine::default()
            .open_owned(pdf(source), OpenOptions::default())
            .unwrap();
        let usage = document.page_resource_usage(0, &context()).unwrap();
        assert!(!usage.complete(), "invalid image color accepted: {color}");
        assert!(!usage.gaps.is_empty(), "no image color gap for {color}");
    }
}

#[test]
fn device_rgb_operators_include_the_default_rgb_resource_override() {
    for operator in [
        b"/DeviceRGB cs 0.1 0.2 0.3 sc".as_slice(),
        b"0.1 0.2 0.3 rg",
    ] {
        let mut source = objects();
        source.insert(5, stream(operator, ""));
        source.insert(
            3,
            b"<< /Type /Page /Parent 2 0 R /Contents 5 0 R /Resources << /ColorSpace << /DefaultRGB 12 0 R >> >> >>".to_vec(),
        );
        source.insert(12, b"[/CalRGB << /WhitePoint [1 1 1] >>]".to_vec());
        let document = PdfEngine::default()
            .open_owned(pdf(source), OpenOptions::default())
            .unwrap();
        let usage = document.page_resource_usage(0, &context()).unwrap();
        assert!(usage.complete(), "{:?}", usage.gaps);
        assert!(usage.uses.iter().any(|item| {
            item.kind == PdfResourceKind::ColorSpace
                && item.target.is_some_and(|target| target.number == 12)
                && item.name == b"DefaultRGB"
        }));
    }
}

#[test]
fn named_device_space_aliases_apply_defaults_without_reapplying_the_replacement() {
    for replacement in [
        b"[/CalRGB << /WhitePoint [1 1 1] >>]".as_slice(),
        b"/DeviceRGB",
    ] {
        let mut source = objects();
        source.insert(3, b"<< /Type /Page /Parent 2 0 R /Contents 5 0 R /Resources << /XObject << /Unused 9 0 R >> /ColorSpace << /CS0 /DeviceRGB /DefaultRGB 16 0 R >> >> >>".to_vec());
        source.insert(5, stream(b"/CS0 cs /Unused Do", ""));
        source.insert(9, stream(b"abc", "/Type /XObject /Subtype /Image /Width 1 /Height 1 /BitsPerComponent 8 /ColorSpace /CS0"));
        source.insert(16, replacement.to_vec());
        let document = PdfEngine::default()
            .open_owned(pdf(source.clone()), OpenOptions::default())
            .unwrap();
        let usage = document.page_resource_usage(0, &context()).unwrap();
        assert!(usage.complete(), "{:?}", usage.gaps);
        assert_eq!(
            usage
                .uses
                .iter()
                .filter(|item| item.name == b"DefaultRGB"
                    && item.target.is_some_and(|target| target.number == 16))
                .count(),
            2
        );
        let profile = ImageResolutionProfile::new(72.0).unwrap();
        assert_eq!(
            inspect_document_images(
                &document,
                &profile,
                ImagePreflightScope::Page(0),
                &context()
            )
            .unwrap()
            .result(),
            ImagePreflightResult::Passed
        );

        // An invalid replacement reached through a named alias cannot disappear
        // from coverage or leave image preflight reporting a complete pass.
        source.insert(16, b"42".to_vec());
        let invalid = PdfEngine::default()
            .open_owned(pdf(source), OpenOptions::default())
            .unwrap();
        let report =
            inspect_document_images(&invalid, &profile, ImagePreflightScope::Page(0), &context())
                .unwrap();
        assert_eq!(report.result(), ImagePreflightResult::Incomplete);
        assert!(!report.census_gaps().is_empty());
    }
}

#[test]
fn named_color_alias_cycles_remain_bounded() {
    let mut source = objects();
    source.insert(3, b"<< /Type /Page /Parent 2 0 R /Contents 5 0 R /Resources << /ColorSpace << /CS0 /CS1 /CS1 /CS0 >> >> >>".to_vec());
    source.insert(5, stream(b"/CS0 cs", ""));
    let document = PdfEngine::default()
        .open_owned(pdf(source), OpenOptions::default())
        .unwrap();
    let bounded = OperationContext::new(
        ResourceBudget::new(BudgetLimits {
            max_live_bytes: 32 * 1024 * 1024,
            max_work_units: 256,
        }),
        CancellationToken::new(),
    );
    assert_eq!(
        document.page_resource_usage(0, &bounded).unwrap_err().code,
        PdfErrorCode::ResourceLimit
    );
    assert_eq!(bounded.budget().snapshot().live_bytes, 0);
    assert!(bounded.budget().snapshot().work_units <= 256);
}

#[test]
fn indirect_type3_subtypes_are_resolved_for_tf_and_extgstate_fonts() {
    for use_extgstate in [false, true] {
        let mut source = type3_objects();
        let font = String::from_utf8(source[&10].clone())
            .unwrap()
            .replace("/Subtype /Type3", "/Subtype 14 0 R");
        source.insert(10, font.into_bytes());
        source.insert(14, b"/Type3".to_vec());
        if use_extgstate {
            source.insert(3, b"<< /Type /Page /Parent 2 0 R /Contents 5 0 R /Resources << /ExtGState << /GS << /Font [10 0 R 12] >> >> >> >>".to_vec());
            source.insert(5, stream(b"/GS gs BT (A) Tj ET", ""));
        }
        let document = PdfEngine::default()
            .open_owned(pdf(source), OpenOptions::default())
            .unwrap();
        let usage = document.page_resource_usage(0, &context()).unwrap();
        assert!(usage.complete(), "{:?}", usage.gaps);
        assert!(
            usage
                .uses
                .iter()
                .any(|item| item.kind == PdfResourceKind::Type3Glyph
                    && item.target.is_some_and(|target| target.number == 12))
        );
        assert!(
            usage
                .uses
                .iter()
                .any(|item| item.is_image && item.target.is_some_and(|target| target.number == 9))
        );
        let report = inspect_document_images(
            &document,
            &ImageResolutionProfile::new(72.0).unwrap(),
            ImagePreflightScope::Page(0),
            &context(),
        )
        .unwrap();
        assert_eq!(report.result(), ImagePreflightResult::Incomplete);
        assert_eq!(report.findings().len(), 1);
        assert_eq!(
            report.findings().next().unwrap().status,
            ImageResolutionStatus::NotChecked
        );
    }
}

#[test]
fn absent_or_unknown_font_subtypes_cannot_hide_resource_coverage() {
    for subtype in [
        "",
        "/Subtype /VendorFont",
        "/Subtype null",
        "/Subtype 14 0 R",
    ] {
        let mut source = objects();
        source.insert(5, stream(b"BT /F1 12 Tf (A) Tj ET", ""));
        source.insert(
            10,
            format!("<< /Type /Font {subtype} /BaseFont /Helvetica >>").into_bytes(),
        );
        source.insert(14, b"42".to_vec());
        let document = PdfEngine::default()
            .open_owned(pdf(source), OpenOptions::default())
            .unwrap();
        let report = inspect_document_images(
            &document,
            &ImageResolutionProfile::new(72.0).unwrap(),
            ImagePreflightScope::Page(0),
            &context(),
        )
        .unwrap();
        assert_eq!(report.result(), ImagePreflightResult::Incomplete);
        assert!(!report.census_gaps().is_empty());
    }
}

#[test]
fn fractional_image_dimensions_cannot_support_a_resolution_pass() {
    for (width, height, expected) in [
        ("1.5", "1.5", ImagePreflightResult::Incomplete),
        ("1.5", "2", ImagePreflightResult::Incomplete),
        ("2", "1.5", ImagePreflightResult::Incomplete),
        ("2", "2", ImagePreflightResult::Passed),
    ] {
        let mut source = objects();
        source.insert(5, stream(b"/Unused Do", ""));
        source.insert(9, stream(b"abcd", &format!("/Type /XObject /Subtype /Image /Width {width} /Height {height} /BitsPerComponent 8 /ColorSpace /DeviceGray")));
        let document = PdfEngine::default()
            .open_owned(pdf(source), OpenOptions::default())
            .unwrap();
        let report = inspect_document_images(
            &document,
            &ImageResolutionProfile::new(100.0).unwrap(),
            ImagePreflightScope::Page(0),
            &context(),
        )
        .unwrap();
        assert_eq!(report.result(), expected);
        assert_eq!(report.findings().len(), 1);
        let finding = report.findings().next().unwrap();
        if expected == ImagePreflightResult::Passed {
            assert_eq!(finding.resource.image_dpi, Some([144.0, 144.0]));
            assert_eq!(finding.status, ImageResolutionStatus::Passed);
        } else {
            assert_eq!(finding.resource.image_dpi, None);
            assert_eq!(finding.status, ImageResolutionStatus::NotChecked);
        }
    }
}

#[test]
fn malformed_appearance_entries_cannot_produce_complete_resource_coverage() {
    for appearance in [
        "42",
        "/Broken",
        "<< /N 42 >>",
        "<< /N << /On 42 >> >>",
        "<< /N 99 0 R >>",
    ] {
        let mut source = appearance_and_mask_objects();
        source.insert(
            13,
            format!(
                "<< /Type /Annot /Subtype /Stamp /Rect [0 0 10 10] /P 3 0 R /AP {appearance} >>"
            )
            .into_bytes(),
        );
        let document = PdfEngine::default()
            .open_owned(pdf(source), OpenOptions::default())
            .unwrap();
        let usage = document.page_resource_usage(0, &context()).unwrap();
        assert!(!usage.complete(), "invalid AP accepted: {appearance}");
        assert!(!usage.gaps.is_empty(), "no AP gap for {appearance}");
    }
}

#[test]
fn type3_without_resources_falls_back_to_the_page_instead_of_the_calling_form() {
    let mut source = type3_objects();
    source.insert(
        3,
        b"<< /Type /Page /Parent 2 0 R /Contents 5 0 R /Resources << /Font << /F1 10 0 R >> /XObject << /Outer 7 0 R /Pixel 9 0 R >> >> >>".to_vec(),
    );
    source.insert(5, stream(b"/Outer Do", ""));
    source.insert(
        7,
        stream(
            b"BT /F1 12 Tf (A) Tj ET",
            "/Type /XObject /Subtype /Form /BBox [0 0 10 10] /Resources << /Font << /F1 10 0 R >> /XObject << /Pixel 15 0 R >> >>",
        ),
    );
    source.insert(
        10,
        b"<< /Type /Font /Subtype /Type3 /FontBBox [0 0 600 700] /FontMatrix [0.001 0 0 0.001 0 0] /CharProcs << /A 12 0 R >> /Encoding << /Type /Encoding /Differences [65 /A] >> /FirstChar 65 /LastChar 65 /Widths [600] >>".to_vec(),
    );
    source.insert(
        15,
        stream(
            &[0x80],
            "/Type /XObject /Subtype /Image /Width 1 /Height 1 /BitsPerComponent 8 /ColorSpace /DeviceGray",
        ),
    );
    let document = PdfEngine::default()
        .open_owned(pdf(source), OpenOptions::default())
        .unwrap();
    let usage = document.page_resource_usage(0, &context()).unwrap();
    assert!(usage.complete(), "{:?}", usage.gaps);
    let images = usage
        .uses
        .iter()
        .filter(|item| item.kind == PdfResourceKind::Image)
        .collect::<Vec<_>>();
    assert_eq!(images.len(), 1);
    assert_eq!(images[0].target.unwrap().number, 9);
    assert_eq!(images[0].stream.number, 12);
    assert_eq!(
        images[0]
            .invocation_path
            .iter()
            .map(|reference| reference.number)
            .collect::<Vec<_>>(),
        [7, 12]
    );
    assert!(
        !usage
            .uses
            .iter()
            .any(|item| { item.target.is_some_and(|reference| reference.number == 15) })
    );
}

#[test]
fn external_form_references_leave_gaps_while_local_resources_are_still_scanned() {
    for external in [
        "/Ref << /F (external.pdf) /Page 0 >>",
        "/OPI << /2.0 << /F (original.tif) >> >>",
    ] {
        let mut source = objects();
        source.insert(5, stream(b"/Outer Do", ""));
        source.insert(
            7,
            stream(
                b"/Inner Do",
                &format!(
                    "/Type /XObject /Subtype /Form /BBox [0 0 10 10] /Resources << /XObject << /Inner 8 0 R >> >> {external}"
                ),
            ),
        );
        let document = PdfEngine::default()
            .open_owned(pdf(source), OpenOptions::default())
            .unwrap();
        let usage = document.page_resource_usage(0, &context()).unwrap();
        assert!(
            !usage.complete(),
            "external dependency accepted: {external}"
        );
        assert!(!usage.gaps.is_empty());
        assert!(usage.uses.iter().any(|item| {
            item.kind == PdfResourceKind::Image
                && item.target.is_some_and(|target| target.number == 9)
        }));
        for local in [7, 8] {
            assert!(
                usage
                    .scanned_streams
                    .iter()
                    .any(|stream| stream.number == local)
            );
        }
    }
}

#[test]
fn annotation_optional_content_applies_to_every_appearance_child_use() {
    let mut source = appearance_and_mask_objects();
    source.insert(
        13,
        b"<< /Type /Annot /Subtype /Stamp /Rect [0 0 10 10] /P 3 0 R /OC 16 0 R /AS /On /AP << /N << /On 12 0 R /Off 14 0 R >> >> >>".to_vec(),
    );
    source.insert(
        16,
        b"<< /Type /OCG /Name (conditional annotation) >>".to_vec(),
    );
    let document = PdfEngine::default()
        .open_owned(pdf(source), OpenOptions::default())
        .unwrap();
    let usage = document.page_resource_usage(0, &context()).unwrap();
    assert!(usage.complete(), "{:?}", usage.gaps);
    let children = usage
        .uses
        .iter()
        .filter(|item| item.appearance)
        .collect::<Vec<_>>();
    assert!(children.len() >= 3);
    assert!(children.iter().all(|item| item.conditional));
    for kind in [
        PdfResourceKind::Font,
        PdfResourceKind::Image,
        PdfResourceKind::SoftMask,
    ] {
        assert!(children.iter().any(|item| item.kind == kind));
    }
    assert!(usage.uses.iter().any(|item| {
        item.kind == PdfResourceKind::Image && !item.appearance && !item.conditional
    }));
}

#[test]
fn image_optional_content_propagates_to_its_soft_mask_in_every_placement() {
    let mut source = appearance_and_mask_objects();
    source.insert(
        9,
        stream(
            &[0x40],
            "/Type /XObject /Subtype /Image /Width 1 /Height 1 /BitsPerComponent 8 /ColorSpace /DeviceGray /SMask 15 0 R /OC 16 0 R",
        ),
    );
    source.insert(16, b"<< /Type /OCG /Name (conditional image) >>".to_vec());
    let document = PdfEngine::default()
        .open_owned(pdf(source), OpenOptions::default())
        .unwrap();
    let usage = document.page_resource_usage(0, &context()).unwrap();
    assert!(usage.complete(), "{:?}", usage.gaps);
    let images = usage
        .uses
        .iter()
        .filter(|item| item.kind == PdfResourceKind::Image)
        .collect::<Vec<_>>();
    let masks = usage
        .uses
        .iter()
        .filter(|item| item.kind == PdfResourceKind::SoftMask)
        .collect::<Vec<_>>();
    assert_eq!(images.len(), 3);
    assert_eq!(masks.len(), images.len());
    assert!(images.iter().all(|item| item.conditional));
    assert!(
        masks.iter().all(|item| {
            item.conditional && item.target.is_some_and(|target| target.number == 15)
        })
    );
    assert!(masks.iter().any(|item| item.appearance));
    assert!(masks.iter().any(|item| !item.appearance));
}

#[test]
fn transparency_groups_resolve_page_and_form_color_resources_in_their_own_context() {
    for (page_color, form_color, complete) in [
        ("CS0", "CS0", true),
        ("MissingPage", "CS0", false),
        ("CS0", "MissingForm", false),
    ] {
        let mut source = objects();
        source.insert(
            3,
            format!(
                "<< /Type /Page /Parent 2 0 R /Contents 5 0 R /Group << /S /Transparency /CS /{page_color} >> /Resources << /Font << /F1 10 0 R >> /XObject << /Outer 7 0 R >> /ColorSpace << /CS0 16 0 R >> >> >>"
            )
            .into_bytes(),
        );
        source.insert(
            7,
            stream(
                b"/Inner Do",
                &format!(
                    "/Type /XObject /Subtype /Form /BBox [0 0 10 10] /Group << /S /Transparency /CS /{form_color} >> /Resources << /XObject << /Inner 8 0 R >> /ColorSpace << /CS0 17 0 R >> >>"
                ),
            ),
        );
        source.insert(16, b"[/CalGray << /WhitePoint [1 1 1] >>]".to_vec());
        source.insert(17, b"[/CalRGB << /WhitePoint [1 1 1] >>]".to_vec());
        let document = PdfEngine::default()
            .open_owned(pdf(source), OpenOptions::default())
            .unwrap();
        let usage = document.page_resource_usage(0, &context()).unwrap();
        assert_eq!(usage.complete(), complete, "{:?}", usage.gaps);
        if complete {
            for target in [16, 17] {
                assert!(usage.uses.iter().any(|item| {
                    item.kind == PdfResourceKind::ColorSpace
                        && item.name == b"CS0"
                        && item
                            .target
                            .is_some_and(|reference| reference.number == target)
                }));
            }
        } else {
            assert!(!usage.gaps.is_empty());
        }
    }
}

#[test]
fn inline_rgb_records_its_default_override_and_implicit_gray_paint_keeps_a_gap() {
    let mut inline = objects();
    inline.insert(
        3,
        b"<< /Type /Page /Parent 2 0 R /Contents 5 0 R /Resources << /ColorSpace << /DefaultRGB 16 0 R >> >> >>".to_vec(),
    );
    inline.insert(5, stream(b"BI /W 1 /H 1 /CS /RGB /BPC 8 ID abc EI", ""));
    inline.insert(16, b"[/CalRGB << /WhitePoint [1 1 1] >>]".to_vec());
    let document = PdfEngine::default()
        .open_owned(pdf(inline), OpenOptions::default())
        .unwrap();
    let usage = document.page_resource_usage(0, &context()).unwrap();
    assert!(usage.complete(), "{:?}", usage.gaps);
    assert!(usage.uses.iter().any(|item| {
        item.kind == PdfResourceKind::ColorSpace
            && item.name == b"DefaultRGB"
            && item.target.is_some_and(|target| target.number == 16)
    }));
    assert!(usage.uses.iter().any(|item| {
        item.kind == PdfResourceKind::InlineImage && item.image_dpi == Some([72., 72.])
    }));

    let mut gray = objects();
    gray.insert(
        3,
        b"<< /Type /Page /Parent 2 0 R /Contents 5 0 R /Resources << /ColorSpace << /DefaultGray 16 0 R >> >> >>".to_vec(),
    );
    gray.insert(5, stream(b"0 0 10 10 re f", ""));
    gray.insert(16, b"[/CalGray << /WhitePoint [1 1 1] >>]".to_vec());
    let document = PdfEngine::default()
        .open_owned(pdf(gray), OpenOptions::default())
        .unwrap();
    let usage = document.page_resource_usage(0, &context()).unwrap();
    assert!(!usage.complete());
    assert!(
        usage
            .gaps
            .iter()
            .any(|gap| gap.code == "pdf.usage.initial_color")
    );
}

#[test]
fn null_extgstate_fonts_and_array_soft_masks_cannot_report_complete_coverage() {
    let mut bad_font = objects();
    bad_font.insert(5, stream(b"/GS0 gs", ""));
    bad_font.insert(
        3,
        b"<< /Type /Page /Parent 2 0 R /Contents 5 0 R /Resources << /ExtGState << /GS0 12 0 R >> >> >>".to_vec(),
    );
    bad_font.insert(12, b"<< /Type /ExtGState /Font [null 12] >>".to_vec());
    let mut bad_mask = objects();
    bad_mask.insert(5, stream(b"/Unused Do", ""));
    bad_mask.insert(
        9,
        stream(
            &[0x40],
            "/Type /XObject /Subtype /Image /Width 1 /Height 1 /BitsPerComponent 8 /ColorSpace /DeviceGray /SMask [99 0 R]",
        ),
    );
    for source in [bad_font, bad_mask] {
        let document = PdfEngine::default()
            .open_owned(pdf(source), OpenOptions::default())
            .unwrap();
        let usage = document.page_resource_usage(0, &context()).unwrap();
        assert!(!usage.complete());
        assert!(!usage.gaps.is_empty());
    }
}

#[test]
fn reports_bind_the_source_and_release_live_memory_without_refunding_work() {
    let bytes = pdf(objects());
    let document = PdfEngine::default()
        .open(&bytes, OpenOptions::default())
        .unwrap();
    let context = context();
    let before = context.budget().snapshot();
    let usage = document.resource_usage(&context).unwrap();
    assert!(usage.complete(), "{:?}", usage.gaps);
    assert_eq!(
        usage.snapshot(),
        SnapshotId::new(document.source_id(), RevisionId::INITIAL)
    );
    let held = context.budget().snapshot();
    assert!(held.live_bytes > before.live_bytes);
    assert!(held.work_units > before.work_units);
    drop(usage);
    let released = context.budget().snapshot();
    assert_eq!(released.live_bytes, before.live_bytes);
    assert_eq!(released.work_units, held.work_units);
    let second = document.resource_usage(&context).unwrap();
    assert!(context.budget().snapshot().work_units > released.work_units);
    drop(second);
    assert_eq!(context.budget().snapshot().live_bytes, before.live_bytes);

    let no_memory = OperationContext::new(
        ResourceBudget::new(BudgetLimits {
            max_live_bytes: 0,
            max_work_units: 64 * 1024 * 1024,
        }),
        CancellationToken::new(),
    );
    assert_eq!(
        document.resource_usage(&no_memory).unwrap_err().code,
        PdfErrorCode::ResourceLimit
    );
    assert_eq!(no_memory.budget().snapshot().live_bytes, 0);

    let mut different = objects();
    different.insert(6, stream(b"BT /F1 12 Tf (z) Tj ET", ""));
    let different = pdf(different);
    assert_eq!(different.len(), bytes.len());
    let other = PdfEngine::default()
        .open_owned(different, OpenOptions::default())
        .unwrap();
    assert_ne!(
        document.resource_usage(&context).unwrap().snapshot(),
        other.resource_usage(&context).unwrap().snapshot()
    );
}

#[test]
fn retained_census_tracks_committed_text_and_page_order_without_changing_old_reports() {
    let mut source = objects();
    source.insert(
        10,
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /ToUnicode 16 0 R >>".to_vec(),
    );
    source.insert(
        16,
        stream(
            b"begincmap\n1 begincodespacerange\n<00> <FF>\nendcodespacerange\n1 beginbfrange\n<20> <7E> <0020>\nendbfrange\nendcmap",
            "",
        ),
    );
    let document = PdfEngine::default()
        .open_owned(pdf(source), OpenOptions::default())
        .unwrap();
    let mut revision = PdfRevision::new(document);
    let operation = context();
    let original = revision.resource_usage(&operation).unwrap();
    let original_snapshot = revision.snapshot();
    assert_eq!(original.snapshot(), original_snapshot);
    assert_eq!(original_snapshot.revision(), RevisionId::INITIAL);
    let plan = revision
        .plan(
            PdfRevisionRequest {
                page_text: vec![PdfPageTextEditRequest {
                    page_index: 0,
                    edit: SurgicalTextEditRequest {
                        old_text: "a".into(),
                        replacement: "updated".into(),
                        match_index: 0,
                    },
                    shared: PdfSharedEditMode::Reject,
                }],
                reorder_pages: Some(vec![1, 0]),
                ..Default::default()
            },
            &operation,
        )
        .unwrap();
    assert!(plan.font_effects().any(|effect| effect.cmap.number == 16));
    let receipt = revision.commit(&plan, &operation).unwrap();
    let committed = revision.resource_usage(&operation).unwrap();
    assert!(committed.complete(), "{:?}", committed.gaps);
    assert_eq!(committed.snapshot(), receipt.after);
    assert_eq!(committed.snapshot(), revision.snapshot());
    assert_eq!(committed.snapshot().revision().value(), 1);
    assert_eq!(committed.snapshot().source(), original_snapshot.source());
    assert!(committed.uses.iter().all(|item| match item.page_index {
        0 => item.page.number == 4,
        1 => item.page.number == 3,
        _ => false,
    }));
    assert!(committed.uses.iter().any(|item| {
        item.page_index == 1
            && item.page.number == 3
            && item.stream.number == 5
            && item.encoded_text.as_deref() == Some(b"updated")
    }));
    assert!(
        !committed
            .uses
            .iter()
            .any(|item| item.encoded_text.as_deref() == Some(b"a"))
    );

    assert_eq!(original.snapshot(), original_snapshot);
    assert!(original.uses.iter().any(|item| {
        item.page_index == 0 && item.page.number == 3 && item.encoded_text.as_deref() == Some(b"a")
    }));
    assert!(
        !original
            .uses
            .iter()
            .any(|item| item.encoded_text.as_deref() == Some(b"updated"))
    );
    let first_page = revision.page_resource_usage(0, &operation).unwrap();
    assert_eq!(first_page.snapshot(), committed.snapshot());
    assert!(first_page.uses.iter().all(|item| item.page.number == 4));
    assert_eq!(
        first_page
            .uses
            .iter()
            .filter_map(|item| item.encoded_text.as_deref())
            .collect::<Vec<_>>(),
        [b"b".as_slice()]
    );

    let cancelled = context();
    cancelled.cancellation().cancel();
    assert_eq!(
        revision.resource_usage(&cancelled).unwrap_err().code,
        PdfErrorCode::Cancelled
    );
    assert_eq!(revision.snapshot(), committed.snapshot());
    assert_eq!(
        revision.resource_usage(&operation).unwrap().snapshot(),
        committed.snapshot()
    );
}

#[test]
fn all_appearance_states_and_image_soft_masks_are_in_the_census() {
    let document = PdfEngine::default()
        .open_owned(pdf(appearance_and_mask_objects()), OpenOptions::default())
        .unwrap();
    let usage = document.page_resource_usage(0, &context()).unwrap();
    assert!(usage.complete(), "{:?}", usage.gaps);
    for appearance in [12, 14] {
        assert!(usage.uses.iter().any(|item| {
            item.kind == PdfResourceKind::Appearance
                && item
                    .target
                    .is_some_and(|target| target.number == appearance)
        }));
    }
    assert!(usage.uses.iter().any(|item| item.appearance
        && item.kind == PdfResourceKind::Image
        && item.target.unwrap().number == 9));
    assert!(usage.uses.iter().any(|item| item.appearance
        && item.kind == PdfResourceKind::Font
        && item.target.unwrap().number == 11));
    assert!(usage.uses.iter().any(|item|item.kind==PdfResourceKind::SoftMask&&item.target.unwrap().number==15));
}

#[test]
fn hidden_optional_content_is_still_counted_and_marked_conditional() {
    let document = PdfEngine::default()
        .open_owned(pdf(optional_content_objects()), OpenOptions::default())
        .unwrap();
    let usage = document.page_resource_usage(0, &context()).unwrap();
    assert!(
        usage
            .uses
            .iter()
            .any(|item| item.kind == PdfResourceKind::Properties
                && item.target.unwrap().number == 12)
    );
    let images = usage
        .uses
        .iter()
        .filter(|item| item.kind == PdfResourceKind::Image)
        .collect::<Vec<_>>();
    assert_eq!(images.len(), 1);
    assert!(images[0].conditional);
}

#[test]
fn selected_type3_glyph_resources_are_followed() {
    let document = PdfEngine::default()
        .open_owned(pdf(type3_objects()), OpenOptions::default())
        .unwrap();
    let usage = document.page_resource_usage(0, &context()).unwrap();
    assert!(usage.complete(), "{:?}", usage.gaps);
    assert!(
        usage
            .uses
            .iter()
            .any(|item| item.kind == PdfResourceKind::Type3Glyph
                && item.target.unwrap().number == 12)
    );
    assert!(
        usage
            .uses
            .iter()
            .any(|item| item.kind == PdfResourceKind::Image && item.target.unwrap().number == 9)
    );
}

#[test]
fn unknown_stream_resources_operators_and_cycles_are_explicit_gaps() {
    for (number, value) in [
        (5, stream(b"opaque", "/Filter /FooDecode")),
        (5, stream(b"/Missing Do", "")),
        (5, stream(b"1 MysteryOperator", "")),
        (
            8,
            stream(
                b"/Loop Do",
                "/Type /XObject /Subtype /Form /BBox [0 0 10 10] /Resources << /XObject << /Loop 7 0 R >> >>",
            ),
        ),
        (5, stream(b"/Dangling Do", "")),
    ] {
        let mut source = objects();
        source.insert(number, value);
        source.insert(3,b"<< /Type /Page /Parent 2 0 R /Contents 5 0 R /Resources << /Font << /F1 10 0 R >> /XObject << /Outer 7 0 R /Dangling 99 0 R >> >> >>".to_vec());
        let document = PdfEngine::default()
            .open_owned(pdf(source), OpenOptions::default())
            .unwrap();
        let usage = document.page_resource_usage(0, &context()).unwrap();
        assert!(!usage.complete());
        assert!(!usage.gaps.is_empty());
        assert!(
            usage
                .gaps
                .iter()
                .all(|gap| !gap.code.is_empty() && !gap.detail.is_empty())
        );
    }
}

#[test]
fn selective_census_avoids_unrelated_decode_and_honors_cancellation_and_work_limit() {
    let mut source = objects();
    source.insert(6, stream(b"G>", "/Filter /ASCIIHexDecode"));
    let document = PdfEngine::default()
        .open_owned(pdf(source), OpenOptions::default())
        .unwrap();
    assert!(
        document
            .page_resource_usage(0, &context())
            .unwrap()
            .complete()
    );
    assert_eq!(
        document.resource_usage(&context()).unwrap_err().code,
        PdfErrorCode::InvalidSyntax
    );
    let cancelled = context();
    cancelled.cancellation().cancel();
    assert_eq!(
        document
            .page_resource_usage(0, &cancelled)
            .unwrap_err()
            .code,
        PdfErrorCode::Cancelled
    );
    let limited = OperationContext::new(
        ResourceBudget::new(BudgetLimits {
            max_live_bytes: 1024 * 1024,
            max_work_units: 0,
        }),
        CancellationToken::new(),
    );
    assert_eq!(
        document.page_resource_usage(0, &limited).unwrap_err().code,
        PdfErrorCode::ResourceLimit
    );
}
