package pdf

import (
	"errors"
	"testing"
)

func TestSignatureInfoNoSignature(t *testing.T) {
	info := inspectSignatureInfo(testPDF("<< /Type /Catalog >>", "<<>>"))

	if info.HasSignatureMarker {
		t.Fatalf("HasSignatureMarker = true, want false")
	}
	if info.ByteRangeCount != 0 || len(info.ByteRanges) != 0 {
		t.Fatalf("byte ranges = count %d values %+v, want none", info.ByteRangeCount, info.ByteRanges)
	}
	if info.MalformedByteRangeError != nil {
		t.Fatalf("MalformedByteRangeError = %v, want nil", info.MalformedByteRangeError)
	}
	if info.CryptographicValidation || info.CryptographicValidationStatus != signatureCryptographicValidationNotPerformed {
		t.Fatalf("cryptographic validation = %t/%q, want false/%q", info.CryptographicValidation, info.CryptographicValidationStatus, signatureCryptographicValidationNotPerformed)
	}
	if info.SignatureContainer != signatureContainerUnknown || info.DigestAlgorithm != signatureDigestAlgorithmUnknown || info.DigestAlgorithmStatus != signatureDigestAlgorithmNotParsed {
		t.Fatalf("signature diagnostics = container %q digest %q/%q, want unknown/unknown/%q", info.SignatureContainer, info.DigestAlgorithm, info.DigestAlgorithmStatus, signatureDigestAlgorithmNotParsed)
	}
}

func TestSignatureInfoValidByteRange(t *testing.T) {
	info := inspectSignatureInfo(signedTextPDF("08-15-2024"))

	if !info.HasSignatureMarker {
		t.Fatalf("HasSignatureMarker = false, want true")
	}
	if info.ByteRangeCount != 2 {
		t.Fatalf("ByteRangeCount = %d, want 2", info.ByteRangeCount)
	}
	if len(info.ByteRanges) != 2 {
		t.Fatalf("ByteRanges length = %d, want 2", len(info.ByteRanges))
	}
	if got := info.ByteRanges[0]; got.Offset != 0 || got.Length != 10 {
		t.Fatalf("first ByteRange = %+v, want offset 0 length 10", got)
	}
	if got := info.ByteRanges[1]; got.Offset != 20 || got.Length != 30 {
		t.Fatalf("second ByteRange = %+v, want offset 20 length 30", got)
	}
	if info.MalformedByteRangeError != nil {
		t.Fatalf("MalformedByteRangeError = %v, want nil", info.MalformedByteRangeError)
	}
	if info.CryptographicValidation || info.CryptographicValidationStatus != signatureCryptographicValidationNotPerformed {
		t.Fatalf("cryptographic validation = %t/%q, want false/%q", info.CryptographicValidation, info.CryptographicValidationStatus, signatureCryptographicValidationNotPerformed)
	}
	if info.SignatureContainer != signatureContainerUnknown || info.DigestAlgorithm != signatureDigestAlgorithmUnknown || info.DigestAlgorithmStatus != signatureDigestAlgorithmNotParsed {
		t.Fatalf("signature diagnostics = container %q digest %q/%q, want unknown/unknown/%q", info.SignatureContainer, info.DigestAlgorithm, info.DigestAlgorithmStatus, signatureDigestAlgorithmNotParsed)
	}
}

func TestSignatureInfoDirectSignatureDictionaryMetadata(t *testing.T) {
	info := inspectSignatureInfo(testPDF(
		"<< /Type /Catalog /SigFlags 3 /AcroForm << /Fields [2 0 R] >> >>",
		"<< /FT /Sig /T (Approval) /V 3 0 R >>",
		"<< /Type /Sig /Filter /Adobe.PPKLite /SubFilter /adbe.pkcs7.detached /M (D:20260518123456-07'00') /ByteRange [0 10 20 30] /Contents <01020f> >>",
	))

	if !info.HasSignatureMarker {
		t.Fatalf("HasSignatureMarker = false, want true")
	}
	if info.ContentsByteLength == nil || *info.ContentsByteLength != 3 {
		t.Fatalf("ContentsByteLength = %v, want 3", info.ContentsByteLength)
	}
	if info.SubFilter != "adbe.pkcs7.detached" {
		t.Fatalf("SubFilter = %q, want adbe.pkcs7.detached", info.SubFilter)
	}
	if info.Filter != "Adobe.PPKLite" {
		t.Fatalf("Filter = %q, want Adobe.PPKLite", info.Filter)
	}
	if info.SigningTime != "D:20260518123456-07'00'" {
		t.Fatalf("SigningTime = %q, want PDF signing date", info.SigningTime)
	}
	if info.SignatureContainer != "pkcs7" {
		t.Fatalf("SignatureContainer = %q, want pkcs7", info.SignatureContainer)
	}
	if info.DigestAlgorithm != signatureDigestAlgorithmUnknown || info.DigestAlgorithmStatus != signatureDigestAlgorithmNotParsed {
		t.Fatalf("digest algorithm = %q/%q, want unknown/%q", info.DigestAlgorithm, info.DigestAlgorithmStatus, signatureDigestAlgorithmNotParsed)
	}
	if info.CryptographicValidation || info.CryptographicValidationStatus != signatureCryptographicValidationNotPerformed {
		t.Fatalf("cryptographic validation = %t/%q, want false/%q", info.CryptographicValidation, info.CryptographicValidationStatus, signatureCryptographicValidationNotPerformed)
	}
}

func TestSignatureInfoSubFilterDigestAlgorithmHint(t *testing.T) {
	info := inspectSignatureInfo(testPDF(
		"<< /Type /Catalog /SigFlags 3 >>",
		"<< /Type /Sig /SubFilter /adbe.pkcs7.sha1 /ByteRange [0 10 20 30] /Contents <01020f> >>",
	))

	if info.SignatureContainer != "pkcs7" {
		t.Fatalf("SignatureContainer = %q, want pkcs7", info.SignatureContainer)
	}
	if info.DigestAlgorithm != "sha1" || info.DigestAlgorithmStatus != signatureDigestAlgorithmSubFilterHint {
		t.Fatalf("digest algorithm = %q/%q, want sha1/%q", info.DigestAlgorithm, info.DigestAlgorithmStatus, signatureDigestAlgorithmSubFilterHint)
	}
	if info.CryptographicValidation || info.CryptographicValidationStatus != signatureCryptographicValidationNotPerformed {
		t.Fatalf("cryptographic validation = %t/%q, want false/%q", info.CryptographicValidation, info.CryptographicValidationStatus, signatureCryptographicValidationNotPerformed)
	}
}

func TestSignatureInfoContentsEnvelopeDigestAlgorithmHint(t *testing.T) {
	info := inspectSignatureInfo(testPDF(
		"<< /Type /Catalog /SigFlags 3 >>",
		"<< /Type /Sig /ByteRange [0 10 20 30] /Contents <302006092a864886f70d010702060960864801650304020100000000000000000000> >>",
	))

	if info.SignatureContainer != "pkcs7" {
		t.Fatalf("SignatureContainer = %q, want pkcs7", info.SignatureContainer)
	}
	if info.DigestAlgorithm != "sha256" || info.DigestAlgorithmStatus != signatureDigestAlgorithmContentsOIDHint {
		t.Fatalf("digest algorithm = %q/%q, want sha256/%q", info.DigestAlgorithm, info.DigestAlgorithmStatus, signatureDigestAlgorithmContentsOIDHint)
	}
	if info.CryptographicValidation || info.CryptographicValidationStatus != signatureCryptographicValidationNotPerformed {
		t.Fatalf("cryptographic validation = %t/%q, want false/%q", info.CryptographicValidation, info.CryptographicValidationStatus, signatureCryptographicValidationNotPerformed)
	}
}

func TestSignatureInfoCAdESSubFilterContainerHint(t *testing.T) {
	info := inspectSignatureInfo(testPDF(
		"<< /Type /Catalog /SigFlags 3 >>",
		"<< /Type /Sig /SubFilter /ETSI.CAdES.detached /ByteRange [0 10 20 30] /Contents <01020f> >>",
	))

	if info.SignatureContainer != "cades" {
		t.Fatalf("SignatureContainer = %q, want cades", info.SignatureContainer)
	}
	if info.DigestAlgorithm != signatureDigestAlgorithmUnknown || info.DigestAlgorithmStatus != signatureDigestAlgorithmNotParsed {
		t.Fatalf("digest algorithm = %q/%q, want unknown/%q", info.DigestAlgorithm, info.DigestAlgorithmStatus, signatureDigestAlgorithmNotParsed)
	}
}

func TestSignatureInfoMalformedByteRange(t *testing.T) {
	info := inspectSignatureInfo(testPDF("<< /Type /Catalog /ByteRange [0 1 999999 10] >>", "<<>>"))

	if !info.HasSignatureMarker {
		t.Fatalf("HasSignatureMarker = false, want true")
	}
	if info.ByteRangeCount != 0 || len(info.ByteRanges) != 0 {
		t.Fatalf("byte ranges = count %d values %+v, want none", info.ByteRangeCount, info.ByteRanges)
	}
	if !errors.Is(info.MalformedByteRangeError, ErrSignedPDFByteRangeProofRequired) {
		t.Fatalf("MalformedByteRangeError = %v, want ErrSignedPDFByteRangeProofRequired", info.MalformedByteRangeError)
	}
	if info.CryptographicValidation || info.CryptographicValidationStatus != signatureCryptographicValidationNotPerformed {
		t.Fatalf("cryptographic validation = %t/%q, want false/%q", info.CryptographicValidation, info.CryptographicValidationStatus, signatureCryptographicValidationNotPerformed)
	}
}

func TestSignatureInfoMalformedDirectDictionaryMetadataIsAbsent(t *testing.T) {
	info := inspectSignatureInfo(testPDF(
		"<< /Type /Catalog /SigFlags 3 >>",
		"<< /Type /Sig /Filter 42 /SubFilter (not-a-name) /M /Name /Contents <0Z> >>",
	))

	if !info.HasSignatureMarker {
		t.Fatalf("HasSignatureMarker = false, want true")
	}
	if info.ContentsByteLength != nil {
		t.Fatalf("ContentsByteLength = %v, want absent", *info.ContentsByteLength)
	}
	if info.Filter != "" || info.SubFilter != "" || info.SigningTime != "" {
		t.Fatalf("metadata = filter %q subfilter %q signing time %q, want absent", info.Filter, info.SubFilter, info.SigningTime)
	}
	if info.SignatureContainer != signatureContainerUnknown || info.DigestAlgorithm != signatureDigestAlgorithmUnknown || info.DigestAlgorithmStatus != signatureDigestAlgorithmNotParsed {
		t.Fatalf("signature diagnostics = container %q digest %q/%q, want unknown/unknown/%q", info.SignatureContainer, info.DigestAlgorithm, info.DigestAlgorithmStatus, signatureDigestAlgorithmNotParsed)
	}
}

func TestSignatureInfoIgnoresMarkersInsideLiteralStringsCommentsAndHexStrings(t *testing.T) {
	info := inspectSignatureInfo(testPDF(
		"<< /Type /Catalog % /Type /Sig /ByteRange [0 10 20 30]\n/Contents <2F54797065202F536967202F4279746552616E6765> /Subject (/SubFilter /Filter /M /Contents /ByteRange) >>",
		"<<>>",
	))

	if info.HasSignatureMarker {
		t.Fatalf("HasSignatureMarker = true, want false")
	}
	if info.ByteRangeCount != 0 || len(info.ByteRanges) != 0 || info.ContentsByteLength != nil || info.SubFilter != "" || info.Filter != "" || info.SigningTime != "" {
		t.Fatalf("signature metadata = %+v, want absent", info)
	}
	if info.SignatureContainer != signatureContainerUnknown || info.DigestAlgorithm != signatureDigestAlgorithmUnknown || info.DigestAlgorithmStatus != signatureDigestAlgorithmNotParsed {
		t.Fatalf("signature diagnostics = container %q digest %q/%q, want unknown/unknown/%q", info.SignatureContainer, info.DigestAlgorithm, info.DigestAlgorithmStatus, signatureDigestAlgorithmNotParsed)
	}
}
