mod annotations;
mod appearance;
mod batch;
mod blank;
mod canonical;
mod cmap;
mod cms_validation;
mod content;
mod document;
mod document_structures;
mod edit;
mod encryption;
mod error;
mod filtered_edit;
mod filters;
mod font_edit;
mod fonts;
mod forms;
mod images;
mod incremental;
mod inline_images;
mod limits;
mod ocr;
mod overlay;
mod pages;
mod parser;
mod profile;
mod public_key_encryption;
mod security;
mod signatures;
mod signing;
mod streams;
mod writer;
mod xfa;

pub use annotations::{
    Annotation, AnnotationContentsMutationOutcome, AnnotationContentsMutationReport,
    AnnotationContentsMutationRequest, AnnotationContentsMutationVerification,
    AnnotationCreateRequest, AnnotationLifecycleOutcome, AnnotationLifecycleReport,
    AnnotationLifecycleVerification, AnnotationRemoveRequest, AnnotationSubtype, list_annotations,
};
pub use appearance::{
    FreeTextAppearanceOutcome, FreeTextAppearanceReport, FreeTextAppearanceRequest,
    FreeTextAppearanceVerification, TextFieldAppearanceOutcome, TextFieldAppearanceReport,
    TextFieldAppearanceRequest, TextFieldAppearanceVerification,
};
pub use batch::{
    BatchTextEditOutcome, BatchTextEditPlan, BatchTextEditReport, BatchTextEditRequest,
    BatchTextEditVerification, PlannedTextEdit,
};
pub use blank::BlankPageSize;
pub use canonical::{CanonicalizeOutcome, CanonicalizeReport, CanonicalizeVerification};
pub use cmap::ToUnicodeCMap;
pub use document::{
    InspectResult, PageGeometry, PdfDocument, PdfEngine, QueryMatch, TextExtraction,
    TextExtractionWarning, TextGeometry, TextGeometryConfidence, TextSpan, ValidationResult,
};
pub use document_structures::{
    DocumentInfoMetadata, DocumentInfoUpdate, DocumentStructureOutcome, DocumentStructureReport,
    DocumentStructureVerification, EmbeddedAttachment, EmbeddedAttachmentUpdate, JavaScriptAction,
    JavaScriptActionInventory, NamedDestination, NamedDestinationUpdate, OutlineCreateRequest,
    OutlineItem, OutlineRemoveRequest, PageLabel, PageLabelSpec, PageLabelStyle, PageLabelUpdate,
    XmpMetadata, XmpMetadataUpdate, read_document_info, read_embedded_attachment_bytes,
    read_embedded_attachments, read_javascript_actions, read_named_destinations, read_outlines,
    read_page_labels, read_xmp_metadata,
};
pub use edit::{
    SurgicalEditOutcome, SurgicalEditReport, SurgicalEditVerification, SurgicalTextEditRequest,
};
pub use encryption::{
    DecryptionOutcome, DecryptionReport, DecryptionVerification, EncryptionOutcome,
    EncryptionReport, EncryptionVerification, StandardEncryptionOptions,
    StandardEncryptionRevision,
};
pub use error::{PdfError, PdfErrorCode};
pub use filtered_edit::{
    FilteredEditOutcome, FilteredEditReport, FilteredEditVerification, FilteredTextEditRequest,
};
pub use filters::{DecodeParams, PdfFilter, decode_filter_chain};
pub use font_edit::{FontEditOutcome, FontEditReport, FontEditVerification, FontTextEditRequest};
pub use fonts::FontDecoder;
pub use forms::{
    AppearanceStatus, ButtonChoiceMutationRequest, ButtonFieldMutationOutcome,
    ButtonFieldMutationReport, ButtonFieldMutationVerification, CheckboxFieldMutationRequest,
    FormField, FormFieldCreateRequest, FormFieldKind, FormFieldRemoveRequest, FormLifecycleOutcome,
    FormLifecycleReport, FormLifecycleVerification, FormValueMutationOutcome,
    FormValueMutationReport, FormValueMutationRequest, FormValueMutationVerification,
    FormWidgetRef, list_form_fields,
};
pub use images::{
    EncodedImageReplacementRequest, ImageColorSpace, ImageDecodeParams, ImageFilter,
    ImageMaskPolicy, ImageReplacementOutcome, ImageReplacementReport, ImageReplacementRequest,
    ImageReplacementVerification, ImageXObjectInventoryEntry, RawFlateImageSamples,
    list_image_xobjects, read_jpeg_xobject_bytes, read_jpx_xobject_bytes,
    read_raw_flate_image_samples,
};
pub use incremental::{
    IncrementalEditOutcome, IncrementalEditReport, IncrementalEditVerification,
    IncrementalTextEditRequest,
};
pub use inline_images::{
    InlineImageColorSpace, InlineImageFilter, InlineImageInventoryEntry,
    InlineImageReplacementOutcome, InlineImageReplacementReport, InlineImageReplacementRequest,
    InlineImageReplacementVerification, list_inline_images,
};
pub use limits::{EngineConfig, Limits, OpenOptions};
pub use ocr::{
    OcrParseLimits, OcrPlacedText, OcrTextBox, OcrTextLayerOutcome, OcrTextLayerPlan,
    OcrTextLayerReport, OcrTextLayerRequest, OcrTextLayerVerification, parse_alto_xml,
    parse_ocr_json,
};
pub use overlay::{
    OverlayStampOutcome, OverlayStampReport, OverlayStampRequest, OverlayStampVerification,
    TextOverlayOutcome, TextOverlayReport, TextOverlayRequest, TextOverlayVerification,
};
pub use pages::{
    PageCompositionPlacement, PageCompositionRequest, PageOperationOutcome, PageOperationReport,
    PageOperationVerification, PageTransform,
};
pub use profile::{CapabilityDecision, DocumentCapabilityProfile, OperationCapability};
pub use public_key_encryption::{
    PublicKeyDecryptionOutcome, PublicKeyEncryptionMethod, PublicKeyEncryptionOptions,
    PublicKeyEncryptionOutcome, PublicKeyEncryptionVerification,
};
pub use security::{EncryptionMetadata, inspect_encryption};
pub use signatures::{
    CmsParseStatus, CmsValidation, DigestMatchStatus, RevocationStatus, SignatureCryptoStatus,
    SignatureInspection, SignatureTrustOptions, TimestampInspection, TimestampStatus, TrustStatus,
    inspect_signatures, inspect_signatures_with_options,
};
pub use signing::{
    AppliedExternalSignature, ExternalSignatureFieldOptions, ExternalSignaturePlan,
    ExternalSignaturePlanDescriptor,
};
pub use streams::{
    StreamFilterMetadata, StreamInventoryEntry, StreamMutationOutcome, StreamMutationReport,
    StreamMutationRequest, StreamMutationVerification, StreamObjectRef, list_streams,
    read_decoded_stream,
};
pub use xfa::{
    XfaDatasetField, XfaDatasetMutationOutcome, XfaDatasetMutationReport,
    XfaDatasetMutationVerification, XfaDatasetSetRequest, XfaDynamicReport, XfaPacket,
    XfaReplaceOutcome, XfaReplaceReport, XfaReplaceRequest, XfaReplaceVerification,
    XfaTemplateDatasetMapping, inspect_xfa_dynamic, list_xfa_dataset_fields, list_xfa_packets,
    list_xfa_template_dataset_mappings,
};
