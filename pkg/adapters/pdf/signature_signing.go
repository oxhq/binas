package pdf

import (
	"context"
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
}

type SignatureSigningPlan struct {
	Supported         bool                             `json:"supported"`
	UnsupportedReason string                           `json:"unsupported_reason,omitempty"`
	CallbackMetadata  SignatureSigningCallbackMetadata `json:"callback_metadata,omitempty"`
	Signature         SignatureMetadata                `json:"signature"`
	ByteRanges        []SignatureByteRange             `json:"byte_ranges,omitempty"`
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
