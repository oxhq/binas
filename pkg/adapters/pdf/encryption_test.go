package pdf

import (
	"bytes"
	"crypto/aes"
	"crypto/cipher"
	"crypto/sha256"
	"crypto/sha512"
	"encoding/binary"
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

func TestStandardSecurityAESV3Revision5And6PasswordsRecoverFileKeyAndDecryptObject(t *testing.T) {
	for _, revision := range []int{5, 6} {
		t.Run("R"+string(rune('0'+revision)), func(t *testing.T) {
			fileKey := mustHex(t, "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f")
			dict := standardSecurityAESV3FixtureDict(t, revision, fileKey, []byte("user"), []byte("owner"), -1028, true)
			security := mustPDFStandardSecurity(t, dict, []byte("aesv3-file-id"))

			for _, password := range [][]byte{[]byte("user"), []byte("owner")} {
				recovered, ok, err := security.authenticateUserPassword(password)
				if err != nil {
					t.Fatal(err)
				}
				if !ok {
					t.Fatalf("password %q did not authenticate", password)
				}
				if !bytes.Equal(recovered, fileKey) {
					t.Fatalf("recovered file key = %x, want fixture file key", recovered)
				}
				if security.cryptMethod != pdfStandardCryptAESV3 {
					t.Fatalf("crypt method = %q, want AESV3", security.cryptMethod)
				}

				plaintext := []byte("Revision five/six AESV3 secret")
				ciphertext := deterministicAESV3FixtureEncryptedObject(t, fileKey, plaintext)
				decrypted, err := security.decryptObject(recovered, pdfObjectID{Number: 17, Generation: 0}, ciphertext)
				if err != nil {
					t.Fatal(err)
				}
				if !bytes.Equal(decrypted, plaintext) {
					t.Fatalf("decrypted = %q, want %q", decrypted, plaintext)
				}
			}
		})
	}
}

func TestStandardSecurityAESV3WrongPasswordFailsWithoutLeakingSecrets(t *testing.T) {
	fileID := []byte("aesv3-file-id")
	fileKey := mustHex(t, "102132435465768798a9babbdcddfeff00112233445566778899aabbccddeeff")
	userPassword := "correct-user"
	ownerPassword := "correct-owner"
	wrongPassword := "definitely-wrong"
	dict := standardSecurityAESV3FixtureDict(t, 6, fileKey, []byte(userPassword), []byte(ownerPassword), -1028, false)
	graph := &pdfGraph{
		Trailer: pdfDict{
			"Encrypt": dict,
			"ID": pdfArray{
				pdfHexString(hex.EncodeToString(fileID)),
				pdfHexString(hex.EncodeToString(fileID)),
			},
		},
		Objects: map[pdfObjectID]*pdfIndirectObject{},
	}

	_, err := graph.prepareStandardSecurityEncryption([]byte(wrongPassword))
	if !errors.Is(err, ErrEncryptedPDFPasswordRequired) {
		t.Fatalf("wrong-password error = %v, want ErrEncryptedPDFPasswordRequired", err)
	}
	message := err.Error()
	for _, secret := range []string{wrongPassword, userPassword, ownerPassword, hex.EncodeToString(fileKey)} {
		if strings.Contains(message, secret) {
			t.Fatalf("wrong-password error leaked secret %q: %s", secret, message)
		}
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

func standardSecurityAESV3FixtureDict(t *testing.T, revision int, fileKey, userPassword, ownerPassword []byte, permissions int, encryptMetadata bool) pdfDict {
	t.Helper()
	if len(fileKey) != 32 {
		t.Fatalf("file key length = %d, want 32", len(fileKey))
	}
	userPassword = truncateAESV3Password(userPassword)
	ownerPassword = truncateAESV3Password(ownerPassword)
	userValidationSalt := []byte{0x10, 0x11, 0x12, 0x13, byte(revision), 0x15, 0x16, 0x17}
	userKeySalt := []byte{0x20, 0x21, 0x22, 0x23, byte(revision), 0x25, 0x26, 0x27}
	ownerValidationSalt := []byte{0x30, 0x31, 0x32, 0x33, byte(revision), 0x35, 0x36, 0x37}
	ownerKeySalt := []byte{0x40, 0x41, 0x42, 0x43, byte(revision), 0x45, 0x46, 0x47}

	userHash := standardSecurityAESV3Hash(t, revision, userPassword, userValidationSalt, nil)
	userKeyHash := standardSecurityAESV3Hash(t, revision, userPassword, userKeySalt, nil)
	userValue := append(append(bytes.Clone(userHash), userValidationSalt...), userKeySalt...)
	userEncryptionKey := deterministicAESCBCNoPaddingEncrypt(t, userKeyHash, bytes.Repeat([]byte{0}, aes.BlockSize), fileKey)

	ownerHash := standardSecurityAESV3Hash(t, revision, ownerPassword, ownerValidationSalt, userValue[:48])
	ownerKeyHash := standardSecurityAESV3Hash(t, revision, ownerPassword, ownerKeySalt, userValue[:48])
	ownerValue := append(append(bytes.Clone(ownerHash), ownerValidationSalt...), ownerKeySalt...)
	ownerEncryptionKey := deterministicAESCBCNoPaddingEncrypt(t, ownerKeyHash, bytes.Repeat([]byte{0}, aes.BlockSize), fileKey)

	perms := deterministicAESV3Perms(t, fileKey, permissions, encryptMetadata)
	return pdfDict{
		"Filter":          pdfName("Standard"),
		"V":               5,
		"R":               revision,
		"Length":          256,
		"O":               pdfHexString(hex.EncodeToString(ownerValue)),
		"U":               pdfHexString(hex.EncodeToString(userValue)),
		"OE":              pdfHexString(hex.EncodeToString(ownerEncryptionKey)),
		"UE":              pdfHexString(hex.EncodeToString(userEncryptionKey)),
		"Perms":           pdfHexString(hex.EncodeToString(perms)),
		"P":               permissions,
		"EncryptMetadata": encryptMetadata,
		"StmF":            pdfName("StdCF"),
		"StrF":            pdfName("StdCF"),
		"CF": pdfDict{
			"StdCF": pdfDict{
				"CFM":       pdfName("AESV3"),
				"AuthEvent": pdfName("DocOpen"),
				"Length":    32,
			},
		},
	}
}

func standardSecurityAESV3Hash(t *testing.T, revision int, password, salt, udata []byte) []byte {
	t.Helper()
	h := sha256.New()
	h.Write(password)
	h.Write(salt)
	h.Write(udata)
	k := h.Sum(nil)
	if revision < 6 {
		return k
	}
	for count := 1; ; count++ {
		k1 := make([]byte, 0, (len(password)+len(k)+len(udata))*64)
		for i := 0; i < 64; i++ {
			k1 = append(k1, password...)
			k1 = append(k1, k...)
			k1 = append(k1, udata...)
		}
		encrypted := deterministicAESCBCNoPaddingEncrypt(t, k[:16], k[16:32], k1)
		sum := 0
		for _, b := range encrypted[:16] {
			sum += int(b)
		}
		switch sum % 3 {
		case 0:
			next := sha256.Sum256(encrypted)
			k = next[:]
		case 1:
			next := sha512.Sum384(encrypted)
			k = next[:]
		default:
			next := sha512.Sum512(encrypted)
			k = next[:]
		}
		if count >= 64 && encrypted[len(encrypted)-1] <= byte(count-32) {
			return bytes.Clone(k[:32])
		}
	}
}

func deterministicAESV3Perms(t *testing.T, fileKey []byte, permissions int, encryptMetadata bool) []byte {
	t.Helper()
	block := make([]byte, aes.BlockSize)
	binary.LittleEndian.PutUint32(block[:4], uint32(int32(permissions)))
	copy(block[4:8], []byte{0xff, 0xff, 0xff, 0xff})
	if encryptMetadata {
		block[8] = 'T'
	} else {
		block[8] = 'F'
	}
	copy(block[9:12], []byte("adb"))
	copy(block[12:16], []byte{0xa0, 0xa1, 0xa2, 0xa3})
	ciphertext := bytes.Clone(block)
	blockCipher, err := aes.NewCipher(fileKey)
	if err != nil {
		t.Fatal(err)
	}
	for offset := 0; offset < len(ciphertext); offset += aes.BlockSize {
		blockCipher.Encrypt(ciphertext[offset:offset+aes.BlockSize], ciphertext[offset:offset+aes.BlockSize])
	}
	return ciphertext
}

func deterministicAESV3FixtureEncryptedObject(t *testing.T, fileKey, plaintext []byte) []byte {
	t.Helper()
	iv := bytes.Repeat([]byte{0x5a}, aes.BlockSize)
	encrypted := deterministicAESCBCNoPaddingEncrypt(t, fileKey, iv, pkcs7Pad(plaintext, aes.BlockSize))
	return append(iv, encrypted...)
}

func deterministicAESCBCNoPaddingEncrypt(t *testing.T, key, iv, plaintext []byte) []byte {
	t.Helper()
	if len(iv) != aes.BlockSize || len(plaintext)%aes.BlockSize != 0 {
		t.Fatalf("invalid AES-CBC input: iv=%d plaintext=%d", len(iv), len(plaintext))
	}
	block, err := aes.NewCipher(key)
	if err != nil {
		t.Fatal(err)
	}
	out := bytes.Clone(plaintext)
	cipher.NewCBCEncrypter(block, iv).CryptBlocks(out, out)
	return out
}

func truncateAESV3Password(password []byte) []byte {
	if len(password) > 127 {
		return bytes.Clone(password[:127])
	}
	return bytes.Clone(password)
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
