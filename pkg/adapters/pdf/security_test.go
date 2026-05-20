package pdf

import (
	"errors"
	"fmt"
	"strings"
	"testing"

	"github.com/oxhq/binas/pkg/core"
)

func TestSecurityEncryptUsesPasswordPathErrorContract(t *testing.T) {
	input := testPDF("<< /Type /Catalog /Encrypt 2 0 R >>", "<<>>")

	_, err := NewAdapter().Parse(input, core.ParseOptions{Strict: true})
	if !errors.Is(err, ErrEncryptedPDFPasswordRequired) {
		t.Fatalf("Parse() error = %v, want ErrEncryptedPDFPasswordRequired", err)
	}

	_, err = parsePDFGraphWithOptions(input, pdfGraphParseOptions{AllowEncryption: true})
	if !errors.Is(err, ErrEncryptedPDFPasswordRequired) {
		t.Fatalf("parsePDFGraphWithOptions(AllowEncryption) error = %v, want ErrEncryptedPDFPasswordRequired", err)
	}
}

func TestSecurityTrailerEncryptDetectedAfterBinaryStreamConfuser(t *testing.T) {
	input := []byte("%PDF-1.3\n1 0 obj\n<< /Length 1 >>\nstream\n(\nendstream\nendobj\n2 0 obj\n<<>>\nendobj\nxref\n0 3\n0000000000 65535 f \n0000000009 00000 n \n0000000060 00000 n \ntrailer\n<< /Size 3 /Root 1 0 R /Encrypt 2 0 R >>\nstartxref\n90\n%%EOF\n")

	_, err := NewAdapter().Parse(input, core.ParseOptions{Strict: true})
	if !errors.Is(err, ErrEncryptedPDFPasswordRequired) {
		t.Fatalf("Parse() error = %v, want ErrEncryptedPDFPasswordRequired", err)
	}
}

func TestSecurityPasswordOptionFailsUnsupportedEncryptionAlgorithmWithMetadata(t *testing.T) {
	input := testPDF(
		"<< /Type /Catalog /Encrypt 2 0 R >>",
		"<< /Filter /Standard /SubFilter /adbe.pkcs7.s5 /V 4 /R 4 /Length 128 /EncryptMetadata false >>",
	)

	err := CheckSecurity(input, SecurityOptions{Password: "secret"})
	if !errors.Is(err, ErrEncryptedPDFUnsupportedAlgorithm) {
		t.Fatalf("CheckSecurity() error = %v, want ErrEncryptedPDFUnsupportedAlgorithm", err)
	}
	var unsupported *UnsupportedEncryptionAlgorithmError
	if !errors.As(err, &unsupported) {
		t.Fatalf("CheckSecurity() error type = %T, want *UnsupportedEncryptionAlgorithmError", err)
	}
	metadata := unsupported.Encryption
	if !metadata.Present || metadata.Filter != "Standard" || metadata.SubFilter != "adbe.pkcs7.s5" {
		t.Fatalf("encryption metadata = %+v, want parsed filter/subfilter", metadata)
	}
	if metadata.V == nil || *metadata.V != 4 || metadata.R == nil || *metadata.R != 4 || metadata.Length == nil || *metadata.Length != 128 {
		t.Fatalf("encryption version metadata = %+v, want V=4 R=4 Length=128", metadata)
	}
	if metadata.EncryptMetadata == nil || *metadata.EncryptMetadata {
		t.Fatalf("EncryptMetadata = %v, want false pointer", metadata.EncryptMetadata)
	}
	if metadata.ObjectNumber == nil || *metadata.ObjectNumber != 2 || metadata.ObjectGeneration == nil || *metadata.ObjectGeneration != 0 {
		t.Fatalf("encryption object metadata = %+v, want 2 0", metadata)
	}
	if got := err.Error(); got == "" || !strings.Contains(got, "unsupported encryption algorithm/handler") || !strings.Contains(got, "Filter=Standard") {
		t.Fatalf("error = %q, want specific unsupported encryption error with metadata", got)
	}
}

func TestSecurityPasswordOptionAcceptsSupportedStandardRC4(t *testing.T) {
	input := standardEncryptedTextFixture(t, "08-15-2024")

	err := CheckSecurity(input, SecurityOptions{Password: "user"})
	if err != nil {
		t.Fatalf("CheckSecurity() error = %v, want supported encrypted PDF", err)
	}
}

func TestSecuritySignaturePreserveIncrementalAcceptsParseableByteRangeProof(t *testing.T) {
	input := signedTextPDF("08-15-2024")

	err := CheckSecurity(input, SecurityOptions{SignatureInvalidation: SignatureInvalidationPreserveIncremental})
	if err != nil {
		t.Fatalf("CheckSecurity() error = %v, want parseable ByteRange proof accepted", err)
	}
}

func TestSecurityMetadataIncludesDirectSignatureDictionaryMetadata(t *testing.T) {
	input := testPDF(
		"<< /Type /Catalog /SigFlags 3 /AcroForm << /Fields [2 0 R] >> >>",
		"<< /FT /Sig /T (Approval) /V 3 0 R >>",
		"<< /Type /Sig /Filter /Adobe.PPKLite /SubFilter /adbe.pkcs7.detached /M (D:20260518123456-07'00') /ByteRange [0 10 20 30] /Contents <01020f> >>",
	)

	metadata := SecurityMetadataForInput(input)
	signature := metadata.Signature
	if !metadata.Signed || !signature.Present {
		t.Fatalf("signature metadata = %+v, want present signed PDF", metadata)
	}
	if signature.ByteRangeCount != 2 {
		t.Fatalf("ByteRangeCount = %d, want 2", signature.ByteRangeCount)
	}
	if signature.ContentsByteLength == nil || *signature.ContentsByteLength != 3 {
		t.Fatalf("ContentsByteLength = %v, want 3", signature.ContentsByteLength)
	}
	if signature.Filter != "Adobe.PPKLite" || signature.SubFilter != "adbe.pkcs7.detached" || signature.SigningTime != "D:20260518123456-07'00'" {
		t.Fatalf("signature dictionary metadata = %+v, want parsed filter/subfilter/signing time", signature)
	}
	if signature.SignatureContainer != "pkcs7" || signature.DigestAlgorithm != signatureDigestAlgorithmUnknown || signature.DigestAlgorithmStatus != signatureDigestAlgorithmNotParsed {
		t.Fatalf("signature diagnostics = container %q digest %q/%q, want pkcs7/unknown/%q", signature.SignatureContainer, signature.DigestAlgorithm, signature.DigestAlgorithmStatus, signatureDigestAlgorithmNotParsed)
	}
	if signature.CryptographicValidation || signature.CryptographicValidationStatus != signatureCryptographicValidationNotPerformed {
		t.Fatalf("cryptographic validation = %t/%q, want false/%q", signature.CryptographicValidation, signature.CryptographicValidationStatus, signatureCryptographicValidationNotPerformed)
	}
}

func TestSecurityMetadataIncludesSignatureDigestDiagnostics(t *testing.T) {
	input := testPDF(
		"<< /Type /Catalog /SigFlags 3 >>",
		"<< /Type /Sig /ByteRange [0 10 20 30] /Contents <302006092a864886f70d010702060960864801650304020100000000000000000000> >>",
	)

	signature := SecurityMetadataForInput(input).Signature
	if signature.SignatureContainer != "pkcs7" {
		t.Fatalf("SignatureContainer = %q, want pkcs7", signature.SignatureContainer)
	}
	if signature.DigestAlgorithm != "sha256" || signature.DigestAlgorithmStatus != signatureDigestAlgorithmContentsOIDHint {
		t.Fatalf("digest algorithm = %q/%q, want sha256/%q", signature.DigestAlgorithm, signature.DigestAlgorithmStatus, signatureDigestAlgorithmContentsOIDHint)
	}
	if signature.CryptographicValidation || signature.CryptographicValidationStatus != signatureCryptographicValidationNotPerformed {
		t.Fatalf("cryptographic validation = %t/%q, want false/%q", signature.CryptographicValidation, signature.CryptographicValidationStatus, signatureCryptographicValidationNotPerformed)
	}
}

func TestSecuritySignaturePreserveIncrementalRequiresByteRangeProof(t *testing.T) {
	input := testPDF("<< /Type /Catalog /SigFlags 3 >>", "<<>>")

	err := CheckSecurity(input, SecurityOptions{SignatureInvalidation: SignatureInvalidationPreserveIncremental})
	if !errors.Is(err, ErrSignedPDFByteRangeProofRequired) {
		t.Fatalf("CheckSecurity() error = %v, want ErrSignedPDFByteRangeProofRequired", err)
	}
}

func TestSecuritySignaturePreserveIncrementalRejectsOutOfBoundsByteRange(t *testing.T) {
	input := testPDF("<< /Type /Catalog /ByteRange [0 1 999999 10] >>", "<<>>")

	err := CheckSecurity(input, SecurityOptions{SignatureInvalidation: SignatureInvalidationPreserveIncremental})
	if !errors.Is(err, ErrSignedPDFByteRangeProofRequired) {
		t.Fatalf("CheckSecurity() error = %v, want ErrSignedPDFByteRangeProofRequired", err)
	}
}

func TestSecuritySignatureDefaultsFailClosed(t *testing.T) {
	input := signedTextPDF("08-15-2024")

	_, err := NewAdapter().Parse(input, core.ParseOptions{Strict: true})
	if !errors.Is(err, ErrSignedPDFRequiresInvalidation) {
		t.Fatalf("Parse() error = %v, want ErrSignedPDFRequiresInvalidation", err)
	}

	_, _, _, err = ApplyCanonicalEdit(input, core.Match{Kind: KindTextShow, Text: "08-15-2024"}, core.Mutation{Replace: "May 5, 2026"}, nil)
	if !errors.Is(err, ErrSignedPDFRequiresInvalidation) {
		t.Fatalf("ApplyCanonicalEdit() error = %v, want ErrSignedPDFRequiresInvalidation", err)
	}
}

func TestSecuritySignatureByteRangeOnlyDefaultsFailClosed(t *testing.T) {
	input := testPDF("<< /Type /Catalog /ByteRange [0 10 20 30] >>", "<<>>")

	_, err := NewAdapter().Parse(input, core.ParseOptions{Strict: true})
	if !errors.Is(err, ErrSignedPDFRequiresInvalidation) {
		t.Fatalf("Parse() error = %v, want ErrSignedPDFRequiresInvalidation", err)
	}
}

func TestSecuritySignatureSigFlagsOnlyDefaultsFailClosed(t *testing.T) {
	input := testPDF("<< /Type /Catalog /SigFlags 3 >>", "<<>>")

	_, err := NewAdapter().Parse(input, core.ParseOptions{Strict: true})
	if !errors.Is(err, ErrSignedPDFRequiresInvalidation) {
		t.Fatalf("Parse() error = %v, want ErrSignedPDFRequiresInvalidation", err)
	}
}

func TestSecuritySignatureInvalidationHelperCanonicalWritesWhenExplicit(t *testing.T) {
	input := signedTextPDF("08-15-2024")

	output, report, verification, err := ApplyCanonicalEditInvalidatingSignatures(
		input,
		core.Match{Kind: KindTextShow, Text: "08-15-2024"},
		core.Mutation{Replace: "May 5, 2026"},
		nil,
	)
	if err != nil {
		t.Fatal(err)
	}
	if report.Edit != "pdf.canonical_content_stream_text_rewrite" {
		t.Fatalf("report edit = %q", report.Edit)
	}
	if !verification.ReparseOK || !verification.OldTextRemoved || !verification.NewSelectable || !verification.PageUnchanged {
		t.Fatalf("verification = %+v", verification)
	}

	_, err = NewAdapter().Parse(output, core.ParseOptions{Strict: true})
	if !errors.Is(err, ErrSignedPDFRequiresInvalidation) {
		t.Fatalf("default Parse(output) error = %v, want signed-PDF refusal", err)
	}
}

func signedTextPDF(text string) []byte {
	content := []byte(fmt.Sprintf("BT\n(%s) Tj\nET\n", encodeLiteralString(text)))
	return testPDF(
		"<< /Type /Catalog /Pages 2 0 R /SigFlags 3 /AcroForm << /Fields [5 0 R] >> >>",
		"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
		"<< /Type /Page /Parent 2 0 R /Contents 4 0 R >>",
		fmt.Sprintf("<< /Length %d >>\nstream\n%sendstream", len(content), content),
		"<< /FT /Sig /T (Approval) /V 6 0 R >>",
		"<< /Type /Sig /ByteRange [0 10 20 30] /Contents <00> >>",
	)
}
