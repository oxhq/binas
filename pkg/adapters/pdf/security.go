package pdf

import (
	"bytes"
	"errors"
	"fmt"
	"sort"
	"strings"
)

var (
	ErrEncryptedPDFPasswordRequired     = errors.New("unsupported PDF: encrypted PDFs require an explicit password-capable path; Adapter.Parse does not decrypt")
	ErrEncryptedPDFUnsupportedAlgorithm = errors.New("unsupported PDF: encrypted PDFs are not parser-wired; unsupported encryption algorithm/handler or crypt filter")
	ErrSignedPDFRequiresInvalidation    = errors.New("unsupported PDF: digital signatures require explicit invalidation mode; Adapter.Parse refuses silent signature invalidation")
	ErrSignedPDFByteRangeProofRequired  = errors.New("unsupported PDF: signature-mode preserve-incremental requires parseable /ByteRange proof")
)

type SignatureInvalidationMode string

const (
	SignatureInvalidationRefuse              SignatureInvalidationMode = "refuse"
	SignatureInvalidationInvalidate          SignatureInvalidationMode = "invalidate"
	SignatureInvalidationPreserveIncremental SignatureInvalidationMode = "preserve-incremental"
)

type SecurityOptions struct {
	SignatureInvalidation SignatureInvalidationMode
	Password              string
}

type SecurityMetadataOptions struct {
	SignatureTrust SignatureTrustOptions
}

type SecurityMetadata struct {
	Encrypted  bool                `json:"encrypted"`
	Signed     bool                `json:"signed"`
	Encryption *EncryptionMetadata `json:"encryption,omitempty"`
	Signature  SignatureMetadata   `json:"signature"`
}

type EncryptionMetadata struct {
	Present            bool                    `json:"present"`
	Filter             string                  `json:"filter,omitempty"`
	V                  *int                    `json:"v,omitempty"`
	R                  *int                    `json:"r,omitempty"`
	Length             *int                    `json:"length,omitempty"`
	EncryptMetadata    *bool                   `json:"encrypt_metadata,omitempty"`
	SubFilter          string                  `json:"sub_filter,omitempty"`
	StreamFilter       string                  `json:"stream_filter,omitempty"`
	StringFilter       string                  `json:"string_filter,omitempty"`
	EmbeddedFileFilter string                  `json:"embedded_file_filter,omitempty"`
	CryptFilters       []EncryptionCryptFilter `json:"crypt_filters,omitempty"`
	PublicKey          bool                    `json:"public_key,omitempty"`
	RecipientCount     *int                    `json:"recipient_count,omitempty"`
	ObjectNumber       *int                    `json:"object_number,omitempty"`
	ObjectGeneration   *int                    `json:"object_generation,omitempty"`
	DictionaryParsed   bool                    `json:"dictionary_parsed"`
}

type EncryptionCryptFilter struct {
	Name      string `json:"name"`
	CFM       string `json:"cfm,omitempty"`
	AuthEvent string `json:"auth_event,omitempty"`
	Length    *int   `json:"length,omitempty"`
}

type SignatureMetadata struct {
	Present                          bool   `json:"present"`
	ByteRangeCount                   int    `json:"byte_range_count,omitempty"`
	ByteRangeTotalRanges             int    `json:"byte_range_total_ranges,omitempty"`
	ByteRangeCoveredBytes            int    `json:"byte_range_covered_bytes,omitempty"`
	ByteRangeStatus                  string `json:"byte_range_status,omitempty"`
	ContentsByteLength               *int   `json:"contents_byte_length,omitempty"`
	SubFilter                        string `json:"sub_filter,omitempty"`
	Filter                           string `json:"filter,omitempty"`
	SigningTime                      string `json:"signing_time,omitempty"`
	ObjectNumber                     *int   `json:"object_number,omitempty"`
	ObjectGeneration                 *int   `json:"object_generation,omitempty"`
	SignatureContainer               string `json:"signature_container,omitempty"`
	DigestAlgorithm                  string `json:"digest_algorithm,omitempty"`
	DigestAlgorithmStatus            string `json:"digest_algorithm_status,omitempty"`
	CertificateCount                 int    `json:"certificate_count,omitempty"`
	SignerCertificateSubject         string `json:"signer_certificate_subject,omitempty"`
	SignerCertificateIssuer          string `json:"signer_certificate_issuer,omitempty"`
	ByteRangeDigestValidation        bool   `json:"byte_range_digest_validation"`
	ByteRangeDigestValidationStatus  string `json:"byte_range_digest_validation_status"`
	CertificateTrustValidation       bool   `json:"certificate_trust_validation"`
	CertificateTrustValidationStatus string `json:"certificate_trust_validation_status"`
	CryptographicValidation          bool   `json:"cryptographic_validation"`
	CryptographicValidationStatus    string `json:"cryptographic_validation_status"`
}

type UnsupportedEncryptionAlgorithmError struct {
	Encryption EncryptionMetadata
}

func (e *UnsupportedEncryptionAlgorithmError) Error() string {
	if e == nil {
		return ErrEncryptedPDFUnsupportedAlgorithm.Error()
	}
	if parts := e.Encryption.summaryParts(); len(parts) > 0 {
		return ErrEncryptedPDFUnsupportedAlgorithm.Error() + " (" + strings.Join(parts, ", ") + ")"
	}
	return ErrEncryptedPDFUnsupportedAlgorithm.Error()
}

func (e *UnsupportedEncryptionAlgorithmError) Unwrap() error {
	return ErrEncryptedPDFUnsupportedAlgorithm
}

func defaultSecurityOptions() SecurityOptions {
	return SecurityOptions{SignatureInvalidation: SignatureInvalidationRefuse}
}

func (o SecurityOptions) allowsSignatureInvalidation() bool {
	return o.SignatureInvalidation == SignatureInvalidationInvalidate
}

func (o SecurityOptions) preservesSignaturesIncrementally() bool {
	return o.SignatureInvalidation == SignatureInvalidationPreserveIncremental
}

func CheckSecurity(input []byte, opts SecurityOptions) error {
	boundaries := summarizeResidualBoundariesForInput(input)
	return rejectUnsupportedSecurityBoundariesWithInput(input, boundaries, opts)
}

func SecurityMetadataForInput(input []byte) SecurityMetadata {
	return SecurityMetadataForInputWithOptions(input, SecurityMetadataOptions{})
}

func SecurityMetadataForInputWithOptions(input []byte, opts SecurityMetadataOptions) SecurityMetadata {
	boundaries := summarizeResidualBoundariesForInput(input)
	signature := inspectSignatureInfoWithOptions(input, signerCertificateTrustOptions{
		Roots:         opts.SignatureTrust.Roots,
		Intermediates: opts.SignatureTrust.Intermediates,
		CurrentTime:   opts.SignatureTrust.CurrentTime,
	})
	metadata := SecurityMetadata{
		Encrypted: boundaries.HasEncryption,
		Signed:    boundaries.HasSignature,
		Signature: signatureMetadataFromInfo(signature),
	}
	if boundaries.HasEncryption {
		encryption := encryptionMetadataForInput(input)
		if !encryption.Present {
			encryption.Present = true
		}
		metadata.Encryption = &encryption
	}
	return metadata
}

func signatureMetadataFromInfo(info signatureInfo) SignatureMetadata {
	byteRangeCoveredBytes := 0
	for _, byteRange := range info.ByteRanges {
		byteRangeCoveredBytes += byteRange.Length
	}
	return SignatureMetadata{
		Present:                          info.HasSignatureMarker,
		ByteRangeCount:                   info.ByteRangeCount,
		ByteRangeTotalRanges:             len(info.ByteRanges),
		ByteRangeCoveredBytes:            byteRangeCoveredBytes,
		ByteRangeStatus:                  info.ByteRangeStatus,
		ContentsByteLength:               info.ContentsByteLength,
		SubFilter:                        info.SubFilter,
		Filter:                           info.Filter,
		SigningTime:                      info.SigningTime,
		ObjectNumber:                     info.ObjectNumber,
		ObjectGeneration:                 info.ObjectGeneration,
		SignatureContainer:               info.SignatureContainer,
		DigestAlgorithm:                  info.DigestAlgorithm,
		DigestAlgorithmStatus:            info.DigestAlgorithmStatus,
		CertificateCount:                 info.CertificateCount,
		SignerCertificateSubject:         info.SignerCertificateSubject,
		SignerCertificateIssuer:          info.SignerCertificateIssuer,
		ByteRangeDigestValidation:        info.ByteRangeDigestValidation,
		ByteRangeDigestValidationStatus:  info.ByteRangeDigestValidationStatus,
		CertificateTrustValidation:       info.CertificateTrustValidation,
		CertificateTrustValidationStatus: info.CertificateTrustValidationStatus,
		CryptographicValidation:          info.CryptographicValidation,
		CryptographicValidationStatus:    info.CryptographicValidationStatus,
	}
}

func (m SecurityMetadata) HasSecurityBoundary() bool {
	return m.Encrypted || m.Signed
}

func rejectUnsupportedSecurityBoundaries(boundaries residualBoundarySummary) error {
	return rejectUnsupportedSecurityBoundariesWithOptions(boundaries, defaultSecurityOptions())
}

func rejectUnsupportedSecurityBoundariesWithOptions(boundaries residualBoundarySummary, opts SecurityOptions) error {
	return rejectUnsupportedSecurityBoundariesWithInput(nil, boundaries, opts)
}

func rejectUnsupportedSecurityBoundariesWithInput(input []byte, boundaries residualBoundarySummary, opts SecurityOptions) error {
	if boundaries.HasEncryption {
		encryption := encryptionMetadataForInput(input)
		if encryption.isPublicKeyBoundary() || encryption.SubFilter != "" || (encryption.Filter != "" && encryption.Filter != "Standard") {
			return &UnsupportedEncryptionAlgorithmError{Encryption: encryption}
		}
		if opts.Password == "" {
			return ErrEncryptedPDFPasswordRequired
		}
		parseOpts := pdfGraphParseOptions{
			AllowEncryption: true,
			AllowSignature:  true,
			AllowXFA:        true,
			Password:        opts.Password,
		}
		if _, err := parsePDFGraphWithOptions(input, parseOpts); err != nil {
			if errors.Is(err, ErrEncryptedPDFUnsupportedAlgorithm) {
				var unsupported *UnsupportedEncryptionAlgorithmError
				if errors.As(err, &unsupported) {
					return err
				}
				return &UnsupportedEncryptionAlgorithmError{Encryption: encryption}
			}
			return err
		}
	}
	if boundaries.HasSignature && opts.preservesSignaturesIncrementally() {
		_, err := signatureByteRanges(input)
		if err != nil {
			return err
		}
		return nil
	}
	if boundaries.HasSignature && !opts.allowsSignatureInvalidation() {
		return ErrSignedPDFRequiresInvalidation
	}
	return nil
}

func encryptionMetadataForInput(input []byte) EncryptionMetadata {
	metadata := EncryptionMetadata{}
	if len(input) == 0 {
		return metadata
	}
	value, ok := findEncryptValue(input)
	if !ok {
		return metadata
	}
	metadata.Present = true
	switch v := value.(type) {
	case pdfDict:
		applyEncryptionDictMetadata(&metadata, v)
	case pdfRef:
		number := v.ID.Number
		generation := v.ID.Generation
		metadata.ObjectNumber = &number
		metadata.ObjectGeneration = &generation
		if dict, ok := encryptionDictForRef(input, v.ID); ok {
			applyEncryptionDictMetadata(&metadata, dict)
		}
	}
	return metadata
}

func findEncryptValue(input []byte) (pdfValue, bool) {
	nameAt, ok := findPDFNameOutsideStringOrComment(input, "Encrypt")
	if !ok {
		return nil, false
	}
	valueStart := skipPDFSpaceAndComments(input, nameAt+len("/Encrypt"))
	if valueStart >= len(input) {
		return nil, false
	}
	parser := pdfValueParser{input: input[valueStart:]}
	value, err := parser.parseValue()
	if err != nil {
		return nil, false
	}
	return value, true
}

func encryptionDictForRef(input []byte, id pdfObjectID) (pdfDict, bool) {
	objects := findXrefObjectOffsets(input)
	for _, object := range objects {
		if object.Number != id.Number || object.Generation != id.Generation {
			continue
		}
		value, err := parsePDFObjectValueAt(input, object, objects)
		if err != nil {
			return nil, false
		}
		if dict, ok := value.(pdfDict); ok {
			return dict, true
		}
		if stream, ok := value.(pdfStreamObject); ok {
			return stream.Dict, true
		}
	}
	return nil, false
}

func applyEncryptionDictMetadata(metadata *EncryptionMetadata, dict pdfDict) {
	metadata.DictionaryParsed = true
	if filter, ok := dictPDFName(dict, "Filter"); ok {
		metadata.Filter = filter
	}
	if subFilter, ok := dictPDFName(dict, "SubFilter"); ok {
		metadata.SubFilter = subFilter
	}
	if metadata.Filter == "Adobe.PubSec" {
		metadata.PublicKey = true
	}
	if value, ok := dict["Recipients"]; ok {
		metadata.PublicKey = true
		count := encryptionRecipientCount(value)
		metadata.RecipientCount = &count
	}
	if value, ok := dictInt(dict, "V"); ok {
		metadata.V = &value
	}
	if value, ok := dictInt(dict, "R"); ok {
		metadata.R = &value
	}
	if value, ok := dictInt(dict, "Length"); ok {
		metadata.Length = &value
	}
	if value, ok := dict["EncryptMetadata"].(bool); ok {
		metadata.EncryptMetadata = &value
	}
	if name, ok := dictPDFName(dict, "StmF"); ok {
		metadata.StreamFilter = name
	}
	if name, ok := dictPDFName(dict, "StrF"); ok {
		metadata.StringFilter = name
	}
	if name, ok := dictPDFName(dict, "EFF"); ok {
		metadata.EmbeddedFileFilter = name
	}
	if cf, ok := dict["CF"].(pdfDict); ok {
		metadata.CryptFilters = encryptionCryptFilterMetadata(cf)
	}
}

func dictPDFName(dict pdfDict, key string) (string, bool) {
	value, ok := dict[key].(pdfName)
	if !ok {
		return "", false
	}
	return string(value), true
}

func (m EncryptionMetadata) summaryParts() []string {
	parts := make([]string, 0, 8)
	if m.Filter != "" {
		parts = append(parts, "Filter="+m.Filter)
	}
	if m.V != nil {
		parts = append(parts, fmt.Sprintf("V=%d", *m.V))
	}
	if m.R != nil {
		parts = append(parts, fmt.Sprintf("R=%d", *m.R))
	}
	if m.Length != nil {
		parts = append(parts, fmt.Sprintf("Length=%d", *m.Length))
	}
	if m.EncryptMetadata != nil {
		parts = append(parts, fmt.Sprintf("EncryptMetadata=%t", *m.EncryptMetadata))
	}
	if m.SubFilter != "" {
		parts = append(parts, "SubFilter="+m.SubFilter)
	}
	if m.StreamFilter != "" {
		parts = append(parts, "StmF="+m.StreamFilter)
	}
	if m.StringFilter != "" {
		parts = append(parts, "StrF="+m.StringFilter)
	}
	if m.EmbeddedFileFilter != "" {
		parts = append(parts, "EFF="+m.EmbeddedFileFilter)
	}
	if m.PublicKey {
		parts = append(parts, "PublicKey=true")
	}
	if m.RecipientCount != nil {
		parts = append(parts, fmt.Sprintf("Recipients=%d", *m.RecipientCount))
	}
	for _, filter := range m.CryptFilters {
		if filter.Name == "" {
			continue
		}
		label := "CF." + filter.Name
		values := make([]string, 0, 3)
		if filter.CFM != "" {
			values = append(values, "CFM="+filter.CFM)
		}
		if filter.AuthEvent != "" {
			values = append(values, "AuthEvent="+filter.AuthEvent)
		}
		if filter.Length != nil {
			values = append(values, fmt.Sprintf("Length=%d", *filter.Length))
		}
		if len(values) > 0 {
			parts = append(parts, label+"("+strings.Join(values, ",")+")")
		} else {
			parts = append(parts, label)
		}
	}
	if m.ObjectNumber != nil && m.ObjectGeneration != nil {
		parts = append(parts, fmt.Sprintf("Object=%d %d R", *m.ObjectNumber, *m.ObjectGeneration))
	}
	return parts
}

func (m EncryptionMetadata) isPublicKeyBoundary() bool {
	return m.PublicKey || (m.RecipientCount != nil && *m.RecipientCount > 0)
}

func encryptionCryptFilterMetadata(cf pdfDict) []EncryptionCryptFilter {
	if len(cf) == 0 {
		return nil
	}
	names := make([]string, 0, len(cf))
	for name := range cf {
		names = append(names, name)
	}
	sort.Strings(names)
	filters := make([]EncryptionCryptFilter, 0, len(names))
	for _, name := range names {
		filter := EncryptionCryptFilter{Name: name}
		if dict, ok := cf[name].(pdfDict); ok {
			if cfm, ok := dictPDFName(dict, "CFM"); ok {
				filter.CFM = cfm
			}
			if authEvent, ok := dictPDFName(dict, "AuthEvent"); ok {
				filter.AuthEvent = authEvent
			}
			if length, ok := dictInt(dict, "Length"); ok {
				filter.Length = &length
			}
		}
		filters = append(filters, filter)
	}
	return filters
}

func encryptionRecipientCount(value pdfValue) int {
	switch v := value.(type) {
	case pdfArray:
		return len(v)
	case pdfLiteralString, pdfHexString, pdfDecryptedString:
		return 1
	default:
		return 0
	}
}

type signatureByteRange struct {
	Offset int
	Length int
}

func signatureByteRanges(input []byte) ([]signatureByteRange, error) {
	positions := findAllPDFNamesOutsideStringOrComment(input, "ByteRange")
	if len(positions) == 0 {
		return nil, ErrSignedPDFByteRangeProofRequired
	}
	ranges := make([]signatureByteRange, 0, len(positions)*2)
	for _, nameAt := range positions {
		valueStart := skipPDFSpaceAndComments(input, nameAt+len("/ByteRange"))
		if valueStart >= len(input) {
			return nil, fmt.Errorf("%w: missing /ByteRange value", ErrSignedPDFByteRangeProofRequired)
		}
		parser := pdfValueParser{input: input[valueStart:]}
		value, err := parser.parseValue()
		if err != nil {
			return nil, fmt.Errorf("%w: %v", ErrSignedPDFByteRangeProofRequired, err)
		}
		listRanges, err := signatureByteRangesFromValue(input, value)
		if err != nil {
			return nil, err
		}
		ranges = append(ranges, listRanges...)
	}
	if len(ranges) == 0 {
		return nil, ErrSignedPDFByteRangeProofRequired
	}
	return ranges, nil
}

func signatureByteRangesFromValue(input []byte, value pdfValue) ([]signatureByteRange, error) {
	array, ok := value.(pdfArray)
	if !ok || len(array) == 0 || len(array)%2 != 0 {
		return nil, fmt.Errorf("%w: /ByteRange must be an array of offset/length pairs", ErrSignedPDFByteRangeProofRequired)
	}
	if pairCount := len(array) / 2; pairCount%2 != 0 {
		return nil, fmt.Errorf("%w: /ByteRange must contain an even number of offset/length pairs", ErrSignedPDFByteRangeProofRequired)
	}
	listRanges := make([]signatureByteRange, 0, len(array)/2)
	for i := 0; i < len(array); i += 2 {
		offset, ok := pdfIntegerValue(array[i])
		if !ok {
			return nil, fmt.Errorf("%w: /ByteRange offset must be an integer", ErrSignedPDFByteRangeProofRequired)
		}
		length, ok := pdfIntegerValue(array[i+1])
		if !ok {
			return nil, fmt.Errorf("%w: /ByteRange length must be an integer", ErrSignedPDFByteRangeProofRequired)
		}
		if offset < 0 || length < 0 {
			return nil, fmt.Errorf("%w: /ByteRange values must be nonnegative", ErrSignedPDFByteRangeProofRequired)
		}
		if offset > len(input) || length > len(input)-offset {
			return nil, fmt.Errorf("%w: /ByteRange %d %d is outside the original file", ErrSignedPDFByteRangeProofRequired, offset, length)
		}
		listRanges = append(listRanges, signatureByteRange{Offset: offset, Length: length})
	}
	if err := validateSignatureByteRangeList(listRanges); err != nil {
		return nil, err
	}
	return listRanges, nil
}

func validateSignatureByteRangeList(ranges []signatureByteRange) error {
	previousEnd := 0
	for i, byteRange := range ranges {
		if byteRange.Length == 0 {
			return fmt.Errorf("%w: /ByteRange contains a zero-length signed range", ErrSignedPDFByteRangeProofRequired)
		}
		if i == 0 {
			previousEnd = byteRange.Offset + byteRange.Length
			continue
		}
		if byteRange.Offset < previousEnd {
			return fmt.Errorf("%w: /ByteRange ranges must be sorted and non-overlapping", ErrSignedPDFByteRangeProofRequired)
		}
		previousEnd = byteRange.Offset + byteRange.Length
	}
	return nil
}

func pdfIntegerValue(value pdfValue) (int, bool) {
	switch v := value.(type) {
	case int:
		return v, true
	case int64:
		return int(v), true
	case float64:
		return int(v), v == float64(int(v))
	default:
		return 0, false
	}
}

func findPDFNameOutsideStringOrComment(input []byte, name string) (int, bool) {
	positions := findAllPDFNamesOutsideStringOrComment(input, name)
	if len(positions) == 0 {
		return 0, false
	}
	return positions[0], true
}

func findAllPDFNamesOutsideStringOrComment(input []byte, name string) []int {
	needle := []byte("/" + name)
	positions := make([]int, 0)
	literalDepth := 0
	escaped := false
	for i := 0; i < len(input); i++ {
		if literalDepth > 0 {
			if escaped {
				escaped = false
				continue
			}
			switch input[i] {
			case '\\':
				escaped = true
			case '(':
				literalDepth++
			case ')':
				literalDepth--
			}
			continue
		}
		if input[i] == '(' {
			literalDepth = 1
			continue
		}
		if input[i] == '%' {
			i = skipPDFComment(input, i+1)
			continue
		}
		if input[i] == '<' {
			if i+1 < len(input) && input[i+1] == '<' {
				i++
				continue
			}
			for i++; i < len(input) && input[i] != '>'; i++ {
			}
			continue
		}
		if !bytes.HasPrefix(input[i:], needle) {
			continue
		}
		end := i + len(needle)
		if isPDFTokenEnd(input, end) {
			positions = append(positions, i)
			i = end - 1
			continue
		}
		i = end - 1
	}
	return positions
}
