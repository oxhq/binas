mod adapter;
mod annotations;
mod appearance;
mod batch;
mod blank;
mod canonical;
mod cmap;
mod cms_validation;
mod content;
mod decode_cache;
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
mod import;
mod incremental;
mod inline_images;
mod legacy_evidence;
mod limits;
mod object;
mod ocr;
pub mod optimize;
mod overlay;
mod ownership;
mod pages;
mod parser;
mod payload;
pub mod preflight;
mod preservation;
mod profile;
mod public_key_encryption;
mod reference_index;
mod resource_usage;
mod security;
mod signatures;
mod signing;
pub mod standards;
mod store;
mod streams;
mod transaction;
mod writer;
mod xfa;

pub use adapter::{
    PdfAdapter, PdfAdapterError, PdfAdapterPlan, PdfGraphInspection, PdfGraphNodeId,
};
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
pub use decode_cache::{DecodeMetrics, DecodedPayload};
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
pub use forms::checkbox::PdfCheckboxEffect;
pub use forms::flatten::{PdfFieldFlattenEffect, PdfFieldFlattenRequest, PdfFlattenedWidgetEffect};
pub use forms::text::{PdfTextFieldFillEffect, PdfTextFieldFillRequest, PdfTextWidgetFillEffect};
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
pub use legacy_evidence::{
    PdfLegacyEvidenceDetail, PdfLegacyInvariant, legacy_verification_evidence,
};
pub use limits::{EngineConfig, Limits, OpenOptions};
pub use ocr::{
    OcrParseLimits, OcrPlacedText, OcrTextBox, OcrTextLayerOutcome, OcrTextLayerPlan,
    OcrTextLayerReport, OcrTextLayerRequest, OcrTextLayerVerification, parse_alto_xml,
    parse_ocr_json,
};
pub use optimize::{
    PdfFlateEffect, PdfFlatePlan, PdfFlateReceipt, PdfFlateSkip, PdfFlateSkipReason,
    PdfFlateStreamRole, PdfLosslessImageEffect, PdfLosslessImagePlan, PdfLosslessImageReceipt,
    PdfLosslessImageSkip, PdfLosslessImageSkipReason,
};
pub use overlay::{
    OverlayStampOutcome, OverlayStampReport, OverlayStampRequest, OverlayStampVerification,
    TextOverlayOutcome, TextOverlayReport, TextOverlayRequest, TextOverlayVerification,
};
pub use pages::{
    DocumentEntryLocation, OmittedDocumentEntry, PageAssemblyPolicy, PageCompositionPlacement,
    PageCompositionRequest, PageOperationOutcome, PageOperationReport, PageOperationVerification,
    PageTransform,
};
pub use preservation::delete::{
    PdfExcludedPageTargetPolicy, PdfPageDeleteEffect, PdfPageDeleteRequest, PdfPageDestinationRemap,
};
pub use preservation::duplicate::{
    PdfDuplicateFieldName, PdfDuplicatedAnnotationEffect, PdfDuplicatedFieldEffect,
    PdfPageDuplicateEffect, PdfPageDuplicateRequest,
};
pub use preservation::geometry::{PdfPageGeometryEffect, PdfPageTransformRequest};
pub use profile::{CapabilityDecision, DocumentCapabilityProfile, OperationCapability};
pub use public_key_encryption::{
    PublicKeyDecryptionOutcome, PublicKeyEncryptionMethod, PublicKeyEncryptionOptions,
    PublicKeyEncryptionOutcome, PublicKeyEncryptionVerification,
};
pub use reference_index::{
    PdfReference, PdfReferenceIndex, PdfReferenceOrigin, PdfReferencePathStep, PdfReferenceTarget,
};
pub use resource_usage::{PdfResourceKind, PdfResourceUsage, PdfResourceUse, PdfUsageGap};
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
    read_decoded_stream, read_decoded_stream_shared,
};
pub use transaction::{
    PdfFontTextEffect, PdfPageTextEditRequest, PdfRevision, PdfRevisionPlan, PdfRevisionReceipt,
    PdfRevisionRequest, PdfSharedEditMode, PdfStreamEditEffect,
};
pub use writer::{PdfRevisionExport, PdfWriterMode};
pub use xfa::{
    XfaDatasetField, XfaDatasetMutationOutcome, XfaDatasetMutationReport,
    XfaDatasetMutationVerification, XfaDatasetSetRequest, XfaDynamicReport, XfaPacket,
    XfaReplaceOutcome, XfaReplaceReport, XfaReplaceRequest, XfaReplaceVerification,
    XfaTemplateDatasetMapping, inspect_xfa_dynamic, list_xfa_dataset_fields, list_xfa_packets,
    list_xfa_template_dataset_mappings,
};
