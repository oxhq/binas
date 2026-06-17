package pdfapi

import (
	"context"

	"github.com/oxhq/binas/pkg/adapters/pdf"
	"github.com/oxhq/binas/pkg/core"
)

const (
	KindDocument = pdf.KindDocument
	KindStream   = pdf.KindStream
	KindTextShow = pdf.KindTextShow
)

type GraphOptions = pdf.GraphOptions
type Graph = pdf.Graph
type Value = pdf.Value
type Name = pdf.Name
type String = pdf.String
type HexString = pdf.HexString
type Bool = pdf.Bool
type Null = pdf.Null
type Number = pdf.Number
type Array = pdf.Array
type Dict = pdf.Dict
type Ref = pdf.Ref
type ObjectRef = pdf.ObjectRef
type Stream = pdf.Stream
type ObjectSourceKind = pdf.ObjectSourceKind
type Object = pdf.Object
type PageNode = pdf.PageNode
type NameTreeKind = pdf.NameTreeKind
type NameTreeEntry = pdf.NameTreeEntry

const (
	NameTreeDests         = pdf.NameTreeDests
	NameTreeEmbeddedFiles = pdf.NameTreeEmbeddedFiles
	NameTreeJavaScript    = pdf.NameTreeJavaScript

	ObjectSourceNormal                = pdf.ObjectSourceNormal
	ObjectSourceObjectStream          = pdf.ObjectSourceObjectStream
	ObjectSourceObjectStreamContainer = pdf.ObjectSourceObjectStreamContainer
	ObjectSourceXRefStream            = pdf.ObjectSourceXRefStream

	SignatureInvalidationRefuse              = pdf.SignatureInvalidationRefuse
	SignatureInvalidationInvalidate          = pdf.SignatureInvalidationInvalidate
	SignatureInvalidationPreserveIncremental = pdf.SignatureInvalidationPreserveIncremental

	FallbackNone         = pdf.FallbackNone
	FallbackOverlay      = pdf.FallbackOverlay
	FallbackOCRTextLayer = pdf.FallbackOCRTextLayer
	FallbackModeNone     = pdf.FallbackModeNone
	FallbackModeExplicit = pdf.FallbackModeExplicit
)

type PageOperationOptions = pdf.PageOperationOptions
type PageOperationReport = pdf.PageOperationReport
type PageCopyInfo = pdf.PageCopyInfo
type PageOperationVerificationOptions = pdf.PageOperationVerificationOptions
type PageOperationVerification = pdf.PageOperationVerification
type PageOperationDanglingRef = pdf.PageOperationDanglingRef
type PageSource = pdf.PageSource
type PageWriteOptions = pdf.PageWriteOptions
type PageSelector = pdf.PageSelector
type PageBox = pdf.PageBox
type PageTransform = pdf.PageTransform
type PageScale = pdf.PageScale
type XFADynamicReport = pdf.XFADynamicReport
type XFADynamicMarker = pdf.XFADynamicMarker
type EncryptOptions = pdf.EncryptOptions
type ChangePasswordOptions = pdf.ChangePasswordOptions
type PublicKeyEncryptOptions = pdf.PublicKeyEncryptOptions
type UnsupportedEncryptionAlgorithmError = pdf.UnsupportedEncryptionAlgorithmError
type UnsupportedEncryptionWriteError = pdf.UnsupportedEncryptionWriteError
type SecurityOptions = pdf.SecurityOptions
type SecurityMetadataOptions = pdf.SecurityMetadataOptions
type SecurityMetadata = pdf.SecurityMetadata
type EncryptionMetadata = pdf.EncryptionMetadata
type EncryptionCryptFilter = pdf.EncryptionCryptFilter
type SignatureInvalidationMode = pdf.SignatureInvalidationMode
type SignatureMetadata = pdf.SignatureMetadata
type SignatureTrustOptions = pdf.SignatureTrustOptions
type SignaturePreservationVerification = pdf.SignaturePreservationVerification
type SignatureByteRange = pdf.SignatureByteRange
type SignatureSigningRequest = pdf.SignatureSigningRequest
type SignatureSigningResponse = pdf.SignatureSigningResponse
type SignatureSigningCallback = pdf.SignatureSigningCallback
type SignatureSigningCallbackMetadata = pdf.SignatureSigningCallbackMetadata
type SignatureSigningPlanOptions = pdf.SignatureSigningPlanOptions
type SignatureSigningPlan = pdf.SignatureSigningPlan
type SignatureReSigningVerification = pdf.SignatureReSigningVerification
type FormFieldMetadata = pdf.FormFieldMetadata
type FormFieldEditOptions = pdf.FormFieldEditOptions
type FormFieldEditReport = pdf.FormFieldEditReport
type FormFieldEditVerification = pdf.FormFieldEditVerification
type FormFieldCreateOptions = pdf.FormFieldCreateOptions
type FormFieldMutationReport = pdf.FormFieldMutationReport
type FormFieldMutationVerification = pdf.FormFieldMutationVerification
type UnsupportedFormFlatteningError = pdf.UnsupportedFormFlatteningError
type AnnotationCandidateMetadata = pdf.AnnotationCandidateMetadata
type AnnotationContentsEditOptions = pdf.AnnotationContentsEditOptions
type AnnotationContentsEditReport = pdf.AnnotationContentsEditReport
type AnnotationContentsEditVerification = pdf.AnnotationContentsEditVerification
type XFAPacketMetadata = pdf.XFAPacketMetadata
type XFASelector = pdf.XFASelector
type XFAPacketListOptions = pdf.XFAPacketListOptions
type XFADatasetField = pdf.XFADatasetField
type XFAReplaceOptions = pdf.XFAReplaceOptions
type XFADatasetFieldUpdateOptions = pdf.XFADatasetFieldUpdateOptions
type XFADatasetFieldListOptions = pdf.XFADatasetFieldListOptions
type XFATemplateDatasetMapping = pdf.XFATemplateDatasetMapping
type XFASemanticMetadata = pdf.XFASemanticMetadata
type OCRTextLayerBox = pdf.OCRTextLayerBox
type OCRTextLayerOptions = pdf.OCRTextLayerOptions
type OCRTextLayerPlan = pdf.OCRTextLayerPlan
type ExplicitOverlayStampOptions = pdf.ExplicitOverlayStampOptions
type FallbackKind = pdf.FallbackKind
type FallbackMode = pdf.FallbackMode
type OverlayPolicy = pdf.OverlayPolicy
type StreamObjectRef = pdf.StreamObjectRef
type StreamName = pdf.StreamName
type StreamMutationOptions = pdf.StreamMutationOptions
type StreamMutationReport = pdf.StreamMutationReport
type StreamMutationVerification = pdf.StreamMutationVerification
type ReplaceImageXObjectOptions = pdf.ReplaceImageXObjectOptions
type ReplaceInlineImageOptions = pdf.ReplaceInlineImageOptions

var (
	ErrEncryptedPDFPasswordRequired     = pdf.ErrEncryptedPDFPasswordRequired
	ErrEncryptedPDFUnsupportedAlgorithm = pdf.ErrEncryptedPDFUnsupportedAlgorithm
	ErrEncryptedPDFWriteUnsupported     = pdf.ErrEncryptedPDFWriteUnsupported
	ErrSignedPDFRequiresInvalidation    = pdf.ErrSignedPDFRequiresInvalidation
	ErrSignedPDFByteRangeProofRequired  = pdf.ErrSignedPDFByteRangeProofRequired
	ErrUnsupportedImageMutation         = pdf.ErrUnsupportedImageMutation

	ErrFallbackRequiresExplicitMode      = pdf.ErrFallbackRequiresExplicitMode
	ErrFallbackModeWithoutFallback       = pdf.ErrFallbackModeWithoutFallback
	ErrUnknownFallbackKind               = pdf.ErrUnknownFallbackKind
	ErrUnknownFallbackMode               = pdf.ErrUnknownFallbackMode
	ErrTrueTextEditRejectsFallbackPolicy = pdf.ErrTrueTextEditRejectsFallbackPolicy

	ErrSignatureSigningCallbackRequired        = pdf.ErrSignatureSigningCallbackRequired
	ErrSignatureSigningCallbackMetadataInvalid = pdf.ErrSignatureSigningCallbackMetadataInvalid
)

func ParseGraph(input []byte, opts GraphOptions) (*Graph, error) {
	return pdf.ParseGraph(input, opts)
}

func CopyPages(input []byte, pages []int, options ...PageOperationOptions) ([]byte, PageOperationReport, PageOperationVerification, error) {
	return pdf.CopyPages(input, pages, options...)
}

func ExtractPages(input []byte, pages []int, options ...PageOperationOptions) ([]byte, PageOperationReport, PageOperationVerification, error) {
	return pdf.ExtractPages(input, pages, options...)
}

func InsertPages(input []byte, index int, sources []PageSource, options ...PageOperationOptions) ([]byte, PageOperationReport, PageOperationVerification, error) {
	return pdf.InsertPages(input, index, sources, options...)
}

func Merge(inputs [][]byte, options ...PageOperationOptions) ([]byte, PageOperationReport, PageOperationVerification, error) {
	return pdf.Merge(inputs, options...)
}

func TransformPages(input []byte, selector PageSelector, transform PageTransform, opts PageWriteOptions) ([]byte, PageOperationReport, PageOperationVerification, error) {
	return pdf.TransformPages(input, selector, transform, opts)
}

func VerifyPageOperationOutput(output []byte, opts PageOperationVerificationOptions) (PageOperationVerification, error) {
	return pdf.VerifyPageOperationOutput(output, opts)
}

func InspectXFADynamic(input []byte) (XFADynamicReport, error) {
	return pdf.InspectXFADynamic(input)
}

func ParseWithPassword(input []byte, opts core.ParseOptions, password string) (*core.Tree, error) {
	return pdf.ParseWithPassword(input, opts, password)
}

func ParseWithSecurityOptions(input []byte, opts core.ParseOptions, security SecurityOptions) (*core.Tree, error) {
	return pdf.ParseWithSecurityOptions(input, opts, security)
}

func CheckSecurity(input []byte, opts SecurityOptions) error {
	return pdf.CheckSecurity(input, opts)
}

func SecurityMetadataForInput(input []byte) SecurityMetadata {
	return pdf.SecurityMetadataForInput(input)
}

func SecurityMetadataForInputWithOptions(input []byte, opts SecurityMetadataOptions) SecurityMetadata {
	return pdf.SecurityMetadataForInputWithOptions(input, opts)
}

func DecryptToPlain(input []byte, password string) ([]byte, error) {
	return pdf.DecryptToPlain(input, password)
}

func Encrypt(input []byte, opts EncryptOptions) ([]byte, error) {
	return pdf.Encrypt(input, opts)
}

func ChangePassword(input []byte, opts ChangePasswordOptions) ([]byte, error) {
	return pdf.ChangePassword(input, opts)
}

func PublicKeyEncrypt(input []byte, opts PublicKeyEncryptOptions) ([]byte, error) {
	return pdf.PublicKeyEncrypt(input, opts)
}

func ApplyIncrementalTextEditPreservingSignatures(input []byte, selector core.Match, mutation core.Mutation, invariants []core.Invariant) ([]byte, core.Report, core.Verification, SignaturePreservationVerification, error) {
	return pdf.ApplyIncrementalTextEditPreservingSignatures(input, selector, mutation, invariants)
}

func ValidateSignatureSigningPlan(options SignatureSigningPlanOptions) error {
	return pdf.ValidateSignatureSigningPlan(options)
}

func PlanIncrementalReSigning(input []byte, options SignatureSigningPlanOptions) (SignatureSigningPlan, error) {
	return pdf.PlanIncrementalReSigning(input, options)
}

func ApplyIncrementalReSigning(ctx context.Context, input []byte, options SignatureSigningPlanOptions) ([]byte, SignatureSigningPlan, SignatureReSigningVerification, error) {
	return pdf.ApplyIncrementalReSigning(ctx, input, options)
}

func ListFormFields(input []byte) ([]FormFieldMetadata, error) {
	return pdf.ListFormFields(input)
}

func ApplyFormFieldEdit(input []byte, fieldName, value string, matchIndexArg ...*int) ([]byte, FormFieldEditReport, FormFieldEditVerification, error) {
	return pdf.ApplyFormFieldEdit(input, fieldName, value, matchIndexArg...)
}

func ApplyFormFieldEditWithOptions(input []byte, fieldName, value string, options FormFieldEditOptions) ([]byte, FormFieldEditReport, FormFieldEditVerification, error) {
	return pdf.ApplyFormFieldEditWithOptions(input, fieldName, value, options)
}

func CreateFormField(input []byte, opts FormFieldCreateOptions) ([]byte, FormFieldMutationReport, FormFieldMutationVerification, error) {
	return pdf.CreateFormField(input, opts)
}

func RemoveFormField(input []byte, fieldName string, matchIndex ...*int) ([]byte, FormFieldMutationReport, FormFieldMutationVerification, error) {
	return pdf.RemoveFormField(input, fieldName, matchIndex...)
}

func FlattenFormFields(input []byte) ([]byte, FormFieldMutationReport, FormFieldMutationVerification, error) {
	return pdf.FlattenFormFields(input)
}

func ListAnnotationCandidates(input []byte) ([]AnnotationCandidateMetadata, error) {
	return pdf.ListAnnotationCandidates(input)
}

func ApplyAnnotationContentsEdit(input []byte, index int, contents string, options ...AnnotationContentsEditOptions) ([]byte, AnnotationContentsEditReport, AnnotationContentsEditVerification, error) {
	return pdf.ApplyAnnotationContentsEdit(input, index, contents, options...)
}

func ListXFAPackets(input []byte) ([]XFAPacketMetadata, error) {
	return pdf.ListXFAPackets(input)
}

func ListXFAPacketsWithOptions(input []byte, options XFAPacketListOptions) ([]XFAPacketMetadata, error) {
	return pdf.ListXFAPacketsWithOptions(input, options)
}

func ApplyXFAReplace(input []byte, oldText, newText string, matchIndexArg ...*int) ([]byte, core.Report, core.Verification, error) {
	return pdf.ApplyXFAReplace(input, oldText, newText, matchIndexArg...)
}

func ApplyXFAReplaceWithOptions(input []byte, oldText, newText string, options XFAReplaceOptions) ([]byte, core.Report, core.Verification, error) {
	return pdf.ApplyXFAReplaceWithOptions(input, oldText, newText, options)
}

func InspectXFASemantics(input []byte) (XFASemanticMetadata, error) {
	return pdf.InspectXFASemantics(input)
}

func ListXFADatasetFields(input []byte) ([]XFADatasetField, error) {
	return pdf.ListXFADatasetFields(input)
}

func ListXFADatasetFieldsWithOptions(input []byte, options XFADatasetFieldListOptions) ([]XFADatasetField, error) {
	return pdf.ListXFADatasetFieldsWithOptions(input, options)
}

func ListXFATemplateDatasetMappings(input []byte) ([]XFATemplateDatasetMapping, error) {
	return pdf.ListXFATemplateDatasetMappings(input)
}

func ApplyXFADatasetFieldUpdate(input []byte, path, value string) ([]byte, core.Report, core.Verification, error) {
	return pdf.ApplyXFADatasetFieldUpdate(input, path, value)
}

func ApplyXFADatasetFieldUpdateWithOptions(input []byte, path, value string, options XFADatasetFieldUpdateOptions) ([]byte, core.Report, core.Verification, error) {
	return pdf.ApplyXFADatasetFieldUpdateWithOptions(input, path, value, options)
}

func PlanExplicitOCRTextLayer(input []byte, opts OCRTextLayerOptions) (OCRTextLayerPlan, error) {
	return pdf.PlanExplicitOCRTextLayer(input, opts)
}

func ApplyExplicitOCRTextLayer(input []byte, opts OCRTextLayerOptions) ([]byte, core.Report, core.Verification, error) {
	return pdf.ApplyExplicitOCRTextLayer(input, opts)
}

func ParseOCRTextLayerALTOXML(input []byte) ([]OCRTextLayerOptions, error) {
	return pdf.ParseOCRTextLayerALTOXML(input)
}

func PlanExplicitOCRTextLayerALTOXML(pdfBytes, altoBytes []byte) ([]OCRTextLayerPlan, error) {
	return pdf.PlanExplicitOCRTextLayerALTOXML(pdfBytes, altoBytes)
}

func ParseOCRTextLayerJSON(input []byte) ([]OCRTextLayerOptions, error) {
	return pdf.ParseOCRTextLayerJSON(input)
}

func PlanExplicitOCRTextLayerJSON(pdfBytes, jsonBytes []byte) ([]OCRTextLayerPlan, error) {
	return pdf.PlanExplicitOCRTextLayerJSON(pdfBytes, jsonBytes)
}

func ApplyExplicitOverlayStamp(input []byte, opts ExplicitOverlayStampOptions) ([]byte, core.Report, core.Verification, error) {
	return pdf.ApplyExplicitOverlayStamp(input, opts)
}

func DefaultOverlayPolicy() OverlayPolicy {
	return pdf.DefaultOverlayPolicy()
}

func ValidateTrueTextEditFallbackPolicy(policy OverlayPolicy, invariants []core.Invariant) error {
	return pdf.ValidateTrueTextEditFallbackPolicy(policy, invariants)
}

func WithNoFallbackPolicy(report core.Report) core.Report {
	return pdf.WithNoFallbackPolicy(report)
}

func WithFallbackPolicy(report core.Report, policy OverlayPolicy) core.Report {
	return pdf.WithFallbackPolicy(report, policy)
}

func ValidateTrueTextEditReportFallbackPolicy(report core.Report) error {
	return pdf.ValidateTrueTextEditReportFallbackPolicy(report)
}

func MutateStream(input []byte, opts StreamMutationOptions) ([]byte, StreamMutationReport, StreamMutationVerification, error) {
	return pdf.MutateStream(input, opts)
}

func ReplaceImageXObject(input []byte, opts ReplaceImageXObjectOptions) ([]byte, StreamMutationReport, StreamMutationVerification, error) {
	return pdf.ReplaceImageXObject(input, opts)
}

func ReplaceInlineImage(input []byte, opts ReplaceInlineImageOptions) ([]byte, StreamMutationReport, StreamMutationVerification, error) {
	return pdf.ReplaceInlineImage(input, opts)
}
