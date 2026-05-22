package pdf

import (
	"bytes"
	"crypto/sha256"
	"encoding/hex"
	"errors"
	"fmt"
	"strings"
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
	if info.ByteRangeStatus != signatureByteRangeStatusAbsent {
		t.Fatalf("ByteRangeStatus = %q, want %q", info.ByteRangeStatus, signatureByteRangeStatusAbsent)
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
	if info.ByteRangeStatus != signatureByteRangeStatusValid {
		t.Fatalf("ByteRangeStatus = %q, want %q", info.ByteRangeStatus, signatureByteRangeStatusValid)
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
	if info.ObjectNumber == nil || *info.ObjectNumber != 3 || info.ObjectGeneration == nil || *info.ObjectGeneration != 0 {
		t.Fatalf("signature object = %v %v, want 3 0", info.ObjectNumber, info.ObjectGeneration)
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
	if info.CryptographicValidation || info.CryptographicValidationStatus != signatureCryptographicValidationUnsupported {
		t.Fatalf("cryptographic validation = %t/%q, want false/%q", info.CryptographicValidation, info.CryptographicValidationStatus, signatureCryptographicValidationUnsupported)
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
	if info.CryptographicValidation || info.CryptographicValidationStatus != signatureCryptographicValidationUnsupported {
		t.Fatalf("cryptographic validation = %t/%q, want false/%q", info.CryptographicValidation, info.CryptographicValidationStatus, signatureCryptographicValidationUnsupported)
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
	if info.CryptographicValidation || info.CryptographicValidationStatus != signatureCryptographicValidationUnsupported {
		t.Fatalf("cryptographic validation = %t/%q, want false/%q", info.CryptographicValidation, info.CryptographicValidationStatus, signatureCryptographicValidationUnsupported)
	}
}

func TestSignatureInfoValidatesCMSDetachedMessageDigestAgainstByteRange(t *testing.T) {
	input := signedPDFWithCMSMessageDigest(t, nil)

	info := inspectSignatureInfo(input)

	if !info.CryptographicValidation {
		t.Fatalf("CryptographicValidation = false, want true")
	}
	if info.CryptographicValidationStatus != signatureCryptographicValidationByteRangeDigestValid {
		t.Fatalf("CryptographicValidationStatus = %q, want %q", info.CryptographicValidationStatus, signatureCryptographicValidationByteRangeDigestValid)
	}
	if info.SignatureContainer != "pkcs7" {
		t.Fatalf("SignatureContainer = %q, want pkcs7", info.SignatureContainer)
	}
	if info.DigestAlgorithm != "sha256" || info.DigestAlgorithmStatus != signatureDigestAlgorithmCMSAuthenticatedAttribute {
		t.Fatalf("digest algorithm = %q/%q, want sha256/%q", info.DigestAlgorithm, info.DigestAlgorithmStatus, signatureDigestAlgorithmCMSAuthenticatedAttribute)
	}
	if info.CertificateCount != 0 || info.SignerCertificateSubject != "" || info.SignerCertificateIssuer != "" {
		t.Fatalf("certificate metadata = count %d subject %q issuer %q, want no certificates", info.CertificateCount, info.SignerCertificateSubject, info.SignerCertificateIssuer)
	}
}

func TestSignatureInfoFailsClosedWhenCMSDetachedMessageDigestMismatchesByteRange(t *testing.T) {
	input := signedPDFWithCMSMessageDigest(t, func(digest []byte) []byte {
		out := bytes.Clone(digest)
		out[0] ^= 0xff
		return out
	})

	info := inspectSignatureInfo(input)

	if info.CryptographicValidation {
		t.Fatalf("CryptographicValidation = true, want false")
	}
	if info.CryptographicValidationStatus != signatureCryptographicValidationByteRangeDigestMismatch {
		t.Fatalf("CryptographicValidationStatus = %q, want %q", info.CryptographicValidationStatus, signatureCryptographicValidationByteRangeDigestMismatch)
	}
	if info.DigestAlgorithm != "sha256" || info.DigestAlgorithmStatus != signatureDigestAlgorithmCMSAuthenticatedAttribute {
		t.Fatalf("digest algorithm = %q/%q, want sha256/%q", info.DigestAlgorithm, info.DigestAlgorithmStatus, signatureDigestAlgorithmCMSAuthenticatedAttribute)
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

func signedPDFWithCMSMessageDigest(t *testing.T, mutateDigest func([]byte) []byte) []byte {
	t.Helper()

	zeroDigest := make([]byte, sha256.Size)
	placeholderCMS := append(minimalDetachedCMSWithMessageDigest(zeroDigest), make([]byte, 8)...)
	input, ranges := signedPDFWithContentsPlaceholder(t, len(placeholderCMS))
	digest := sha256DigestForRanges(input, ranges)
	if mutateDigest != nil {
		digest = mutateDigest(digest)
	}
	cms := append(minimalDetachedCMSWithMessageDigest(digest), make([]byte, 8)...)
	if len(cms) != len(placeholderCMS) {
		t.Fatalf("CMS length changed from %d to %d", len(placeholderCMS), len(cms))
	}
	return replaceSignatureContentsHex(t, input, cms)
}

func signedPDFWithContentsPlaceholder(t *testing.T, contentsLen int) ([]byte, []signatureByteRange) {
	t.Helper()

	placeholderHex := strings.Repeat("0", contentsLen*2)
	byteRangePlaceholder := "[0000000000 0000000000 0000000000 0000000000]"
	input := testPDF(
		"<< /Type /Catalog /SigFlags 3 /AcroForm << /Fields [2 0 R] >> >>",
		"<< /FT /Sig /T (Approval) /V 3 0 R >>",
		fmt.Sprintf("<< /Type /Sig /Filter /Adobe.PPKLite /SubFilter /adbe.pkcs7.detached /ByteRange %s /Contents <%s> >>", byteRangePlaceholder, placeholderHex),
	)
	contentsStart := bytes.Index(input, []byte("<"+placeholderHex+">"))
	if contentsStart < 0 {
		t.Fatal("signature contents placeholder not found")
	}
	contentsEnd := contentsStart + 1 + len(placeholderHex) + 1
	ranges := []signatureByteRange{
		{Offset: 0, Length: contentsStart},
		{Offset: contentsEnd, Length: len(input) - contentsEnd},
	}
	byteRange := fmt.Sprintf("[%010d %010d %010d %010d]", ranges[0].Offset, ranges[0].Length, ranges[1].Offset, ranges[1].Length)
	if len(byteRange) != len(byteRangePlaceholder) {
		t.Fatalf("ByteRange replacement length = %d, want %d", len(byteRange), len(byteRangePlaceholder))
	}
	input = bytes.Replace(input, []byte(byteRangePlaceholder), []byte(byteRange), 1)
	return input, ranges
}

func replaceSignatureContentsHex(t *testing.T, input []byte, contents []byte) []byte {
	t.Helper()

	placeholder := []byte("<" + strings.Repeat("0", len(contents)*2) + ">")
	replacement := []byte("<" + strings.ToUpper(hex.EncodeToString(contents)) + ">")
	if !bytes.Contains(input, placeholder) {
		t.Fatal("signature contents placeholder not found")
	}
	return bytes.Replace(input, placeholder, replacement, 1)
}

func sha256DigestForRanges(input []byte, ranges []signatureByteRange) []byte {
	h := sha256.New()
	for _, r := range ranges {
		h.Write(input[r.Offset : r.Offset+r.Length])
	}
	return h.Sum(nil)
}

func minimalDetachedCMSWithMessageDigest(digest []byte) []byte {
	messageDigestAttr := derSeq(
		derOID(0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x09, 0x04),
		derSet(derOctetString(digest)),
	)
	signerInfo := derSeq(
		derInteger(1),
		derSeq(derSeq(), derInteger(1)),
		derAlgorithmIdentifier(derOID(0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02, 0x01)),
		derConstructed(0, messageDigestAttr),
		derAlgorithmIdentifier(derOID(0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x01)),
		derOctetString([]byte{0}),
	)
	signedData := derSeq(
		derInteger(1),
		derSet(derAlgorithmIdentifier(derOID(0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02, 0x01))),
		derSeq(derOID(0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x07, 0x01)),
		derSet(signerInfo),
	)
	return derSeq(
		derOID(0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x07, 0x02),
		derConstructed(0, signedData),
	)
}

func derAlgorithmIdentifier(oid []byte) []byte {
	return derSeq(oid, []byte{0x05, 0x00})
}

func derSeq(parts ...[]byte) []byte {
	return derTLV(0x30, bytes.Join(parts, nil))
}

func derSet(parts ...[]byte) []byte {
	return derTLV(0x31, bytes.Join(parts, nil))
}

func derInteger(value byte) []byte {
	return derTLV(0x02, []byte{value})
}

func derOID(body ...byte) []byte {
	return derTLV(0x06, body)
}

func derOctetString(value []byte) []byte {
	return derTLV(0x04, value)
}

func derConstructed(tag byte, value []byte) []byte {
	return derTLV(0xa0+tag, value)
}

func derTLV(tag byte, value []byte) []byte {
	out := []byte{tag}
	if len(value) < 0x80 {
		out = append(out, byte(len(value)))
	} else if len(value) <= 0xff {
		out = append(out, 0x81, byte(len(value)))
	} else {
		out = append(out, 0x82, byte(len(value)>>8), byte(len(value)))
	}
	out = append(out, value...)
	return out
}

func TestSignatureInfoUnsupportedSubFiltersDoNotClaimValidation(t *testing.T) {
	tests := []struct {
		name             string
		subFilter        string
		wantContainer    string
		wantDigest       string
		wantDigestStatus string
	}{
		{
			name:             "document timestamp",
			subFilter:        "ETSI.RFC3161",
			wantContainer:    signatureContainerUnknown,
			wantDigest:       signatureDigestAlgorithmUnknown,
			wantDigestStatus: signatureDigestAlgorithmNotParsed,
		},
		{
			name:             "x509 rsa sha1",
			subFilter:        "adbe.x509.rsa_sha1",
			wantContainer:    signatureContainerUnknown,
			wantDigest:       "sha1",
			wantDigestStatus: signatureDigestAlgorithmSubFilterHint,
		},
		{
			name:             "vendor extension",
			subFilter:        "vendor.detached",
			wantContainer:    signatureContainerUnknown,
			wantDigest:       signatureDigestAlgorithmUnknown,
			wantDigestStatus: signatureDigestAlgorithmNotParsed,
		},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			info := inspectSignatureInfo(testPDF(
				"<< /Type /Catalog /SigFlags 3 >>",
				"<< /Type /Sig /Filter /Adobe.PPKLite /SubFilter /"+tt.subFilter+" /ByteRange [0 10 20 30] /Contents <01020f> >>",
			))

			if !info.HasSignatureMarker {
				t.Fatalf("HasSignatureMarker = false, want true")
			}
			if info.SubFilter != tt.subFilter {
				t.Fatalf("SubFilter = %q, want %q", info.SubFilter, tt.subFilter)
			}
			if info.SignatureContainer != tt.wantContainer {
				t.Fatalf("SignatureContainer = %q, want %q", info.SignatureContainer, tt.wantContainer)
			}
			if info.DigestAlgorithm != tt.wantDigest || info.DigestAlgorithmStatus != tt.wantDigestStatus {
				t.Fatalf("digest algorithm = %q/%q, want %q/%q", info.DigestAlgorithm, info.DigestAlgorithmStatus, tt.wantDigest, tt.wantDigestStatus)
			}
			if info.CryptographicValidation || info.CryptographicValidationStatus != signatureCryptographicValidationNotPerformed {
				t.Fatalf("cryptographic validation = %t/%q, want false/%q", info.CryptographicValidation, info.CryptographicValidationStatus, signatureCryptographicValidationNotPerformed)
			}
		})
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
	if info.ByteRangeStatus != signatureByteRangeStatusMalformed {
		t.Fatalf("ByteRangeStatus = %q, want %q", info.ByteRangeStatus, signatureByteRangeStatusMalformed)
	}
	if !errors.Is(info.MalformedByteRangeError, ErrSignedPDFByteRangeProofRequired) {
		t.Fatalf("MalformedByteRangeError = %v, want ErrSignedPDFByteRangeProofRequired", info.MalformedByteRangeError)
	}
	if info.CryptographicValidation || info.CryptographicValidationStatus != signatureCryptographicValidationNotPerformed {
		t.Fatalf("cryptographic validation = %t/%q, want false/%q", info.CryptographicValidation, info.CryptographicValidationStatus, signatureCryptographicValidationNotPerformed)
	}
}

func TestSignatureInfoMalformedByteRangeOrderingProof(t *testing.T) {
	tests := []struct {
		name   string
		object string
	}{
		{
			name:   "odd number of pairs",
			object: "<< /Type /Catalog /ByteRange [0 1 2 3 4 5] >>",
		},
		{
			name:   "zero length range",
			object: "<< /Type /Catalog /ByteRange [0 1 2 0] >>",
		},
		{
			name:   "overlapping ranges",
			object: "<< /Type /Catalog /ByteRange [0 10 5 10] >>",
		},
		{
			name:   "unsorted ranges",
			object: "<< /Type /Catalog /ByteRange [20 10 0 10] >>",
		},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			info := inspectSignatureInfo(testPDF(tt.object, "<<>>"))

			if !info.HasSignatureMarker {
				t.Fatalf("HasSignatureMarker = false, want true")
			}
			if info.ByteRangeCount != 0 || len(info.ByteRanges) != 0 {
				t.Fatalf("byte ranges = count %d values %+v, want none", info.ByteRangeCount, info.ByteRanges)
			}
			if !errors.Is(info.MalformedByteRangeError, ErrSignedPDFByteRangeProofRequired) {
				t.Fatalf("MalformedByteRangeError = %v, want ErrSignedPDFByteRangeProofRequired", info.MalformedByteRangeError)
			}
		})
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

func TestSignatureInfoMalformedSignatureDictionariesDoNotClaimValidation(t *testing.T) {
	tests := []struct {
		name    string
		objects []string
	}{
		{
			name: "bad contents hex",
			objects: []string{
				"<< /Type /Catalog /SigFlags 3 >>",
				"<< /Type /Sig /Filter /Adobe.PPKLite /SubFilter /ETSI.RFC3161 /ByteRange [0 10 20 30] /Contents <0Z> >>",
			},
		},
		{
			name: "malformed signature object",
			objects: []string{
				"<< /Type /Catalog /SigFlags 3 >>",
				"<< /Type /Sig /Filter /Adobe.PPKLite /SubFilter /ETSI.RFC3161 /ByteRange [0 10 20 30] /Contents <0102> ",
			},
		},
		{
			name: "wrong metadata value types",
			objects: []string{
				"<< /Type /Catalog /SigFlags 3 >>",
				"<< /Type /Sig /Filter 42 /SubFilter (ETSI.RFC3161) /M /Name /ByteRange [0 10 20 30] /Contents [1 2] >>",
			},
		},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			info := inspectSignatureInfo(testPDF(tt.objects...))

			if !info.HasSignatureMarker {
				t.Fatalf("HasSignatureMarker = false, want true")
			}
			if info.ContentsByteLength != nil {
				t.Fatalf("ContentsByteLength = %v, want absent", *info.ContentsByteLength)
			}
			if info.SigningTime != "" {
				t.Fatalf("SigningTime = %q, want absent", info.SigningTime)
			}
			if info.CryptographicValidation || info.CryptographicValidationStatus != signatureCryptographicValidationNotPerformed {
				t.Fatalf("cryptographic validation = %t/%q, want false/%q", info.CryptographicValidation, info.CryptographicValidationStatus, signatureCryptographicValidationNotPerformed)
			}
		})
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
