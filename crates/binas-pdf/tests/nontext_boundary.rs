//! Public consumer workflows over the existing licensed preservation cohort.
use binas_core::{BudgetLimits, CancellationToken, OperationContext, ResourceBudget};
use binas_pdf::{
    AnnotationContentsMutationRequest, AnnotationCreateRequest, AnnotationRemoveRequest,
    AnnotationSubtype, AppearanceStatus, FormFieldCreateRequest, FormFieldKind,
    FormFieldRemoveRequest, FormValueMutationRequest, ImageColorSpace, ImageFilter,
    ImageMaskPolicy, ImageReplacementRequest, OpenOptions, PdfEngine, PdfErrorCode, PdfRevision,
    PdfRevisionRequest, PdfWriterMode, list_annotations, list_form_fields, list_image_xobjects,
};
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    std::env::var_os("BINAS_PRESERVATION_INPUT_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../qa/preservation"))
}
fn source(name: &str) -> Vec<u8> {
    std::fs::read(root().join(format!("fixtures/{name}.pdf"))).unwrap()
}
fn open(bytes: &[u8]) -> binas_pdf::PdfDocument {
    PdfEngine::default()
        .open(bytes, OpenOptions::default())
        .unwrap()
}
fn emit(name: &str, bytes: &[u8]) {
    if let Some(directory) = std::env::var_os("BINAS_NONTEXT_OUTPUT_DIR") {
        let directory = PathBuf::from(directory);
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(directory.join(format!("{name}.pdf")), bytes).unwrap();
    }
}
fn annotation() -> AnnotationCreateRequest {
    AnnotationCreateRequest {
        page_index: 0,
        subtype: AnnotationSubtype::Text,
        rect: [40.0, 40.0, 64.0, 64.0],
        contents: "boundary note".into(),
        quad_points: vec![],
        uri: String::new(),
    }
}
fn context() -> OperationContext {
    OperationContext::new(
        ResourceBudget::new(BudgetLimits {
            max_live_bytes: 64 * 1024 * 1024,
            max_work_units: 256 * 1024 * 1024,
        }),
        CancellationToken::new(),
    )
}
fn handshake(state: &str) {
    if std::env::var_os("BINAS_NONTEXT_MEMORY_HOLD").is_some() {
        use std::io::Write;
        println!("{state}");
        std::io::stdout().flush().unwrap();
        let mut line = String::new();
        std::io::stdin().read_line(&mut line).unwrap();
    }
}

#[test]
fn public_annotation_lifecycle_and_retained_contents() {
    handshake("READY");
    let input = source("text-field-fill");
    let original = list_annotations(&open(&input)).unwrap();
    let created = open(&input).create_annotation(annotation()).unwrap();
    assert!(created.verification.passed);
    emit("annotation-created", &created.bytes);
    let annotations = list_annotations(&open(&created.bytes)).unwrap();
    assert_eq!(annotations.len(), original.len() + 1);
    let index = annotations
        .iter()
        .position(|a| a.contents.as_deref() == Some("boundary note"))
        .unwrap();
    let request = AnnotationContentsMutationRequest {
        annotation_index: index,
        contents: "boundary changed é".into(),
    };
    assert_eq!(
        open(&created.bytes)
            .set_annotation_contents(request.clone())
            .unwrap_err()
            .code,
        PdfErrorCode::UnsafeRewrite
    );
    let refused_revision = PdfRevision::new(open(&created.bytes));
    assert!(
        refused_revision
            .plan(
                PdfRevisionRequest {
                    annotations: vec![request],
                    ..Default::default()
                },
                &context()
            )
            .is_err()
    );
    let removed = open(&created.bytes)
        .remove_annotation(AnnotationRemoveRequest {
            annotation_index: index,
        })
        .unwrap();
    assert!(removed.verification.passed);
    assert_eq!(list_annotations(&open(&removed.bytes)).unwrap(), original);
    emit("annotation-removed", &removed.bytes);

    // Same frozen PDF, one explicitly recorded shape control: convert Link18
    // to an appearance-free Text note, preserving byte count and every offset.
    let old = b"<< /Type /Annot /Subtype /Link /P 3 0 R /Rect [30 35 200 55] /Dest [4 0 R /Fit] /Border [0 0 1] /Contents (Retained link) >>";
    let new = b"<< /Type /Annot /Subtype /Text /P 3 0 R /Rect [30 35 200 55] /Contents (boundary note) >>";
    let offset = input.windows(old.len()).position(|v| v == old).unwrap();
    let mut seed = input.clone();
    seed[offset..offset + old.len()].fill(b' ');
    seed[offset..offset + new.len()].copy_from_slice(new);
    emit("annotation-seed", &seed);
    let index = list_annotations(&open(&seed))
        .unwrap()
        .iter()
        .position(|a| a.subtype == "Text")
        .unwrap();
    let request = AnnotationContentsMutationRequest {
        annotation_index: index,
        contents: "boundary changed é".into(),
    };
    let legacy = open(&seed)
        .set_annotation_contents(request.clone())
        .unwrap();
    assert!(legacy.verification.passed);
    // Changing Contents does not regenerate the existing note appearance.
    assert_ne!(
        legacy.report.appearance_status,
        AppearanceStatus::Regenerated
    );
    emit("annotation-legacy", &legacy.bytes);
    let mut revision = PdfRevision::new(open(&seed));
    let ctx = context();
    let plan = revision
        .plan(
            PdfRevisionRequest {
                annotations: vec![request],
                ..Default::default()
            },
            &ctx,
        )
        .unwrap();
    revision.commit(&plan, &ctx).unwrap();
    for (name, mode) in [
        ("incremental", PdfWriterMode::Incremental),
        ("compact", PdfWriterMode::Compact),
    ] {
        let exported = revision.export(mode, &ctx).unwrap();
        let after = list_annotations(&open(exported.output.as_bytes())).unwrap();
        assert_eq!(after[index].contents.as_deref(), Some("boundary changed é"));
        emit(&format!("annotation-{name}"), exported.output.as_bytes());
    }
    handshake("DONE");
}

#[test]
fn public_form_lifecycle_value_and_registration() {
    handshake("READY");
    let input = source("text-field-fill");
    let original = list_form_fields(&open(&input)).unwrap();
    let created = open(&input)
        .create_form_field(FormFieldCreateRequest {
            name: "boundary_field".into(),
            page_index: 0,
            rect: [40.0, 80.0, 180.0, 104.0],
            kind: FormFieldKind::Text,
            value: "initial".into(),
            options: vec![],
        })
        .unwrap();
    assert!(created.verification.passed);
    emit("form-created", &created.bytes);
    let error = open(&created.bytes)
        .set_form_field_value(FormValueMutationRequest {
            field_name: "boundary_field".into(),
            match_index: 0,
            value: "updated é".into(),
        })
        .unwrap_err();
    assert_eq!(error.code, PdfErrorCode::UnsafeRewrite);
    let removed = open(&created.bytes)
        .remove_form_field(FormFieldRemoveRequest {
            field_name: "boundary_field".into(),
            match_index: 0,
        })
        .unwrap();
    assert!(removed.verification.passed);
    assert_eq!(list_form_fields(&open(&removed.bytes)).unwrap(), original);
    emit("form-removed", &removed.bytes);
    handshake("DONE");
}

#[test]
fn public_invalid_selections_and_geometry_refuse_without_changing_source() {
    handshake("READY");
    let input = source("text-field-fill");
    let document = open(&input);
    let original = list_annotations(&document).unwrap();
    assert_eq!(
        document
            .remove_annotation(AnnotationRemoveRequest {
                annotation_index: usize::MAX
            })
            .unwrap_err()
            .code,
        PdfErrorCode::SelectionNotFound
    );
    let mut request = annotation();
    request.rect = [64.0, 40.0, 40.0, 64.0];
    assert_eq!(
        document.create_annotation(request).unwrap_err().code,
        PdfErrorCode::UnsafeRewrite
    );
    assert_eq!(
        document
            .remove_form_field(FormFieldRemoveRequest {
                field_name: "absent".into(),
                match_index: 0
            })
            .unwrap_err()
            .code,
        PdfErrorCode::SelectionNotFound
    );
    assert_eq!(list_annotations(&document).unwrap(), original);
    handshake("DONE");
}

#[test]
fn public_image_resource_query_and_nontext_subtype_refusal() {
    handshake("READY");
    let image_input = source("lossless-images");
    let images = list_image_xobjects(&open(&image_input)).unwrap();
    assert!(!images.is_empty());
    let input = source("text-field-fill");
    let document = open(&input);
    let annotations = list_annotations(&document).unwrap();
    let link = annotations
        .iter()
        .position(|a| a.subtype == "Link")
        .unwrap();
    assert_eq!(
        document
            .set_annotation_contents(AnnotationContentsMutationRequest {
                annotation_index: link,
                contents: "refused".into(),
            })
            .unwrap_err()
            .code,
        PdfErrorCode::UnsafeRewrite
    );
    assert_eq!(list_annotations(&document).unwrap(), annotations);
    handshake("DONE");
}

#[test]
fn public_raw_rgb_image_replacement_and_refusals() {
    handshake("READY");
    let input = source("lossless-images");
    let document = open(&input);
    let original = list_image_xobjects(&document).unwrap();
    let request = ImageReplacementRequest {
        object_number: 13,
        object_generation: 0,
        encoded_bytes: [20u8, 80, 180].repeat(4),
        width: 2,
        height: 2,
        bits_per_component: 8,
        color_space: ImageColorSpace::DeviceRgb,
        filter: ImageFilter::Raw,
        decode_params: None,
        mask_policy: ImageMaskPolicy::Reject,
    };
    let replaced = document.replace_image_xobject(request.clone()).unwrap();
    assert!(replaced.verification.passed);
    assert!(replaced.verification.object_reference_preserved);
    emit("image-replaced", &replaced.bytes);
    let mut invalid = request.clone();
    invalid.encoded_bytes.pop();
    assert!(document.replace_image_xobject(invalid).is_err());
    let mut masked = request;
    masked.object_number = 7;
    masked.width = 96;
    masked.height = 64;
    masked.encoded_bytes = [20u8, 80, 180].repeat(96 * 64);
    assert_eq!(
        document.replace_image_xobject(masked).unwrap_err().code,
        PdfErrorCode::UnsupportedFeature
    );
    assert_eq!(list_image_xobjects(&document).unwrap(), original);
    handshake("DONE");
}
