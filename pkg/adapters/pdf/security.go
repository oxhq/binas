package pdf

import "errors"

var (
	ErrEncryptedPDFPasswordRequired  = errors.New("unsupported PDF: encrypted PDFs require an explicit password-capable path; Adapter.Parse does not decrypt")
	ErrSignedPDFRequiresInvalidation = errors.New("unsupported PDF: digital signatures require explicit invalidation mode; Adapter.Parse refuses silent signature invalidation")
)

type SignatureInvalidationMode string

const (
	SignatureInvalidationRefuse     SignatureInvalidationMode = "refuse"
	SignatureInvalidationInvalidate SignatureInvalidationMode = "invalidate"
)

type SecurityOptions struct {
	SignatureInvalidation SignatureInvalidationMode
}

func defaultSecurityOptions() SecurityOptions {
	return SecurityOptions{SignatureInvalidation: SignatureInvalidationRefuse}
}

func (o SecurityOptions) allowsSignatureInvalidation() bool {
	return o.SignatureInvalidation == SignatureInvalidationInvalidate
}

func rejectUnsupportedSecurityBoundaries(boundaries residualBoundarySummary) error {
	return rejectUnsupportedSecurityBoundariesWithOptions(boundaries, defaultSecurityOptions())
}

func rejectUnsupportedSecurityBoundariesWithOptions(boundaries residualBoundarySummary, opts SecurityOptions) error {
	if boundaries.HasEncryption {
		return ErrEncryptedPDFPasswordRequired
	}
	if boundaries.HasSignature && !opts.allowsSignatureInvalidation() {
		return ErrSignedPDFRequiresInvalidation
	}
	return nil
}
