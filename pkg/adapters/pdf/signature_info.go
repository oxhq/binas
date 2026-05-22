package pdf

import (
	"bytes"
	"strings"
)

const signatureCryptographicValidationNotPerformed = "not_performed"
const signatureContainerUnknown = "unknown"
const signatureDigestAlgorithmUnknown = "unknown"
const signatureDigestAlgorithmNotParsed = "not_parsed"
const signatureDigestAlgorithmSubFilterHint = "sub_filter_hint"
const signatureDigestAlgorithmContentsOIDHint = "contents_oid_hint"
const signatureByteRangeStatusAbsent = "absent"
const signatureByteRangeStatusValid = "valid"
const signatureByteRangeStatusMalformed = "malformed"

type signatureInfo struct {
	HasSignatureMarker            bool
	ByteRanges                    []signatureByteRange
	ByteRangeCount                int
	ByteRangeStatus               string
	ContentsByteLength            *int
	SubFilter                     string
	Filter                        string
	SigningTime                   string
	ObjectNumber                  *int
	ObjectGeneration              *int
	SignatureContainer            string
	DigestAlgorithm               string
	DigestAlgorithmStatus         string
	MalformedByteRangeError       error
	CryptographicValidation       bool
	CryptographicValidationStatus string
}

func inspectSignatureInfo(input []byte) signatureInfo {
	info := signatureInfo{
		HasSignatureMarker:            hasPDFSignatureBoundary(input),
		ByteRangeStatus:               signatureByteRangeStatusAbsent,
		SignatureContainer:            signatureContainerUnknown,
		DigestAlgorithm:               signatureDigestAlgorithmUnknown,
		DigestAlgorithmStatus:         signatureDigestAlgorithmNotParsed,
		CryptographicValidationStatus: signatureCryptographicValidationNotPerformed,
	}
	applySignatureDictionaryMetadata(&info, input)
	if len(findAllPDFNamesOutsideStringOrComment(input, "ByteRange")) == 0 {
		return info
	}

	ranges, err := signatureByteRanges(input)
	if err != nil {
		info.MalformedByteRangeError = err
		info.ByteRangeStatus = signatureByteRangeStatusMalformed
		return info
	}

	info.ByteRanges = ranges
	info.ByteRangeCount = len(ranges)
	info.ByteRangeStatus = signatureByteRangeStatusValid
	return info
}

func applySignatureDictionaryMetadata(info *signatureInfo, input []byte) {
	if info == nil || len(input) == 0 {
		return
	}
	for _, signatureDict := range signatureDictionariesForInput(input) {
		dict := signatureDict.Dict
		if info.ObjectNumber == nil && signatureDict.ObjectID != nil {
			number := signatureDict.ObjectID.Number
			generation := signatureDict.ObjectID.Generation
			info.ObjectNumber = &number
			info.ObjectGeneration = &generation
		}
		contents, hasContents := signatureContentsBytes(dict)
		if info.ContentsByteLength == nil {
			if hasContents {
				length := len(contents)
				info.ContentsByteLength = &length
			}
		}
		if info.SubFilter == "" {
			if subFilter, ok := dictPDFName(dict, "SubFilter"); ok {
				info.SubFilter = subFilter
			}
		}
		if info.Filter == "" {
			if filter, ok := dictPDFName(dict, "Filter"); ok {
				info.Filter = filter
			}
		}
		if info.SigningTime == "" {
			if signingTime, ok := signatureStringValue(dict["M"]); ok {
				info.SigningTime = signingTime
			}
		}
		applySignatureDigestHints(info, contents, hasContents)
		if info.ObjectNumber != nil && info.ContentsByteLength != nil && info.SubFilter != "" && info.Filter != "" && info.SigningTime != "" && info.SignatureContainer != signatureContainerUnknown && info.DigestAlgorithm != signatureDigestAlgorithmUnknown {
			return
		}
	}
}

type signatureDictionaryInfo struct {
	Dict     pdfDict
	ObjectID *pdfObjectID
}

func signatureDictionariesForInput(input []byte) []signatureDictionaryInfo {
	graph, err := parsePDFGraphWithOptions(input, pdfGraphParseOptions{
		AllowSignature: true,
		AllowXFA:       true,
	})
	if err != nil || graph == nil {
		return nil
	}
	dicts := make([]signatureDictionaryInfo, 0)
	for _, object := range sortedPDFObjects(graph.Objects) {
		collectSignatureDictionaries(object.Value, &dicts, &object.ID, true, 0)
	}
	return dicts
}

func collectSignatureDictionaries(value pdfValue, out *[]signatureDictionaryInfo, objectID *pdfObjectID, topLevel bool, depth int) {
	if depth > 32 {
		return
	}
	switch v := value.(type) {
	case pdfDict:
		if isSignatureDictionary(v) {
			*out = append(*out, signatureDictionaryInfo{Dict: v, ObjectID: indirectObjectIDForSignatureDictionary(objectID, topLevel)})
		}
		for _, child := range v {
			collectSignatureDictionaries(child, out, nil, false, depth+1)
		}
	case pdfArray:
		for _, child := range v {
			collectSignatureDictionaries(child, out, nil, false, depth+1)
		}
	case pdfStreamObject:
		collectSignatureDictionaries(v.Dict, out, objectID, topLevel, depth+1)
	}
}

func indirectObjectIDForSignatureDictionary(objectID *pdfObjectID, topLevel bool) *pdfObjectID {
	if objectID == nil || !topLevel {
		return nil
	}
	id := *objectID
	return &id
}

func isSignatureDictionary(dict pdfDict) bool {
	if dictHasType(dict, "Sig") {
		return true
	}
	if _, hasContents := dict["Contents"]; !hasContents {
		return false
	}
	_, hasByteRange := dict["ByteRange"]
	_, hasSubFilter := dict["SubFilter"]
	return hasByteRange || hasSubFilter
}

func signatureContentsBytes(dict pdfDict) ([]byte, bool) {
	value, ok := dict["Contents"]
	if !ok {
		return nil, false
	}
	bytes, ok, err := pdfStringBytes(value)
	if err != nil || !ok {
		return nil, false
	}
	return bytes, true
}

func signatureStringValue(value pdfValue) (string, bool) {
	bytes, ok, err := pdfStringBytes(value)
	if err != nil || !ok {
		return "", false
	}
	return string(bytes), true
}

func applySignatureDigestHints(info *signatureInfo, contents []byte, hasContents bool) {
	if info == nil {
		return
	}
	if info.SubFilter != "" {
		if container := signatureContainerFromSubFilter(info.SubFilter); container != signatureContainerUnknown {
			info.SignatureContainer = container
		}
		if digest := signatureDigestAlgorithmFromSubFilter(info.SubFilter); digest != signatureDigestAlgorithmUnknown && info.DigestAlgorithm == signatureDigestAlgorithmUnknown {
			info.DigestAlgorithm = digest
			info.DigestAlgorithmStatus = signatureDigestAlgorithmSubFilterHint
		}
	}
	if !hasContents {
		return
	}
	if info.SignatureContainer == signatureContainerUnknown {
		if container := signatureContainerFromContentsEnvelope(contents); container != signatureContainerUnknown {
			info.SignatureContainer = container
		}
	}
	if info.DigestAlgorithm == signatureDigestAlgorithmUnknown {
		if digest := signatureDigestAlgorithmFromContentsOID(contents); digest != signatureDigestAlgorithmUnknown {
			info.DigestAlgorithm = digest
			info.DigestAlgorithmStatus = signatureDigestAlgorithmContentsOIDHint
		}
	}
}

func signatureContainerFromSubFilter(subFilter string) string {
	switch strings.ToLower(subFilter) {
	case "adbe.pkcs7.detached", "adbe.pkcs7.sha1":
		return "pkcs7"
	case "etsi.cades.detached":
		return "cades"
	default:
		return signatureContainerUnknown
	}
}

func signatureDigestAlgorithmFromSubFilter(subFilter string) string {
	switch strings.ToLower(subFilter) {
	case "adbe.pkcs7.sha1", "adbe.x509.rsa_sha1":
		return "sha1"
	default:
		return signatureDigestAlgorithmUnknown
	}
}

func signatureContainerFromContentsEnvelope(contents []byte) string {
	if len(contents) < 16 || contents[0] != 0x30 {
		return signatureContainerUnknown
	}
	if !asn1LengthLooksPlausible(contents[1:], len(contents)-2) {
		return signatureContainerUnknown
	}
	if bytes.Contains(contents, []byte{0x06, 0x09, 0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x07, 0x02}) {
		return "pkcs7"
	}
	return signatureContainerUnknown
}

func asn1LengthLooksPlausible(lengthBytes []byte, remainingAfterShortForm int) bool {
	if len(lengthBytes) == 0 {
		return false
	}
	first := lengthBytes[0]
	if first&0x80 == 0 {
		return int(first) <= remainingAfterShortForm
	}
	count := int(first & 0x7f)
	if count == 0 || count > 4 || len(lengthBytes) < 1+count {
		return false
	}
	length := 0
	for _, b := range lengthBytes[1 : 1+count] {
		length = length<<8 | int(b)
	}
	return length <= len(lengthBytes)-1-count
}

func signatureDigestAlgorithmFromContentsOID(contents []byte) string {
	for _, candidate := range []struct {
		name string
		oid  []byte
	}{
		{name: "sha512", oid: []byte{0x06, 0x09, 0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02, 0x03}},
		{name: "sha384", oid: []byte{0x06, 0x09, 0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02, 0x02}},
		{name: "sha256", oid: []byte{0x06, 0x09, 0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02, 0x01}},
		{name: "sha224", oid: []byte{0x06, 0x09, 0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02, 0x04}},
		{name: "sha1", oid: []byte{0x06, 0x05, 0x2b, 0x0e, 0x03, 0x02, 0x1a}},
	} {
		if bytes.Contains(contents, candidate.oid) {
			return candidate.name
		}
	}
	return signatureDigestAlgorithmUnknown
}
