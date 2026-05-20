package pdf

import (
	"bytes"
	"crypto/aes"
	"crypto/cipher"
	"crypto/md5"
	"crypto/rand"
	"crypto/rc4"
	"encoding/binary"
	"encoding/hex"
	"fmt"
)

var pdfStandardSecurityPasswordPadding = []byte{
	0x28, 0xbf, 0x4e, 0x5e, 0x4e, 0x75, 0x8a, 0x41,
	0x64, 0x00, 0x4e, 0x56, 0xff, 0xfa, 0x01, 0x08,
	0x2e, 0x2e, 0x00, 0xb6, 0xd0, 0x68, 0x3e, 0x80,
	0x2f, 0x0c, 0xa9, 0xfe, 0x64, 0x53, 0x69, 0x7a,
}

type pdfStandardCryptMethod string

const (
	pdfStandardCryptRC4   pdfStandardCryptMethod = "RC4"
	pdfStandardCryptAESV2 pdfStandardCryptMethod = "AESV2"
)

type pdfStandardSecurity struct {
	version         int
	revision        int
	keyLengthBytes  int
	ownerKey        []byte
	userKey         []byte
	permissions     int32
	fileID          []byte
	encryptMetadata bool
	cryptMethod     pdfStandardCryptMethod
}

type pdfGraphEncryption struct {
	security      *pdfStandardSecurity
	fileKey       []byte
	encryptObject *pdfObjectID
}

func (g *pdfGraph) prepareStandardSecurityEncryption(password []byte) (*pdfGraphEncryption, error) {
	if g == nil || g.Trailer == nil {
		return nil, unsupportedPDFEncryption("encrypted PDF is missing a parseable trailer")
	}
	value, ok := g.Trailer["Encrypt"]
	if !ok {
		return nil, unsupportedPDFEncryption("encrypted PDF trailer is missing /Encrypt")
	}
	var (
		dict      pdfDict
		encryptID *pdfObjectID
	)
	switch v := value.(type) {
	case pdfDict:
		dict = v
	case pdfRef:
		object := g.Objects[v.ID]
		if object == nil {
			return nil, unsupportedPDFEncryption("encryption dictionary object %d %d is missing", v.ID.Number, v.ID.Generation)
		}
		var ok bool
		dict, ok = object.Value.(pdfDict)
		if !ok {
			if stream, streamOK := object.Value.(pdfStreamObject); streamOK {
				dict = stream.Dict
				ok = true
			}
		}
		if !ok {
			return nil, unsupportedPDFEncryption("encryption dictionary object %d %d is not a dictionary", v.ID.Number, v.ID.Generation)
		}
		id := v.ID
		encryptID = &id
	default:
		return nil, unsupportedPDFEncryption("trailer /Encrypt must be a dictionary or indirect dictionary reference")
	}
	fileID, err := firstPDFTrailerFileID(g.Trailer)
	if err != nil {
		return nil, err
	}
	security, err := newPDFStandardSecurityFromDict(dict, fileID)
	if err != nil {
		return nil, err
	}
	fileKey, ok, err := security.authenticateUserPassword(password)
	if err != nil {
		return nil, err
	}
	if !ok {
		return nil, fmt.Errorf("%w: supplied password did not authenticate", ErrEncryptedPDFPasswordRequired)
	}
	return &pdfGraphEncryption{
		security:      security,
		fileKey:       bytes.Clone(fileKey),
		encryptObject: encryptID,
	}, nil
}

func firstPDFTrailerFileID(trailer pdfDict) ([]byte, error) {
	ids, ok := trailer["ID"].(pdfArray)
	if !ok || len(ids) == 0 {
		return nil, unsupportedPDFEncryption("Standard Security requires a trailer /ID array")
	}
	fileID, ok, err := pdfStringBytes(ids[0])
	if err != nil {
		return nil, err
	}
	if !ok || len(fileID) == 0 {
		return nil, unsupportedPDFEncryption("Standard Security requires the first trailer /ID value to be a string")
	}
	return fileID, nil
}

func (g *pdfGraph) decryptStandardSecurityObjects() error {
	if g == nil || g.Encryption == nil || g.Encryption.security == nil {
		return nil
	}
	for _, object := range sortedPDFObjects(g.Objects) {
		if g.Encryption.encryptObject != nil && object.ID == *g.Encryption.encryptObject {
			continue
		}
		if stream, ok := object.Value.(pdfStreamObject); ok {
			if dictHasType(stream.Dict, "XRef") {
				decrypted, err := decryptPDFXrefStreamValue(g.Encryption.security, g.Encryption.fileKey, object.ID, stream)
				if err != nil {
					return fmt.Errorf("decrypt object %d %d: %w", object.ID.Number, object.ID.Generation, err)
				}
				object.Value = decrypted
				continue
			}
		}
		value, err := decryptPDFObjectValue(g.Encryption.security, g.Encryption.fileKey, object.ID, object.Value)
		if err != nil {
			return fmt.Errorf("decrypt object %d %d: %w", object.ID.Number, object.ID.Generation, err)
		}
		object.Value = value
	}
	return nil
}

func decryptPDFXrefStreamValue(security *pdfStandardSecurity, fileKey []byte, id pdfObjectID, stream pdfStreamObject) (pdfStreamObject, error) {
	decrypted := stream
	if pdfStreamUsesCryptFilter(stream.Dict) || !security.encryptsStreamData(stream.Dict) {
		return decrypted, nil
	}
	if _, err := parsePDFXrefStream(stream.Dict, stream.Data); err == nil {
		return stream, nil
	}
	decryptedData, err := security.decryptObject(fileKey, id, stream.Data)
	if err != nil {
		return pdfStreamObject{}, err
	}
	decrypted.Data = decryptedData
	if _, err := parsePDFXrefStream(decrypted.Dict, decrypted.Data); err == nil {
		return decrypted, nil
	}
	return decrypted, nil
}

func encryptPDFObjectValue(security *pdfStandardSecurity, fileKey []byte, id pdfObjectID, value pdfValue) (pdfValue, error) {
	switch v := value.(type) {
	case pdfDecryptedString:
		ciphertext, err := security.encryptObject(fileKey, id, []byte(v))
		if err != nil {
			return nil, err
		}
		return pdfHexString(hex.EncodeToString(ciphertext)), nil
	case pdfArray:
		out := make(pdfArray, len(v))
		for i, item := range v {
			encrypted, err := encryptPDFObjectValue(security, fileKey, id, item)
			if err != nil {
				return nil, err
			}
			out[i] = encrypted
		}
		return out, nil
	case pdfDict:
		out := make(pdfDict, len(v))
		for key, item := range v {
			encrypted, err := encryptPDFObjectValue(security, fileKey, id, item)
			if err != nil {
				return nil, err
			}
			out[key] = encrypted
		}
		return out, nil
	case pdfStreamObject:
		usesCryptFilter := pdfStreamUsesCryptFilter(v.Dict)
		dict, err := encryptPDFObjectValue(security, fileKey, id, v.Dict)
		if err != nil {
			return nil, err
		}
		stream := v
		stream.Dict = dict.(pdfDict)
		if !usesCryptFilter && security.encryptsStreamData(v.Dict) {
			encryptedData, err := security.encryptObject(fileKey, id, v.Data)
			if err != nil {
				return nil, err
			}
			stream.Data = encryptedData
		}
		return stream, nil
	default:
		return value, nil
	}
}

func decryptPDFObjectValue(security *pdfStandardSecurity, fileKey []byte, id pdfObjectID, value pdfValue) (pdfValue, error) {
	switch v := value.(type) {
	case pdfLiteralString, pdfHexString:
		plaintext, err := decryptPDFStringValue(security, fileKey, id, v)
		if err != nil {
			return nil, err
		}
		return pdfDecryptedString(plaintext), nil
	case pdfArray:
		out := make(pdfArray, len(v))
		for i, item := range v {
			decrypted, err := decryptPDFObjectValue(security, fileKey, id, item)
			if err != nil {
				return nil, err
			}
			out[i] = decrypted
		}
		return out, nil
	case pdfDict:
		out := make(pdfDict, len(v))
		for key, item := range v {
			decrypted, err := decryptPDFObjectValue(security, fileKey, id, item)
			if err != nil {
				return nil, err
			}
			out[key] = decrypted
		}
		return out, nil
	case pdfStreamObject:
		usesCryptFilter := pdfStreamUsesCryptFilter(v.Dict)
		dict, err := decryptPDFObjectValue(security, fileKey, id, v.Dict)
		if err != nil {
			return nil, err
		}
		decryptedData := bytes.Clone(v.Data)
		if !usesCryptFilter && security.encryptsStreamData(v.Dict) {
			var err error
			decryptedData, err = security.decryptObject(fileKey, id, v.Data)
			if err != nil {
				return nil, err
			}
		}
		stream := v
		stream.Dict = dict.(pdfDict)
		stream.Data = decryptedData
		return stream, nil
	default:
		return value, nil
	}
}

func (s *pdfStandardSecurity) encryptsStreamData(dict pdfDict) bool {
	if s == nil {
		return true
	}
	if s.revision >= 4 && !s.encryptMetadata && dictHasType(dict, "Metadata") {
		return false
	}
	return true
}

func pdfStreamUsesCryptFilter(dict pdfDict) bool {
	value, ok := dict["Filter"]
	if !ok {
		return false
	}
	switch v := value.(type) {
	case pdfName:
		return v == "Crypt"
	case pdfArray:
		for _, item := range v {
			if name, ok := item.(pdfName); ok && name == "Crypt" {
				return true
			}
		}
	}
	return false
}

func (g *pdfGraph) streamCryptHandler(id pdfObjectID) *pdfStreamCryptHandler {
	if g == nil || g.Encryption == nil || g.Encryption.security == nil {
		return nil
	}
	return &pdfStreamCryptHandler{
		Decrypt: func(name string, input []byte) ([]byte, error) {
			return g.Encryption.decryptStreamCryptFilter(id, name, input)
		},
		Encrypt: func(name string, input []byte) ([]byte, error) {
			return g.Encryption.encryptStreamCryptFilter(id, name, input)
		},
	}
}

func (e *pdfGraphEncryption) decryptStreamCryptFilter(id pdfObjectID, name string, input []byte) ([]byte, error) {
	if name == "" || name == "Identity" {
		return bytes.Clone(input), nil
	}
	if name != "StdCF" {
		return nil, unsupportedPDFEncryption("unsupported stream-level /Crypt filter /%s", name)
	}
	return e.security.decryptObject(e.fileKey, id, input)
}

func (e *pdfGraphEncryption) encryptStreamCryptFilter(id pdfObjectID, name string, input []byte) ([]byte, error) {
	if name == "" || name == "Identity" {
		return bytes.Clone(input), nil
	}
	if name != "StdCF" {
		return nil, unsupportedPDFEncryption("unsupported stream-level /Crypt filter /%s", name)
	}
	return e.security.encryptObject(e.fileKey, id, input)
}

func decryptPDFStringValue(security *pdfStandardSecurity, fileKey []byte, id pdfObjectID, value pdfValue) ([]byte, error) {
	ciphertext, ok, err := pdfStringBytes(value)
	if err != nil {
		return nil, err
	}
	if !ok {
		return nil, unsupportedPDFEncryption("encrypted object string has unsupported value type %T", value)
	}
	return security.decryptObject(fileKey, id, ciphertext)
}

func newPDFStandardSecurityFromDict(dict pdfDict, fileID []byte) (*pdfStandardSecurity, error) {
	if filter, ok := dictPDFName(dict, "Filter"); ok && filter != "Standard" {
		return nil, unsupportedPDFEncryption("unsupported encryption handler /Filter /%s", filter)
	}
	if subFilter, ok := dictPDFName(dict, "SubFilter"); ok && subFilter != "" {
		return nil, unsupportedPDFEncryption("unsupported encryption /SubFilter /%s", subFilter)
	}
	version, ok := dictInt(dict, "V")
	if !ok {
		return nil, unsupportedPDFEncryption("Standard Security dictionary is missing /V")
	}
	revision, ok := dictInt(dict, "R")
	if !ok {
		return nil, unsupportedPDFEncryption("Standard Security dictionary is missing /R")
	}
	permissions, ok := dictInt(dict, "P")
	if !ok {
		return nil, unsupportedPDFEncryption("Standard Security dictionary is missing /P")
	}
	if len(fileID) == 0 {
		return nil, unsupportedPDFEncryption("Standard Security requires the first trailer /ID value")
	}
	ownerKey, err := dictPDFStringBytes(dict, "O")
	if err != nil {
		return nil, err
	}
	userKey, err := dictPDFStringBytes(dict, "U")
	if err != nil {
		return nil, err
	}
	if len(ownerKey) != 32 {
		return nil, unsupportedPDFEncryption("Standard Security /O must be 32 bytes, got %d", len(ownerKey))
	}
	if len(userKey) != 32 {
		return nil, unsupportedPDFEncryption("Standard Security /U must be 32 bytes, got %d", len(userKey))
	}

	lengthBits := 40
	if value, ok := dictInt(dict, "Length"); ok {
		lengthBits = value
	}
	if revision == 2 {
		lengthBits = 40
	}
	if revision < 2 || revision > 4 {
		return nil, unsupportedPDFEncryption("unsupported Standard Security revision R=%d", revision)
	}
	if lengthBits < 40 || lengthBits > 128 || lengthBits%8 != 0 {
		return nil, unsupportedPDFEncryption("unsupported Standard Security key length %d bits", lengthBits)
	}
	if revision == 2 && version != 1 {
		return nil, unsupportedPDFEncryption("unsupported Standard Security R=2 with V=%d", version)
	}
	if revision == 3 && version != 2 {
		return nil, unsupportedPDFEncryption("unsupported Standard Security R=3 with V=%d", version)
	}
	if revision == 4 && version != 4 {
		return nil, unsupportedPDFEncryption("unsupported Standard Security R=4 with V=%d", version)
	}
	if version != 4 {
		if _, hasCF := dict["CF"]; hasCF {
			return nil, unsupportedPDFEncryption("unsupported encryption crypt filters require V=4")
		}
		if _, hasStmF := dict["StmF"]; hasStmF {
			return nil, unsupportedPDFEncryption("unsupported encryption /StmF requires V=4")
		}
		if _, hasStrF := dict["StrF"]; hasStrF {
			return nil, unsupportedPDFEncryption("unsupported encryption /StrF requires V=4")
		}
	}
	cryptMethod := pdfStandardCryptRC4
	if version == 4 {
		method, err := pdfStandardCryptMethodFromFilters(dict)
		if err != nil {
			return nil, err
		}
		cryptMethod = method
	}

	encryptMetadata := true
	if value, ok := dict["EncryptMetadata"].(bool); ok {
		encryptMetadata = value
	}
	return &pdfStandardSecurity{
		version:         version,
		revision:        revision,
		keyLengthBytes:  lengthBits / 8,
		ownerKey:        bytes.Clone(ownerKey),
		userKey:         bytes.Clone(userKey),
		permissions:     int32(permissions),
		fileID:          bytes.Clone(fileID),
		encryptMetadata: encryptMetadata,
		cryptMethod:     cryptMethod,
	}, nil
}

func pdfStandardCryptMethodFromFilters(dict pdfDict) (pdfStandardCryptMethod, error) {
	stmF, ok := dictPDFName(dict, "StmF")
	if !ok || stmF != "StdCF" {
		return "", unsupportedPDFEncryption("unsupported encryption crypt filter /StmF /%s", stmF)
	}
	strF, ok := dictPDFName(dict, "StrF")
	if !ok || strF != "StdCF" {
		return "", unsupportedPDFEncryption("unsupported encryption crypt filter /StrF /%s", strF)
	}
	if eff, ok := dictPDFName(dict, "EFF"); ok && eff != "StdCF" {
		return "", unsupportedPDFEncryption("unsupported encryption crypt filter /EFF /%s", eff)
	}
	cf, ok := dict["CF"].(pdfDict)
	if !ok {
		return "", unsupportedPDFEncryption("unsupported encryption crypt filter: missing /CF dictionary")
	}
	for name := range cf {
		if name != "StdCF" {
			return "", unsupportedPDFEncryption("unsupported encryption crypt filter /%s", name)
		}
	}
	stdCF, ok := cf["StdCF"].(pdfDict)
	if !ok {
		return "", unsupportedPDFEncryption("unsupported encryption crypt filter: missing /CF /StdCF dictionary")
	}
	cfm, ok := dictPDFName(stdCF, "CFM")
	if !ok {
		return "", unsupportedPDFEncryption("unsupported encryption crypt filter /StdCF missing /CFM")
	}
	var method pdfStandardCryptMethod
	switch cfm {
	case "V2":
		method = pdfStandardCryptRC4
	case "AESV2":
		method = pdfStandardCryptAESV2
	default:
		return "", unsupportedPDFEncryption("unsupported encryption crypt filter /StdCF /CFM /%s", cfm)
	}
	if authEvent, ok := dictPDFName(stdCF, "AuthEvent"); ok && authEvent != "DocOpen" {
		return "", unsupportedPDFEncryption("unsupported encryption crypt filter /StdCF /AuthEvent /%s", authEvent)
	}
	if length, ok := dictInt(stdCF, "Length"); ok && method == pdfStandardCryptAESV2 && length != 128 {
		return "", unsupportedPDFEncryption("unsupported encryption crypt filter /StdCF /Length %d", length)
	}
	return method, nil
}

func (s *pdfStandardSecurity) authenticateUserPassword(password []byte) ([]byte, bool, error) {
	if s == nil {
		return nil, false, unsupportedPDFEncryption("missing Standard Security state")
	}
	fileKey, err := s.deriveFileKey(password)
	if err != nil {
		return nil, false, err
	}
	expected, err := s.computeUserPasswordValue(fileKey)
	if err != nil {
		return nil, false, err
	}
	if s.revision == 2 {
		return fileKey, bytes.Equal(expected, s.userKey), nil
	}
	return fileKey, bytes.Equal(expected[:16], s.userKey[:16]), nil
}

func (s *pdfStandardSecurity) deriveFileKey(password []byte) ([]byte, error) {
	if s.keyLengthBytes <= 0 || s.keyLengthBytes > 16 {
		return nil, unsupportedPDFEncryption("unsupported Standard Security key length %d bytes", s.keyLengthBytes)
	}
	h := md5.New()
	h.Write(padPDFStandardPassword(password))
	h.Write(s.ownerKey)
	var permissions [4]byte
	binary.LittleEndian.PutUint32(permissions[:], uint32(s.permissions))
	h.Write(permissions[:])
	h.Write(s.fileID)
	if s.revision >= 4 && !s.encryptMetadata {
		h.Write([]byte{0xff, 0xff, 0xff, 0xff})
	}
	digest := h.Sum(nil)
	if s.revision >= 3 {
		for i := 0; i < 50; i++ {
			next := md5.Sum(digest[:s.keyLengthBytes])
			digest = next[:]
		}
	}
	return bytes.Clone(digest[:s.keyLengthBytes]), nil
}

func (s *pdfStandardSecurity) computeUserPasswordValue(fileKey []byte) ([]byte, error) {
	if s.revision == 2 {
		return rc4Crypt(fileKey, pdfStandardSecurityPasswordPadding)
	}
	h := md5.New()
	h.Write(pdfStandardSecurityPasswordPadding)
	h.Write(s.fileID)
	digest := h.Sum(nil)
	out, err := rc4Crypt(fileKey, digest)
	if err != nil {
		return nil, err
	}
	for i := 1; i <= 19; i++ {
		iterationKey := xorPDFStandardKey(fileKey, byte(i))
		out, err = rc4Crypt(iterationKey, out)
		if err != nil {
			return nil, err
		}
	}
	return out, nil
}

func (s *pdfStandardSecurity) objectKey(fileKey []byte, id pdfObjectID) ([]byte, error) {
	return s.objectKeyWithMethod(fileKey, id, pdfStandardCryptRC4)
}

func (s *pdfStandardSecurity) objectKeyWithMethod(fileKey []byte, id pdfObjectID, method pdfStandardCryptMethod) ([]byte, error) {
	if s == nil {
		return nil, unsupportedPDFEncryption("missing Standard Security state")
	}
	if len(fileKey) != s.keyLengthBytes {
		return nil, unsupportedPDFEncryption("file key length %d does not match Standard Security key length %d", len(fileKey), s.keyLengthBytes)
	}
	input := make([]byte, 0, len(fileKey)+9)
	input = append(input, fileKey...)
	input = append(input,
		byte(id.Number),
		byte(id.Number>>8),
		byte(id.Number>>16),
		byte(id.Generation),
		byte(id.Generation>>8),
	)
	if method == pdfStandardCryptAESV2 {
		input = append(input, 's', 'A', 'l', 'T')
	}
	digest := md5.Sum(input)
	keyLength := len(fileKey) + 5
	if keyLength > 16 {
		keyLength = 16
	}
	return bytes.Clone(digest[:keyLength]), nil
}

func (s *pdfStandardSecurity) decryptRC4Object(fileKey []byte, id pdfObjectID, ciphertext []byte) ([]byte, error) {
	objectKey, err := s.objectKey(fileKey, id)
	if err != nil {
		return nil, err
	}
	return rc4Crypt(objectKey, ciphertext)
}

func (s *pdfStandardSecurity) decryptObject(fileKey []byte, id pdfObjectID, ciphertext []byte) ([]byte, error) {
	switch s.cryptMethod {
	case "", pdfStandardCryptRC4:
		return s.decryptRC4Object(fileKey, id, ciphertext)
	case pdfStandardCryptAESV2:
		objectKey, err := s.objectKeyWithMethod(fileKey, id, pdfStandardCryptAESV2)
		if err != nil {
			return nil, err
		}
		return aesV2Decrypt(objectKey, ciphertext)
	default:
		return nil, unsupportedPDFEncryption("unsupported Standard Security crypt method %q", s.cryptMethod)
	}
}

func (s *pdfStandardSecurity) encryptObject(fileKey []byte, id pdfObjectID, plaintext []byte) ([]byte, error) {
	switch s.cryptMethod {
	case "", pdfStandardCryptRC4:
		return s.decryptRC4Object(fileKey, id, plaintext)
	case pdfStandardCryptAESV2:
		objectKey, err := s.objectKeyWithMethod(fileKey, id, pdfStandardCryptAESV2)
		if err != nil {
			return nil, err
		}
		return aesV2Encrypt(objectKey, plaintext)
	default:
		return nil, unsupportedPDFEncryption("unsupported Standard Security crypt method %q", s.cryptMethod)
	}
}

func aesV2Decrypt(key, ciphertext []byte) ([]byte, error) {
	if len(ciphertext) < aes.BlockSize || (len(ciphertext)-aes.BlockSize)%aes.BlockSize != 0 {
		return nil, unsupportedPDFEncryption("malformed AESV2 object ciphertext length %d", len(ciphertext))
	}
	block, err := aes.NewCipher(key)
	if err != nil {
		return nil, err
	}
	iv := ciphertext[:aes.BlockSize]
	data := bytes.Clone(ciphertext[aes.BlockSize:])
	cipher.NewCBCDecrypter(block, iv).CryptBlocks(data, data)
	return pkcs7Unpad(data, aes.BlockSize)
}

func aesV2Encrypt(key, plaintext []byte) ([]byte, error) {
	block, err := aes.NewCipher(key)
	if err != nil {
		return nil, err
	}
	iv := make([]byte, aes.BlockSize)
	if _, err := rand.Read(iv); err != nil {
		return nil, err
	}
	data := pkcs7Pad(plaintext, aes.BlockSize)
	cipher.NewCBCEncrypter(block, iv).CryptBlocks(data, data)
	out := make([]byte, 0, aes.BlockSize+len(data))
	out = append(out, iv...)
	out = append(out, data...)
	return out, nil
}

func pkcs7Pad(input []byte, blockSize int) []byte {
	padding := blockSize - len(input)%blockSize
	if padding == 0 {
		padding = blockSize
	}
	out := make([]byte, 0, len(input)+padding)
	out = append(out, input...)
	for i := 0; i < padding; i++ {
		out = append(out, byte(padding))
	}
	return out
}

func pkcs7Unpad(input []byte, blockSize int) ([]byte, error) {
	if len(input) == 0 || len(input)%blockSize != 0 {
		return nil, unsupportedPDFEncryption("malformed AESV2 object padding length %d", len(input))
	}
	padding := int(input[len(input)-1])
	if padding == 0 || padding > blockSize || padding > len(input) {
		return nil, unsupportedPDFEncryption("malformed AESV2 object padding value %d", padding)
	}
	for _, value := range input[len(input)-padding:] {
		if int(value) != padding {
			return nil, unsupportedPDFEncryption("malformed AESV2 object padding bytes")
		}
	}
	return bytes.Clone(input[:len(input)-padding]), nil
}

func padPDFStandardPassword(password []byte) []byte {
	out := make([]byte, 32)
	if len(password) > 32 {
		password = password[:32]
	}
	copy(out, password)
	copy(out[len(password):], pdfStandardSecurityPasswordPadding)
	return out
}

func xorPDFStandardKey(key []byte, value byte) []byte {
	out := bytes.Clone(key)
	for i := range out {
		out[i] ^= value
	}
	return out
}

func rc4Crypt(key, input []byte) ([]byte, error) {
	cipher, err := rc4.NewCipher(key)
	if err != nil {
		return nil, err
	}
	out := bytes.Clone(input)
	cipher.XORKeyStream(out, out)
	return out, nil
}

func dictPDFStringBytes(dict pdfDict, key string) ([]byte, error) {
	value, ok := dict[key]
	if !ok {
		return nil, unsupportedPDFEncryption("Standard Security dictionary is missing /%s", key)
	}
	out, ok, err := pdfStringBytes(value)
	if err != nil {
		return nil, err
	}
	if !ok {
		return nil, unsupportedPDFEncryption("Standard Security /%s must be a direct string", key)
	}
	return out, nil
}

func pdfStringBytes(value pdfValue) ([]byte, bool, error) {
	switch v := value.(type) {
	case pdfDecryptedString:
		return bytes.Clone(v), true, nil
	case pdfHexString:
		out, err := decodePDFHexStringBytes([]byte(v))
		return out, true, err
	case pdfLiteralString:
		return decodePDFLiteralStringBytes([]byte(v)), true, nil
	default:
		return nil, false, nil
	}
}

func decodePDFHexStringBytes(input []byte) ([]byte, error) {
	clean := make([]byte, 0, len(input))
	for _, c := range input {
		if isPDFSpace(c) {
			continue
		}
		clean = append(clean, c)
	}
	if len(clean)%2 == 1 {
		clean = append(clean, '0')
	}
	out := make([]byte, hex.DecodedLen(len(clean)))
	if _, err := hex.Decode(out, clean); err != nil {
		return nil, unsupportedPDFEncryption("malformed Standard Security hex string: %v", err)
	}
	return out, nil
}

func decodePDFLiteralStringBytes(input []byte) []byte {
	out := make([]byte, 0, len(input))
	for i := 0; i < len(input); i++ {
		c := input[i]
		if c != '\\' || i+1 >= len(input) {
			out = append(out, c)
			continue
		}
		i++
		switch escaped := input[i]; escaped {
		case 'n':
			out = append(out, '\n')
		case 'r':
			out = append(out, '\r')
		case 't':
			out = append(out, '\t')
		case 'b':
			out = append(out, '\b')
		case 'f':
			out = append(out, '\f')
		case '(', ')', '\\':
			out = append(out, escaped)
		case '\r':
			if i+1 < len(input) && input[i+1] == '\n' {
				i++
			}
		case '\n':
		default:
			if escaped < '0' || escaped > '7' {
				out = append(out, escaped)
				continue
			}
			value := int(escaped - '0')
			for digits := 1; digits < 3 && i+1 < len(input); digits++ {
				next := input[i+1]
				if next < '0' || next > '7' {
					break
				}
				i++
				value = value*8 + int(next-'0')
			}
			out = append(out, byte(value))
		}
	}
	return out
}

func unsupportedPDFEncryption(format string, args ...any) error {
	return fmt.Errorf("%w: %s", ErrEncryptedPDFUnsupportedAlgorithm, fmt.Sprintf(format, args...))
}
