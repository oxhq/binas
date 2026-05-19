package pdf

const signatureCryptographicValidationNotPerformed = "not_performed"

type signatureInfo struct {
	HasSignatureMarker            bool
	ByteRanges                    []signatureByteRange
	ByteRangeCount                int
	MalformedByteRangeError       error
	CryptographicValidation       bool
	CryptographicValidationStatus string
}

func inspectSignatureInfo(input []byte) signatureInfo {
	info := signatureInfo{
		HasSignatureMarker:            hasPDFSignatureBoundary(input),
		CryptographicValidationStatus: signatureCryptographicValidationNotPerformed,
	}
	if len(findAllPDFNamesOutsideStringOrComment(input, "ByteRange")) == 0 {
		return info
	}

	ranges, err := signatureByteRanges(input)
	if err != nil {
		info.MalformedByteRangeError = err
		return info
	}

	info.ByteRanges = ranges
	info.ByteRangeCount = len(ranges)
	return info
}
