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
