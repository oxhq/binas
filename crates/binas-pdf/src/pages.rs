use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::{
    OpenOptions, PdfDocument, PdfEngine, PdfError, PdfErrorCode,
    parser::{self, IndirectObject, ObjectRef, ParseBudget, ParsedDocument, Value},
    writer::refuse_security_boundaries,
};

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct PageTransform {
    /// Replaces the page's rotation. Only multiples of 90 degrees are accepted.
    pub rotation_degrees: Option<i32>,
    pub media_box: Option<[f64; 4]>,
    pub crop_box: Option<[f64; 4]>,
    pub translate: Option<[f64; 2]>,
    pub scale: Option<[f64; 2]>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PageCompositionPlacement {
    Underlay,
    Overlay,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct PageCompositionRequest {
    pub target_page_index: usize,
    pub source_page_index: usize,
    pub transform: [f64; 6],
    pub placement: PageCompositionPlacement,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct PageOperationReport {
    pub operation: String,
    pub input_pages: usize,
    pub output_pages: usize,
    pub copied_objects: usize,
    pub input_bytes: usize,
    pub output_bytes: usize,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct PageOperationVerification {
    pub passed: bool,
    pub reparsed: bool,
    pub page_count_matches: bool,
    pub page_order_unchanged: bool,
    pub no_dangling_references: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PageOperationOutcome {
    pub bytes: Vec<u8>,
    pub report: PageOperationReport,
    pub verification: PageOperationVerification,
}

#[derive(Clone, Copy)]
struct PageSpec<'a> {
    document: &'a PdfDocument,
    reference: ObjectRef,
}

impl PdfDocument {
    pub fn copy_pages(&self, page_indices: &[usize]) -> Result<PageOperationOutcome, PdfError> {
        let specs = selected_specs(self, page_indices)?;
        assemble_pages("copy_pages", &specs, self)
    }

    pub fn extract_pages(&self, page_indices: &[usize]) -> Result<PageOperationOutcome, PdfError> {
        let specs = selected_specs(self, page_indices)?;
        assemble_pages("extract_pages", &specs, self)
    }

    pub fn insert_pages(
        &self,
        at: usize,
        source: &PdfDocument,
        source_page_indices: &[usize],
    ) -> Result<PageOperationOutcome, PdfError> {
        let current = all_specs(self)?;
        if at > current.len() {
            return Err(selection_error(format!(
                "page insertion index {at} exceeds page count {}",
                current.len()
            )));
        }
        let inserted = selected_specs(source, source_page_indices)?;
        let total = current
            .len()
            .checked_add(inserted.len())
            .ok_or_else(|| PdfError::limit("inserted page count overflows"))?;
        if total > self.parsed().limits.max_pages {
            return Err(PdfError::limit("inserted page count exceeds max_pages"));
        }
        let mut specs = Vec::with_capacity(total);
        specs.extend_from_slice(&current[..at]);
        specs.extend(inserted);
        specs.extend_from_slice(&current[at..]);
        assemble_pages("insert_pages", &specs, self)
    }

    pub fn merge_pages(&self, sources: &[&PdfDocument]) -> Result<PageOperationOutcome, PdfError> {
        let mut specs = all_specs(self)?;
        let mut input_pages = specs.len();
        for source in sources {
            let pages = all_specs(source)?;
            input_pages = input_pages
                .checked_add(pages.len())
                .ok_or_else(|| PdfError::limit("merged page count overflows"))?;
            if input_pages > self.parsed().limits.max_pages {
                return Err(PdfError::limit("merged page count exceeds max_pages"));
            }
            specs.extend(pages);
        }
        assemble_pages("merge_pages", &specs, self)
    }

    /// Paints one page from `source` onto a page in this document.
    ///
    /// The source page becomes a Form XObject, so its resource namespace stays
    /// isolated from the target page. All indirect resources reachable from the
    /// source page's effective resource dictionary are copied and remapped.
    pub fn compose_page(
        &self,
        source: &PdfDocument,
        request: PageCompositionRequest,
    ) -> Result<PageOperationOutcome, PdfError> {
        refuse_security_boundaries(self.parsed())?;
        refuse_security_boundaries(source.parsed())?;
        validate_composition_transform(request.transform)?;
        let target_pages = self.page_refs()?;
        let source_pages = source.page_refs()?;
        let target = *target_pages.get(request.target_page_index).ok_or_else(|| {
            selection_error(format!(
                "target page index {} exceeds page count {}",
                request.target_page_index,
                target_pages.len()
            ))
        })?;
        let source_page = *source_pages.get(request.source_page_index).ok_or_else(|| {
            selection_error(format!(
                "source page index {} exceeds page count {}",
                request.source_page_index,
                source_pages.len()
            ))
        })?;

        let source_dictionary = materialize_page(source, source_page)?;
        let bbox = match source_dictionary.get(b"CropBox".as_slice()) {
            Some(value) => page_rectangle(source, Some(value), "source crop box")?,
            None => page_rectangle(
                source,
                source_dictionary.get(b"MediaBox".as_slice()),
                "source media box",
            )?,
        };
        let source_resources = source_dictionary
            .get(b"Resources".as_slice())
            .cloned()
            .unwrap_or_else(|| Value::Dict(BTreeMap::new()));
        let form_stream = decoded_page_content(source, source_page)?;

        let mut parsed = self.parsed().clone();
        let mut next = next_object_number(&parsed)?;
        let mut copied = BTreeMap::new();
        let mut mapping = BTreeMap::new();
        let remapped_resources = remap_value(
            &source_resources,
            PageSpec {
                document: source,
                reference: source_page,
            },
            &mut mapping,
            &mut copied,
            &mut next,
            self,
            0,
        )?;
        let remapped_group = source_dictionary
            .get(b"Group".as_slice())
            .map(|group| {
                remap_value(
                    group,
                    PageSpec {
                        document: source,
                        reference: source_page,
                    },
                    &mut mapping,
                    &mut copied,
                    &mut next,
                    self,
                    0,
                )
            })
            .transpose()?;
        let form_ref = allocate_reference_from_limits(self, &mut next)?;
        let placement_ref = allocate_reference_from_limits(self, &mut next)?;
        let mut form_dictionary = BTreeMap::from([
            (b"Type".to_vec(), Value::Name(b"XObject".to_vec())),
            (b"Subtype".to_vec(), Value::Name(b"Form".to_vec())),
            (b"FormType".to_vec(), Value::Integer(1)),
            (b"BBox".to_vec(), rectangle_value(bbox)),
            (b"Resources".to_vec(), remapped_resources),
        ]);
        if let Some(group) = remapped_group {
            form_dictionary.insert(b"Group".to_vec(), group);
        }
        copied.insert(
            form_ref,
            IndirectObject {
                value: Value::Dict(form_dictionary),
                stream: Some(form_stream),
                stream_offset: 0,
                offset: 0,
            },
        );

        let target_resources = materialize_page(self, target)?
            .get(b"Resources".as_slice())
            .cloned()
            .unwrap_or_else(|| Value::Dict(BTreeMap::new()));
        let mut target_resources =
            resolve_dictionary(&parsed, &target_resources, "target resources")?;
        let mut xobjects = match target_resources.remove(b"XObject".as_slice()) {
            Some(value) => resolve_dictionary(&parsed, &value, "target XObjects")?,
            None => BTreeMap::new(),
        };
        let resource_name = available_compositor_name(&xobjects);
        xobjects.insert(resource_name.clone(), Value::Ref(form_ref));
        target_resources.insert(b"XObject".to_vec(), Value::Dict(xobjects));
        let command = composition_command(&resource_name, request.transform);
        copied.insert(placement_ref, stream_object(command));
        parsed.objects.extend(copied);

        let page = parsed
            .objects
            .get_mut(&target)
            .ok_or_else(|| PdfError::syntax("target page object is missing", 0))?;
        let dictionary = as_dict_mut(&mut page.value, "target page")?;
        dictionary.insert(b"Resources".to_vec(), Value::Dict(target_resources));
        place_content(dictionary, placement_ref, request.placement)?;

        let operation = match request.placement {
            PageCompositionPlacement::Underlay => "compose_page_underlay",
            PageCompositionPlacement::Overlay => "compose_page_overlay",
        };
        finish_operation(
            operation,
            self,
            parsed,
            target_pages.len(),
            target_pages.len() + source_pages.len(),
            self.source_len()
                .checked_add(source.source_len())
                .ok_or_else(|| PdfError::limit("composition input byte count overflows"))?,
        )
    }

    pub fn transform_pages(
        &self,
        page_indices: &[usize],
        transform: PageTransform,
    ) -> Result<PageOperationOutcome, PdfError> {
        refuse_security_boundaries(self.parsed())?;
        validate_transform(&transform)?;
        let page_refs = self.page_refs()?;
        let selected = selected_indices(page_indices, page_refs.len())?;
        let mut parsed = self.parsed().clone();
        let matrix = transform_matrix(&transform);
        let selected_annotations = if matrix.is_some() {
            selected
                .iter()
                .map(|&index| {
                    let page = page_refs[index];
                    Ok((page, page_annotation_refs(&parsed, page)?))
                })
                .collect::<Result<BTreeMap<_, _>, PdfError>>()?
        } else {
            BTreeMap::new()
        };
        let annotation_owners = if selected_annotations.values().any(|refs| !refs.is_empty()) {
            annotation_owners(&parsed, &page_refs, &selected_annotations)?
        } else {
            BTreeMap::new()
        };
        let mut next = next_object_number(&parsed)?;
        if matrix.is_some() {
            let additions = selected.len();
            if parsed
                .objects
                .len()
                .checked_add(additions)
                .is_none_or(|count| count > parsed.limits.max_objects)
                || usize::try_from(next)
                    .ok()
                    .and_then(|next| next.checked_add(additions))
                    .is_none_or(|size| size > parsed.limits.max_xref_entries)
            {
                return Err(PdfError::limit(
                    "page transform objects exceed object or xref limits",
                ));
            }
        }
        for index in selected {
            let page_ref = page_refs[index];
            if transform.media_box.is_some() || transform.crop_box.is_some() {
                validate_effective_boxes(self, page_ref, &transform)?;
            }
            let wrapped_contents = if let Some(matrix) = matrix {
                let contents = page_contents(&parsed.object(page_ref)?.value)?;
                let annotations = selected_annotations.get(&page_ref).ok_or_else(|| {
                    PdfError::verification("selected page is absent from the annotation graph")
                })?;
                transform_page_annotations(
                    &mut parsed,
                    page_ref,
                    annotations,
                    &annotation_owners,
                    matrix,
                )?;
                let transformed = allocate_reference(&parsed, &mut next)?;
                let bytes = transformed_content(&parsed, &contents, matrix)?;
                parsed.objects.insert(transformed, stream_object(bytes));
                Some(Value::Ref(transformed))
            } else {
                None
            };
            let page = parsed
                .objects
                .get_mut(&page_ref)
                .ok_or_else(|| PdfError::syntax("selected page object is missing", 0))?;
            let dictionary = as_dict_mut(&mut page.value, "page")?;
            if let Some(rotation) = transform.rotation_degrees {
                dictionary.insert(
                    b"Rotate".to_vec(),
                    Value::Integer(i64::from(rotation.rem_euclid(360))),
                );
            }
            if let Some(rectangle) = transform.media_box {
                dictionary.insert(b"MediaBox".to_vec(), rectangle_value(rectangle));
            }
            if let Some(rectangle) = transform.crop_box {
                dictionary.insert(b"CropBox".to_vec(), rectangle_value(rectangle));
            }
            if let Some(contents) = wrapped_contents {
                dictionary.insert(b"Contents".to_vec(), contents);
            }
        }
        finish_operation(
            "transform_pages",
            self,
            parsed,
            page_refs.len(),
            page_refs.len(),
            self.source_len(),
        )
    }
}

fn validate_composition_transform(matrix: [f64; 6]) -> Result<(), PdfError> {
    if matrix.iter().any(|value| !value.is_finite()) {
        return Err(PdfError::unsupported(
            "page composition transform must be finite",
        ));
    }
    let determinant = matrix[0] * matrix[3] - matrix[1] * matrix[2];
    if !determinant.is_finite() || determinant == 0.0 {
        return Err(PdfError::unsupported(
            "page composition transform must be invertible",
        ));
    }
    Ok(())
}

fn decoded_page_content(document: &PdfDocument, page: ObjectRef) -> Result<Vec<u8>, PdfError> {
    let contents = page_contents(&document.parsed().object(page)?.value)?;
    transformed_content(document.parsed(), &contents, [1.0, 1.0, 0.0, 0.0])
}

fn resolve_dictionary(
    parsed: &ParsedDocument,
    value: &Value,
    label: &str,
) -> Result<BTreeMap<Vec<u8>, Value>, PdfError> {
    let mut value = value;
    let mut seen = BTreeSet::new();
    for _ in 0..=parsed.limits.max_parser_depth {
        match value {
            Value::Dict(dictionary) => return Ok(dictionary.clone()),
            Value::Ref(reference) => {
                if !seen.insert(*reference) {
                    return Err(PdfError::syntax(format!("cycle in {label}"), 0));
                }
                value = &parsed.object(*reference)?.value;
            }
            _ => {
                return Err(PdfError::unsupported(format!(
                    "{label} is not a dictionary"
                )));
            }
        }
    }
    Err(PdfError::limit(format!(
        "{label} indirection exceeds limit"
    )))
}

fn available_compositor_name(xobjects: &BTreeMap<Vec<u8>, Value>) -> Vec<u8> {
    for suffix in 0_u32.. {
        let name = if suffix == 0 {
            b"BinasPage".to_vec()
        } else {
            format!("BinasPage{suffix}").into_bytes()
        };
        if !xobjects.contains_key(&name) {
            return name;
        }
    }
    unreachable!()
}

fn composition_command(name: &[u8], matrix: [f64; 6]) -> Vec<u8> {
    format!(
        "q {} {} {} {} {} {} cm /{} Do Q\n",
        pdf_number(matrix[0]),
        pdf_number(matrix[1]),
        pdf_number(matrix[2]),
        pdf_number(matrix[3]),
        pdf_number(matrix[4]),
        pdf_number(matrix[5]),
        String::from_utf8_lossy(name)
    )
    .into_bytes()
}

fn place_content(
    dictionary: &mut BTreeMap<Vec<u8>, Value>,
    content: ObjectRef,
    placement: PageCompositionPlacement,
) -> Result<(), PdfError> {
    let existing = dictionary.remove(b"Contents".as_slice());
    let value = match (existing, placement) {
        (None, _) => Value::Ref(content),
        (Some(Value::Ref(existing)), PageCompositionPlacement::Underlay) => {
            Value::Array(vec![Value::Ref(content), Value::Ref(existing)])
        }
        (Some(Value::Ref(existing)), PageCompositionPlacement::Overlay) => {
            Value::Array(vec![Value::Ref(existing), Value::Ref(content)])
        }
        (Some(Value::Array(mut values)), PageCompositionPlacement::Underlay)
            if values.iter().all(|value| matches!(value, Value::Ref(_))) =>
        {
            values.insert(0, Value::Ref(content));
            Value::Array(values)
        }
        (Some(Value::Array(mut values)), PageCompositionPlacement::Overlay)
            if values.iter().all(|value| matches!(value, Value::Ref(_))) =>
        {
            values.push(Value::Ref(content));
            Value::Array(values)
        }
        (Some(_), _) => {
            return Err(PdfError::unsupported(
                "page composition requires indirect target content streams",
            ));
        }
    };
    dictionary.insert(b"Contents".to_vec(), value);
    Ok(())
}

fn assemble_pages(
    operation: &str,
    specs: &[PageSpec<'_>],
    base: &PdfDocument,
) -> Result<PageOperationOutcome, PdfError> {
    if specs.len() > base.parsed().limits.max_pages {
        return Err(PdfError::limit("output page count exceeds max_pages"));
    }
    refuse_security_boundaries(base.parsed())?;
    for spec in specs {
        refuse_security_boundaries(spec.document.parsed())?;
    }
    let catalog_ref = ObjectRef {
        number: 1,
        generation: 0,
    };
    let pages_ref = ObjectRef {
        number: 2,
        generation: 0,
    };
    let mut objects = BTreeMap::new();
    let mut kids = Vec::with_capacity(specs.len());
    let mut next = 3_u32;
    for spec in specs {
        let page = copy_page_closure(*spec, pages_ref, &mut objects, &mut next, base)?;
        kids.push(Value::Ref(page));
    }
    let mut catalog = BTreeMap::new();
    catalog.insert(b"Type".to_vec(), Value::Name(b"Catalog".to_vec()));
    catalog.insert(b"Pages".to_vec(), Value::Ref(pages_ref));
    objects.insert(catalog_ref, plain_object(Value::Dict(catalog)));
    let mut pages = BTreeMap::new();
    pages.insert(b"Type".to_vec(), Value::Name(b"Pages".to_vec()));
    pages.insert(b"Kids".to_vec(), Value::Array(kids));
    pages.insert(
        b"Count".to_vec(),
        Value::Integer(
            i64::try_from(specs.len()).map_err(|_| PdfError::limit("page count exceeds i64"))?,
        ),
    );
    objects.insert(pages_ref, plain_object(Value::Dict(pages)));
    let mut trailer = BTreeMap::new();
    trailer.insert(b"Root".to_vec(), Value::Ref(catalog_ref));
    trailer.insert(b"Size".to_vec(), Value::Integer(i64::from(next)));
    let mut parsed = base.parsed().clone();
    parsed.version = "1.7".into();
    parsed.objects = objects;
    parsed.trailer = Value::Dict(trailer);
    let mut documents = BTreeSet::new();
    documents.insert(std::ptr::from_ref(base) as usize);
    let mut input_bytes = base.source_len();
    let mut input_pages = base.page_refs()?.len();
    for spec in specs {
        if !documents.insert(std::ptr::from_ref(spec.document) as usize) {
            continue;
        }
        input_bytes = input_bytes
            .checked_add(spec.document.source_len())
            .ok_or_else(|| PdfError::limit("input byte count overflows"))?;
        input_pages = input_pages
            .checked_add(spec.document.page_refs()?.len())
            .ok_or_else(|| PdfError::limit("input page count overflows"))?;
    }
    finish_operation(
        operation,
        base,
        parsed,
        specs.len(),
        input_pages,
        input_bytes,
    )
}

fn copy_page_closure(
    spec: PageSpec<'_>,
    parent: ObjectRef,
    output: &mut BTreeMap<ObjectRef, IndirectObject>,
    next: &mut u32,
    base: &PdfDocument,
) -> Result<ObjectRef, PdfError> {
    let page_ref = allocate_reference_from_limits(base, next)?;
    let mut mapping = BTreeMap::new();
    mapping.insert(spec.reference, page_ref);
    let page = materialize_page(spec.document, spec.reference)?;
    let mut value = remap_value(
        &Value::Dict(page),
        spec,
        &mut mapping,
        output,
        next,
        base,
        0,
    )?;
    as_dict_mut(&mut value, "copied page")?.insert(b"Parent".to_vec(), Value::Ref(parent));
    output.insert(page_ref, plain_object(value));
    Ok(page_ref)
}

#[allow(clippy::too_many_arguments)]
fn remap_value(
    value: &Value,
    spec: PageSpec<'_>,
    mapping: &mut BTreeMap<ObjectRef, ObjectRef>,
    output: &mut BTreeMap<ObjectRef, IndirectObject>,
    next: &mut u32,
    base: &PdfDocument,
    depth: usize,
) -> Result<Value, PdfError> {
    if depth > base.parsed().limits.max_parser_depth {
        return Err(PdfError::limit("copied object graph depth exceeds limit"));
    }
    match value {
        Value::Ref(reference) => {
            copy_reference(*reference, spec, mapping, output, next, base, depth + 1).map(Value::Ref)
        }
        Value::Array(values) => values
            .iter()
            .map(|value| remap_value(value, spec, mapping, output, next, base, depth + 1))
            .collect::<Result<Vec<_>, _>>()
            .map(Value::Array),
        Value::Dict(dictionary) => dictionary
            .iter()
            .map(|(key, value)| {
                Ok((
                    key.clone(),
                    remap_value(value, spec, mapping, output, next, base, depth + 1)?,
                ))
            })
            .collect::<Result<BTreeMap<_, _>, PdfError>>()
            .map(Value::Dict),
        _ => Ok(value.clone()),
    }
}

#[allow(clippy::too_many_arguments)]
fn copy_reference(
    reference: ObjectRef,
    spec: PageSpec<'_>,
    mapping: &mut BTreeMap<ObjectRef, ObjectRef>,
    output: &mut BTreeMap<ObjectRef, IndirectObject>,
    next: &mut u32,
    base: &PdfDocument,
    depth: usize,
) -> Result<ObjectRef, PdfError> {
    if let Some(mapped) = mapping.get(&reference) {
        return Ok(*mapped);
    }
    let source = spec.document.parsed().object(reference)?;
    if is_type(&source.value, b"Page") || is_type(&source.value, b"Pages") {
        return Err(PdfError::unsupported(
            "page dependencies that reference another page or page-tree node are not supported",
        ));
    }
    if is_subtype(&source.value, b"Widget") {
        return Err(PdfError::unsupported(
            "copying widget annotations without their document AcroForm is not supported",
        ));
    }
    let mapped = allocate_reference_from_limits(base, next)?;
    mapping.insert(reference, mapped);
    let value = remap_value(&source.value, spec, mapping, output, next, base, depth)?;
    output.insert(
        mapped,
        IndirectObject {
            value,
            stream: source.stream.clone(),
            stream_offset: 0,
            offset: 0,
        },
    );
    Ok(mapped)
}

fn materialize_page(
    document: &PdfDocument,
    page_ref: ObjectRef,
) -> Result<BTreeMap<Vec<u8>, Value>, PdfError> {
    let page = document.parsed().object(page_ref)?;
    let Value::Dict(mut result) = page.value.clone() else {
        return Err(PdfError::syntax(
            "page object is not a dictionary",
            page.offset,
        ));
    };
    if !is_type(&page.value, b"Page") || page.stream.is_some() {
        return Err(PdfError::syntax(
            "page reference is not a page dictionary",
            page.offset,
        ));
    }
    if let Some(Value::Array(annotations)) = result.get(b"Annots".as_slice())
        && annotations
            .iter()
            .any(|annotation| is_subtype(annotation, b"Widget"))
    {
        return Err(PdfError::unsupported(
            "copying widget annotations without their document AcroForm is not supported",
        ));
    }
    result.remove(b"Parent".as_slice());
    let keys: [&[u8]; 7] = [
        b"Resources",
        b"MediaBox",
        b"CropBox",
        b"Rotate",
        b"BleedBox",
        b"TrimBox",
        b"ArtBox",
    ];
    let mut current = page_ref;
    let mut visited = BTreeSet::new();
    for _ in 0..=document.parsed().limits.max_parser_depth {
        if !visited.insert(current) {
            return Err(PdfError::syntax("cycle in page inheritance", 0));
        }
        let object = document.parsed().object(current)?;
        let dictionary = as_dict(&object.value, "page inheritance node")?;
        for key in keys {
            if !result.contains_key(key)
                && let Some(value) = dictionary.get(key)
            {
                result.insert(key.to_vec(), value.clone());
            }
        }
        match dictionary.get(b"Parent".as_slice()) {
            Some(Value::Ref(parent)) => current = *parent,
            None => return Ok(result),
            Some(_) => {
                return Err(PdfError::syntax(
                    "page /Parent is not a reference",
                    object.offset,
                ));
            }
        }
    }
    Err(PdfError::limit("page inheritance depth exceeds limit"))
}

fn finish_operation(
    operation: &str,
    base: &PdfDocument,
    parsed: ParsedDocument,
    expected_pages: usize,
    input_pages: usize,
    input_bytes: usize,
) -> Result<PageOperationOutcome, PdfError> {
    validate_references(&parsed)?;
    let staged = base.with_parsed(parsed);
    if staged.page_count()? != expected_pages {
        return Err(PdfError::verification(
            "staged page count does not match request",
        ));
    }
    let copied_objects = staged.parsed().objects.len();
    let canonical = staged.canonicalize()?;
    let reopened = PdfEngine::new(base.engine_config().clone())
        .open(&canonical.bytes, OpenOptions::default())
        .map_err(|error| PdfError::verification(format!("page output did not reparse: {error}")))?;
    let page_count_matches = reopened.page_count()? == expected_pages;
    let no_dangling_references = validate_references(reopened.parsed()).is_ok();
    let page_order_unchanged = page_values(&staged)? == page_values(&reopened)?;
    let verification = PageOperationVerification {
        passed: page_count_matches && page_order_unchanged && no_dangling_references,
        reparsed: true,
        page_count_matches,
        page_order_unchanged,
        no_dangling_references,
    };
    if !verification.passed {
        return Err(PdfError::verification(
            "page output failed post-write verification",
        ));
    }
    Ok(PageOperationOutcome {
        report: PageOperationReport {
            operation: operation.into(),
            input_pages,
            output_pages: expected_pages,
            copied_objects,
            input_bytes,
            output_bytes: canonical.bytes.len(),
        },
        bytes: canonical.bytes,
        verification,
    })
}

fn all_specs(document: &PdfDocument) -> Result<Vec<PageSpec<'_>>, PdfError> {
    Ok(document
        .page_refs()?
        .into_iter()
        .map(|reference| PageSpec {
            document,
            reference,
        })
        .collect())
}

fn selected_specs<'a>(
    document: &'a PdfDocument,
    indices: &[usize],
) -> Result<Vec<PageSpec<'a>>, PdfError> {
    let pages = document.page_refs()?;
    indices
        .iter()
        .map(|&index| {
            pages
                .get(index)
                .copied()
                .map(|reference| PageSpec {
                    document,
                    reference,
                })
                .ok_or_else(|| {
                    selection_error(format!(
                        "page index {index} exceeds page count {}",
                        pages.len()
                    ))
                })
        })
        .collect()
}

fn selected_indices(indices: &[usize], page_count: usize) -> Result<Vec<usize>, PdfError> {
    let mut seen = BTreeSet::new();
    for &index in indices {
        if index >= page_count {
            return Err(selection_error(format!(
                "page index {index} exceeds page count {page_count}"
            )));
        }
        if !seen.insert(index) {
            return Err(PdfError::unsupported(
                "a page transform selection cannot contain duplicates",
            ));
        }
    }
    Ok(seen.into_iter().collect())
}

fn page_values(document: &PdfDocument) -> Result<Vec<Value>, PdfError> {
    document
        .page_refs()?
        .into_iter()
        .map(|reference| Ok(document.parsed().object(reference)?.value.clone()))
        .collect()
}

fn validate_references(parsed: &ParsedDocument) -> Result<(), PdfError> {
    fn visit(value: &Value, parsed: &ParsedDocument, depth: usize) -> Result<(), PdfError> {
        if depth > parsed.limits.max_parser_depth {
            return Err(PdfError::limit("reference validation depth exceeds limit"));
        }
        match value {
            Value::Ref(reference) if !parsed.objects.contains_key(reference) => {
                return Err(PdfError::verification(format!(
                    "dangling reference {} {} R",
                    reference.number, reference.generation
                )));
            }
            Value::Array(values) => {
                for value in values {
                    visit(value, parsed, depth + 1)?;
                }
            }
            Value::Dict(dictionary) => {
                for value in dictionary.values() {
                    visit(value, parsed, depth + 1)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    visit(&parsed.trailer, parsed, 0)?;
    for object in parsed.objects.values() {
        visit(&object.value, parsed, 0)?;
    }
    Ok(())
}

fn validate_transform(transform: &PageTransform) -> Result<(), PdfError> {
    if transform == &PageTransform::default() {
        return Err(PdfError::unsupported("page transform has no operations"));
    }
    if let Some(rotation) = transform.rotation_degrees
        && rotation % 90 != 0
    {
        return Err(PdfError::unsupported(
            "page rotation must be a multiple of 90 degrees",
        ));
    }
    if let Some(rectangle) = transform.media_box {
        validate_rectangle(rectangle, "media box")?;
    }
    if let Some(rectangle) = transform.crop_box {
        validate_rectangle(rectangle, "crop box")?;
    }
    if let (Some(media), Some(crop)) = (transform.media_box, transform.crop_box)
        && (crop[0] < media[0] || crop[1] < media[1] || crop[2] > media[2] || crop[3] > media[3])
    {
        return Err(PdfError::unsafe_rewrite(
            "crop box must fit within media box",
        ));
    }
    if let Some(translate) = transform.translate
        && !translate.iter().all(|value| value.is_finite())
    {
        return Err(PdfError::unsupported("page translation must be finite"));
    }
    if let Some(scale) = transform.scale
        && (!scale.iter().all(|value| value.is_finite()) || scale.contains(&0.0))
    {
        return Err(PdfError::unsupported(
            "page scale must be finite and non-zero",
        ));
    }
    Ok(())
}

fn validate_rectangle(rectangle: [f64; 4], label: &str) -> Result<(), PdfError> {
    if !rectangle.iter().all(|value| value.is_finite())
        || rectangle[2] <= rectangle[0]
        || rectangle[3] <= rectangle[1]
    {
        return Err(PdfError::unsupported(format!(
            "{label} must be finite with positive width and height"
        )));
    }
    Ok(())
}

fn validate_effective_boxes(
    document: &PdfDocument,
    page_ref: ObjectRef,
    transform: &PageTransform,
) -> Result<(), PdfError> {
    let page = materialize_page(document, page_ref)?;
    let media = match transform.media_box {
        Some(media) => media,
        None => page_rectangle(document, page.get(b"MediaBox".as_slice()), "media box")?,
    };
    let crop = match transform.crop_box {
        Some(crop) => crop,
        None => match page.get(b"CropBox".as_slice()) {
            Some(value) => page_rectangle(document, Some(value), "crop box")?,
            None => media,
        },
    };
    if crop[0] < media[0] || crop[1] < media[1] || crop[2] > media[2] || crop[3] > media[3] {
        return Err(PdfError::unsafe_rewrite(
            "effective crop box must fit within effective media box",
        ));
    }
    Ok(())
}

pub(crate) fn page_rectangle(
    document: &PdfDocument,
    value: Option<&Value>,
    label: &str,
) -> Result<[f64; 4], PdfError> {
    let mut value = value.ok_or_else(|| PdfError::syntax(format!("page has no {label}"), 0))?;
    let mut visited = BTreeSet::new();
    for _ in 0..=document.parsed().limits.max_parser_depth {
        match value {
            Value::Ref(reference) => {
                if !visited.insert(*reference) {
                    return Err(PdfError::syntax(format!("cycle in page {label}"), 0));
                }
                value = &document.parsed().object(*reference)?.value;
            }
            Value::Array(values) if values.len() == 4 => {
                let mut rectangle = [0.0; 4];
                for (index, value) in values.iter().enumerate() {
                    rectangle[index] = match value {
                        Value::Integer(value) => *value as f64,
                        Value::Real(value) => *value,
                        _ => {
                            return Err(PdfError::unsupported(format!(
                                "page {label} contains a non-number"
                            )));
                        }
                    };
                }
                validate_rectangle(rectangle, label)?;
                return Ok(rectangle);
            }
            _ => {
                return Err(PdfError::unsupported(format!(
                    "page {label} is not a four-number array"
                )));
            }
        }
    }
    Err(PdfError::limit(format!(
        "page {label} indirection exceeds limit"
    )))
}

fn transform_matrix(transform: &PageTransform) -> Option<[f64; 4]> {
    if transform.translate.is_none() && transform.scale.is_none() {
        return None;
    }
    let [sx, sy] = transform.scale.unwrap_or([1.0, 1.0]);
    let [tx, ty] = transform.translate.unwrap_or([0.0, 0.0]);
    Some([sx, sy, tx, ty])
}

fn page_annotation_refs(
    parsed: &ParsedDocument,
    page: ObjectRef,
) -> Result<Vec<ObjectRef>, PdfError> {
    let page = parsed.object(page)?;
    let dictionary = as_dict(&page.value, "page")?;
    let Some(value) = dictionary.get(b"Annots".as_slice()) else {
        return Ok(Vec::new());
    };
    let mut value = value;
    let mut seen_arrays = BTreeSet::new();
    for _ in 0..=parsed.limits.max_parser_depth {
        match value {
            Value::Array(values) => {
                if values.len() > parsed.limits.max_container_items {
                    return Err(PdfError::limit("page /Annots exceeds container limit"));
                }
                let mut references = Vec::with_capacity(values.len());
                let mut unique = BTreeSet::new();
                for value in values {
                    let Value::Ref(reference) = value else {
                        return Err(PdfError::unsupported(
                            "page annotation transforms require indirect annotation dictionaries",
                        ));
                    };
                    if !unique.insert(*reference) {
                        return Err(PdfError::unsupported(
                            "page annotation transforms require unique annotations",
                        ));
                    }
                    references.push(*reference);
                }
                return Ok(references);
            }
            Value::Ref(reference) => {
                if !seen_arrays.insert(*reference) {
                    return Err(PdfError::syntax("cycle in page /Annots", 0));
                }
                value = &parsed.object(*reference)?.value;
            }
            _ => {
                return Err(PdfError::unsupported(
                    "page /Annots must be an array or indirect array",
                ));
            }
        }
    }
    Err(PdfError::limit("page /Annots indirection exceeds limit"))
}

fn annotation_owners(
    parsed: &ParsedDocument,
    pages: &[ObjectRef],
    selected_annotations: &BTreeMap<ObjectRef, Vec<ObjectRef>>,
) -> Result<BTreeMap<ObjectRef, BTreeSet<ObjectRef>>, PdfError> {
    let selected = selected_annotations
        .values()
        .flatten()
        .copied()
        .collect::<BTreeSet<_>>();
    let mut owners = selected
        .iter()
        .copied()
        .map(|annotation| (annotation, BTreeSet::new()))
        .collect::<BTreeMap<_, _>>();
    for &page in pages {
        for annotation in page_selected_annotation_refs(parsed, page, &selected)? {
            owners
                .get_mut(&annotation)
                .ok_or_else(|| PdfError::verification("selected annotation owner is missing"))?
                .insert(page);
        }
    }
    Ok(owners)
}

fn page_selected_annotation_refs(
    parsed: &ParsedDocument,
    page: ObjectRef,
    selected: &BTreeSet<ObjectRef>,
) -> Result<BTreeSet<ObjectRef>, PdfError> {
    let page = parsed.object(page)?;
    let dictionary = as_dict(&page.value, "page")?;
    let Some(value) = dictionary.get(b"Annots".as_slice()) else {
        return Ok(BTreeSet::new());
    };
    let mut value = value;
    let mut seen_arrays = BTreeSet::new();
    for _ in 0..=parsed.limits.max_parser_depth {
        match value {
            Value::Array(values) => {
                return Ok(values
                    .iter()
                    .filter_map(|value| match value {
                        Value::Ref(reference) if selected.contains(reference) => Some(*reference),
                        _ => None,
                    })
                    .collect());
            }
            Value::Ref(reference) => {
                if selected.contains(reference) {
                    return Ok(BTreeSet::from([*reference]));
                }
                if !seen_arrays.insert(*reference) {
                    return Ok(BTreeSet::new());
                }
                let Some(object) = parsed.objects.get(reference) else {
                    return Ok(BTreeSet::new());
                };
                value = &object.value;
            }
            _ => return Ok(BTreeSet::new()),
        }
    }
    Ok(BTreeSet::new())
}

fn transform_page_annotations(
    parsed: &mut ParsedDocument,
    page: ObjectRef,
    annotations: &[ObjectRef],
    owners: &BTreeMap<ObjectRef, BTreeSet<ObjectRef>>,
    matrix: [f64; 4],
) -> Result<(), PdfError> {
    if annotations.is_empty() {
        return Ok(());
    }
    if matrix[0] <= 0.0 || matrix[1] <= 0.0 {
        return Err(PdfError::unsupported(
            "page annotation transforms require positive scales",
        ));
    }
    for &annotation in annotations {
        let owner_pages = owners.get(&annotation).ok_or_else(|| {
            PdfError::verification("selected annotation is absent from the ownership graph")
        })?;
        if owner_pages.len() != 1 || !owner_pages.contains(&page) {
            return Err(PdfError::unsupported(
                "page annotation transforms require annotations owned by one selected page",
            ));
        }
        let (rect, quad_points) =
            transformed_annotation_geometry(parsed, annotation, page, matrix)?;
        let object = parsed
            .objects
            .get_mut(&annotation)
            .ok_or_else(|| PdfError::syntax("annotation object is missing", 0))?;
        let dictionary = as_dict_mut(&mut object.value, "annotation")?;
        dictionary.insert(b"Rect".to_vec(), rectangle_value(rect));
        if let Some(quad_points) = quad_points {
            dictionary.insert(b"QuadPoints".to_vec(), Value::Array(quad_points));
        }
    }
    Ok(())
}

fn transformed_annotation_geometry(
    parsed: &ParsedDocument,
    annotation: ObjectRef,
    page: ObjectRef,
    matrix: [f64; 4],
) -> Result<([f64; 4], Option<Vec<Value>>), PdfError> {
    let object = parsed.object(annotation)?;
    if object.stream.is_some() {
        return Err(PdfError::unsupported(
            "page annotation transforms do not support annotation streams",
        ));
    }
    let dictionary = as_dict(&object.value, "annotation")?;
    for key in [
        b"AP".as_slice(),
        b"AS".as_slice(),
        b"Border".as_slice(),
        b"BS".as_slice(),
        b"CL".as_slice(),
        b"InkList".as_slice(),
        b"L".as_slice(),
        b"Popup".as_slice(),
        b"RD".as_slice(),
        b"Vertices".as_slice(),
    ] {
        if dictionary.contains_key(key) {
            return Err(PdfError::unsupported(
                "page annotation transforms do not support appearances or alternate geometry",
            ));
        }
    }
    if let Some(value) = dictionary.get(b"P".as_slice())
        && !matches!(value, Value::Ref(reference) if *reference == page)
    {
        return Err(PdfError::unsupported(
            "annotation /P must reference its transformed page",
        ));
    }
    let subtype = match dictionary.get(b"Subtype".as_slice()) {
        Some(Value::Name(value)) => value.as_slice(),
        _ => {
            return Err(PdfError::unsupported(
                "page annotation transforms require a named /Subtype",
            ));
        }
    };
    let markup = matches!(
        subtype,
        b"Highlight" | b"Underline" | b"StrikeOut" | b"Squiggly"
    );
    // ponytail: support only native geometry; add appearance-matrix rewriting before broader annotation support.
    if !markup && subtype != b"Link" {
        return Err(PdfError::unsupported(
            "page annotation transforms only support Link and text markup annotations",
        ));
    }
    if subtype == b"Link" {
        refuse_link_destinations(parsed, dictionary)?;
    }
    let rect = annotation_rectangle(dictionary.get(b"Rect".as_slice()))?;
    let rect = transform_annotation_rect(rect, matrix)?;
    let quad_points = match dictionary.get(b"QuadPoints".as_slice()) {
        None if markup => {
            return Err(PdfError::unsupported(
                "text markup annotation transforms require /QuadPoints",
            ));
        }
        None => None,
        Some(value) => Some(transform_annotation_quad_points(
            value,
            rect,
            matrix,
            parsed.limits.max_container_items,
        )?),
    };
    Ok((rect, quad_points))
}

fn refuse_link_destinations(
    parsed: &ParsedDocument,
    dictionary: &BTreeMap<Vec<u8>, Value>,
) -> Result<(), PdfError> {
    if dictionary.contains_key(b"Dest".as_slice()) {
        return Err(PdfError::unsupported(
            "Link /Dest cannot be preserved while page coordinates are transformed",
        ));
    }
    let Some(action) = dictionary.get(b"A".as_slice()) else {
        return Ok(());
    };
    let action = resolve_dictionary(parsed, action, "Link /A")?;
    if action.contains_key(b"D".as_slice())
        || action.contains_key(b"Next".as_slice())
        || matches!(action.get(b"S".as_slice()), Some(Value::Name(name)) if name == b"GoTo" || name == b"GoToR")
    {
        return Err(PdfError::unsupported(
            "Link destination actions cannot be preserved while page coordinates are transformed",
        ));
    }
    Ok(())
}

fn annotation_rectangle(value: Option<&Value>) -> Result<[f64; 4], PdfError> {
    let Some(Value::Array(values)) = value else {
        return Err(PdfError::unsupported(
            "page annotation transforms require a direct four-number /Rect",
        ));
    };
    if values.len() != 4 {
        return Err(PdfError::unsupported(
            "page annotation transforms require a direct four-number /Rect",
        ));
    }
    let mut rect = [0.0; 4];
    for (index, value) in values.iter().enumerate() {
        rect[index] = annotation_number(value)?;
    }
    validate_rectangle(rect, "annotation /Rect")?;
    Ok(rect)
}

fn transform_annotation_rect(rect: [f64; 4], matrix: [f64; 4]) -> Result<[f64; 4], PdfError> {
    let transformed = [
        rect[0] * matrix[0] + matrix[2],
        rect[1] * matrix[1] + matrix[3],
        rect[2] * matrix[0] + matrix[2],
        rect[3] * matrix[1] + matrix[3],
    ];
    validate_rectangle(transformed, "transformed annotation /Rect")?;
    Ok(transformed)
}

fn transform_annotation_quad_points(
    value: &Value,
    rect: [f64; 4],
    matrix: [f64; 4],
    max_items: usize,
) -> Result<Vec<Value>, PdfError> {
    let Value::Array(values) = value else {
        return Err(PdfError::unsupported(
            "page annotation /QuadPoints must be a direct number array",
        ));
    };
    if values.is_empty() || !values.len().is_multiple_of(8) || values.len() > max_items {
        return Err(PdfError::unsupported(
            "page annotation /QuadPoints must contain finite groups of eight numbers",
        ));
    }
    let mut result = Vec::with_capacity(values.len());
    for pair in values.chunks_exact(2) {
        let x = annotation_number(&pair[0])?;
        let y = annotation_number(&pair[1])?;
        let x = x * matrix[0] + matrix[2];
        let y = y * matrix[1] + matrix[3];
        if !x.is_finite()
            || !y.is_finite()
            || x < rect[0]
            || x > rect[2]
            || y < rect[1]
            || y > rect[3]
        {
            return Err(PdfError::unsupported(
                "page annotation /QuadPoints must stay inside its transformed /Rect",
            ));
        }
        result.push(Value::Real(x));
        result.push(Value::Real(y));
    }
    Ok(result)
}

fn annotation_number(value: &Value) -> Result<f64, PdfError> {
    match value {
        Value::Integer(value) if value.unsigned_abs() < (1_u64 << 53) => Ok(*value as f64),
        Value::Real(value) if value.is_finite() => Ok(*value),
        _ => Err(PdfError::unsupported(
            "page annotation geometry must contain finite numbers",
        )),
    }
}

fn page_contents(value: &Value) -> Result<Vec<ObjectRef>, PdfError> {
    let dictionary = as_dict(value, "page")?;
    match dictionary.get(b"Contents".as_slice()) {
        None => Ok(Vec::new()),
        Some(Value::Ref(reference)) => Ok(vec![*reference]),
        Some(Value::Array(values)) => values
            .iter()
            .map(|value| match value {
                Value::Ref(reference) => Ok(*reference),
                _ => Err(PdfError::unsupported(
                    "page /Contents arrays must contain only references",
                )),
            })
            .collect(),
        Some(_) => Err(PdfError::unsupported(
            "direct page content streams are not supported",
        )),
    }
}

fn transformed_content(
    parsed: &ParsedDocument,
    contents: &[ObjectRef],
    matrix: [f64; 4],
) -> Result<Vec<u8>, PdfError> {
    let mut output = format!(
        "q {} 0 0 {} {} {} cm\n",
        pdf_number(matrix[0]),
        pdf_number(matrix[1]),
        pdf_number(matrix[2]),
        pdf_number(matrix[3])
    )
    .into_bytes();
    let mut budget = ParseBudget::default();
    for reference in contents {
        let object = parsed.object(*reference)?;
        let stream = object
            .stream
            .as_deref()
            .ok_or_else(|| PdfError::unsupported("page /Contents reference is not a stream"))?;
        let decoded = parser::decode_stream(&object.value, stream, &parsed.limits, &mut budget)?;
        let length = output
            .len()
            .checked_add(decoded.len())
            .and_then(|length| length.checked_add(1))
            .ok_or_else(|| PdfError::limit("transformed content length overflows"))?;
        if length > parsed.limits.max_stream_bytes {
            return Err(PdfError::limit(
                "transformed content exceeds max_stream_bytes",
            ));
        }
        output.extend_from_slice(&decoded);
        output.push(b'\n');
    }
    if output
        .len()
        .checked_add(1)
        .is_none_or(|length| length > parsed.limits.max_stream_bytes)
    {
        return Err(PdfError::limit(
            "transformed content exceeds max_stream_bytes",
        ));
    }
    output.push(b'Q');
    Ok(output)
}

fn next_object_number(parsed: &ParsedDocument) -> Result<u32, PdfError> {
    parsed
        .objects
        .keys()
        .map(|reference| reference.number)
        .max()
        .unwrap_or(0)
        .checked_add(1)
        .ok_or_else(|| PdfError::limit("object number overflows u32"))
}

fn allocate_reference(parsed: &ParsedDocument, next: &mut u32) -> Result<ObjectRef, PdfError> {
    let count = parsed
        .objects
        .len()
        .checked_add(1)
        .ok_or_else(|| PdfError::limit("object count overflows"))?;
    if count > parsed.limits.max_objects {
        return Err(PdfError::limit("new page objects exceed max_objects"));
    }
    let reference = ObjectRef {
        number: *next,
        generation: 0,
    };
    *next = next
        .checked_add(1)
        .ok_or_else(|| PdfError::limit("object number overflows u32"))?;
    if usize::try_from(*next).map_or(true, |size| size > parsed.limits.max_xref_entries) {
        return Err(PdfError::limit("new page objects exceed max_xref_entries"));
    }
    Ok(reference)
}

fn allocate_reference_from_limits(
    base: &PdfDocument,
    next: &mut u32,
) -> Result<ObjectRef, PdfError> {
    let object_count = usize::try_from(*next)
        .ok()
        .and_then(|next| next.checked_sub(1))
        .ok_or_else(|| PdfError::limit("copied object count overflows"))?;
    if object_count >= base.parsed().limits.max_objects {
        return Err(PdfError::limit("copied object closure exceeds max_objects"));
    }
    let reference = ObjectRef {
        number: *next,
        generation: 0,
    };
    *next = next
        .checked_add(1)
        .ok_or_else(|| PdfError::limit("copied object number overflows"))?;
    if usize::try_from(*next).map_or(true, |size| size > base.parsed().limits.max_xref_entries) {
        return Err(PdfError::limit(
            "copied object closure exceeds max_xref_entries",
        ));
    }
    Ok(reference)
}

fn selection_error(message: impl Into<String>) -> PdfError {
    PdfError {
        code: PdfErrorCode::SelectionNotFound,
        message: message.into(),
        span: None,
        object: None,
    }
}

fn stream_object(stream: Vec<u8>) -> IndirectObject {
    IndirectObject {
        value: Value::Dict(BTreeMap::new()),
        stream: Some(stream),
        stream_offset: 0,
        offset: 0,
    }
}

fn plain_object(value: Value) -> IndirectObject {
    IndirectObject {
        value,
        stream: None,
        stream_offset: 0,
        offset: 0,
    }
}

fn rectangle_value(rectangle: [f64; 4]) -> Value {
    Value::Array(rectangle.into_iter().map(Value::Real).collect())
}

fn pdf_number(value: f64) -> String {
    if value == 0.0 {
        "0".into()
    } else {
        value.to_string()
    }
}

fn is_type(value: &Value, expected: &[u8]) -> bool {
    matches!(as_dict(value, "object"), Ok(dictionary) if matches!(dictionary.get(b"Type".as_slice()), Some(Value::Name(name)) if name == expected))
}

fn is_subtype(value: &Value, expected: &[u8]) -> bool {
    matches!(as_dict(value, "object"), Ok(dictionary) if matches!(dictionary.get(b"Subtype".as_slice()), Some(Value::Name(name)) if name == expected))
}

fn as_dict<'a>(value: &'a Value, label: &str) -> Result<&'a BTreeMap<Vec<u8>, Value>, PdfError> {
    match value {
        Value::Dict(dictionary) => Ok(dictionary),
        _ => Err(PdfError::syntax(format!("{label} is not a dictionary"), 0)),
    }
}

fn as_dict_mut<'a>(
    value: &'a mut Value,
    label: &str,
) -> Result<&'a mut BTreeMap<Vec<u8>, Value>, PdfError> {
    match value {
        Value::Dict(dictionary) => Ok(dictionary),
        _ => Err(PdfError::syntax(format!("{label} is not a dictionary"), 0)),
    }
}
