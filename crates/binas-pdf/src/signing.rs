use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    EngineConfig, OpenOptions, PdfDocument, PdfEngine, PdfError, SignatureInspection,
    forms::list_form_fields,
    inspect_signatures,
    parser::{IndirectObject, ObjectRef, Value},
    security::inspect_encryption,
    writer::{
        Output, dict_get, dict_integer, next_object_reference, require_classic_offset, write_name,
        write_object, write_value,
    },
};

const BYTE_RANGE_WIDTH: usize = 20;

#[derive(Clone, Debug)]
pub struct ExternalSignaturePlan {
    pub bytes: Vec<u8>,
    pub digest_algorithm: String,
    pub digest_to_sign: Vec<u8>,
    pub byte_range: [u64; 4],
    pub signature_object_number: u32,
    pub field_object_number: u32,
    pub field_object_generation: u16,
    pub reserved_cms_bytes: usize,
    byte_range_offsets: [usize; 4],
    contents_hex_start: usize,
    contents_hex_end: usize,
    config: EngineConfig,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ExternalSignaturePlanDescriptor {
    pub digest_algorithm: String,
    pub digest_to_sign: Vec<u8>,
    pub byte_range: [u64; 4],
    pub signature_object_number: u32,
    pub field_object_number: u32,
    pub field_object_generation: u16,
    pub reserved_cms_bytes: usize,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ExternalSignatureFieldOptions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field_name: Option<String>,
    #[serde(default)]
    pub page_index: usize,
    #[serde(default)]
    pub rect: [f64; 4],
}

impl Default for ExternalSignatureFieldOptions {
    fn default() -> Self {
        Self {
            field_name: None,
            page_index: 0,
            rect: [0.0; 4],
        }
    }
}

#[derive(Clone, Debug)]
pub struct AppliedExternalSignature {
    pub bytes: Vec<u8>,
    pub inspection: SignatureInspection,
}

impl PdfDocument {
    pub fn prepare_external_signature(
        &self,
        reserved_cms_bytes: usize,
    ) -> Result<ExternalSignaturePlan, PdfError> {
        self.prepare_external_signature_with_field(
            reserved_cms_bytes,
            ExternalSignatureFieldOptions::default(),
        )
    }

    pub fn prepare_external_signature_with_field(
        &self,
        reserved_cms_bytes: usize,
        field_options: ExternalSignatureFieldOptions,
    ) -> Result<ExternalSignaturePlan, PdfError> {
        if reserved_cms_bytes == 0 {
            return Err(PdfError::unsafe_rewrite(
                "signature CMS reservation must be non-zero",
            ));
        }
        if reserved_cms_bytes > self.engine_config().limits.max_stream_bytes {
            return Err(PdfError::limit(
                "signature CMS reservation exceeds max_stream_bytes",
            ));
        }
        if inspect_encryption(self)?.encrypted {
            return Err(PdfError::unsafe_rewrite(
                "external signing requires an unencrypted PDF",
            ));
        }
        let reference = next_object_reference(self)?;
        let (field_reference, mut attachments) =
            signature_attachments(self, reference, &field_options)?;
        let previous_xref = previous_xref_offset(self.source())?;
        let mut output = Output::new(self.engine_config().limits.max_output_bytes);
        output.push(self.source())?;
        output.push(b"\n")?;
        attachments.push((reference, plain_object(Value::Null)));
        attachments.sort_by_key(|(reference, _)| *reference);
        let mut offsets = Vec::with_capacity(attachments.len());
        let mut byte_range_offsets = [0; 4];
        let mut contents_hex_start = 0;
        let mut contents_hex_end = 0;
        for (object_reference, object) in &attachments {
            let object_offset = output.len();
            require_classic_offset(object_offset)?;
            offsets.push((*object_reference, object_offset));
            output.formatted(format_args!(
                "{} {} obj\n",
                object_reference.number, object_reference.generation
            ))?;
            if *object_reference == reference {
                output.push(b"<< /Type /Sig /Filter /Adobe.PPKLite /SubFilter /adbe.pkcs7.detached /ByteRange [")?;
                for (index, offset) in byte_range_offsets.iter_mut().enumerate() {
                    if index != 0 {
                        output.push(b" ")?;
                    }
                    *offset = output.len();
                    output.push(&[b'0'; BYTE_RANGE_WIDTH])?;
                }
                output.push(b"] /Contents <")?;
                contents_hex_start = output.len();
                let contents_hex_len = reserved_cms_bytes
                    .checked_mul(2)
                    .ok_or_else(|| PdfError::limit("signature CMS reservation overflows"))?;
                output.push(&vec![b'0'; contents_hex_len])?;
                contents_hex_end = output.len();
                output.push(b"> >>")?;
            } else {
                write_object(&mut output, object, self.parsed().limits.max_parser_depth)?;
            }
            output.push(b"\nendobj\n")?;
        }
        let xref_offset = output.len();
        require_classic_offset(xref_offset)?;
        output.push(b"xref\n")?;
        for (object_reference, object_offset) in &offsets {
            output.formatted(format_args!(
                "{} 1\n{object_offset:010} {:05} n \n",
                object_reference.number, object_reference.generation
            ))?;
        }
        let current_size = dict_integer(&self.parsed().trailer, b"Size")
            .and_then(|size| u32::try_from(size).ok())
            .ok_or_else(|| PdfError::unsafe_rewrite("trailer has no direct u32 /Size"))?;
        let output_size = current_size.max(
            offsets
                .iter()
                .map(|(reference, _)| reference.number)
                .max()
                .unwrap_or(0)
                .checked_add(1)
                .ok_or_else(|| PdfError::limit("signature trailer /Size overflows"))?,
        );
        output.formatted(format_args!("trailer\n<< /Size {output_size}"))?;
        for key in [b"Root".as_slice(), b"Info".as_slice(), b"ID".as_slice()] {
            if let Some(value) = dict_get(&self.parsed().trailer, key) {
                output.push(b" ")?;
                write_name(&mut output, key)?;
                output.push(b" ")?;
                write_value(&mut output, value, 0, self.parsed().limits.max_parser_depth)?;
            }
        }
        output.formatted(format_args!(
            " /Prev {previous_xref} >>\nstartxref\n{xref_offset}\n%%EOF\n"
        ))?;
        let mut bytes = output.into_bytes();
        let gap_start = contents_hex_start - 1;
        let gap_end = contents_hex_end + 1;
        let byte_range = [
            0,
            u64::try_from(gap_start)
                .map_err(|_| PdfError::limit("signature offset exceeds u64"))?,
            u64::try_from(gap_end).map_err(|_| PdfError::limit("signature offset exceeds u64"))?,
            u64::try_from(bytes.len() - gap_end)
                .map_err(|_| PdfError::limit("signature length exceeds u64"))?,
        ];
        for (offset, value) in byte_range_offsets.into_iter().zip(byte_range) {
            let encoded = format!("{value:0BYTE_RANGE_WIDTH$}");
            if encoded.len() != BYTE_RANGE_WIDTH {
                return Err(PdfError::limit(
                    "signature ByteRange value exceeds reservation",
                ));
            }
            bytes[offset..offset + BYTE_RANGE_WIDTH].copy_from_slice(encoded.as_bytes());
        }
        let digest_to_sign = signed_digest(&bytes, byte_range)?;
        let prepared =
            PdfEngine::new(self.engine_config().clone()).open(&bytes, OpenOptions::default())?;
        verify_field_reachability(&prepared, field_reference, reference)?;
        Ok(ExternalSignaturePlan {
            bytes,
            digest_algorithm: "sha256".into(),
            digest_to_sign,
            byte_range,
            signature_object_number: reference.number,
            field_object_number: field_reference.number,
            field_object_generation: field_reference.generation,
            reserved_cms_bytes,
            byte_range_offsets,
            contents_hex_start,
            contents_hex_end,
            config: self.engine_config().clone(),
        })
    }
}

impl ExternalSignaturePlan {
    pub fn descriptor(&self) -> ExternalSignaturePlanDescriptor {
        ExternalSignaturePlanDescriptor {
            digest_algorithm: self.digest_algorithm.clone(),
            digest_to_sign: self.digest_to_sign.clone(),
            byte_range: self.byte_range,
            signature_object_number: self.signature_object_number,
            field_object_number: self.field_object_number,
            field_object_generation: self.field_object_generation,
            reserved_cms_bytes: self.reserved_cms_bytes,
        }
    }

    pub fn from_prepared_pdf(
        bytes: Vec<u8>,
        descriptor: ExternalSignaturePlanDescriptor,
    ) -> Result<Self, PdfError> {
        Self::from_prepared_pdf_with_config(bytes, descriptor, EngineConfig::default())
    }

    pub fn from_prepared_pdf_with_config(
        bytes: Vec<u8>,
        descriptor: ExternalSignaturePlanDescriptor,
        config: EngineConfig,
    ) -> Result<Self, PdfError> {
        if descriptor.digest_algorithm != "sha256" || descriptor.reserved_cms_bytes == 0 {
            return Err(PdfError::verification(
                "external signature descriptor is unsupported or malformed",
            ));
        }
        let document = PdfEngine::new(config.clone()).open(&bytes, OpenOptions::default())?;
        let signature = inspect_signatures(&document)?
            .into_iter()
            .find(|inspection| {
                inspection.object_number == descriptor.signature_object_number
                    && inspection.object_generation == 0
            })
            .ok_or_else(|| PdfError::verification("prepared signature object was not found"))?;
        if signature.byte_range != descriptor.byte_range || signature.contents_bytes != 0 {
            return Err(PdfError::verification(
                "prepared signature reservation or ByteRange was modified",
            ));
        }
        let field = ObjectRef {
            number: descriptor.field_object_number,
            generation: descriptor.field_object_generation,
        };
        let signature_reference = ObjectRef {
            number: descriptor.signature_object_number,
            generation: 0,
        };
        verify_field_reachability(&document, field, signature_reference)?;
        let (contents_hex_start, contents_hex_end) =
            reservation_offsets(&bytes, descriptor.byte_range)?;
        let expected_hex_len = descriptor
            .reserved_cms_bytes
            .checked_mul(2)
            .ok_or_else(|| PdfError::verification("signature reservation size overflows"))?;
        if contents_hex_end - contents_hex_start != expected_hex_len
            || bytes[contents_hex_start..contents_hex_end]
                .iter()
                .any(|byte| *byte != b'0')
        {
            return Err(PdfError::verification(
                "prepared signature contents reservation was modified",
            ));
        }
        let object = document.parsed().object(signature_reference)?;
        let byte_range_offsets = byte_range_offsets(
            &bytes,
            object.offset,
            contents_hex_start,
            descriptor.byte_range,
        )?;
        let plan = Self {
            bytes,
            digest_algorithm: descriptor.digest_algorithm,
            digest_to_sign: descriptor.digest_to_sign,
            byte_range: descriptor.byte_range,
            signature_object_number: descriptor.signature_object_number,
            field_object_number: descriptor.field_object_number,
            field_object_generation: descriptor.field_object_generation,
            reserved_cms_bytes: descriptor.reserved_cms_bytes,
            byte_range_offsets,
            contents_hex_start,
            contents_hex_end,
            config,
        };
        validate_plan(&plan)?;
        Ok(plan)
    }

    pub fn apply_cms(&self, cms_der: &[u8]) -> Result<AppliedExternalSignature, PdfError> {
        if cms_der.is_empty() {
            return Err(PdfError::unsafe_rewrite("external CMS must be non-empty"));
        }
        if cms_der.len() > self.reserved_cms_bytes {
            return Err(PdfError::unsafe_rewrite(
                "external CMS exceeds the reserved signature size",
            ));
        }
        validate_plan(self)?;
        let mut bytes = self.bytes.clone();
        const HEX: &[u8; 16] = b"0123456789ABCDEF";
        for (index, byte) in cms_der.iter().copied().enumerate() {
            let offset = self.contents_hex_start + index * 2;
            bytes[offset] = HEX[usize::from(byte >> 4)];
            bytes[offset + 1] = HEX[usize::from(byte & 15)];
        }
        let document = PdfEngine::new(self.config.clone()).open(&bytes, OpenOptions::default())?;
        let inspection = inspect_signatures(&document)?
            .into_iter()
            .find(|inspection| inspection.object_number == self.signature_object_number)
            .ok_or_else(|| PdfError::verification("applied signature object was not found"))?;
        if !inspection.cms_verified || inspection.byte_range != self.byte_range {
            return Err(PdfError::verification(
                "applied CMS failed detached signature verification",
            ));
        }
        Ok(AppliedExternalSignature { bytes, inspection })
    }
}

fn signature_attachments(
    document: &PdfDocument,
    signature: ObjectRef,
    options: &ExternalSignatureFieldOptions,
) -> Result<(ObjectRef, Vec<(ObjectRef, IndirectObject)>), PdfError> {
    if options.rect.iter().any(|value| !value.is_finite())
        || options.rect[0] > options.rect[2]
        || options.rect[1] > options.rect[3]
    {
        return Err(PdfError::unsafe_rewrite(
            "signature widget rectangle must be finite with x1 <= x2 and y1 <= y2",
        ));
    }
    let fields = list_form_fields(document)?;
    let candidates = fields
        .iter()
        .filter(|field| field.field_type.as_deref() == Some("Sig"))
        .filter(|field| {
            options
                .field_name
                .as_deref()
                .is_none_or(|name| field.name == name)
        })
        .collect::<Vec<_>>();
    if candidates.len() > 1 {
        return Err(PdfError::unsafe_rewrite(
            "multiple signature fields match; select one by name",
        ));
    }
    if let Some(field) = candidates.first() {
        let reference = ObjectRef {
            number: field
                .object_number
                .ok_or_else(|| PdfError::unsafe_rewrite("signature field must be indirect"))?,
            generation: field.object_generation.unwrap_or(0),
        };
        if field.widget_refs.is_empty() {
            return Err(PdfError::unsafe_rewrite(
                "selected signature field has no reachable widget",
            ));
        }
        let object = document.parsed().object(reference)?;
        let Value::Dict(mut dictionary) = object.value.clone() else {
            return Err(PdfError::unsafe_rewrite(
                "selected signature field is not a dictionary",
            ));
        };
        if dictionary.contains_key(b"V".as_slice()) {
            return Err(PdfError::unsafe_rewrite(
                "selected signature field is already signed",
            ));
        }
        dictionary.insert(b"V".to_vec(), Value::Ref(signature));
        return Ok((
            reference,
            vec![(reference, plain_object(Value::Dict(dictionary)))],
        ));
    }

    let page = document
        .page_refs()?
        .get(options.page_index)
        .copied()
        .ok_or_else(|| PdfError::selection("signature page index is out of range"))?;
    let field = allocate_after(document, signature, 1)?[0];
    let name = options
        .field_name
        .clone()
        .unwrap_or_else(|| format!("BinasSignature{}", field.number));
    if name.is_empty() || name.len() > document.engine_config().limits.max_token_bytes {
        return Err(PdfError::unsafe_rewrite(
            "signature field name is empty or exceeds limits",
        ));
    }
    let mut attachments = Vec::new();
    let field_dictionary = BTreeMap::from([
        (b"Type".to_vec(), Value::Name(b"Annot".to_vec())),
        (b"Subtype".to_vec(), Value::Name(b"Widget".to_vec())),
        (b"FT".to_vec(), Value::Name(b"Sig".to_vec())),
        (b"T".to_vec(), Value::String(name.into_bytes())),
        (b"Rect".to_vec(), number_array(options.rect)),
        (b"P".to_vec(), Value::Ref(page)),
        (b"V".to_vec(), Value::Ref(signature)),
    ]);
    attachments.push((field, plain_object(Value::Dict(field_dictionary))));

    let page_object = document.parsed().object(page)?;
    let Value::Dict(mut page_dictionary) = page_object.value.clone() else {
        return Err(PdfError::unsafe_rewrite(
            "signature page is not a dictionary",
        ));
    };
    match page_dictionary
        .entry(b"Annots".to_vec())
        .or_insert_with(|| Value::Array(Vec::new()))
    {
        Value::Array(values) => values.push(Value::Ref(field)),
        _ => {
            return Err(PdfError::unsafe_rewrite(
                "signature page /Annots must be a direct array",
            ));
        }
    }
    attachments.push((page, plain_object(Value::Dict(page_dictionary))));

    let root = match dict_get(&document.parsed().trailer, b"Root") {
        Some(Value::Ref(reference)) => *reference,
        _ => {
            return Err(PdfError::unsafe_rewrite(
                "signature requires an indirect catalog",
            ));
        }
    };
    let root_object = document.parsed().object(root)?;
    let Value::Dict(mut catalog) = root_object.value.clone() else {
        return Err(PdfError::unsafe_rewrite("catalog is not a dictionary"));
    };
    match catalog.get(b"AcroForm".as_slice()).cloned() {
        None | Some(Value::Null) => {
            let acro = allocate_after(document, signature, 2)?[1];
            catalog.insert(b"AcroForm".to_vec(), Value::Ref(acro));
            attachments.push((
                acro,
                plain_object(Value::Dict(BTreeMap::from([(
                    b"Fields".to_vec(),
                    Value::Array(vec![Value::Ref(field)]),
                )]))),
            ));
        }
        Some(Value::Ref(acro)) => {
            let object = document.parsed().object(acro)?;
            let Value::Dict(mut dictionary) = object.value.clone() else {
                return Err(PdfError::unsafe_rewrite("AcroForm is not a dictionary"));
            };
            add_field(&mut dictionary, field)?;
            attachments.push((acro, plain_object(Value::Dict(dictionary))));
        }
        Some(Value::Dict(mut dictionary)) => {
            add_field(&mut dictionary, field)?;
            catalog.insert(b"AcroForm".to_vec(), Value::Dict(dictionary));
        }
        Some(_) => {
            return Err(PdfError::unsafe_rewrite(
                "catalog /AcroForm must be a dictionary or reference",
            ));
        }
    }
    attachments.push((root, plain_object(Value::Dict(catalog))));
    Ok((field, attachments))
}

fn add_field(dictionary: &mut BTreeMap<Vec<u8>, Value>, field: ObjectRef) -> Result<(), PdfError> {
    match dictionary
        .entry(b"Fields".to_vec())
        .or_insert_with(|| Value::Array(Vec::new()))
    {
        Value::Array(fields) => {
            fields.push(Value::Ref(field));
            Ok(())
        }
        _ => Err(PdfError::unsafe_rewrite(
            "AcroForm /Fields must be a direct array",
        )),
    }
}

fn allocate_after(
    document: &PdfDocument,
    first: ObjectRef,
    count: usize,
) -> Result<Vec<ObjectRef>, PdfError> {
    let mut number = first.number;
    let mut output = Vec::with_capacity(count);
    while output.len() < count {
        number = number
            .checked_add(1)
            .ok_or_else(|| PdfError::limit("signature object number overflows"))?;
        if !document
            .parsed()
            .objects
            .keys()
            .any(|reference| reference.number == number)
        {
            output.push(ObjectRef {
                number,
                generation: 0,
            });
        }
    }
    let total = document
        .parsed()
        .objects
        .len()
        .checked_add(count + 1)
        .ok_or_else(|| PdfError::limit("signature object count overflows"))?;
    if total > document.parsed().limits.max_objects
        || usize::try_from(number)
            .ok()
            .and_then(|number| number.checked_add(1))
            .is_none_or(|size| size > document.parsed().limits.max_xref_entries)
    {
        return Err(PdfError::limit(
            "signature object allocation exceeds limits",
        ));
    }
    Ok(output)
}

fn verify_field_reachability(
    document: &PdfDocument,
    field: ObjectRef,
    signature: ObjectRef,
) -> Result<(), PdfError> {
    let value_reachable = matches!(
        &document.parsed().object(field)?.value,
        Value::Dict(dictionary) if dictionary.get(b"V".as_slice()) == Some(&Value::Ref(signature))
    );
    let root = match dict_get(&document.parsed().trailer, b"Root") {
        Some(Value::Ref(reference)) => *reference,
        _ => return Err(PdfError::verification("signature catalog is not reachable")),
    };
    let Value::Dict(catalog) = &document.parsed().object(root)?.value else {
        return Err(PdfError::verification(
            "signature catalog is not a dictionary",
        ));
    };
    let acro = match catalog.get(b"AcroForm".as_slice()) {
        Some(Value::Ref(reference)) => &document.parsed().object(*reference)?.value,
        Some(value @ Value::Dict(_)) => value,
        _ => {
            return Err(PdfError::verification(
                "signature AcroForm is not reachable",
            ));
        }
    };
    let Value::Dict(acro) = acro else {
        return Err(PdfError::verification(
            "signature AcroForm is not a dictionary",
        ));
    };
    let Some(Value::Array(fields)) = acro.get(b"Fields".as_slice()) else {
        return Err(PdfError::verification(
            "signature AcroForm fields are not reachable",
        ));
    };
    let mut pending = fields
        .iter()
        .filter_map(|value| match value {
            Value::Ref(reference) => Some(*reference),
            _ => None,
        })
        .collect::<Vec<_>>();
    let mut field_reachable = false;
    let mut widgets = Vec::new();
    while let Some(reference) = pending.pop() {
        let Value::Dict(dictionary) = &document.parsed().object(reference)?.value else {
            continue;
        };
        if reference == field {
            field_reachable = true;
        }
        if matches!(dictionary.get(b"Subtype".as_slice()), Some(Value::Name(name)) if name == b"Widget")
        {
            widgets.push(reference);
        }
        if let Some(Value::Array(kids)) = dictionary.get(b"Kids".as_slice()) {
            pending.extend(kids.iter().filter_map(|value| match value {
                Value::Ref(reference) => Some(*reference),
                _ => None,
            }));
        }
    }
    let pages = document.page_refs()?;
    let widget_reachable = widgets.iter().any(|widget| {
        pages.iter().any(|page| {
            matches!(
                &document.parsed().object(*page).ok().map(|object| &object.value),
                Some(Value::Dict(dictionary))
                    if matches!(dictionary.get(b"Annots".as_slice()), Some(Value::Array(annots)) if annots.contains(&Value::Ref(*widget)))
            )
        })
    });
    if field_reachable && widget_reachable && value_reachable {
        Ok(())
    } else {
        Err(PdfError::verification(
            "signature field/widget is not reachable through AcroForm",
        ))
    }
}

fn number_array(values: [f64; 4]) -> Value {
    Value::Array(
        values
            .into_iter()
            .map(|value| {
                if value.fract() == 0.0 && value >= i64::MIN as f64 && value <= i64::MAX as f64 {
                    Value::Integer(value as i64)
                } else {
                    Value::Real(value)
                }
            })
            .collect(),
    )
}

fn plain_object(value: Value) -> IndirectObject {
    IndirectObject {
        value,
        stream: None,
        stream_offset: 0,
        offset: 0,
    }
}

fn reservation_offsets(bytes: &[u8], byte_range: [u64; 4]) -> Result<(usize, usize), PdfError> {
    let gap_start = usize::try_from(byte_range[1])
        .map_err(|_| PdfError::verification("signature reservation exceeds usize"))?;
    let gap_end = usize::try_from(byte_range[2])
        .map_err(|_| PdfError::verification("signature reservation exceeds usize"))?;
    if gap_end <= gap_start + 1
        || bytes.get(gap_start) != Some(&b'<')
        || bytes.get(gap_end - 1) != Some(&b'>')
    {
        return Err(PdfError::verification(
            "signature ByteRange does not delimit a hex contents reservation",
        ));
    }
    Ok((gap_start + 1, gap_end - 1))
}

fn byte_range_offsets(
    bytes: &[u8],
    object_start: usize,
    contents_hex_start: usize,
    byte_range: [u64; 4],
) -> Result<[usize; 4], PdfError> {
    let marker = b"/ByteRange [";
    let relative = bytes
        .get(object_start..contents_hex_start)
        .and_then(|input| {
            input
                .windows(marker.len())
                .position(|window| window == marker)
        })
        .ok_or_else(|| PdfError::verification("prepared signature has no ByteRange marker"))?;
    let mut cursor = object_start + relative + marker.len();
    let mut offsets = [0; 4];
    for (index, value) in byte_range.into_iter().enumerate() {
        while bytes.get(cursor).is_some_and(u8::is_ascii_whitespace) {
            cursor += 1;
        }
        let encoded = format!("{value:0BYTE_RANGE_WIDTH$}");
        if bytes.get(cursor..cursor + BYTE_RANGE_WIDTH) != Some(encoded.as_bytes()) {
            return Err(PdfError::verification(
                "prepared signature ByteRange encoding was modified",
            ));
        }
        offsets[index] = cursor;
        cursor += BYTE_RANGE_WIDTH;
    }
    Ok(offsets)
}

fn validate_plan(plan: &ExternalSignaturePlan) -> Result<(), PdfError> {
    if plan.contents_hex_end > plan.bytes.len()
        || plan.contents_hex_end - plan.contents_hex_start != plan.reserved_cms_bytes * 2
        || plan.bytes[plan.contents_hex_start..plan.contents_hex_end]
            .iter()
            .any(|byte| *byte != b'0')
        || signed_digest(&plan.bytes, plan.byte_range)? != plan.digest_to_sign
    {
        return Err(PdfError::verification(
            "external signature plan bytes or ranges were modified",
        ));
    }
    for (offset, value) in plan.byte_range_offsets.into_iter().zip(plan.byte_range) {
        let end = offset + BYTE_RANGE_WIDTH;
        if plan.bytes.get(offset..end) != Some(format!("{value:0BYTE_RANGE_WIDTH$}").as_bytes()) {
            return Err(PdfError::verification(
                "external signature plan ByteRange was modified",
            ));
        }
    }
    Ok(())
}

fn signed_digest(bytes: &[u8], byte_range: [u64; 4]) -> Result<Vec<u8>, PdfError> {
    let [first_start, first_len, second_start, second_len] = byte_range;
    let first_start = usize::try_from(first_start)
        .map_err(|_| PdfError::verification("signature range exceeds usize"))?;
    let first_end = first_start
        .checked_add(
            usize::try_from(first_len)
                .map_err(|_| PdfError::verification("signature range length exceeds usize"))?,
        )
        .ok_or_else(|| PdfError::verification("signature range overflows"))?;
    let second_start = usize::try_from(second_start)
        .map_err(|_| PdfError::verification("signature range exceeds usize"))?;
    let second_end = second_start
        .checked_add(
            usize::try_from(second_len)
                .map_err(|_| PdfError::verification("signature range length exceeds usize"))?,
        )
        .ok_or_else(|| PdfError::verification("signature range overflows"))?;
    let mut digest = Sha256::new();
    digest.update(
        bytes
            .get(first_start..first_end)
            .ok_or_else(|| PdfError::verification("signature range exceeds source"))?,
    );
    digest.update(
        bytes
            .get(second_start..second_end)
            .ok_or_else(|| PdfError::verification("signature range exceeds source"))?,
    );
    Ok(digest.finalize().to_vec())
}

fn previous_xref_offset(input: &[u8]) -> Result<usize, PdfError> {
    let marker = b"startxref";
    let start = input
        .windows(marker.len())
        .rposition(|window| window == marker)
        .ok_or_else(|| PdfError::unsafe_rewrite("missing startxref"))?;
    let rest = input[start + marker.len()..]
        .iter()
        .copied()
        .skip_while(u8::is_ascii_whitespace)
        .take_while(u8::is_ascii_digit)
        .collect::<Vec<_>>();
    std::str::from_utf8(&rest)
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|offset| *offset < input.len())
        .ok_or_else(|| PdfError::unsafe_rewrite("startxref offset is invalid"))
}
