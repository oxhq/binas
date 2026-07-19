use std::{collections::BTreeMap, io::Read};

use flate2::bufread::ZlibDecoder;
use serde::{Deserialize, Serialize};

use crate::{
    OpenOptions, PdfDocument, PdfEngine, PdfError,
    filters::encode_flate,
    limits::Limits,
    parser::{self, IndirectObject, ObjectRef, ParseBudget, ParsedDocument, Value},
    streams::{
        StreamFilterMetadata, StreamObjectRef, is_image_xobject, read_decoded_stream,
        stream_filter_chain,
    },
    writer::refuse_security_boundaries,
};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageFilter {
    Raw,
    Flate,
    Jpeg,
    Jpx,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageColorSpace {
    DeviceGray,
    DeviceRgb,
    DeviceCmyk,
}

impl ImageColorSpace {
    fn name(self) -> &'static [u8] {
        match self {
            Self::DeviceGray => b"DeviceGray",
            Self::DeviceRgb => b"DeviceRGB",
            Self::DeviceCmyk => b"DeviceCMYK",
        }
    }

    fn from_name(name: &[u8]) -> Option<Self> {
        match name {
            b"DeviceGray" | b"G" => Some(Self::DeviceGray),
            b"DeviceRGB" | b"RGB" => Some(Self::DeviceRgb),
            b"DeviceCMYK" | b"CMYK" => Some(Self::DeviceCmyk),
            _ => None,
        }
    }

    fn components(self) -> usize {
        match self {
            Self::DeviceGray => 1,
            Self::DeviceRgb => 3,
            Self::DeviceCmyk => 4,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ImageDecodeParams {
    pub predictor: u8,
    pub colors: u8,
    pub bits_per_component: u8,
    pub columns: u32,
}

/// Read-only metadata for one indirect image XObject stream.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImageXObjectInventoryEntry {
    pub object: StreamObjectRef,
    pub width: u32,
    pub height: u32,
    pub filter_chain: Vec<StreamFilterMetadata>,
    /// The device color space when it is directly declared; complex or indirect spaces stay unknown.
    pub color_space: Option<ImageColorSpace>,
}

/// Raw samples from one exact, direct-Flate image XObject.
///
/// This is intentionally limited to unmasked 8-bit DeviceGray, DeviceRGB, and
/// DeviceCMYK images. No color conversion or image encoding is performed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RawFlateImageSamples {
    pub object: StreamObjectRef,
    pub width: u32,
    pub height: u32,
    pub color_space: ImageColorSpace,
    pub samples: Vec<u8>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageMaskPolicy {
    #[default]
    Reject,
    PreserveCompatible,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ImageReplacementRequest {
    pub object_number: u32,
    #[serde(default)]
    pub object_generation: u16,
    pub encoded_bytes: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub bits_per_component: u8,
    pub color_space: ImageColorSpace,
    pub filter: ImageFilter,
    pub decode_params: Option<ImageDecodeParams>,
    #[serde(default)]
    pub mask_policy: ImageMaskPolicy,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct EncodedImageReplacementRequest {
    pub object_number: u32,
    #[serde(default)]
    pub object_generation: u16,
    pub encoded_bytes: Vec<u8>,
    #[serde(default)]
    pub mask_policy: ImageMaskPolicy,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ImageReplacementReport {
    pub operation: String,
    pub object_number: u32,
    pub object_generation: u16,
    pub filter: ImageFilter,
    pub width: u32,
    pub height: u32,
    pub input_bytes: usize,
    pub output_bytes: usize,
    pub encoded_bytes: usize,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ImageReplacementVerification {
    pub passed: bool,
    pub reparsed: bool,
    pub dimensions_match: bool,
    pub metadata_matches: bool,
    pub encoded_stream_matches: bool,
    pub object_reference_preserved: bool,
    pub page_count_unchanged: bool,
    pub no_dangling_references: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ImageReplacementOutcome {
    pub bytes: Vec<u8>,
    pub report: ImageReplacementReport,
    pub verification: ImageReplacementVerification,
}

#[derive(Clone, Debug)]
struct GeneratedSoftMask {
    samples: Vec<u8>,
    encoded_bytes: Vec<u8>,
}

/// Lists image XObject metadata without reading or decoding image bytes.
pub fn list_image_xobjects(
    document: &PdfDocument,
) -> Result<Vec<ImageXObjectInventoryEntry>, PdfError> {
    let parsed = document.parsed();
    let mut images = Vec::new();
    for (reference, object) in &parsed.objects {
        if object.stream.is_none() || !is_image_xobject(&object.value) {
            continue;
        }
        images.push(image_xobject_inventory_entry(parsed, *reference, object)?);
    }
    Ok(images)
}

/// Returns the opaque bytes for one exact, direct-DCT JPEG image XObject.
///
/// The selected entry must be returned by [`list_image_xobjects`] for this
/// document. The reader does not decode pixels, convert colors, follow masks,
/// or handle inline images; it only copies a bounded, validated JPEG stream.
pub fn read_jpeg_xobject_bytes(
    document: &PdfDocument,
    selected: &ImageXObjectInventoryEntry,
) -> Result<Vec<u8>, PdfError> {
    let entry = exact_image_xobject_entry(document, selected, "JPEG")?;

    let reference = ObjectRef {
        number: entry.object.object_number,
        generation: entry.object.object_generation,
    };
    let parsed = document.parsed();
    let object = parsed.objects.get(&reference).ok_or_else(|| {
        image_metadata_at(
            PdfError::selection("JPEG image XObject stream was not found"),
            reference,
        )
    })?;
    if !is_image_xobject(&object.value) {
        return Err(image_metadata_at(
            PdfError::selection("selected object is not an image XObject"),
            reference,
        ));
    }
    let Value::Dict(dictionary) = &object.value else {
        return Err(image_metadata_error(
            reference,
            "image XObject dictionary is invalid",
        ));
    };
    match dictionary.get(b"Filter".as_slice()) {
        Some(Value::Name(name)) if name == b"DCTDecode" => {}
        _ => {
            return Err(image_metadata_at(
                PdfError::unsupported(
                    "JPEG image XObject must use exactly one direct /DCTDecode filter",
                ),
                reference,
            ));
        }
    }
    reject_image_semantics(dictionary, reference, "JPEG")?;
    let encoded = object.stream.as_deref().ok_or_else(|| {
        image_metadata_at(
            PdfError::selection("selected image XObject is not a stream"),
            reference,
        )
    })?;
    if encoded.len() > parsed.limits.max_stream_bytes {
        return Err(image_metadata_at(
            PdfError::limit("JPEG image XObject exceeds max_stream_bytes"),
            reference,
        ));
    }
    let (width, height, _, _) =
        jpeg_dimensions(encoded).map_err(|error| image_metadata_at(error, reference))?;
    if (width, height) != (entry.width, entry.height) {
        return Err(image_metadata_at(
            PdfError::syntax(
                "JPEG dimensions do not match the image XObject dictionary",
                0,
            ),
            reference,
        ));
    }
    Ok(encoded.to_vec())
}

/// Returns the opaque bytes for one exact, direct-JPX JPEG 2000 image XObject.
///
/// The selected entry must be returned by [`list_image_xobjects`] for this
/// document. The reader validates only the JPEG 2000 container/codestream and
/// dimensions; it does not decode pixels, convert colors, follow masks, or
/// handle inline images.
pub fn read_jpx_xobject_bytes(
    document: &PdfDocument,
    selected: &ImageXObjectInventoryEntry,
) -> Result<Vec<u8>, PdfError> {
    let entry = exact_image_xobject_entry(document, selected, "JPX")?;
    let reference = ObjectRef {
        number: entry.object.object_number,
        generation: entry.object.object_generation,
    };
    let parsed = document.parsed();
    let object = parsed.objects.get(&reference).ok_or_else(|| {
        image_metadata_at(
            PdfError::selection("JPX image XObject stream was not found"),
            reference,
        )
    })?;
    if !is_image_xobject(&object.value) {
        return Err(image_metadata_at(
            PdfError::selection("selected object is not an image XObject"),
            reference,
        ));
    }
    let Value::Dict(dictionary) = &object.value else {
        return Err(image_metadata_error(
            reference,
            "image XObject dictionary is invalid",
        ));
    };
    match dictionary.get(b"Filter".as_slice()) {
        Some(Value::Name(name)) if name == b"JPXDecode" => {}
        _ => {
            return Err(image_metadata_at(
                PdfError::unsupported(
                    "JPX image XObject must use exactly one direct /JPXDecode filter",
                ),
                reference,
            ));
        }
    }
    reject_image_semantics(dictionary, reference, "JPX")?;
    let encoded = object.stream.as_deref().ok_or_else(|| {
        image_metadata_at(
            PdfError::selection("selected image XObject is not a stream"),
            reference,
        )
    })?;
    if encoded.len() > parsed.limits.max_stream_bytes {
        return Err(image_metadata_at(
            PdfError::limit("JPX image XObject exceeds max_stream_bytes"),
            reference,
        ));
    }
    let (width, height, _, _) =
        jpx_dimensions(encoded).map_err(|error| image_metadata_at(error, reference))?;
    if (width, height) != (entry.width, entry.height) {
        return Err(image_metadata_at(
            PdfError::syntax(
                "JPX dimensions do not match the image XObject dictionary",
                0,
            ),
            reference,
        ));
    }
    Ok(encoded.to_vec())
}

/// Returns raw decoded samples from one exact, direct-Flate image XObject.
///
/// The selected entry must be returned by [`list_image_xobjects`] for this
/// document. Filter predictors, masks, decode transforms, color conversion,
/// inline images, and generic image extraction are deliberately unsupported.
pub fn read_raw_flate_image_samples(
    document: &PdfDocument,
    selected: &ImageXObjectInventoryEntry,
) -> Result<RawFlateImageSamples, PdfError> {
    let entry = exact_image_xobject_entry(document, selected, "raw-Flate")?;
    let reference = ObjectRef {
        number: entry.object.object_number,
        generation: entry.object.object_generation,
    };
    let parsed = document.parsed();
    let object = parsed.objects.get(&reference).ok_or_else(|| {
        image_metadata_at(
            PdfError::selection("raw-Flate image XObject stream was not found"),
            reference,
        )
    })?;
    if !is_image_xobject(&object.value) {
        return Err(image_metadata_at(
            PdfError::selection("selected object is not an image XObject"),
            reference,
        ));
    }
    let Value::Dict(dictionary) = &object.value else {
        return Err(image_metadata_error(
            reference,
            "image XObject dictionary is invalid",
        ));
    };
    match dictionary.get(b"Filter".as_slice()) {
        Some(Value::Name(name)) if name == b"FlateDecode" => {}
        _ => {
            return Err(image_metadata_at(
                PdfError::unsupported(
                    "raw-Flate image XObject must use exactly one direct /FlateDecode filter",
                ),
                reference,
            ));
        }
    }
    reject_image_semantics(dictionary, reference, "raw-Flate")?;
    let color_space = raw_flate_color_space(dictionary, reference)?;
    match dictionary.get(b"BitsPerComponent".as_slice()) {
        Some(Value::Integer(8)) => {}
        Some(Value::Integer(_)) => {
            return Err(image_metadata_at(
                PdfError::unsupported("raw-Flate image XObject /BitsPerComponent must be 8"),
                reference,
            ));
        }
        _ => {
            return Err(image_metadata_error(
                reference,
                "raw-Flate image XObject /BitsPerComponent must be the integer 8",
            ));
        }
    }
    let expected = usize::try_from(entry.width)
        .ok()
        .and_then(|width| width.checked_mul(usize::try_from(entry.height).ok()?))
        .and_then(|pixels| pixels.checked_mul(color_space.components()))
        .ok_or_else(|| {
            image_metadata_at(
                PdfError::limit("raw-Flate image sample length overflows"),
                reference,
            )
        })?;
    if expected > parsed.limits.max_stream_bytes {
        return Err(image_metadata_at(
            PdfError::limit("raw-Flate image samples exceed max_stream_bytes"),
            reference,
        ));
    }
    let samples = read_decoded_stream(document, entry.object)?;
    if samples.len() != expected {
        return Err(image_metadata_at(
            PdfError::syntax(
                "raw-Flate image decoded length does not match dimensions and color space",
                0,
            ),
            reference,
        ));
    }
    Ok(RawFlateImageSamples {
        object: entry.object,
        width: entry.width,
        height: entry.height,
        color_space,
        samples,
    })
}

fn exact_image_xobject_entry(
    document: &PdfDocument,
    selected: &ImageXObjectInventoryEntry,
    kind: &str,
) -> Result<ImageXObjectInventoryEntry, PdfError> {
    let mut matches = list_image_xobjects(document)?
        .into_iter()
        .filter(|entry| entry == selected);
    let entry = matches.next().ok_or_else(|| {
        PdfError::selection(format!(
            "{kind} image XObject inventory entry was not found"
        ))
    })?;
    if matches.next().is_some() {
        return Err(PdfError::unsupported(format!(
            "{kind} image XObject inventory entry is not a unique selector"
        )));
    }
    Ok(entry)
}

fn raw_flate_color_space(
    dictionary: &BTreeMap<Vec<u8>, Value>,
    reference: ObjectRef,
) -> Result<ImageColorSpace, PdfError> {
    match dictionary.get(b"ColorSpace".as_slice()) {
        Some(Value::Name(name)) => match name.as_slice() {
            b"DeviceGray" => Ok(ImageColorSpace::DeviceGray),
            b"DeviceRGB" => Ok(ImageColorSpace::DeviceRgb),
            b"DeviceCMYK" => Ok(ImageColorSpace::DeviceCmyk),
            _ => Err(image_metadata_at(
                PdfError::unsupported(
                    "raw-Flate image XObject /ColorSpace must be DeviceGray, DeviceRGB, or DeviceCMYK",
                ),
                reference,
            )),
        },
        Some(_) => Err(image_metadata_at(
            PdfError::unsupported(
                "raw-Flate image XObject /ColorSpace must be a direct device color-space name",
            ),
            reference,
        )),
        None => Err(image_metadata_at(
            PdfError::unsupported("raw-Flate image XObject must declare /ColorSpace"),
            reference,
        )),
    }
}

fn reject_image_semantics(
    dictionary: &BTreeMap<Vec<u8>, Value>,
    reference: ObjectRef,
    kind: &str,
) -> Result<(), PdfError> {
    for key in [
        b"DecodeParms".as_slice(),
        b"Decode",
        b"SMask",
        b"Mask",
        b"ImageMask",
        b"SMaskInData",
    ] {
        if dictionary.contains_key(key) {
            return Err(image_metadata_at(
                PdfError::unsupported(format!(
                    "{kind} image XObject must not declare /{}",
                    String::from_utf8_lossy(key)
                )),
                reference,
            ));
        }
    }
    Ok(())
}

fn image_xobject_inventory_entry(
    parsed: &ParsedDocument,
    reference: ObjectRef,
    object: &IndirectObject,
) -> Result<ImageXObjectInventoryEntry, PdfError> {
    let Value::Dict(dictionary) = &object.value else {
        return Err(image_metadata_error(
            reference,
            "image XObject dictionary is invalid",
        ));
    };
    Ok(ImageXObjectInventoryEntry {
        object: StreamObjectRef {
            object_number: reference.number,
            object_generation: reference.generation,
        },
        width: image_dimension(dictionary, b"Width", reference)?,
        height: image_dimension(dictionary, b"Height", reference)?,
        filter_chain: stream_filter_chain(&object.value, parsed.limits.max_container_items)
            .map_err(|error| image_metadata_at(error, reference))?,
        color_space: image_color_space(dictionary, reference)?,
    })
}

fn image_dimension(
    dictionary: &BTreeMap<Vec<u8>, Value>,
    key: &[u8],
    reference: ObjectRef,
) -> Result<u32, PdfError> {
    let Some(Value::Integer(value)) = dictionary.get(key) else {
        return Err(image_metadata_error(
            reference,
            format!(
                "image XObject /{} must be a positive integer",
                String::from_utf8_lossy(key)
            ),
        ));
    };
    let value = u32::try_from(*value).map_err(|_| {
        image_metadata_error(
            reference,
            format!(
                "image XObject /{} exceeds u32",
                String::from_utf8_lossy(key)
            ),
        )
    })?;
    if value == 0 {
        return Err(image_metadata_error(
            reference,
            format!(
                "image XObject /{} must be positive",
                String::from_utf8_lossy(key)
            ),
        ));
    }
    Ok(value)
}

fn image_color_space(
    dictionary: &BTreeMap<Vec<u8>, Value>,
    reference: ObjectRef,
) -> Result<Option<ImageColorSpace>, PdfError> {
    match dictionary.get(b"ColorSpace".as_slice()) {
        None => Ok(None),
        Some(Value::Name(name)) => Ok(ImageColorSpace::from_name(name)),
        Some(Value::Array(_)) | Some(Value::Ref(_)) => Ok(None),
        Some(_) => Err(image_metadata_error(
            reference,
            "image XObject /ColorSpace must be a name, array, or reference",
        )),
    }
}

fn image_metadata_error(reference: ObjectRef, message: impl Into<String>) -> PdfError {
    image_metadata_at(PdfError::syntax(message, 0), reference)
}

fn image_metadata_at(mut error: PdfError, reference: ObjectRef) -> PdfError {
    error.object = Some((reference.number, reference.generation));
    error
}

impl PdfDocument {
    pub fn replace_image_xobject_encoded(
        &self,
        request: EncodedImageReplacementRequest,
    ) -> Result<ImageReplacementOutcome, PdfError> {
        refuse_security_boundaries(self.parsed())?;
        if request.encoded_bytes.is_empty() {
            return Err(PdfError::unsupported(
                "replacement image bytes must not be empty",
            ));
        }
        if request.encoded_bytes.len() > self.parsed().limits.max_stream_bytes {
            return Err(PdfError::limit(
                "replacement image exceeds max_stream_bytes",
            ));
        }
        let (encoded_bytes, filter, width, height, bits_per_component, components, generated_smask) =
            if request.encoded_bytes.starts_with(&[0xff, 0xd8]) {
                let (width, height, bits, components) = jpeg_dimensions(&request.encoded_bytes)?;
                (
                    request.encoded_bytes,
                    ImageFilter::Jpeg,
                    width,
                    height,
                    bits,
                    components,
                    None,
                )
            } else if request.encoded_bytes.starts_with(PNG_SIGNATURE) {
                let normalized = normalize_png(&request.encoded_bytes, &self.parsed().limits)?;
                (
                    normalized.encoded_bytes,
                    ImageFilter::Flate,
                    normalized.width,
                    normalized.height,
                    8,
                    normalized.components,
                    normalized.soft_mask,
                )
            } else {
                let (width, height, bits, components) = jpx_dimensions(&request.encoded_bytes)?;
                (
                    request.encoded_bytes,
                    ImageFilter::Jpx,
                    width,
                    height,
                    bits,
                    components,
                    None,
                )
            };
        let color_space = match components {
            1 => ImageColorSpace::DeviceGray,
            3 => ImageColorSpace::DeviceRgb,
            4 => ImageColorSpace::DeviceCmyk,
            _ => {
                return Err(PdfError::unsupported(
                    "encoded image must have 1, 3, or 4 color components",
                ));
            }
        };
        self.replace_image_xobject_inner(
            ImageReplacementRequest {
                object_number: request.object_number,
                object_generation: request.object_generation,
                encoded_bytes,
                width,
                height,
                bits_per_component,
                color_space,
                filter,
                decode_params: None,
                mask_policy: request.mask_policy,
            },
            generated_smask,
        )
    }

    pub fn replace_image_xobject(
        &self,
        request: ImageReplacementRequest,
    ) -> Result<ImageReplacementOutcome, PdfError> {
        self.replace_image_xobject_inner(request, None)
    }

    fn replace_image_xobject_inner(
        &self,
        request: ImageReplacementRequest,
        generated_smask: Option<GeneratedSoftMask>,
    ) -> Result<ImageReplacementOutcome, PdfError> {
        refuse_security_boundaries(self.parsed())?;
        validate_request(self.parsed(), &request)?;
        let reference = ObjectRef {
            number: request.object_number,
            generation: request.object_generation,
        };
        let original = self.parsed().object(reference)?;
        let original_dictionary = image_dictionary(original, "selected object")?;
        if let Some(smask) = generated_smask.as_ref() {
            validate_generated_smask(self.parsed(), original_dictionary, &request, smask)?;
        }
        validate_mask_policy(self.parsed(), original_dictionary, &request)?;
        let smask_reference = generated_smask
            .as_ref()
            .map(|_| allocate_image_reference(self.parsed()))
            .transpose()?;
        let mut dictionary = original_dictionary.clone();
        for key in [
            b"Width".as_slice(),
            b"Height".as_slice(),
            b"BitsPerComponent".as_slice(),
            b"ColorSpace".as_slice(),
            b"Filter".as_slice(),
            b"DecodeParms".as_slice(),
            b"Decode".as_slice(),
            b"ImageMask".as_slice(),
            b"Length".as_slice(),
        ] {
            dictionary.remove(key);
        }
        dictionary.insert(b"Type".to_vec(), Value::Name(b"XObject".to_vec()));
        dictionary.insert(b"Subtype".to_vec(), Value::Name(b"Image".to_vec()));
        dictionary.insert(b"Width".to_vec(), Value::Integer(i64::from(request.width)));
        dictionary.insert(
            b"Height".to_vec(),
            Value::Integer(i64::from(request.height)),
        );
        dictionary.insert(
            b"BitsPerComponent".to_vec(),
            Value::Integer(i64::from(request.bits_per_component)),
        );
        dictionary.insert(
            b"ColorSpace".to_vec(),
            Value::Name(request.color_space.name().to_vec()),
        );
        match request.filter {
            ImageFilter::Raw => {}
            ImageFilter::Flate => {
                dictionary.insert(b"Filter".to_vec(), Value::Name(b"FlateDecode".to_vec()));
            }
            ImageFilter::Jpeg => {
                dictionary.insert(b"Filter".to_vec(), Value::Name(b"DCTDecode".to_vec()));
            }
            ImageFilter::Jpx => {
                dictionary.insert(b"Filter".to_vec(), Value::Name(b"JPXDecode".to_vec()));
            }
        }
        if let Some(params) = request.decode_params {
            dictionary.insert(b"DecodeParms".to_vec(), decode_params_value(params));
        }
        if let Some(smask_reference) = smask_reference {
            dictionary.insert(b"SMask".to_vec(), Value::Ref(smask_reference));
        }
        dictionary.insert(
            b"Length".to_vec(),
            Value::Integer(
                i64::try_from(request.encoded_bytes.len())
                    .map_err(|_| PdfError::limit("image stream length exceeds i64"))?,
            ),
        );
        validate_encoded_bytes(&dictionary, self.parsed(), &request)?;
        let old_pages = self.page_count()?;
        let mut parsed = self.parsed().clone();
        let replacement = parsed
            .objects
            .get_mut(&reference)
            .ok_or_else(|| PdfError::selection("image object was not found"))?;
        replacement.value = Value::Dict(dictionary);
        replacement.stream = Some(request.encoded_bytes.clone());
        replacement.stream_offset = 0;
        replacement.offset = 0;
        if let (Some(smask_reference), Some(smask)) = (smask_reference, generated_smask.as_ref()) {
            parsed.objects.insert(
                smask_reference,
                IndirectObject {
                    value: Value::Dict(soft_mask_dictionary(
                        request.width,
                        request.height,
                        smask.encoded_bytes.len(),
                    )?),
                    stream: Some(smask.encoded_bytes.clone()),
                    stream_offset: 0,
                    offset: 0,
                },
            );
        }
        let canonical = self.with_parsed(parsed).canonicalize()?;
        let reopened = PdfEngine::new(self.engine_config().clone())
            .open(&canonical.bytes, OpenOptions::default())
            .map_err(|error| {
                PdfError::verification(format!("image output did not reparse: {error}"))
            })?;
        let verified = reopened.parsed().object(reference)?;
        let verified_dictionary = image_dictionary(verified, "verified image")?;
        let dimensions_match = integer(verified_dictionary, b"Width")
            == Some(i64::from(request.width))
            && integer(verified_dictionary, b"Height") == Some(i64::from(request.height));
        let metadata_matches = integer(verified_dictionary, b"BitsPerComponent")
            == Some(i64::from(request.bits_per_component))
            && name(verified_dictionary, b"ColorSpace") == Some(request.color_space.name())
            && filter_matches(verified_dictionary, request.filter)
            && verified_dictionary.get(b"DecodeParms".as_slice())
                == request.decode_params.map(decode_params_value).as_ref()
            && integer(verified_dictionary, b"Length")
                == i64::try_from(request.encoded_bytes.len()).ok();
        let encoded_stream_matches = verified.stream.as_deref() == Some(&request.encoded_bytes);
        let object_reference_preserved = reopened.parsed().objects.contains_key(&reference);
        let page_count_unchanged = reopened.page_count()? == old_pages;
        let no_dangling_references = verify_references(reopened.parsed())?;
        let generated_smask_matches = match (smask_reference, generated_smask.as_ref()) {
            (Some(reference), Some(smask)) => generated_smask_matches(
                reopened.parsed(),
                verified_dictionary,
                reference,
                &request,
                smask,
            )?,
            (None, None) => true,
            _ => false,
        };
        let verification = ImageReplacementVerification {
            passed: dimensions_match
                && metadata_matches
                && encoded_stream_matches
                && object_reference_preserved
                && page_count_unchanged
                && no_dangling_references
                && generated_smask_matches,
            reparsed: true,
            dimensions_match,
            metadata_matches,
            encoded_stream_matches,
            object_reference_preserved,
            page_count_unchanged,
            no_dangling_references,
        };
        if !verification.passed {
            return Err(PdfError::verification(
                "image replacement failed post-write verification",
            ));
        }
        Ok(ImageReplacementOutcome {
            report: ImageReplacementReport {
                operation: "replace_image_xobject".into(),
                object_number: reference.number,
                object_generation: reference.generation,
                filter: request.filter,
                width: request.width,
                height: request.height,
                input_bytes: self.source_len(),
                output_bytes: canonical.bytes.len(),
                encoded_bytes: request.encoded_bytes.len(),
            },
            bytes: canonical.bytes,
            verification,
        })
    }
}

fn validate_request(
    parsed: &ParsedDocument,
    request: &ImageReplacementRequest,
) -> Result<(), PdfError> {
    if request.width == 0 || request.height == 0 {
        return Err(PdfError::unsupported(
            "image width and height must be non-zero",
        ));
    }
    if request.encoded_bytes.is_empty() {
        return Err(PdfError::unsupported(
            "replacement image bytes must not be empty",
        ));
    }
    if request.encoded_bytes.len() > parsed.limits.max_stream_bytes {
        return Err(PdfError::limit(
            "replacement image exceeds max_stream_bytes",
        ));
    }
    if !matches!(request.bits_per_component, 1 | 2 | 4 | 8 | 16) {
        return Err(PdfError::unsupported(
            "image bits per component must be 1, 2, 4, 8, or 16",
        ));
    }
    match (request.filter, request.decode_params) {
        (ImageFilter::Flate, Some(params))
            if !matches!(params.predictor, 1 | 2 | 10..=15)
                || usize::from(params.colors) != request.color_space.components()
                || params.bits_per_component != request.bits_per_component
                || params.columns != request.width =>
        {
            return Err(PdfError::unsupported(
                "Flate image DecodeParms must match image dimensions and sample layout",
            ));
        }
        (ImageFilter::Flate, _) | (ImageFilter::Raw, None) => {}
        (_, Some(_)) => {
            return Err(PdfError::unsupported(
                "DecodeParms are only supported for Flate image replacement",
            ));
        }
        _ => {}
    }
    Ok(())
}

fn validate_encoded_bytes(
    dictionary: &BTreeMap<Vec<u8>, Value>,
    parsed: &ParsedDocument,
    request: &ImageReplacementRequest,
) -> Result<(), PdfError> {
    let expected = expected_sample_bytes(request)?;
    if expected > parsed.limits.max_stream_bytes {
        return Err(PdfError::limit(
            "decoded replacement image exceeds max_stream_bytes",
        ));
    }
    match request.filter {
        ImageFilter::Raw => {
            if request.encoded_bytes.len() != expected {
                return Err(PdfError::unsafe_rewrite(format!(
                    "raw image requires exactly {expected} sample bytes"
                )));
            }
        }
        ImageFilter::Flate => {
            let mut budget = ParseBudget::default();
            let decoded = parser::decode_stream(
                &Value::Dict(dictionary.clone()),
                &request.encoded_bytes,
                &parsed.limits,
                &mut budget,
            )?;
            if decoded.len() != expected {
                return Err(PdfError::unsafe_rewrite(format!(
                    "decoded Flate image requires exactly {expected} sample bytes"
                )));
            }
        }
        ImageFilter::Jpeg => {
            let (width, height, bits, components) = jpeg_dimensions(&request.encoded_bytes)?;
            validate_embedded_dimensions(request, width, height, bits, components)?;
        }
        ImageFilter::Jpx => {
            let (width, height, bits, components) = jpx_dimensions(&request.encoded_bytes)?;
            validate_embedded_dimensions(request, width, height, bits, components)?;
        }
    }
    Ok(())
}

fn validate_embedded_dimensions(
    request: &ImageReplacementRequest,
    width: u32,
    height: u32,
    bits: u8,
    components: usize,
) -> Result<(), PdfError> {
    if width != request.width
        || height != request.height
        || bits != request.bits_per_component
        || components != request.color_space.components()
    {
        return Err(PdfError::unsafe_rewrite(
            "encoded image dimensions or sample layout do not match the request",
        ));
    }
    Ok(())
}

fn expected_sample_bytes(request: &ImageReplacementRequest) -> Result<usize, PdfError> {
    let row_bits = usize::try_from(request.width)
        .ok()
        .and_then(|width| width.checked_mul(request.color_space.components()))
        .and_then(|samples| samples.checked_mul(usize::from(request.bits_per_component)))
        .ok_or_else(|| PdfError::limit("image row size overflows"))?;
    let row_bytes = row_bits
        .checked_add(7)
        .map(|bits| bits / 8)
        .ok_or_else(|| PdfError::limit("image row size overflows"))?;
    row_bytes
        .checked_mul(
            usize::try_from(request.height)
                .map_err(|_| PdfError::limit("image height exceeds usize"))?,
        )
        .ok_or_else(|| PdfError::limit("decoded image size overflows"))
}

fn validate_mask_policy(
    parsed: &ParsedDocument,
    dictionary: &BTreeMap<Vec<u8>, Value>,
    request: &ImageReplacementRequest,
) -> Result<(), PdfError> {
    let masks = [
        (b"Mask".as_slice(), dictionary.get(b"Mask".as_slice())),
        (b"SMask".as_slice(), dictionary.get(b"SMask".as_slice())),
    ];
    if masks.iter().all(|(_, value)| value.is_none()) {
        return Ok(());
    }
    if request.mask_policy != ImageMaskPolicy::PreserveCompatible {
        return Err(PdfError::unsupported(
            "image masks require preserve_compatible mask policy",
        ));
    }
    let old_width = integer(dictionary, b"Width");
    let old_height = integer(dictionary, b"Height");
    let old_bits = integer(dictionary, b"BitsPerComponent");
    let old_color = name(dictionary, b"ColorSpace");
    if old_width != Some(i64::from(request.width))
        || old_height != Some(i64::from(request.height))
        || old_bits != Some(i64::from(request.bits_per_component))
        || old_color != Some(request.color_space.name())
    {
        return Err(PdfError::unsupported(
            "preserved image masks require unchanged dimensions and sample layout",
        ));
    }
    for (kind, value) in masks
        .into_iter()
        .filter_map(|(kind, value)| value.map(|value| (kind, value)))
    {
        match value {
            Value::Array(values) if kind == b"Mask" => {
                validate_color_key_mask(values, request)?;
            }
            Value::Ref(reference) => {
                let mask = xobject_image_dictionary(parsed.object(*reference)?, "image mask")?;
                if integer(mask, b"Width") != Some(i64::from(request.width))
                    || integer(mask, b"Height") != Some(i64::from(request.height))
                {
                    return Err(PdfError::unsupported(
                        "preserved image mask dimensions do not match replacement",
                    ));
                }
                if kind == b"SMask"
                    && (matches!(mask.get(b"ImageMask".as_slice()), Some(Value::Bool(true)))
                        || name(mask, b"ColorSpace") != Some(b"DeviceGray"))
                {
                    return Err(PdfError::unsupported(
                        "preserved soft masks must be grayscale image XObjects",
                    ));
                }
            }
            _ => {
                return Err(PdfError::unsupported(
                    "unsupported image mask representation",
                ));
            }
        }
    }
    Ok(())
}

fn validate_generated_smask(
    parsed: &ParsedDocument,
    dictionary: &BTreeMap<Vec<u8>, Value>,
    request: &ImageReplacementRequest,
    smask: &GeneratedSoftMask,
) -> Result<(), PdfError> {
    if dictionary.contains_key(b"Mask".as_slice()) || dictionary.contains_key(b"SMask".as_slice()) {
        return Err(PdfError::unsupported(
            "indexed PNG transparency requires an image without existing Mask or SMask",
        ));
    }
    if dictionary.contains_key(b"SMaskInData".as_slice()) {
        return Err(PdfError::unsupported(
            "indexed PNG transparency does not replace embedded image alpha data",
        ));
    }
    let expected = soft_mask_sample_bytes(request.width, request.height)?;
    if smask.samples.len() != expected {
        return Err(PdfError::unsafe_rewrite(
            "generated soft mask sample layout does not match image dimensions",
        ));
    }
    if smask.encoded_bytes.is_empty() || smask.encoded_bytes.len() > parsed.limits.max_stream_bytes
    {
        return Err(PdfError::limit(
            "generated soft mask exceeds max_stream_bytes",
        ));
    }
    let dictionary =
        soft_mask_dictionary(request.width, request.height, smask.encoded_bytes.len())?;
    let mut budget = ParseBudget::default();
    let decoded = parser::decode_stream(
        &Value::Dict(dictionary),
        &smask.encoded_bytes,
        &parsed.limits,
        &mut budget,
    )?;
    if decoded != smask.samples {
        return Err(PdfError::unsafe_rewrite(
            "generated soft mask stream does not match its samples",
        ));
    }
    Ok(())
}

fn allocate_image_reference(parsed: &ParsedDocument) -> Result<ObjectRef, PdfError> {
    if parsed.objects.len() >= parsed.limits.max_objects {
        return Err(PdfError::limit(
            "generated soft mask object exceeds max_objects",
        ));
    }
    let number = parsed
        .objects
        .keys()
        .map(|reference| reference.number)
        .max()
        .unwrap_or(0)
        .checked_add(1)
        .ok_or_else(|| PdfError::limit("generated soft mask object number overflows"))?;
    if usize::try_from(number)
        .ok()
        .and_then(|value| value.checked_add(1))
        .is_none_or(|value| value > parsed.limits.max_xref_entries)
    {
        return Err(PdfError::limit(
            "generated soft mask object exceeds max_xref_entries",
        ));
    }
    Ok(ObjectRef {
        number,
        generation: 0,
    })
}

fn soft_mask_sample_bytes(width: u32, height: u32) -> Result<usize, PdfError> {
    usize::try_from(width)
        .map_err(|_| PdfError::limit("soft mask width exceeds usize"))?
        .checked_mul(
            usize::try_from(height)
                .map_err(|_| PdfError::limit("soft mask height exceeds usize"))?,
        )
        .ok_or_else(|| PdfError::limit("soft mask sample size overflows"))
}

fn soft_mask_dictionary(
    width: u32,
    height: u32,
    encoded_len: usize,
) -> Result<BTreeMap<Vec<u8>, Value>, PdfError> {
    Ok(BTreeMap::from([
        (b"Type".to_vec(), Value::Name(b"XObject".to_vec())),
        (b"Subtype".to_vec(), Value::Name(b"Image".to_vec())),
        (b"Width".to_vec(), Value::Integer(i64::from(width))),
        (b"Height".to_vec(), Value::Integer(i64::from(height))),
        (b"BitsPerComponent".to_vec(), Value::Integer(8)),
        (b"ColorSpace".to_vec(), Value::Name(b"DeviceGray".to_vec())),
        (b"Filter".to_vec(), Value::Name(b"FlateDecode".to_vec())),
        (
            b"Length".to_vec(),
            Value::Integer(
                i64::try_from(encoded_len)
                    .map_err(|_| PdfError::limit("soft mask stream length exceeds i64"))?,
            ),
        ),
    ]))
}

fn generated_smask_matches(
    parsed: &ParsedDocument,
    image: &BTreeMap<Vec<u8>, Value>,
    reference: ObjectRef,
    request: &ImageReplacementRequest,
    expected: &GeneratedSoftMask,
) -> Result<bool, PdfError> {
    if image.contains_key(b"Mask".as_slice())
        || image.contains_key(b"SMaskInData".as_slice())
        || image.get(b"SMask".as_slice()) != Some(&Value::Ref(reference))
    {
        return Ok(false);
    }
    let object = parsed.object(reference)?;
    let dictionary = xobject_image_dictionary(object, "generated soft mask")?;
    if dictionary.contains_key(b"ImageMask".as_slice())
        || dictionary.contains_key(b"Mask".as_slice())
        || dictionary.contains_key(b"SMask".as_slice())
        || dictionary.contains_key(b"SMaskInData".as_slice())
        || integer(dictionary, b"Width") != Some(i64::from(request.width))
        || integer(dictionary, b"Height") != Some(i64::from(request.height))
        || integer(dictionary, b"BitsPerComponent") != Some(8)
        || name(dictionary, b"ColorSpace") != Some(b"DeviceGray")
        || !filter_matches(dictionary, ImageFilter::Flate)
        || dictionary.contains_key(b"DecodeParms".as_slice())
        || dictionary.contains_key(b"Decode".as_slice())
        || integer(dictionary, b"Length") != i64::try_from(expected.encoded_bytes.len()).ok()
        || object.stream.as_deref() != Some(expected.encoded_bytes.as_slice())
    {
        return Ok(false);
    }
    let Some(stream) = object.stream.as_deref() else {
        return Ok(false);
    };
    let mut budget = ParseBudget::default();
    let decoded = parser::decode_stream(
        &Value::Dict(dictionary.clone()),
        stream,
        &parsed.limits,
        &mut budget,
    )?;
    Ok(decoded == expected.samples)
}

fn image_dictionary<'a>(
    object: &'a crate::parser::IndirectObject,
    label: &str,
) -> Result<&'a BTreeMap<Vec<u8>, Value>, PdfError> {
    let dictionary = xobject_image_dictionary(object, label)?;
    if matches!(
        dictionary.get(b"ImageMask".as_slice()),
        Some(Value::Bool(true))
    ) {
        return Err(PdfError::unsupported(
            "replacing stencil ImageMask XObjects is not supported",
        ));
    }
    Ok(dictionary)
}

fn xobject_image_dictionary<'a>(
    object: &'a crate::parser::IndirectObject,
    label: &str,
) -> Result<&'a BTreeMap<Vec<u8>, Value>, PdfError> {
    let Value::Dict(dictionary) = &object.value else {
        return Err(PdfError::selection(format!("{label} is not a dictionary")));
    };
    if object.stream.is_none()
        || !matches!(dictionary.get(b"Type".as_slice()), Some(Value::Name(name)) if name == b"XObject")
        || !matches!(dictionary.get(b"Subtype".as_slice()), Some(Value::Name(name)) if name == b"Image")
    {
        return Err(PdfError::selection(format!(
            "{label} is not an Image XObject stream"
        )));
    }
    Ok(dictionary)
}

fn validate_color_key_mask(
    values: &[Value],
    request: &ImageReplacementRequest,
) -> Result<(), PdfError> {
    if values.len() != request.color_space.components() * 2 {
        return Err(PdfError::unsupported(
            "color-key mask size does not match image color components",
        ));
    }
    let maximum = (1_u32 << request.bits_per_component) - 1;
    for pair in values.chunks_exact(2) {
        let [Value::Integer(low), Value::Integer(high)] = pair else {
            return Err(PdfError::unsupported(
                "color-key mask entries must be integers",
            ));
        };
        if *low < 0 || high < low || u32::try_from(*high).map_or(true, |high| high > maximum) {
            return Err(PdfError::unsupported(
                "color-key mask range exceeds image sample depth",
            ));
        }
    }
    Ok(())
}

fn decode_params_value(params: ImageDecodeParams) -> Value {
    Value::Dict(BTreeMap::from([
        (
            b"Predictor".to_vec(),
            Value::Integer(i64::from(params.predictor)),
        ),
        (b"Colors".to_vec(), Value::Integer(i64::from(params.colors))),
        (
            b"BitsPerComponent".to_vec(),
            Value::Integer(i64::from(params.bits_per_component)),
        ),
        (
            b"Columns".to_vec(),
            Value::Integer(i64::from(params.columns)),
        ),
    ]))
}

const PNG_SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";

#[derive(Clone, Copy)]
struct PngHeader {
    width: u32,
    height: u32,
    source_components: usize,
    indexed: bool,
}

struct NormalizedPng {
    encoded_bytes: Vec<u8>,
    width: u32,
    height: u32,
    components: usize,
    soft_mask: Option<GeneratedSoftMask>,
}

fn normalize_png(input: &[u8], limits: &Limits) -> Result<NormalizedPng, PdfError> {
    if !input.starts_with(PNG_SIGNATURE) {
        return Err(PdfError::syntax("PNG image is missing its signature", 0));
    }
    let mut offset = PNG_SIGNATURE.len();
    let mut header = None;
    let mut palette = None;
    let mut transparency = None;
    let mut idat = Vec::new();
    let mut saw_idat = false;
    let mut idat_finished = false;
    let mut saw_iend = false;
    let mut chunks = 0_usize;

    while offset < input.len() {
        chunks = chunks
            .checked_add(1)
            .ok_or_else(|| PdfError::limit("PNG chunk count overflows"))?;
        if chunks > limits.max_container_items {
            return Err(PdfError::limit(
                "PNG chunk count exceeds max_container_items",
            ));
        }
        let chunk_header_end = offset
            .checked_add(8)
            .filter(|end| *end <= input.len())
            .ok_or_else(|| PdfError::syntax("PNG chunk header is truncated", offset))?;
        let length = usize::try_from(u32::from_be_bytes(
            input[offset..offset + 4].try_into().unwrap(),
        ))
        .map_err(|_| PdfError::limit("PNG chunk length exceeds usize"))?;
        let data_end = chunk_header_end
            .checked_add(length)
            .ok_or_else(|| PdfError::limit("PNG chunk length overflows"))?;
        let chunk_end = data_end
            .checked_add(4)
            .filter(|end| *end <= input.len())
            .ok_or_else(|| PdfError::syntax("PNG chunk data is truncated", offset))?;
        let kind = &input[offset + 4..chunk_header_end];
        let data = &input[chunk_header_end..data_end];
        let expected_crc = u32::from_be_bytes(input[data_end..chunk_end].try_into().unwrap());
        if png_crc32(kind, data) != expected_crc {
            return Err(PdfError::syntax("PNG chunk CRC does not match", offset));
        }
        if !kind.iter().all(u8::is_ascii_alphabetic) {
            return Err(PdfError::syntax("PNG chunk type is invalid", offset));
        }
        if header.is_none() && kind != b"IHDR" {
            return Err(PdfError::syntax("PNG IHDR must be the first chunk", offset));
        }

        match kind {
            b"IHDR" => {
                if offset != PNG_SIGNATURE.len() || header.is_some() || data.len() != 13 {
                    return Err(PdfError::syntax("PNG IHDR layout is invalid", offset));
                }
                header = Some(png_header(data)?);
            }
            b"IDAT" => {
                if header.is_none() || idat_finished {
                    return Err(PdfError::syntax("PNG IDAT layout is invalid", offset));
                }
                if header.is_some_and(|header: PngHeader| header.indexed) && palette.is_none() {
                    return Err(PdfError::syntax(
                        "indexed PNG requires PLTE before IDAT",
                        offset,
                    ));
                }
                let next_len = idat
                    .len()
                    .checked_add(data.len())
                    .ok_or_else(|| PdfError::limit("PNG IDAT length overflows"))?;
                if next_len > limits.max_stream_bytes {
                    return Err(PdfError::limit("PNG IDAT exceeds max_stream_bytes"));
                }
                idat.extend_from_slice(data);
                saw_idat = true;
            }
            b"IEND" => {
                if header.is_none() || !saw_idat || !data.is_empty() || chunk_end != input.len() {
                    return Err(PdfError::syntax("PNG IEND layout is invalid", offset));
                }
                saw_iend = true;
                break;
            }
            b"PLTE" => {
                let Some(header) = header else {
                    return Err(PdfError::syntax("PNG PLTE has no IHDR", offset));
                };
                if !header.indexed {
                    return Err(PdfError::unsupported(
                        "PNG PLTE is only supported for indexed input",
                    ));
                }
                if saw_idat || palette.is_some() || data.is_empty() || !data.len().is_multiple_of(3)
                {
                    return Err(PdfError::syntax("PNG PLTE layout is invalid", offset));
                }
                if data.len() > 256 * 3 {
                    return Err(PdfError::syntax(
                        "PNG PLTE has more than 256 entries",
                        offset,
                    ));
                }
                palette = Some(data.to_vec());
            }
            b"tRNS" => {
                let Some(header) = header else {
                    return Err(PdfError::syntax("PNG tRNS has no IHDR", offset));
                };
                if !header.indexed {
                    return Err(PdfError::unsupported(
                        "transparent PNG input is not supported for image replacement",
                    ));
                }
                let Some(palette) = palette.as_deref() else {
                    return Err(PdfError::syntax(
                        "indexed PNG tRNS requires PLTE before transparency",
                        offset,
                    ));
                };
                if saw_idat
                    || transparency.is_some()
                    || data.is_empty()
                    || data.len() > palette.len() / 3
                {
                    return Err(PdfError::syntax("PNG tRNS layout is invalid", offset));
                }
                transparency = Some(data.to_vec());
            }
            _ if kind.first().is_some_and(u8::is_ascii_lowercase) => {
                // ponytail: ignore ancillary color/profile data; add a color-management policy before honoring it.
            }
            _ => {
                return Err(PdfError::unsupported(format!(
                    "unsupported critical PNG chunk {}",
                    String::from_utf8_lossy(kind)
                )));
            }
        }
        if saw_idat && kind != b"IDAT" {
            idat_finished = true;
        }
        offset = chunk_end;
    }

    let header = header.ok_or_else(|| PdfError::syntax("PNG image has no IHDR chunk", 0))?;
    if !saw_iend {
        return Err(PdfError::syntax("PNG image has no IEND chunk", input.len()));
    }
    let samples = decode_png_idat(
        &idat,
        header.width,
        header.height,
        header.source_components,
        limits,
    )?;
    let (samples, alpha, components) = if header.indexed {
        let (samples, alpha) = expand_png_palette(
            samples,
            palette
                .as_deref()
                .ok_or_else(|| PdfError::syntax("indexed PNG has no PLTE", 0))?,
            transparency.as_deref(),
            limits,
        )?;
        (samples, alpha, 3)
    } else {
        (samples, None, header.source_components)
    };
    let soft_mask = alpha
        .map(|samples| {
            // ponytail: only indexed tRNS becomes an owned grayscale SMask; general alpha needs separate semantics.
            Ok(GeneratedSoftMask {
                encoded_bytes: encode_flate(&samples, limits.max_stream_bytes)?,
                samples,
            })
        })
        .transpose()?;
    Ok(NormalizedPng {
        encoded_bytes: encode_flate(&samples, limits.max_stream_bytes)?,
        width: header.width,
        height: header.height,
        components,
        soft_mask,
    })
}

fn png_header(data: &[u8]) -> Result<PngHeader, PdfError> {
    let width = u32::from_be_bytes(data[..4].try_into().unwrap());
    let height = u32::from_be_bytes(data[4..8].try_into().unwrap());
    if width == 0 || height == 0 {
        return Err(PdfError::syntax("PNG dimensions must be non-zero", 0));
    }
    if data[8] != 8 {
        return Err(PdfError::unsupported(
            "PNG image replacement only supports 8-bit samples",
        ));
    }
    let (source_components, indexed) = match data[9] {
        0 => (1, false),
        2 => (3, false),
        3 => (1, true),
        4 | 6 => {
            return Err(PdfError::unsupported(
                "alpha PNG input is not supported for image replacement",
            ));
        }
        _ => {
            return Err(PdfError::unsupported(
                "PNG image color type is not supported for image replacement",
            ));
        }
    };
    if data[10] != 0 || data[11] != 0 {
        return Err(PdfError::unsupported(
            "PNG compression or filter method is not supported",
        ));
    }
    if data[12] != 0 {
        return Err(PdfError::unsupported(
            "interlaced PNG input is not supported for image replacement",
        ));
    }
    Ok(PngHeader {
        width,
        height,
        source_components,
        indexed,
    })
}

fn expand_png_palette(
    samples: Vec<u8>,
    palette: &[u8],
    transparency: Option<&[u8]>,
    limits: &Limits,
) -> Result<(Vec<u8>, Option<Vec<u8>>), PdfError> {
    let output_len = samples
        .len()
        .checked_mul(3)
        .ok_or_else(|| PdfError::limit("expanded PNG palette size overflows"))?;
    let alpha_len = transparency.map_or(0, |_| samples.len());
    let total = samples
        .len()
        .checked_add(output_len)
        .and_then(|total| total.checked_add(alpha_len))
        .ok_or_else(|| PdfError::limit("expanded PNG palette allocation overflows"))?;
    let max_decoded = limits.max_stream_bytes.min(limits.max_total_decoded_bytes);
    if total > max_decoded {
        return Err(PdfError::limit(
            "expanded PNG palette samples exceed configured resource limits",
        ));
    }
    let mut output = Vec::with_capacity(output_len);
    let mut alpha = transparency.map(|_| Vec::with_capacity(alpha_len));
    for index in samples {
        let start = usize::from(index) * 3;
        let color = palette
            .get(start..start + 3)
            .ok_or_else(|| PdfError::syntax("PNG palette index is outside PLTE", 0))?;
        output.extend_from_slice(color);
        if let Some(alpha) = alpha.as_mut() {
            alpha.push(
                transparency
                    .and_then(|values| values.get(usize::from(index)).copied())
                    .unwrap_or(255),
            );
        }
    }
    Ok((output, alpha))
}

fn decode_png_idat(
    idat: &[u8],
    width: u32,
    height: u32,
    components: usize,
    limits: &Limits,
) -> Result<Vec<u8>, PdfError> {
    let row_bytes = usize::try_from(width)
        .map_err(|_| PdfError::limit("PNG width exceeds usize"))?
        .checked_mul(components)
        .ok_or_else(|| PdfError::limit("PNG row size overflows"))?;
    let rows = usize::try_from(height).map_err(|_| PdfError::limit("PNG height exceeds usize"))?;
    let samples = row_bytes
        .checked_mul(rows)
        .ok_or_else(|| PdfError::limit("PNG sample size overflows"))?;
    let filtered = row_bytes
        .checked_add(1)
        .and_then(|row| row.checked_mul(rows))
        .ok_or_else(|| PdfError::limit("PNG filtered sample size overflows"))?;
    let max_decoded = limits.max_stream_bytes.min(limits.max_total_decoded_bytes);
    if samples > max_decoded || filtered > max_decoded {
        return Err(PdfError::limit(
            "PNG decoded samples exceed configured resource limits",
        ));
    }

    let mut decoder = ZlibDecoder::new(idat);
    let mut output = Vec::with_capacity(samples);
    let mut previous = vec![0_u8; row_bytes];
    let mut row = vec![0_u8; row_bytes];
    for _ in 0..rows {
        let mut filter = [0_u8; 1];
        decoder
            .read_exact(&mut filter)
            .map_err(|error| PdfError::syntax(format!("invalid PNG IDAT stream: {error}"), 0))?;
        decoder
            .read_exact(&mut row)
            .map_err(|error| PdfError::syntax(format!("invalid PNG IDAT stream: {error}"), 0))?;
        unfilter_png_row(&mut row, &previous, components, filter[0])?;
        output.extend_from_slice(&row);
        std::mem::swap(&mut previous, &mut row);
    }
    let mut extra = [0_u8; 1];
    match decoder.read(&mut extra) {
        Ok(0) => {}
        Ok(_) => {
            return Err(PdfError::syntax(
                "PNG IDAT decoded sample layout exceeds IHDR dimensions",
                0,
            ));
        }
        Err(error) => {
            return Err(PdfError::syntax(
                format!("invalid PNG IDAT stream: {error}"),
                0,
            ));
        }
    }
    if decoder.total_in()
        != u64::try_from(idat.len()).map_err(|_| PdfError::limit("PNG IDAT length exceeds u64"))?
    {
        return Err(PdfError::syntax("PNG IDAT stream has trailing data", 0));
    }
    Ok(output)
}

fn unfilter_png_row(
    row: &mut [u8],
    previous: &[u8],
    components: usize,
    filter: u8,
) -> Result<(), PdfError> {
    if previous.len() != row.len() {
        return Err(PdfError::verification(
            "PNG filter row lengths do not match",
        ));
    }
    if filter > 4 {
        return Err(PdfError::unsupported(format!(
            "PNG scanline filter {filter} is not supported"
        )));
    }
    for index in 0..row.len() {
        let left = (index >= components).then(|| row[index - components]);
        let above = previous[index];
        let upper_left = (index >= components).then(|| previous[index - components]);
        let predictor = match filter {
            0 => 0,
            1 => left.unwrap_or(0),
            2 => above,
            3 => ((u16::from(left.unwrap_or(0)) + u16::from(above)) / 2) as u8,
            4 => png_paeth(left.unwrap_or(0), above, upper_left.unwrap_or(0)),
            _ => unreachable!(),
        };
        row[index] = row[index].wrapping_add(predictor);
    }
    Ok(())
}

fn png_paeth(left: u8, above: u8, upper_left: u8) -> u8 {
    let prediction = i16::from(left) + i16::from(above) - i16::from(upper_left);
    let left_distance = (prediction - i16::from(left)).abs();
    let above_distance = (prediction - i16::from(above)).abs();
    let upper_left_distance = (prediction - i16::from(upper_left)).abs();
    if left_distance <= above_distance && left_distance <= upper_left_distance {
        left
    } else if above_distance <= upper_left_distance {
        above
    } else {
        upper_left
    }
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

fn jpeg_dimensions(input: &[u8]) -> Result<(u32, u32, u8, usize), PdfError> {
    if !input.starts_with(&[0xff, 0xd8]) || !input.ends_with(&[0xff, 0xd9]) {
        return Err(PdfError::syntax("JPEG image is missing SOI or EOI", 0));
    }
    let mut offset = 2;
    let mut dimensions = None;
    while offset + 4 <= input.len() {
        if input[offset] != 0xff {
            offset += 1;
            continue;
        }
        let marker = input[offset + 1];
        offset += 2;
        if marker == 0xda {
            let length = usize::from(u16::from_be_bytes([input[offset], input[offset + 1]]));
            if length < 2
                || offset
                    .checked_add(length)
                    .is_none_or(|end| end > input.len())
            {
                return Err(PdfError::syntax("JPEG SOS marker is truncated", offset));
            }
            return dimensions
                .ok_or_else(|| PdfError::unsupported("JPEG image has no supported SOF marker"));
        }
        if matches!(marker, 0xd8 | 0xd9 | 0x01 | 0xd0..=0xd7) {
            continue;
        }
        let length = usize::from(u16::from_be_bytes([input[offset], input[offset + 1]]));
        if length < 2
            || offset
                .checked_add(length)
                .is_none_or(|end| end > input.len())
        {
            return Err(PdfError::syntax("JPEG marker length is invalid", offset));
        }
        if matches!(marker, 0xc0..=0xc2) {
            if length < 8 {
                return Err(PdfError::syntax("JPEG SOF marker is truncated", offset));
            }
            dimensions = Some((
                u32::from(u16::from_be_bytes([input[offset + 5], input[offset + 6]])),
                u32::from(u16::from_be_bytes([input[offset + 3], input[offset + 4]])),
                input[offset + 2],
                usize::from(input[offset + 7]),
            ));
        }
        offset += length;
    }
    Err(PdfError::unsupported(
        "JPEG image has no supported baseline, extended, or progressive SOF marker",
    ))
}

fn jpx_dimensions(input: &[u8]) -> Result<(u32, u32, u8, usize), PdfError> {
    if input.starts_with(&[0xff, 0x4f, 0xff, 0x51]) {
        if !input.ends_with(&[0xff, 0xd9]) {
            return Err(PdfError::syntax(
                "JPEG 2000 codestream has no EOC marker",
                0,
            ));
        }
        return j2k_siz(&input[2..]);
    }
    let mut offset = 0_usize;
    while offset + 8 <= input.len() {
        let length = usize::try_from(u32::from_be_bytes([
            input[offset],
            input[offset + 1],
            input[offset + 2],
            input[offset + 3],
        ]))
        .map_err(|_| PdfError::limit("JP2 box length exceeds usize"))?;
        if length < 8
            || offset
                .checked_add(length)
                .is_none_or(|end| end > input.len())
        {
            return Err(PdfError::syntax("JP2 box length is invalid", offset));
        }
        if &input[offset + 4..offset + 8] == b"jp2c" {
            let codestream = &input[offset + 8..offset + length];
            if !codestream.starts_with(&[0xff, 0x4f, 0xff, 0x51]) {
                return Err(PdfError::syntax(
                    "JP2 codestream has no SOC/SIZ markers",
                    offset,
                ));
            }
            if !codestream.ends_with(&[0xff, 0xd9]) {
                return Err(PdfError::syntax("JP2 codestream has no EOC marker", offset));
            }
            return j2k_siz(&codestream[2..]);
        }
        offset += length;
    }
    Err(PdfError::unsupported(
        "JPX replacement requires a JPEG 2000 codestream or JP2 jp2c box",
    ))
}

fn j2k_siz(input: &[u8]) -> Result<(u32, u32, u8, usize), PdfError> {
    if input.len() < 42 || !input.starts_with(&[0xff, 0x51]) {
        return Err(PdfError::syntax("JPEG 2000 SIZ marker is truncated", 0));
    }
    let length = usize::from(u16::from_be_bytes([input[2], input[3]]));
    if length < 41
        || length
            .checked_add(4)
            .is_none_or(|minimum| minimum > input.len())
    {
        return Err(PdfError::syntax("JPEG 2000 SIZ length is invalid", 0));
    }
    let xsiz = u32::from_be_bytes([input[6], input[7], input[8], input[9]]);
    let ysiz = u32::from_be_bytes([input[10], input[11], input[12], input[13]]);
    let xosiz = u32::from_be_bytes([input[14], input[15], input[16], input[17]]);
    let yosiz = u32::from_be_bytes([input[18], input[19], input[20], input[21]]);
    let components = usize::from(u16::from_be_bytes([input[38], input[39]]));
    if components == 0 || length < 38 + components * 3 {
        return Err(PdfError::syntax(
            "JPEG 2000 component table is truncated",
            0,
        ));
    }
    let bits = (input[40] & 0x7f) + 1;
    if (0..components).any(|index| (input[40 + index * 3] & 0x7f) + 1 != bits) {
        return Err(PdfError::unsupported(
            "JPEG 2000 components with mixed precision are not supported",
        ));
    }
    Ok((
        xsiz.checked_sub(xosiz)
            .ok_or_else(|| PdfError::syntax("JPEG 2000 width is invalid", 0))?,
        ysiz.checked_sub(yosiz)
            .ok_or_else(|| PdfError::syntax("JPEG 2000 height is invalid", 0))?,
        bits,
        components,
    ))
}

fn integer(dictionary: &BTreeMap<Vec<u8>, Value>, key: &[u8]) -> Option<i64> {
    match dictionary.get(key) {
        Some(Value::Integer(value)) => Some(*value),
        _ => None,
    }
}

fn filter_matches(dictionary: &BTreeMap<Vec<u8>, Value>, filter: ImageFilter) -> bool {
    match (filter, dictionary.get(b"Filter".as_slice())) {
        (ImageFilter::Raw, None) => true,
        (ImageFilter::Flate, Some(Value::Name(name))) => name == b"FlateDecode",
        (ImageFilter::Jpeg, Some(Value::Name(name))) => name == b"DCTDecode",
        (ImageFilter::Jpx, Some(Value::Name(name))) => name == b"JPXDecode",
        _ => false,
    }
}

fn name<'a>(dictionary: &'a BTreeMap<Vec<u8>, Value>, key: &[u8]) -> Option<&'a [u8]> {
    match dictionary.get(key) {
        Some(Value::Name(value)) => Some(value),
        _ => None,
    }
}

fn verify_references(parsed: &ParsedDocument) -> Result<bool, PdfError> {
    fn walk(value: &Value, parsed: &ParsedDocument, depth: usize) -> Result<bool, PdfError> {
        if depth > parsed.limits.max_parser_depth {
            return Err(PdfError::limit(
                "image reference verification exceeds depth limit",
            ));
        }
        match value {
            Value::Ref(reference) => Ok(parsed.objects.contains_key(reference)),
            Value::Array(values) => values.iter().try_fold(true, |valid, value| {
                Ok(valid && walk(value, parsed, depth + 1)?)
            }),
            Value::Dict(values) => values.values().try_fold(true, |valid, value| {
                Ok(valid && walk(value, parsed, depth + 1)?)
            }),
            _ => Ok(true),
        }
    }
    if !walk(&parsed.trailer, parsed, 0)? {
        return Ok(false);
    }
    parsed.objects.values().try_fold(true, |valid, object| {
        Ok(valid && walk(&object.value, parsed, 0)?)
    })
}
