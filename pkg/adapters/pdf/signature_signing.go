package pdf

import (
	"bytes"
	"context"
	"encoding/hex"
	"errors"
	"fmt"
	"strings"
)

var (
	ErrSignatureSigningCallbackRequired        = errors.New("unsupported PDF: incremental re-signing requires an external signature signing callback")
	ErrSignatureSigningCallbackMetadataInvalid = errors.New("unsupported PDF: invalid signature signing callback metadata")
)

type SignatureByteRange struct {
	Offset int `json:"offset"`
	Length int `json:"length"`
}

type SignatureSigningRequest struct {
	Digest             []byte               `json:"digest,omitempty"`
	DigestAlgorithm    string               `json:"digest_algorithm"`
	SignatureContainer string               `json:"signature_container"`
	SubFilter          string               `json:"sub_filter,omitempty"`
	ByteRanges         []SignatureByteRange `json:"byte_ranges,omitempty"`
	Signature          SignatureMetadata    `json:"signature"`
}

type SignatureSigningResponse struct {
	Signature          []byte   `json:"signature,omitempty"`
	SignatureContainer string   `json:"signature_container,omitempty"`
	CertificateChain   [][]byte `json:"certificate_chain,omitempty"`
}

type SignatureSigningCallback func(context.Context, SignatureSigningRequest) (SignatureSigningResponse, error)

type SignatureSigningCallbackMetadata struct {
	Name               string `json:"name"`
	ExternalKeyID      string `json:"external_key_id"`
	DigestAlgorithm    string `json:"digest_algorithm"`
	SignatureContainer string `json:"signature_container"`
	SubFilter          string `json:"sub_filter,omitempty"`
}

type SignatureSigningPlanOptions struct {
	Callback         SignatureSigningCallback         `json:"-"`
	CallbackMetadata SignatureSigningCallbackMetadata `json:"callback_metadata"`
	ReservedBytes    int                              `json:"reserved_bytes,omitempty"`
}

type SignatureSigningPlan struct {
	Supported         bool                             `json:"supported"`
	UnsupportedReason string                           `json:"unsupported_reason,omitempty"`
	CallbackMetadata  SignatureSigningCallbackMetadata `json:"callback_metadata,omitempty"`
	Signature         SignatureMetadata                `json:"signature"`
	ByteRanges        []SignatureByteRange             `json:"byte_ranges,omitempty"`
}

type SignatureReSigningVerification struct {
	IncrementalUpdate                  bool                 `json:"incremental_update"`
	ReparseOK                          bool                 `json:"reparse_ok"`
	ByteRanges                         []SignatureByteRange `json:"byte_ranges,omitempty"`
	ByteRangeDigestValidation          bool                 `json:"byte_range_digest_validation"`
	ByteRangeDigestValidationStatus    string               `json:"byte_range_digest_validation_status"`
	CertificateTrustValidation         bool                 `json:"certificate_trust_validation"`
	CertificateTrustValidationStatus   string               `json:"certificate_trust_validation_status"`
	CryptographicSignatureVerification bool                 `json:"cryptographic_signature_verification"`
	CryptographicSignatureStatus       string               `json:"cryptographic_signature_status"`
}

func ValidateSignatureSigningPlan(options SignatureSigningPlanOptions) error {
	_, err := validateSignatureSigningPlanOptions(options)
	return err
}

func PlanIncrementalReSigning(input []byte, options SignatureSigningPlanOptions) (SignatureSigningPlan, error) {
	metadata, err := validateSignatureSigningPlanOptions(options)
	if err != nil {
		return SignatureSigningPlan{
			Supported:         false,
			UnsupportedReason: err.Error(),
		}, err
	}

	info := inspectSignatureInfo(input)
	plan := SignatureSigningPlan{
		Supported:        false,
		CallbackMetadata: metadata,
		Signature:        signatureMetadataFromInfo(info),
	}
	ranges, err := signatureByteRanges(input)
	if err != nil {
		plan.UnsupportedReason = err.Error()
		return plan, err
	}

	plan.Supported = true
	plan.ByteRanges = publicSignatureByteRanges(ranges)
	return plan, nil
}

func ApplyIncrementalReSigning(ctx context.Context, input []byte, options SignatureSigningPlanOptions) ([]byte, SignatureSigningPlan, SignatureReSigningVerification, error) {
	metadata, err := validateSignatureSigningPlanOptions(options)
	if err != nil {
		return nil, SignatureSigningPlan{Supported: false, UnsupportedReason: err.Error()}, SignatureReSigningVerification{}, err
	}
	if err := validateIncrementalTextReplacementInput(input); err != nil {
		return nil, SignatureSigningPlan{Supported: false, UnsupportedReason: err.Error(), CallbackMetadata: metadata}, SignatureReSigningVerification{}, err
	}
	signatureDict, err := firstIncrementallyReSignableSignatureDictionary(input)
	if err != nil {
		return nil, SignatureSigningPlan{Supported: false, UnsupportedReason: err.Error(), CallbackMetadata: metadata}, SignatureReSigningVerification{}, err
	}
	reservedBytes := signatureReservedBytes(options.ReservedBytes)
	byteRangePlaceholder := "[0000000000 0000000000 0000000000 0000000000]"
	contentsPlaceholder := "<" + strings.Repeat("0", reservedBytes*2) + ">"
	draftObject := incrementalSignatureDictionaryBytes(signatureDict.Dict, metadata, byteRangePlaceholder, contentsPlaceholder)
	draft, err := appendIncrementalUpdate(input, []incrementalObjectUpdate{{
		ID:    *signatureDict.ObjectID,
		Value: pdfRawObject(draftObject),
	}}, nil)
	if err != nil {
		return nil, SignatureSigningPlan{Supported: false, UnsupportedReason: err.Error(), CallbackMetadata: metadata}, SignatureReSigningVerification{}, err
	}
	contentsStart := bytes.LastIndex(draft, []byte(contentsPlaceholder))
	if contentsStart < 0 {
		return nil, SignatureSigningPlan{Supported: false, UnsupportedReason: "signature contents placeholder not found", CallbackMetadata: metadata}, SignatureReSigningVerification{}, errors.New("signature contents placeholder not found")
	}
	contentsEnd := contentsStart + len(contentsPlaceholder)
	ranges := []signatureByteRange{
		{Offset: 0, Length: contentsStart},
		{Offset: contentsEnd, Length: len(draft) - contentsEnd},
	}
	byteRange := fmt.Sprintf("[%010d %010d %010d %010d]", ranges[0].Offset, ranges[0].Length, ranges[1].Offset, ranges[1].Length)
	if len(byteRange) != len(byteRangePlaceholder) {
		return nil, SignatureSigningPlan{Supported: false, UnsupportedReason: "signature ByteRange placeholder overflow", CallbackMetadata: metadata}, SignatureReSigningVerification{}, errors.New("signature ByteRange placeholder overflow")
	}
	draft = bytes.Replace(draft, []byte(byteRangePlaceholder), []byte(byteRange), 1)
	digest, ok := digestByteRanges(draft, ranges, metadata.DigestAlgorithm)
	if !ok {
		err := fmt.Errorf("unsupported digest algorithm %q", metadata.DigestAlgorithm)
		return nil, SignatureSigningPlan{Supported: false, UnsupportedReason: err.Error(), CallbackMetadata: metadata}, SignatureReSigningVerification{}, err
	}
	publicRanges := publicSignatureByteRanges(ranges)
	plan := SignatureSigningPlan{
		Supported:        true,
		CallbackMetadata: metadata,
		Signature:        signatureMetadataFromInfo(inspectSignatureInfo(draft)),
		ByteRanges:       publicRanges,
	}
	response, err := options.Callback(ctx, SignatureSigningRequest{
		Digest:             digest,
		DigestAlgorithm:    metadata.DigestAlgorithm,
		SignatureContainer: metadata.SignatureContainer,
		SubFilter:          metadata.SubFilter,
		ByteRanges:         publicRanges,
		Signature:          plan.Signature,
	})
	if err != nil {
		return nil, plan, SignatureReSigningVerification{}, err
	}
	if len(response.Signature) == 0 {
		return nil, plan, SignatureReSigningVerification{}, errors.New("signature signing callback returned an empty signature")
	}
	if len(response.Signature) > reservedBytes {
		return nil, plan, SignatureReSigningVerification{}, fmt.Errorf("signature signing callback returned %d bytes, exceeding reserved /Contents size %d", len(response.Signature), reservedBytes)
	}
	signatureHex := strings.ToUpper(hex.EncodeToString(response.Signature)) + strings.Repeat("0", (reservedBytes-len(response.Signature))*2)
	output := bytes.Replace(draft, []byte(contentsPlaceholder), []byte("<"+signatureHex+">"), 1)
	verification, err := verifyIncrementalReSigningOutput(output, publicRanges)
	if err != nil {
		return nil, plan, verification, err
	}
	return output, plan, verification, nil
}

func validateSignatureSigningPlanOptions(options SignatureSigningPlanOptions) (SignatureSigningCallbackMetadata, error) {
	if options.Callback == nil {
		return SignatureSigningCallbackMetadata{}, ErrSignatureSigningCallbackRequired
	}
	return normalizeSignatureSigningCallbackMetadata(options.CallbackMetadata)
}

func normalizeSignatureSigningCallbackMetadata(metadata SignatureSigningCallbackMetadata) (SignatureSigningCallbackMetadata, error) {
	metadata.Name = strings.TrimSpace(metadata.Name)
	metadata.ExternalKeyID = strings.TrimSpace(metadata.ExternalKeyID)
	metadata.DigestAlgorithm = strings.ToLower(strings.TrimSpace(metadata.DigestAlgorithm))
	metadata.SignatureContainer = strings.ToLower(strings.TrimSpace(metadata.SignatureContainer))
	metadata.SubFilter = strings.TrimSpace(metadata.SubFilter)

	if metadata.Name == "" {
		return metadata, invalidSignatureSigningCallbackMetadata("name is required")
	}
	if metadata.ExternalKeyID == "" {
		return metadata, invalidSignatureSigningCallbackMetadata("external key id is required; pass an opaque key handle instead of private key material")
	}
	if looksLikePrivateKeyMaterial(metadata.ExternalKeyID) {
		return metadata, invalidSignatureSigningCallbackMetadata("private key material must not be passed in callback metadata; pass an external key id")
	}
	if _, ok := signatureHash(metadata.DigestAlgorithm); !ok {
		return metadata, invalidSignatureSigningCallbackMetadata("unsupported digest algorithm %q", metadata.DigestAlgorithm)
	}
	switch metadata.SignatureContainer {
	case "pkcs7", "cades":
	default:
		return metadata, invalidSignatureSigningCallbackMetadata("unsupported signature container %q", metadata.SignatureContainer)
	}
	if metadata.SubFilter != "" {
		container := signatureContainerFromSubFilter(metadata.SubFilter)
		if container == signatureContainerUnknown {
			return metadata, invalidSignatureSigningCallbackMetadata("unsupported signature subfilter %q", metadata.SubFilter)
		}
		if container != metadata.SignatureContainer {
			return metadata, invalidSignatureSigningCallbackMetadata("signature subfilter %q does not match container %q", metadata.SubFilter, metadata.SignatureContainer)
		}
	}
	return metadata, nil
}

func firstIncrementallyReSignableSignatureDictionary(input []byte) (signatureDictionaryInfo, error) {
	for _, signatureDict := range signatureDictionariesForInput(input) {
		if signatureDict.ObjectID != nil {
			return signatureDict, nil
		}
	}
	return signatureDictionaryInfo{}, errors.New("unsupported PDF: incremental re-signing requires an indirect signature dictionary")
}

func signatureReservedBytes(value int) int {
	if value > 0 {
		return value
	}
	return 8192
}

func incrementalSignatureDictionaryBytes(existing pdfDict, metadata SignatureSigningCallbackMetadata, byteRange, contents string) []byte {
	filter := "Adobe.PPKLite"
	if existingFilter, ok := dictPDFName(existing, "Filter"); ok && existingFilter != "" {
		filter = existingFilter
	}
	subFilter := metadata.SubFilter
	if subFilter == "" {
		if existingSubFilter, ok := dictPDFName(existing, "SubFilter"); ok {
			subFilter = existingSubFilter
		}
	}
	if subFilter == "" {
		subFilter = "adbe.pkcs7.detached"
	}
	return []byte(fmt.Sprintf("<< /Type /Sig /Filter /%s /SubFilter /%s /ByteRange %s /Contents %s >>", filter, subFilter, byteRange, contents))
}

func verifyIncrementalReSigningOutput(output []byte, ranges []SignatureByteRange) (SignatureReSigningVerification, error) {
	info := inspectSignatureInfo(output)
	verification := SignatureReSigningVerification{
		IncrementalUpdate:                  true,
		ReparseOK:                          false,
		ByteRanges:                         ranges,
		ByteRangeDigestValidation:          info.ByteRangeDigestValidation,
		ByteRangeDigestValidationStatus:    info.ByteRangeDigestValidationStatus,
		CertificateTrustValidation:         info.CertificateTrustValidation,
		CertificateTrustValidationStatus:   info.CertificateTrustValidationStatus,
		CryptographicSignatureVerification: false,
		CryptographicSignatureStatus:       "not_performed",
	}
	if _, err := parsePDFGraphWithOptions(output, pdfGraphParseOptions{AllowSignature: true}); err != nil {
		return verification, err
	}
	verification.ReparseOK = true
	if !info.ByteRangeDigestValidation || info.ByteRangeDigestValidationStatus != signatureByteRangeDigestValidationValid {
		return verification, fmt.Errorf("incremental re-signing byte-range digest validation status = %q", info.ByteRangeDigestValidationStatus)
	}
	return verification, nil
}

func invalidSignatureSigningCallbackMetadata(format string, args ...any) error {
	return fmt.Errorf("%w: %s", ErrSignatureSigningCallbackMetadataInvalid, fmt.Sprintf(format, args...))
}

func looksLikePrivateKeyMaterial(value string) bool {
	upper := strings.ToUpper(value)
	return strings.Contains(upper, "PRIVATE KEY") || strings.Contains(upper, "-----BEGIN")
}

func publicSignatureByteRanges(ranges []signatureByteRange) []SignatureByteRange {
	out := make([]SignatureByteRange, 0, len(ranges))
	for _, r := range ranges {
		out = append(out, SignatureByteRange{Offset: r.Offset, Length: r.Length})
	}
	return out
}
