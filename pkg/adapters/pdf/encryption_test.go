package pdf

import (
	"bytes"
	"encoding/hex"
	"errors"
	"strings"
	"testing"
)

func TestStandardSecurityRevision2UserPasswordDecryptsRC4Object(t *testing.T) {
	security := mustPDFStandardSecurity(t, pdfDict{
		"Filter": pdfName("Standard"),
		"V":      1,
		"R":      2,
		"O":      pdfHexString("000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f"),
		"U":      pdfHexString("f2e39758794025127846c07006961602075fb11d4b5c227e0b27c9a2abb73e61"),
		"P":      -44,
	}, []byte("fixture-file-id1"))

	fileKey, ok, err := security.authenticateUserPassword([]byte("user"))
	if err != nil {
		t.Fatal(err)
	}
	if !ok {
		t.Fatal("user password did not authenticate")
	}
	assertHexBytes(t, "file key", fileKey, "317dc68b94")

	objectKey, err := security.objectKey(fileKey, pdfObjectID{Number: 12, Generation: 0})
	if err != nil {
		t.Fatal(err)
	}
	assertHexBytes(t, "object key", objectKey, "ad9e0e14aac91f8edcdd")

	plaintext, err := security.decryptRC4Object(fileKey, pdfObjectID{Number: 12, Generation: 0}, mustHex(t, "089e5e3660ee770d564de9201bf15209a62290"))
	if err != nil {
		t.Fatal(err)
	}
	if string(plaintext) != "Secret object bytes" {
		t.Fatalf("plaintext = %q", plaintext)
	}
}

func TestStandardSecurityRevision3UserPasswordChecksFirst16BytesAndDecryptsObject(t *testing.T) {
	security := mustPDFStandardSecurity(t, pdfDict{
		"Filter": pdfName("Standard"),
		"V":      2,
		"R":      3,
		"Length": 128,
		"O":      pdfHexString("f0e1d2c3b4a5968778695a4b3c2d1e0f00112233445566778899aabbccddeeff"),
		"U":      pdfHexString("7c389e5d61c357b7e42c60399b64c28aa0a1a2a3a4a5a6a7a8a9aaabacadaeaf"),
		"P":      -3904,
	}, []byte("revision3-fileid"))

	fileKey, ok, err := security.authenticateUserPassword([]byte("user"))
	if err != nil {
		t.Fatal(err)
	}
	if !ok {
		t.Fatal("user password did not authenticate")
	}
	assertHexBytes(t, "file key", fileKey, "23cc22898f1f64609809aa246046148d")

	_, ok, err = security.authenticateUserPassword([]byte("wrong"))
	if err != nil {
		t.Fatal(err)
	}
	if ok {
		t.Fatal("wrong user password authenticated")
	}

	plaintext, err := security.decryptRC4Object(fileKey, pdfObjectID{Number: 7, Generation: 2}, mustHex(t, "a6e496c9afd26f450de66d00bd6f82a2f70306cb0a"))
	if err != nil {
		t.Fatal(err)
	}
	if string(plaintext) != "Revision three secret" {
		t.Fatalf("plaintext = %q", plaintext)
	}
}

func TestStandardSecurityRevision4RC4CryptFilterUserPasswordDecryptsObject(t *testing.T) {
	security := mustPDFStandardSecurity(t, pdfDict{
		"Filter":          pdfName("Standard"),
		"V":               4,
		"R":               4,
		"Length":          128,
		"O":               pdfHexString("00112233445566778899aabbccddeeff102132435465768798a9babbdcddfeff"),
		"U":               pdfHexString("cc2a78aa2a17a179ab7fc41a992e19cfb0b1b2b3b4b5b6b7b8b9babbbcbdbebf"),
		"P":               -1028,
		"EncryptMetadata": false,
		"StmF":            pdfName("StdCF"),
		"StrF":            pdfName("StdCF"),
		"CF": pdfDict{
			"StdCF": pdfDict{"CFM": pdfName("V2")},
		},
	}, []byte("revision4-fileid"))

	fileKey, ok, err := security.authenticateUserPassword([]byte("user"))
	if err != nil {
		t.Fatal(err)
	}
	if !ok {
		t.Fatal("user password did not authenticate")
	}
	assertHexBytes(t, "file key", fileKey, "bdda2b081e4dbfbc87390f09115c49b8")
	objectKey, err := security.objectKey(fileKey, pdfObjectID{Number: 3, Generation: 4})
	if err != nil {
		t.Fatal(err)
	}
	assertHexBytes(t, "object key", objectKey, "b2290bef6d484d065b66ffd68091bfaa")

	plaintext, err := security.decryptRC4Object(fileKey, pdfObjectID{Number: 3, Generation: 4}, mustHex(t, "8d900e4e57b8a0cc6263c7c8132fd3c0ed4c38dd"))
	if err != nil {
		t.Fatal(err)
	}
	if string(plaintext) != "Revision four secret" {
		t.Fatalf("plaintext = %q", plaintext)
	}
}

func TestStandardSecurityRevision4AESV2CryptFilterUserPasswordDecryptsObject(t *testing.T) {
	fileID := []byte("revision4-aes-id")
	security := mustPDFStandardSecurity(t, standardSecurityAESV2FixtureDict(t, fileID), fileID)

	fileKey, ok, err := security.authenticateUserPassword([]byte("user"))
	if err != nil {
		t.Fatal(err)
	}
	if !ok {
		t.Fatal("user password did not authenticate")
	}
	if security.cryptMethod != pdfStandardCryptAESV2 {
		t.Fatalf("crypt method = %q, want AESV2", security.cryptMethod)
	}

	plaintext := []byte("Revision four AESV2 secret")
	ciphertext, err := security.encryptObject(fileKey, pdfObjectID{Number: 8, Generation: 0}, plaintext)
	if err != nil {
		t.Fatal(err)
	}
	if len(ciphertext) <= len(plaintext) || len(ciphertext)%16 != 0 || bytes.Contains(ciphertext, plaintext) {
		t.Fatalf("ciphertext length/content = %d/%x, want IV-prefixed padded AES ciphertext", len(ciphertext), ciphertext)
	}
	decrypted, err := security.decryptObject(fileKey, pdfObjectID{Number: 8, Generation: 0}, ciphertext)
	if err != nil {
		t.Fatal(err)
	}
	if string(decrypted) != string(plaintext) {
		t.Fatalf("decrypted = %q, want %q", decrypted, plaintext)
	}
}

func TestStandardSecurityRejectsUnsupportedCryptFilters(t *testing.T) {
	_, err := newPDFStandardSecurityFromDict(pdfDict{
		"Filter": pdfName("Standard"),
		"V":      4,
		"R":      4,
		"Length": 128,
		"O":      pdfHexString("00112233445566778899aabbccddeeff102132435465768798a9babbdcddfeff"),
		"U":      pdfHexString("cc2a78aa2a17a179ab7fc41a992e19cfb0b1b2b3b4b5b6b7b8b9babbbcbdbebf"),
		"P":      -1028,
		"StmF":   pdfName("StdCF"),
		"StrF":   pdfName("StdCF"),
		"CF": pdfDict{
			"StdCF": pdfDict{"CFM": pdfName("AESV3")},
		},
	}, []byte("revision4-fileid"))
	if !errors.Is(err, ErrEncryptedPDFUnsupportedAlgorithm) {
		t.Fatalf("error = %v, want ErrEncryptedPDFUnsupportedAlgorithm", err)
	}
	if !strings.Contains(err.Error(), "unsupported encryption crypt filter") || !strings.Contains(err.Error(), "AESV3") {
		t.Fatalf("error = %q, want explicit crypt filter rejection", err)
	}
}

func standardSecurityAESV2FixtureDict(t *testing.T, fileID []byte) pdfDict {
	t.Helper()
	ownerKey := mustHex(t, "00112233445566778899aabbccddeeff102132435465768798a9babbdcddfeff")
	seed := &pdfStandardSecurity{
		version:         4,
		revision:        4,
		keyLengthBytes:  16,
		ownerKey:        ownerKey,
		permissions:     -1028,
		fileID:          bytes.Clone(fileID),
		encryptMetadata: true,
		cryptMethod:     pdfStandardCryptAESV2,
	}
	fileKey, err := seed.deriveFileKey([]byte("user"))
	if err != nil {
		t.Fatal(err)
	}
	userKey, err := seed.computeUserPasswordValue(fileKey)
	if err != nil {
		t.Fatal(err)
	}
	userEntry := make([]byte, 32)
	copy(userEntry, userKey)
	copy(userEntry[16:], []byte("binas-aes-fixture"))
	return pdfDict{
		"Filter": pdfName("Standard"),
		"V":      4,
		"R":      4,
		"Length": 128,
		"O":      pdfHexString(hex.EncodeToString(ownerKey)),
		"U":      pdfHexString(hex.EncodeToString(userEntry)),
		"P":      -1028,
		"StmF":   pdfName("StdCF"),
		"StrF":   pdfName("StdCF"),
		"CF": pdfDict{
			"StdCF": pdfDict{
				"CFM":    pdfName("AESV2"),
				"Length": 128,
			},
		},
	}
}

func mustPDFStandardSecurity(t *testing.T, dict pdfDict, fileID []byte) *pdfStandardSecurity {
	t.Helper()
	security, err := newPDFStandardSecurityFromDict(dict, fileID)
	if err != nil {
		t.Fatal(err)
	}
	return security
}

func assertHexBytes(t *testing.T, label string, got []byte, want string) {
	t.Helper()
	if encoded := hex.EncodeToString(got); encoded != want {
		t.Fatalf("%s = %s, want %s", label, encoded, want)
	}
}

func mustHex(t *testing.T, input string) []byte {
	t.Helper()
	out, err := hex.DecodeString(input)
	if err != nil {
		t.Fatal(err)
	}
	return out
}
