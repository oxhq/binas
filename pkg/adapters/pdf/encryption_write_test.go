package pdf

import (
	"bytes"
	"errors"
	"fmt"
	"testing"

	"github.com/oxhq/binas/pkg/core"
)

func TestDecryptToPlainRemovesEncryptAndOpensWithoutPassword(t *testing.T) {
	input := standardEncryptedTextFixture(t, "08-15-2024")

	output, err := DecryptToPlain(input, "user")
	if err != nil {
		t.Fatal(err)
	}
	if bytes.Contains(output, []byte("/Encrypt")) {
		t.Fatalf("DecryptToPlain output still contains /Encrypt:\n%s", output)
	}
	if err := CheckSecurity(output, SecurityOptions{}); err != nil {
		t.Fatalf("CheckSecurity(plain output) = %v, want no encryption boundary", err)
	}
	tree, err := NewAdapter().Parse(output, core.ParseOptions{Strict: true})
	if err != nil {
		t.Fatal(err)
	}
	if matches := tree.Query(core.Match{Kind: KindTextShow, Text: "08-15-2024"}); len(matches) != 1 {
		t.Fatalf("plain output matches = %d, want 1", len(matches))
	}

	_, err = DecryptToPlain(input, "wrong")
	if !errors.Is(err, ErrEncryptedPDFPasswordRequired) {
		t.Fatalf("DecryptToPlain(wrong password) error = %v, want ErrEncryptedPDFPasswordRequired", err)
	}
}

func TestEncryptStandardSecurityPasswordRoundTripsAndHidesPlaintext(t *testing.T) {
	input := encryptionWritePlainTextFixture("OPEN-SESAME")

	output, err := Encrypt(input, EncryptOptions{UserPassword: "user", OwnerPassword: "owner"})
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Contains(output, []byte("/Encrypt")) {
		t.Fatalf("Encrypt() output missing /Encrypt:\n%s", output)
	}
	if bytes.Contains(output, []byte("OPEN-SESAME")) {
		t.Fatalf("Encrypt() output contains plaintext stream:\n%s", output)
	}
	if err := CheckSecurity(output, SecurityOptions{Password: "user"}); err != nil {
		t.Fatalf("CheckSecurity(correct password) = %v, want supported encrypted PDF", err)
	}
	plain, err := DecryptToPlain(output, "user")
	if err != nil {
		t.Fatalf("DecryptToPlain(correct password) = %v", err)
	}
	tree, err := NewAdapter().Parse(plain, core.ParseOptions{Strict: true})
	if err != nil {
		t.Fatal(err)
	}
	if matches := tree.Query(core.Match{Kind: KindTextShow, Text: "OPEN-SESAME"}); len(matches) != 1 {
		t.Fatalf("decrypted text matches = %d, want 1", len(matches))
	}
	_, err = DecryptToPlain(output, "wrong")
	if !errors.Is(err, ErrEncryptedPDFPasswordRequired) {
		t.Fatalf("DecryptToPlain(wrong password) error = %v, want ErrEncryptedPDFPasswordRequired", err)
	}
}

func TestChangePasswordReencryptsWithNewPassword(t *testing.T) {
	input, err := Encrypt(encryptionWritePlainTextFixture("ROTATED-SECRET"), EncryptOptions{UserPassword: "old-user", OwnerPassword: "old-owner"})
	if err != nil {
		t.Fatal(err)
	}

	output, err := ChangePassword(input, ChangePasswordOptions{OldPassword: "old-user", NewUserPassword: "new-user", NewOwnerPassword: "new-owner"})
	if err != nil {
		t.Fatal(err)
	}
	if bytes.Contains(output, []byte("ROTATED-SECRET")) {
		t.Fatalf("ChangePassword() output contains plaintext stream:\n%s", output)
	}
	_, err = DecryptToPlain(output, "old-user")
	if !errors.Is(err, ErrEncryptedPDFPasswordRequired) {
		t.Fatalf("DecryptToPlain(old password) error = %v, want ErrEncryptedPDFPasswordRequired", err)
	}
	plain, err := DecryptToPlain(output, "new-user")
	if err != nil {
		t.Fatalf("DecryptToPlain(new password) = %v", err)
	}
	tree, err := NewAdapter().Parse(plain, core.ParseOptions{Strict: true})
	if err != nil {
		t.Fatal(err)
	}
	if matches := tree.Query(core.Match{Kind: KindTextShow, Text: "ROTATED-SECRET"}); len(matches) != 1 {
		t.Fatalf("decrypted text matches = %d, want 1", len(matches))
	}
}

func TestPublicKeyEncryptFailsClosedWithStructuredUnsupportedError(t *testing.T) {
	input := testPDF("<< /Type /Catalog >>")

	_, err := PublicKeyEncrypt(input, PublicKeyEncryptOptions{Recipients: [][]byte{{0x01, 0x02}}})
	if !errors.Is(err, ErrEncryptedPDFWriteUnsupported) {
		t.Fatalf("PublicKeyEncrypt() error = %v, want ErrEncryptedPDFWriteUnsupported", err)
	}
	var unsupported *UnsupportedEncryptionWriteError
	if !errors.As(err, &unsupported) {
		t.Fatalf("PublicKeyEncrypt() error type = %T, want *UnsupportedEncryptionWriteError", err)
	}
	if unsupported.Mode != "public_key" {
		t.Fatalf("unsupported mode = %q, want public_key", unsupported.Mode)
	}
}

func encryptionWritePlainTextFixture(text string) []byte {
	content := []byte("BT\n(" + encodeLiteralString(text) + ") Tj\nET\n")
	return testPDF(
		"<< /Type /Catalog /Pages 2 0 R >>",
		"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
		"<< /Type /Page /Parent 2 0 R /Contents 4 0 R >>",
		fmt.Sprintf("<< /Length %d >>\nstream\n%sendstream", len(content), content),
	)
}
