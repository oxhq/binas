package pdf

import (
	"bytes"
	"crypto/sha1"
	"crypto/sha256"
	"crypto/sha512"
	"crypto/x509"
	"encoding/asn1"
	"errors"
	"hash"
	"math/big"
	"strings"
	"time"
)

const signatureCryptographicValidationNotPerformed = "not_performed"
const signatureCryptographicValidationUnsupported = "unsupported"
const signatureCryptographicValidationByteRangeDigestValid = "byte_range_digest_valid"
const signatureCryptographicValidationByteRangeDigestMismatch = "byte_range_digest_mismatch"
const signatureByteRangeDigestValidationNotPerformed = "not_performed"
const signatureByteRangeDigestValidationUnsupported = "unsupported"
const signatureByteRangeDigestValidationValid = "valid"
const signatureByteRangeDigestValidationMismatch = "mismatch"
const signatureCertificateTrustValidationNotPerformed = "not_performed"
const signatureCertificateTrustValidationInsufficientCertificates = "insufficient_certificates"
const signatureCertificateTrustValidationValid = "valid"
const signatureCertificateTrustValidationInvalid = "invalid"
const signatureContainerUnknown = "unknown"
const signatureDigestAlgorithmUnknown = "unknown"
const signatureDigestAlgorithmNotParsed = "not_parsed"
const signatureDigestAlgorithmSubFilterHint = "sub_filter_hint"
const signatureDigestAlgorithmContentsOIDHint = "contents_oid_hint"
const signatureDigestAlgorithmCMSAuthenticatedAttribute = "cms_authenticated_attribute"
const signatureByteRangeStatusAbsent = "absent"
const signatureByteRangeStatusValid = "valid"
const signatureByteRangeStatusMalformed = "malformed"

type signatureInfo struct {
	HasSignatureMarker               bool
	ByteRanges                       []signatureByteRange
	ByteRangeCount                   int
	ByteRangeStatus                  string
	ContentsByteLength               *int
	SubFilter                        string
	Filter                           string
	SigningTime                      string
	ObjectNumber                     *int
	ObjectGeneration                 *int
	SignatureContainer               string
	DigestAlgorithm                  string
	DigestAlgorithmStatus            string
	CertificateCount                 int
	SignerCertificateSubject         string
	SignerCertificateIssuer          string
	MalformedByteRangeError          error
	ByteRangeDigestValidation        bool
	ByteRangeDigestValidationStatus  string
	CertificateTrustValidation       bool
	CertificateTrustValidationStatus string
	CryptographicValidation          bool
	CryptographicValidationStatus    string
}

type SignatureTrustOptions struct {
	Roots         []*x509.Certificate
	Intermediates []*x509.Certificate
	CurrentTime   time.Time
}

func inspectSignatureInfo(input []byte) signatureInfo {
	return inspectSignatureInfoWithOptions(input, signerCertificateTrustOptions{})
}

func inspectSignatureInfoWithOptions(input []byte, trust signerCertificateTrustOptions) signatureInfo {
	info := signatureInfo{
		HasSignatureMarker:               hasPDFSignatureBoundary(input),
		ByteRangeStatus:                  signatureByteRangeStatusAbsent,
		SignatureContainer:               signatureContainerUnknown,
		DigestAlgorithm:                  signatureDigestAlgorithmUnknown,
		DigestAlgorithmStatus:            signatureDigestAlgorithmNotParsed,
		ByteRangeDigestValidationStatus:  signatureByteRangeDigestValidationNotPerformed,
		CertificateTrustValidationStatus: signatureCertificateTrustValidationNotPerformed,
		CryptographicValidationStatus:    signatureCryptographicValidationNotPerformed,
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
	applySignatureCryptographicValidation(&info, input, trust)
	return info
}

func applySignatureCryptographicValidation(info *signatureInfo, input []byte, trust signerCertificateTrustOptions) {
	if info == nil || len(input) == 0 || info.ByteRangeStatus != signatureByteRangeStatusValid {
		return
	}
	attempted := false
	for _, signatureDict := range signatureDictionariesForInput(input) {
		dict := signatureDict.Dict
		contents, hasContents := signatureContentsBytes(dict)
		if !hasContents || !shouldAttemptCMSDigestValidation(info, dict, contents) {
			continue
		}
		ranges, err := signatureByteRangesFromValue(input, dict["ByteRange"])
		if err != nil {
			continue
		}
		attempted = true
		cms, err := parseCMSDetachedSignature(contents)
		applyCMSMetadata(info, cms, trust)
		if err != nil || len(cms.MessageDigest) == 0 || cms.DigestAlgorithm == signatureDigestAlgorithmUnknown {
			continue
		}
		digest, ok := digestByteRanges(input, ranges, cms.DigestAlgorithm)
		if !ok {
			continue
		}
		if bytes.Equal(digest, cms.MessageDigest) {
			info.ByteRangeDigestValidation = true
			info.ByteRangeDigestValidationStatus = signatureByteRangeDigestValidationValid
			info.CryptographicValidation = true
			info.CryptographicValidationStatus = signatureCryptographicValidationByteRangeDigestValid
			return
		}
		info.ByteRangeDigestValidation = false
		info.ByteRangeDigestValidationStatus = signatureByteRangeDigestValidationMismatch
		info.CryptographicValidation = false
		info.CryptographicValidationStatus = signatureCryptographicValidationByteRangeDigestMismatch
		return
	}
	if attempted && info.CryptographicValidationStatus == signatureCryptographicValidationNotPerformed {
		info.ByteRangeDigestValidationStatus = signatureByteRangeDigestValidationUnsupported
		info.CryptographicValidationStatus = signatureCryptographicValidationUnsupported
	}
}

func shouldAttemptCMSDigestValidation(info *signatureInfo, dict pdfDict, contents []byte) bool {
	if subFilter, ok := dictPDFName(dict, "SubFilter"); ok {
		switch signatureContainerFromSubFilter(subFilter) {
		case "pkcs7", "cades":
			return true
		}
	}
	if info != nil {
		switch info.SignatureContainer {
		case "pkcs7", "cades":
			return true
		}
	}
	return signatureContainerFromContentsEnvelope(contents) == "pkcs7"
}

func applyCMSMetadata(info *signatureInfo, cms cmsDetachedSignature, trust signerCertificateTrustOptions) {
	if info == nil {
		return
	}
	if cms.DigestAlgorithm != "" && cms.DigestAlgorithm != signatureDigestAlgorithmUnknown {
		info.DigestAlgorithm = cms.DigestAlgorithm
		info.DigestAlgorithmStatus = signatureDigestAlgorithmCMSAuthenticatedAttribute
	}
	if cms.CertificateCount > 0 {
		info.CertificateCount = cms.CertificateCount
	}
	if cms.SignerCertificateSubject != "" {
		info.SignerCertificateSubject = cms.SignerCertificateSubject
	}
	if cms.SignerCertificateIssuer != "" {
		info.SignerCertificateIssuer = cms.SignerCertificateIssuer
	}
	if info.SignatureContainer == signatureContainerUnknown && cms.IsPKCS7SignedData {
		info.SignatureContainer = "pkcs7"
	}
	if cms.IsPKCS7SignedData {
		info.CertificateTrustValidation, info.CertificateTrustValidationStatus = validateSignerCertificateTrust(cms, trust)
	}
}

func digestByteRanges(input []byte, ranges []signatureByteRange, algorithm string) ([]byte, bool) {
	h, ok := signatureHash(algorithm)
	if !ok {
		return nil, false
	}
	for _, byteRange := range ranges {
		if byteRange.Offset < 0 || byteRange.Length < 0 || byteRange.Offset > len(input) || byteRange.Length > len(input)-byteRange.Offset {
			return nil, false
		}
		_, _ = h.Write(input[byteRange.Offset : byteRange.Offset+byteRange.Length])
	}
	return h.Sum(nil), true
}

func signatureHash(algorithm string) (hash.Hash, bool) {
	switch strings.ToLower(algorithm) {
	case "sha1":
		return sha1.New(), true
	case "sha224":
		return sha256.New224(), true
	case "sha256":
		return sha256.New(), true
	case "sha384":
		return sha512.New384(), true
	case "sha512":
		return sha512.New(), true
	default:
		return nil, false
	}
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

type cmsDetachedSignature struct {
	IsPKCS7SignedData        bool
	DigestAlgorithm          string
	MessageDigest            []byte
	CertificateCount         int
	SignerCertificateSubject string
	SignerCertificateIssuer  string
	Certificates             []*x509.Certificate
	SignerCertificate        *x509.Certificate
	SignerIdentifierPresent  bool
}

type cmsSignerCertificateIdentifier struct {
	IssuerRaw []byte
	Serial    *big.Int
	Present   bool
}

var (
	oidPKCS7SignedData        = asn1.ObjectIdentifier{1, 2, 840, 113549, 1, 7, 2}
	oidCMSMessageDigest       = asn1.ObjectIdentifier{1, 2, 840, 113549, 1, 9, 4}
	oidDigestSHA1             = asn1.ObjectIdentifier{1, 3, 14, 3, 2, 26}
	oidDigestSHA224           = asn1.ObjectIdentifier{2, 16, 840, 1, 101, 3, 4, 2, 4}
	oidDigestSHA256           = asn1.ObjectIdentifier{2, 16, 840, 1, 101, 3, 4, 2, 1}
	oidDigestSHA384           = asn1.ObjectIdentifier{2, 16, 840, 1, 101, 3, 4, 2, 2}
	oidDigestSHA512           = asn1.ObjectIdentifier{2, 16, 840, 1, 101, 3, 4, 2, 3}
	oidSignatureSHA1WithRSA   = asn1.ObjectIdentifier{1, 2, 840, 113549, 1, 1, 5}
	oidSignatureSHA256WithRSA = asn1.ObjectIdentifier{1, 2, 840, 113549, 1, 1, 11}
	oidSignatureSHA384WithRSA = asn1.ObjectIdentifier{1, 2, 840, 113549, 1, 1, 12}
	oidSignatureSHA512WithRSA = asn1.ObjectIdentifier{1, 2, 840, 113549, 1, 1, 13}
)

func parseCMSDetachedSignature(contents []byte) (cmsDetachedSignature, error) {
	var out cmsDetachedSignature
	contentInfo, err := parseDERElementAllowingTrailingNUL(contents)
	if err != nil {
		return out, err
	}
	children, err := parseDERChildren(contentInfo.Bytes)
	if err != nil || len(children) < 2 {
		return out, firstDERError(err)
	}
	contentType, ok := oidFromDERElement(children[0])
	if !ok || !contentType.Equal(oidPKCS7SignedData) {
		return out, asn1.SyntaxError{Msg: "CMS content type is not signedData"}
	}
	out.IsPKCS7SignedData = true
	if children[1].Class != asn1.ClassContextSpecific || children[1].Tag != 0 || !children[1].IsCompound {
		return out, asn1.SyntaxError{Msg: "CMS signedData wrapper is missing"}
	}
	signedData, err := parseDERElement(children[1].Bytes)
	if err != nil {
		return out, err
	}
	signedDataChildren, err := parseDERChildren(signedData.Bytes)
	if err != nil {
		return out, err
	}
	for _, child := range signedDataChildren {
		if child.Class == asn1.ClassContextSpecific && child.Tag == 0 && child.IsCompound {
			out.Certificates = parseCMSCertificates(child.Bytes)
			out.CertificateCount, out.SignerCertificateSubject, out.SignerCertificateIssuer = cmsCertificateMetadata(out.Certificates, nil, false)
		}
	}
	if len(signedDataChildren) == 0 {
		return out, asn1.SyntaxError{Msg: "CMS signedData has no fields"}
	}
	signerInfos := signedDataChildren[len(signedDataChildren)-1]
	if signerInfos.Class != asn1.ClassUniversal || signerInfos.Tag != asn1.TagSet || !signerInfos.IsCompound {
		return out, asn1.SyntaxError{Msg: "CMS signerInfos set is missing"}
	}
	signers, err := parseDERChildren(signerInfos.Bytes)
	if err != nil || len(signers) == 0 {
		return out, firstDERError(err)
	}
	for _, signer := range signers {
		digestAlgorithm, messageDigest, signerID, ok := parseCMSSignerInfoMessageDigest(signer)
		if !ok {
			continue
		}
		out.SignerCertificate = cmsSignerCertificate(out.Certificates, signerID)
		out.SignerIdentifierPresent = signerID.Present
		out.CertificateCount, out.SignerCertificateSubject, out.SignerCertificateIssuer = cmsCertificateMetadata(out.Certificates, out.SignerCertificate, signerID.Present)
		out.DigestAlgorithm = digestAlgorithm
		out.MessageDigest = messageDigest
		return out, nil
	}
	return out, asn1.SyntaxError{Msg: "CMS messageDigest signed attribute is missing"}
}

func parseCMSSignerInfoMessageDigest(signer asn1.RawValue) (string, []byte, cmsSignerCertificateIdentifier, bool) {
	var signerID cmsSignerCertificateIdentifier
	if signer.Class != asn1.ClassUniversal || signer.Tag != asn1.TagSequence || !signer.IsCompound {
		return "", nil, signerID, false
	}
	children, err := parseDERChildren(signer.Bytes)
	if err != nil || len(children) < 5 {
		return "", nil, signerID, false
	}
	signerID = parseCMSSignerCertificateIdentifier(children[1])
	digestAlgorithm := digestAlgorithmNameFromAlgorithmIdentifier(children[2])
	for _, child := range children[3:] {
		if child.Class != asn1.ClassContextSpecific || child.Tag != 0 || !child.IsCompound {
			continue
		}
		messageDigest, ok := messageDigestFromSignedAttributes(child.Bytes)
		if ok {
			return digestAlgorithm, messageDigest, signerID, true
		}
	}
	return "", nil, signerID, false
}

func parseCMSSignerCertificateIdentifier(input asn1.RawValue) cmsSignerCertificateIdentifier {
	var out cmsSignerCertificateIdentifier
	if input.Class != asn1.ClassUniversal || input.Tag != asn1.TagSequence || !input.IsCompound {
		return out
	}
	children, err := parseDERChildren(input.Bytes)
	if err != nil || len(children) != 2 {
		return out
	}
	issuer := children[0]
	if issuer.Class != asn1.ClassUniversal || issuer.Tag != asn1.TagSequence {
		return out
	}
	var serial big.Int
	rest, err := asn1.Unmarshal(children[1].FullBytes, &serial)
	if err != nil || len(rest) != 0 {
		return out
	}
	return cmsSignerCertificateIdentifier{
		IssuerRaw: bytes.Clone(issuer.FullBytes),
		Serial:    &serial,
		Present:   true,
	}
}

func messageDigestFromSignedAttributes(input []byte) ([]byte, bool) {
	attributes, err := parseDERChildren(input)
	if err != nil {
		return nil, false
	}
	for _, attr := range attributes {
		children, err := parseDERChildren(attr.Bytes)
		if err != nil || len(children) < 2 {
			continue
		}
		oid, ok := oidFromDERElement(children[0])
		if !ok || !oid.Equal(oidCMSMessageDigest) {
			continue
		}
		values := children[1]
		if values.Class != asn1.ClassUniversal || values.Tag != asn1.TagSet || !values.IsCompound {
			return nil, false
		}
		setChildren, err := parseDERChildren(values.Bytes)
		if err != nil || len(setChildren) != 1 {
			return nil, false
		}
		value := setChildren[0]
		if value.Class != asn1.ClassUniversal || value.Tag != asn1.TagOctetString {
			return nil, false
		}
		return bytes.Clone(value.Bytes), true
	}
	return nil, false
}

func parseCMSCertificateMetadata(input []byte) (int, string, string) {
	return cmsCertificateMetadata(parseCMSCertificates(input), nil, false)
}

func parseCMSCertificates(input []byte) []*x509.Certificate {
	certs, err := parseDERChildren(input)
	if err != nil {
		return nil
	}
	out := make([]*x509.Certificate, 0, len(certs))
	for _, candidate := range certs {
		if candidate.Class != asn1.ClassUniversal || candidate.Tag != asn1.TagSequence {
			continue
		}
		cert, err := x509.ParseCertificate(candidate.FullBytes)
		if err != nil {
			continue
		}
		out = append(out, cert)
	}
	return out
}

func cmsCertificateMetadata(certs []*x509.Certificate, signer *x509.Certificate, signerIDPresent bool) (int, string, string) {
	subject := ""
	issuer := ""
	if signer != nil {
		subject = signer.Subject.String()
		issuer = signer.Issuer.String()
		return len(certs), subject, issuer
	}
	if signerIDPresent {
		return len(certs), subject, issuer
	}
	for _, cert := range certs {
		if cert == nil {
			continue
		}
		if subject == "" {
			subject = cert.Subject.String()
		}
		if issuer == "" {
			issuer = cert.Issuer.String()
		}
	}
	return len(certs), subject, issuer
}

func cmsSignerCertificate(certs []*x509.Certificate, signerID cmsSignerCertificateIdentifier) *x509.Certificate {
	if !signerID.Present || signerID.Serial == nil {
		return nil
	}
	for _, cert := range certs {
		if cert == nil || cert.SerialNumber == nil {
			continue
		}
		if cert.SerialNumber.Cmp(signerID.Serial) == 0 && bytes.Equal(cert.RawIssuer, signerID.IssuerRaw) {
			return cert
		}
	}
	return nil
}

type signerCertificateTrustOptions struct {
	Roots         []*x509.Certificate
	Intermediates []*x509.Certificate
	CurrentTime   time.Time
}

func validateSignerCertificateTrust(cms cmsDetachedSignature, opts signerCertificateTrustOptions) (bool, string) {
	signer := cms.SignerCertificate
	if signer == nil && cms.SignerIdentifierPresent {
		return false, signatureCertificateTrustValidationInsufficientCertificates
	}
	if signer == nil && len(cms.Certificates) > 0 {
		signer = cms.Certificates[0]
	}
	if signer == nil {
		return false, signatureCertificateTrustValidationInsufficientCertificates
	}
	if len(opts.Roots) == 0 {
		return false, signatureCertificateTrustValidationNotPerformed
	}
	roots := x509.NewCertPool()
	for _, root := range opts.Roots {
		if root != nil {
			roots.AddCert(root)
		}
	}
	if len(roots.Subjects()) == 0 {
		return false, signatureCertificateTrustValidationNotPerformed
	}
	intermediates := x509.NewCertPool()
	for _, cert := range cms.Certificates[1:] {
		if cert != nil {
			intermediates.AddCert(cert)
		}
	}
	for _, cert := range opts.Intermediates {
		if cert != nil {
			intermediates.AddCert(cert)
		}
	}
	verifyOptions := x509.VerifyOptions{
		Roots:         roots,
		Intermediates: intermediates,
		KeyUsages:     []x509.ExtKeyUsage{x509.ExtKeyUsageAny},
	}
	if !opts.CurrentTime.IsZero() {
		verifyOptions.CurrentTime = opts.CurrentTime
	}
	if _, err := signer.Verify(verifyOptions); err != nil {
		var unknownAuthority x509.UnknownAuthorityError
		if errors.As(err, &unknownAuthority) {
			return false, signatureCertificateTrustValidationInsufficientCertificates
		}
		return false, signatureCertificateTrustValidationInvalid
	}
	return true, signatureCertificateTrustValidationValid
}

func digestAlgorithmNameFromAlgorithmIdentifier(input asn1.RawValue) string {
	children, err := parseDERChildren(input.Bytes)
	if err != nil || len(children) == 0 {
		return signatureDigestAlgorithmUnknown
	}
	oid, ok := oidFromDERElement(children[0])
	if !ok {
		return signatureDigestAlgorithmUnknown
	}
	return digestAlgorithmNameFromOID(oid)
}

func digestAlgorithmNameFromOID(oid asn1.ObjectIdentifier) string {
	switch {
	case oid.Equal(oidDigestSHA1), oid.Equal(oidSignatureSHA1WithRSA):
		return "sha1"
	case oid.Equal(oidDigestSHA224):
		return "sha224"
	case oid.Equal(oidDigestSHA256), oid.Equal(oidSignatureSHA256WithRSA):
		return "sha256"
	case oid.Equal(oidDigestSHA384), oid.Equal(oidSignatureSHA384WithRSA):
		return "sha384"
	case oid.Equal(oidDigestSHA512), oid.Equal(oidSignatureSHA512WithRSA):
		return "sha512"
	default:
		return signatureDigestAlgorithmUnknown
	}
}

func oidFromDERElement(input asn1.RawValue) (asn1.ObjectIdentifier, bool) {
	var oid asn1.ObjectIdentifier
	rest, err := asn1.Unmarshal(input.FullBytes, &oid)
	if err != nil || len(rest) != 0 {
		return nil, false
	}
	return oid, true
}

func parseDERElement(input []byte) (asn1.RawValue, error) {
	var value asn1.RawValue
	rest, err := asn1.Unmarshal(input, &value)
	if err != nil {
		return value, err
	}
	if len(rest) != 0 {
		return value, asn1.SyntaxError{Msg: "DER element has trailing data"}
	}
	return value, nil
}

func parseDERElementAllowingTrailingNUL(input []byte) (asn1.RawValue, error) {
	var value asn1.RawValue
	rest, err := asn1.Unmarshal(input, &value)
	if err != nil {
		return value, err
	}
	for _, b := range rest {
		if b != 0x00 {
			return value, asn1.SyntaxError{Msg: "DER element has non-padding trailing data"}
		}
	}
	return value, nil
}

func parseDERChildren(input []byte) ([]asn1.RawValue, error) {
	children := make([]asn1.RawValue, 0)
	for len(input) > 0 {
		var child asn1.RawValue
		rest, err := asn1.Unmarshal(input, &child)
		if err != nil {
			return children, err
		}
		if len(rest) == len(input) {
			return children, asn1.SyntaxError{Msg: "DER parser made no progress"}
		}
		children = append(children, child)
		input = rest
	}
	return children, nil
}

func firstDERError(err error) error {
	if err != nil {
		return err
	}
	return asn1.SyntaxError{Msg: "missing DER value"}
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
