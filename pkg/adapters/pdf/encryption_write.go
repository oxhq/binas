package pdf

import (
	"bytes"
	"crypto/md5"
	"crypto/rand"
	"encoding/hex"
	"errors"
	"fmt"
)

var ErrEncryptedPDFWriteUnsupported = errors.New("unsupported PDF: encryption write mode is not implemented")

type UnsupportedEncryptionWriteError struct {
	Mode    string
	Message string
}

func (e *UnsupportedEncryptionWriteError) Error() string {
	if e == nil {
		return ErrEncryptedPDFWriteUnsupported.Error()
	}
	if e.Message == "" {
		return ErrEncryptedPDFWriteUnsupported.Error()
	}
	return fmt.Sprintf("%s: %s", ErrEncryptedPDFWriteUnsupported, e.Message)
}

func (e *UnsupportedEncryptionWriteError) Unwrap() error {
	return ErrEncryptedPDFWriteUnsupported
}

type EncryptOptions struct {
	UserPassword  string
	OwnerPassword string
}

type ChangePasswordOptions struct {
	OldPassword      string
	NewUserPassword  string
	NewOwnerPassword string
}

type PublicKeyEncryptOptions struct {
	Recipients [][]byte
}

func DecryptToPlain(input []byte, password string) ([]byte, error) {
	graph, err := parsePDFGraphWithOptions(input, pdfGraphParseOptions{
		AllowEncryption: true,
		Password:        password,
	})
	if err != nil {
		return nil, err
	}
	graph.Boundaries.HasEncryption = false
	graph.Encryption = nil
	return writePDFGraphWithOptions(graph, pdfCanonicalWriteOptions{})
}

func Encrypt(input []byte, opts EncryptOptions) ([]byte, error) {
	graph, err := parsePDFGraph(input)
	if err != nil {
		if errors.Is(err, ErrEncryptedPDFPasswordRequired) {
			return nil, &UnsupportedEncryptionWriteError{
				Mode:    "standard_password",
				Message: "Encrypt supports only unencrypted input; use ChangePassword for encrypted Standard Security PDFs",
			}
		}
		return nil, err
	}
	if err := graph.applyStandardSecurityRevision2(opts.UserPassword, opts.OwnerPassword); err != nil {
		return nil, err
	}
	return writePDFGraphWithOptions(graph, pdfCanonicalWriteOptions{AllowEncryption: true})
}

func ChangePassword(input []byte, opts ChangePasswordOptions) ([]byte, error) {
	graph, err := parsePDFGraphWithOptions(input, pdfGraphParseOptions{
		AllowEncryption: true,
		Password:        opts.OldPassword,
	})
	if err != nil {
		return nil, err
	}
	if graph.Encryption == nil || graph.Encryption.security == nil {
		return nil, &UnsupportedEncryptionWriteError{
			Mode:    "standard_password_change",
			Message: "ChangePassword requires an encrypted Standard Security input",
		}
	}
	if graph.Encryption.encryptObject != nil {
		delete(graph.Objects, *graph.Encryption.encryptObject)
	}
	graph.Boundaries.HasEncryption = false
	graph.Encryption = nil
	if graph.Trailer != nil {
		delete(graph.Trailer, "Encrypt")
	}
	if err := graph.applyStandardSecurityRevision2(opts.NewUserPassword, opts.NewOwnerPassword); err != nil {
		return nil, err
	}
	return writePDFGraphWithOptions(graph, pdfCanonicalWriteOptions{AllowEncryption: true})
}

func PublicKeyEncrypt(input []byte, opts PublicKeyEncryptOptions) ([]byte, error) {
	return nil, &UnsupportedEncryptionWriteError{
		Mode:    "public_key",
		Message: "public-key encryption writer is not implemented",
	}
}

func (g *pdfGraph) applyStandardSecurityRevision2(userPassword, ownerPassword string) error {
	if g == nil {
		return errors.New("missing PDF graph")
	}
	fileID := make([]byte, 16)
	if _, err := rand.Read(fileID); err != nil {
		return err
	}
	ownerKey, err := standardSecurityRevision2OwnerKey([]byte(userPassword), []byte(ownerPassword))
	if err != nil {
		return err
	}
	security := &pdfStandardSecurity{
		version:         1,
		revision:        2,
		keyLengthBytes:  5,
		ownerKey:        ownerKey,
		permissions:     -44,
		fileID:          bytes.Clone(fileID),
		encryptMetadata: true,
		cryptMethod:     pdfStandardCryptRC4,
	}
	fileKey, err := security.deriveFileKey([]byte(userPassword))
	if err != nil {
		return err
	}
	userKey, err := security.computeUserPasswordValue(fileKey)
	if err != nil {
		return err
	}
	security.userKey = bytes.Clone(userKey)

	encryptID := nextPDFObjectID(g)
	g.Objects[encryptID] = &pdfIndirectObject{
		ID: encryptID,
		Value: pdfDict{
			"Filter": pdfName("Standard"),
			"V":      1,
			"R":      2,
			"O":      pdfHexString(hex.EncodeToString(ownerKey)),
			"U":      pdfHexString(hex.EncodeToString(userKey)),
			"P":      int(security.permissions),
		},
	}
	if g.Trailer == nil {
		g.Trailer = make(pdfDict)
	}
	g.Trailer["Encrypt"] = pdfRef{ID: encryptID}
	g.Trailer["ID"] = pdfArray{
		pdfHexString(hex.EncodeToString(fileID)),
		pdfHexString(hex.EncodeToString(fileID)),
	}
	g.Boundaries.HasEncryption = true
	g.Encryption = &pdfGraphEncryption{
		security:      security,
		fileKey:       bytes.Clone(fileKey),
		encryptObject: &encryptID,
	}
	return nil
}

func nextPDFObjectID(g *pdfGraph) pdfObjectID {
	next := 1
	if g != nil {
		for id := range g.Objects {
			if id.Number >= next {
				next = id.Number + 1
			}
		}
	}
	return pdfObjectID{Number: next, Generation: 0}
}

func standardSecurityRevision2OwnerKey(userPassword, ownerPassword []byte) ([]byte, error) {
	if len(ownerPassword) == 0 {
		ownerPassword = userPassword
	}
	digest := md5.Sum(padPDFStandardPassword(ownerPassword))
	return rc4Crypt(digest[:5], padPDFStandardPassword(userPassword))
}
