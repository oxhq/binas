package main

import (
	"bytes"
	"compress/zlib"
	"crypto/aes"
	"crypto/cipher"
	"crypto/md5"
	"crypto/rc4"
	"crypto/sha256"
	"encoding/ascii85"
	"encoding/binary"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"testing"
)

func TestCLIInspectAndQueryAcceptFileFirstFlags(t *testing.T) {
	path := writeFixture(t)

	inspectOut := captureStdout(t, func() error {
		return run([]string{"inspect", path, "--format", "pdf", "--json"})
	})
	var inspect map[string]any
	if err := json.Unmarshal([]byte(inspectOut), &inspect); err != nil {
		t.Fatal(err)
	}
	if inspect["format"] != "pdf" {
		t.Fatalf("format = %v", inspect["format"])
	}

	queryOut := captureStdout(t, func() error {
		return run([]string{"query", path, "--format", "pdf", "--kind", "pdf.content.text_show", "--text", "08-15-2024", "--json"})
	})
	var query struct {
		Count int `json:"count"`
	}
	if err := json.Unmarshal([]byte(queryOut), &query); err != nil {
		t.Fatal(err)
	}
	if query.Count != 1 {
		t.Fatalf("query count = %d, want 1", query.Count)
	}
}

func TestCLIQueryMatchIndexReturnsIndexedMatchAsZeroBasedJSON(t *testing.T) {
	path := writeDuplicateTextFixture(t)

	stdout := captureStdout(t, func() error {
		return run([]string{"query", path, "--format", "pdf", "--kind", "pdf.content.text_show", "--text", "08-15-2024", "--match-index", "1", "--json"})
	})
	var result struct {
		Count        int `json:"count"`
		MatchIndex   int `json:"match_index"`
		TotalMatches int `json:"total_matches"`
		Matches      []struct {
			Span struct {
				Start int64 `json:"start"`
			} `json:"span"`
		} `json:"matches"`
	}
	if err := json.Unmarshal([]byte(stdout), &result); err != nil {
		t.Fatal(err)
	}
	if result.Count != 1 || result.MatchIndex != 1 || result.TotalMatches != 2 {
		t.Fatalf("query result = %+v, want count=1 match_index=1 total_matches=2", result)
	}
	if len(result.Matches) != 1 || result.Matches[0].Span.Start <= 0 {
		t.Fatalf("matches = %+v, want one indexed match with a span", result.Matches)
	}
}

func TestCLIQueryMetaFiltersByExactMetadataAndPreservesMatchIndexJSON(t *testing.T) {
	path := writeTextOperatorsFixture(t)

	stdout := captureStdout(t, func() error {
		return run([]string{"query", path, "--format", "pdf", "--kind", "pdf.content.text_show", "--text", "08-15-2024", "--meta", "operator=TJ", "--match-index", "0", "--json"})
	})
	var result struct {
		Count        int `json:"count"`
		MatchIndex   int `json:"match_index"`
		TotalMatches int `json:"total_matches"`
		Matches      []struct {
			Meta map[string]any `json:"meta"`
		} `json:"matches"`
	}
	if err := json.Unmarshal([]byte(stdout), &result); err != nil {
		t.Fatal(err)
	}
	if result.Count != 1 || result.MatchIndex != 0 || result.TotalMatches != 1 {
		t.Fatalf("query result = %+v, want count=1 match_index=0 total_matches=1", result)
	}
	if len(result.Matches) != 1 || result.Matches[0].Meta["operator"] != "TJ" {
		t.Fatalf("matches = %+v, want only TJ operator match", result.Matches)
	}
}

func TestCLIQueryMetaRejectsMalformedValue(t *testing.T) {
	path := writeFixture(t)

	_, err := captureStdoutAndError(t, func() error {
		return run([]string{"query", path, "--format", "pdf", "--meta", "bad", "--json"})
	})
	if err == nil {
		t.Fatal("query succeeded, want malformed --meta error")
	}
	if err.Error() != `invalid value "bad" for flag -meta: invalid --meta "bad": expected key=value` {
		t.Fatalf("error = %q", err)
	}
}

func TestCLIInspectJSONIncludesXrefObjectStreamMetadata(t *testing.T) {
	path := writeFixture(t)

	stdout := captureStdout(t, func() error {
		return run([]string{"inspect", path, "--format", "pdf", "--json"})
	})
	var result struct {
		Root struct {
			Xref struct {
				HasObjectStream   bool `json:"has_object_stream"`
				ObjectStreamCount int  `json:"object_stream_count"`
			} `json:"xref"`
		} `json:"root"`
	}
	if err := json.Unmarshal([]byte(stdout), &result); err != nil {
		t.Fatal(err)
	}
	if result.Root.Xref.HasObjectStream {
		t.Fatal("xref has_object_stream = true, want false")
	}
	if result.Root.Xref.ObjectStreamCount != 0 {
		t.Fatalf("xref object_stream_count = %d, want 0", result.Root.Xref.ObjectStreamCount)
	}
}

func TestCLIValidateReportsValidPDFAsJSON(t *testing.T) {
	path := writeFixture(t)

	stdout := captureStdout(t, func() error {
		return run([]string{"validate", path, "--format", "pdf", "--json"})
	})
	var result struct {
		Format   string   `json:"format"`
		Valid    bool     `json:"valid"`
		Errors   []string `json:"errors"`
		Warnings []string `json:"warnings"`
	}
	if err := json.Unmarshal([]byte(stdout), &result); err != nil {
		t.Fatal(err)
	}
	if result.Format != "pdf" {
		t.Fatalf("format = %q, want pdf", result.Format)
	}
	if !result.Valid {
		t.Fatalf("valid = false, errors = %v", result.Errors)
	}
	if len(result.Errors) != 0 {
		t.Fatalf("errors = %v, want none", result.Errors)
	}
	if result.Warnings == nil {
		t.Fatal("warnings should be an empty JSON array, got null")
	}
}

func TestCLIValidateReportsStrictParseErrorsAsJSON(t *testing.T) {
	path := writeMalformedFixture(t)

	stdout := captureStdout(t, func() error {
		return run([]string{"validate", path, "--format", "pdf", "--json"})
	})
	var result struct {
		Format   string   `json:"format"`
		Valid    bool     `json:"valid"`
		Errors   []string `json:"errors"`
		Warnings []string `json:"warnings"`
	}
	if err := json.Unmarshal([]byte(stdout), &result); err != nil {
		t.Fatal(err)
	}
	if result.Format != "pdf" {
		t.Fatalf("format = %q, want pdf", result.Format)
	}
	if result.Valid {
		t.Fatal("valid = true, want false")
	}
	if len(result.Errors) != 1 || result.Errors[0] != "malformed PDF: missing EOF marker" {
		t.Fatalf("errors = %v", result.Errors)
	}
	if result.Warnings == nil {
		t.Fatal("warnings should be an empty JSON array, got null")
	}
}

func TestCLIValidateObjectStreamJSONIncludesGraphMetadata(t *testing.T) {
	path := writeObjectStreamFixture(t)

	stdout := captureStdout(t, func() error {
		return run([]string{"validate", path, "--format", "pdf", "--json", "--fail-on-invalid"})
	})
	var result validationResult
	if err := json.Unmarshal([]byte(stdout), &result); err != nil {
		t.Fatal(err)
	}
	if result.Format != "pdf" {
		t.Fatalf("format = %q, want pdf", result.Format)
	}
	if !result.Valid {
		t.Fatalf("valid = false, errors = %v", result.Errors)
	}
	if len(result.Errors) != 0 {
		t.Fatalf("errors = %v, want none", result.Errors)
	}
	if result.Warnings == nil {
		t.Fatal("warnings should be an empty JSON array, got null")
	}
	if result.Root == nil {
		t.Fatal("root metadata is nil, want object stream xref metadata")
	}
	root := result.Root.(map[string]any)
	xref := root["xref"].(map[string]any)
	if xref["has_object_stream"] != true {
		t.Fatalf("xref has_object_stream = %v, want true", xref["has_object_stream"])
	}
	if xref["object_stream_count"] != float64(1) {
		t.Fatalf("xref object_stream_count = %v, want 1", xref["object_stream_count"])
	}
}

func TestCLIInspectJSONIncludesUnsupportedXrefStreamMetadata(t *testing.T) {
	path := writeXrefStreamFixture(t)

	stdout := captureStdout(t, func() error {
		return run([]string{"inspect", path, "--format", "pdf", "--json"})
	})
	var result struct {
		ParseError string `json:"parse_error"`
		Root       struct {
			Xref struct {
				HasStream   bool `json:"has_stream"`
				StreamCount int  `json:"stream_count"`
				Objects     []struct {
					Number int `json:"number"`
					Offset int `json:"offset"`
				} `json:"objects"`
				StreamObjects []struct {
					Number int `json:"number"`
					Offset int `json:"offset"`
				} `json:"stream_objects"`
			} `json:"xref"`
		} `json:"root"`
	}
	if err := json.Unmarshal([]byte(stdout), &result); err != nil {
		t.Fatal(err)
	}
	if result.ParseError != "unsupported PDF: xref streams are not implemented" {
		t.Fatalf("parse_error = %q", result.ParseError)
	}
	if !result.Root.Xref.HasStream || result.Root.Xref.StreamCount != 1 {
		t.Fatalf("xref stream metadata = %+v, want one xref stream", result.Root.Xref)
	}
	if len(result.Root.Xref.Objects) != 2 {
		t.Fatalf("xref objects = %+v, want catalog and xref stream objects", result.Root.Xref.Objects)
	}
	if len(result.Root.Xref.StreamObjects) != 1 || result.Root.Xref.StreamObjects[0].Number != 8 {
		t.Fatalf("xref stream_objects = %+v, want object 8", result.Root.Xref.StreamObjects)
	}
}

func TestCLIValidateMalformedPDFDefaultsToSuccess(t *testing.T) {
	path := writeMalformedFixture(t)

	stdout, err := captureStdoutAndError(t, func() error {
		return run([]string{"validate", path, "--format", "pdf"})
	})
	if err != nil {
		t.Fatalf("validate returned error: %v", err)
	}
	want := "pdf valid=false errors=malformed PDF: missing EOF marker\n"
	if stdout != want {
		t.Fatalf("stdout = %q, want %q", stdout, want)
	}
}

func TestCLIValidateFailOnInvalidReturnsErrorAfterOutput(t *testing.T) {
	for _, tc := range []struct {
		name       string
		args       []string
		wantStdout string
		wantJSON   bool
	}{
		{
			name:       "text",
			args:       []string{"--format", "pdf", "--fail-on-invalid"},
			wantStdout: "pdf valid=false errors=malformed PDF: missing EOF marker\n",
		},
		{
			name:     "json",
			args:     []string{"--format", "pdf", "--json", "--fail-on-invalid"},
			wantJSON: true,
		},
	} {
		t.Run(tc.name, func(t *testing.T) {
			path := writeMalformedFixture(t)
			args := append([]string{"validate", path}, tc.args...)

			stdout, err := captureStdoutAndError(t, func() error {
				return run(args)
			})
			if err == nil {
				t.Fatal("validate succeeded, want invalid parse error")
			}
			if err.Error() != "validation failed: malformed PDF: missing EOF marker" {
				t.Fatalf("error = %q", err)
			}
			if tc.wantJSON {
				var result validationResult
				if err := json.Unmarshal([]byte(stdout), &result); err != nil {
					t.Fatal(err)
				}
				if result.Valid {
					t.Fatal("valid = true, want false")
				}
				if len(result.Errors) != 1 || result.Errors[0] != "malformed PDF: missing EOF marker" {
					t.Fatalf("errors = %v", result.Errors)
				}
				return
			}
			if stdout != tc.wantStdout {
				t.Fatalf("stdout = %q, want %q", stdout, tc.wantStdout)
			}
		})
	}
}

func TestCLIValidateReportsAcroFormAnnotAndCMapWarnings(t *testing.T) {
	path := writeResidualBoundaryFixture(t)

	stdout := captureStdout(t, func() error {
		return run([]string{"validate", path, "--format", "pdf", "--json"})
	})
	var result validationResult
	if err := json.Unmarshal([]byte(stdout), &result); err != nil {
		t.Fatal(err)
	}
	if !result.Valid {
		t.Fatalf("valid = false, errors = %v", result.Errors)
	}
	for _, want := range []string{
		"PDF boundary detected: AcroForm field values require form set; simple text/choice widget appearances require --regenerate-appearance",
		"PDF boundary detected: annotation /Contents updates require annot set-contents; simple text-like appearances require --regenerate-appearance",
		"PDF boundary detected: font/CMap support is limited to page font-scoped ToUnicode CMaps for simple Tf flows, CMap-backed TJ arrays, and one unambiguous fallback; glyph metrics and layout are not verified",
	} {
		if !containsString(result.Warnings, want) {
			t.Fatalf("warnings = %v, missing %q", result.Warnings, want)
		}
	}
}

func TestCLIValidateFailOnInvalidReportsEncryptSignatureAndXFA(t *testing.T) {
	for _, tc := range []struct {
		name      string
		object    string
		wantError string
	}{
		{
			name:      "Encrypt",
			object:    "<< /Type /Catalog /Encrypt 2 0 R >>",
			wantError: "unsupported PDF: encrypted PDFs require an explicit password-capable path; Adapter.Parse does not decrypt",
		},
		{
			name:      "Signature",
			object:    "<< /Type /Catalog /SigFlags 3 /FT /Sig >>",
			wantError: "unsupported PDF: digital signatures require explicit invalidation mode; Adapter.Parse refuses silent signature invalidation",
		},
		{
			name:      "XFA",
			object:    "<< /Type /Catalog /AcroForm << /XFA 2 0 R >> >>",
			wantError: "unsupported PDF: XFA forms are not implemented",
		},
	} {
		t.Run(tc.name, func(t *testing.T) {
			path := writeUnsupportedBoundaryFixture(t, tc.object)
			stdout, err := captureStdoutAndError(t, func() error {
				return run([]string{"validate", path, "--format", "pdf", "--json", "--fail-on-invalid"})
			})
			if err == nil {
				t.Fatal("validate succeeded, want boundary error")
			}
			if err.Error() != "validation failed: "+tc.wantError {
				t.Fatalf("error = %q", err)
			}
			var result validationResult
			if err := json.Unmarshal([]byte(stdout), &result); err != nil {
				t.Fatal(err)
			}
			if result.Valid {
				t.Fatal("valid = true, want false")
			}
			if len(result.Errors) != 1 || result.Errors[0] != tc.wantError {
				t.Fatalf("errors = %v, want %q", result.Errors, tc.wantError)
			}
		})
	}
}

func TestCLIInspectEncryptedPDFJSONIncludesSecurityMetadata(t *testing.T) {
	path := writeEncryptedFixture(t)

	stdout := captureStdout(t, func() error {
		return run([]string{"inspect", path, "--format", "pdf", "--json"})
	})
	var result struct {
		Format     string `json:"format"`
		ParseError string `json:"parse_error"`
		Root       struct {
			Security struct {
				Encrypted  bool `json:"encrypted"`
				Encryption struct {
					Present         bool   `json:"present"`
					Filter          string `json:"filter"`
					SubFilter       string `json:"sub_filter"`
					V               int    `json:"v"`
					R               int    `json:"r"`
					Length          int    `json:"length"`
					EncryptMetadata *bool  `json:"encrypt_metadata"`
					ObjectNumber    int    `json:"object_number"`
				} `json:"encryption"`
			} `json:"security"`
		} `json:"root"`
	}
	if err := json.Unmarshal([]byte(stdout), &result); err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(result.ParseError, "encrypted PDFs require an explicit password-capable path") &&
		(!strings.Contains(result.ParseError, "unsupported PDF: encrypted PDFs are not parser-wired") || !strings.Contains(result.ParseError, "SubFilter=adbe.pkcs7.s5")) {
		t.Fatalf("parse_error = %q", result.ParseError)
	}
	enc := result.Root.Security.Encryption
	if !result.Root.Security.Encrypted || !enc.Present || enc.Filter != "Standard" || enc.SubFilter != "adbe.pkcs7.s5" {
		t.Fatalf("security metadata = %+v", result.Root.Security)
	}
	if enc.V != 4 || enc.R != 4 || enc.Length != 128 || enc.EncryptMetadata == nil || *enc.EncryptMetadata || enc.ObjectNumber != 2 {
		t.Fatalf("encryption metadata = %+v, want parsed Standard metadata", enc)
	}
}

func TestCLIValidateEncryptedPDFJSONIncludesSecurityMetadata(t *testing.T) {
	path := writeEncryptedFixture(t)

	stdout := captureStdout(t, func() error {
		return run([]string{"validate", path, "--format", "pdf", "--json"})
	})
	var result validationResult
	if err := json.Unmarshal([]byte(stdout), &result); err != nil {
		t.Fatal(err)
	}
	if result.Valid {
		t.Fatal("valid = true, want false")
	}
	root := result.Root.(map[string]any)
	security := root["security"].(map[string]any)
	encryption := security["encryption"].(map[string]any)
	if security["encrypted"] != true || encryption["filter"] != "Standard" || encryption["v"] != float64(4) || encryption["r"] != float64(4) {
		t.Fatalf("security metadata = %+v", security)
	}
}

func TestCLIValidateEncryptedPDFPasswordReportsUnsupportedAlgorithmMetadata(t *testing.T) {
	path := writeEncryptedFixture(t)

	stdout, err := captureStdoutAndError(t, func() error {
		return run([]string{"validate", path, "--format", "pdf", "--password", "secret", "--json", "--fail-on-invalid"})
	})
	if err == nil {
		t.Fatal("validate succeeded, want unsupported encryption algorithm error")
	}
	if !strings.Contains(err.Error(), "unsupported encryption algorithm/handler") || !strings.Contains(err.Error(), "Filter=Standard") {
		t.Fatalf("error = %q, want unsupported encryption algorithm metadata", err)
	}
	var result validationResult
	if err := json.Unmarshal([]byte(stdout), &result); err != nil {
		t.Fatal(err)
	}
	if result.Valid || len(result.Errors) != 1 || !strings.Contains(result.Errors[0], "unsupported encryption algorithm/handler") {
		t.Fatalf("validation result = %+v, want unsupported encryption algorithm error", result)
	}
}

func TestCLIPublicKeyEncryptedPDFReportsRecipientMetadataWithoutRecipientBytes(t *testing.T) {
	recipient := "01020304aabbccdd"
	path := writePublicKeyEncryptedFixture(t, recipient)

	inspectOut := captureStdout(t, func() error {
		return run([]string{"inspect", path, "--format", "pdf", "--json"})
	})
	assertStringDoesNotContain(t, inspectOut, "public-key inspect JSON", recipient)
	var inspect struct {
		ParseError string `json:"parse_error"`
		Root       struct {
			Security struct {
				Encryption struct {
					Filter         string `json:"filter"`
					SubFilter      string `json:"sub_filter"`
					PublicKey      bool   `json:"public_key"`
					RecipientCount int    `json:"recipient_count"`
				} `json:"encryption"`
			} `json:"security"`
		} `json:"root"`
	}
	if err := json.Unmarshal([]byte(inspectOut), &inspect); err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(inspect.ParseError, "unsupported encryption algorithm/handler") {
		t.Fatalf("parse_error = %q, want public-key unsupported encryption error", inspect.ParseError)
	}
	enc := inspect.Root.Security.Encryption
	if enc.Filter != "Adobe.PubSec" || enc.SubFilter != "adbe.pkcs7.s5" || !enc.PublicKey || enc.RecipientCount != 1 {
		t.Fatalf("public-key inspect metadata = %+v", enc)
	}

	validateOut, err := captureStdoutAndError(t, func() error {
		return run([]string{"validate", path, "--format", "pdf", "--json", "--fail-on-invalid"})
	})
	if err == nil || !strings.Contains(err.Error(), "unsupported encryption algorithm/handler") || strings.Contains(err.Error(), recipient) {
		t.Fatalf("validate error = %v, want public-key unsupported error without recipient bytes", err)
	}
	assertStringDoesNotContain(t, validateOut, "public-key validate JSON", recipient)
}

func TestCLIQueryEncryptedPDFWithPasswordFindsSelectableText(t *testing.T) {
	path := writeSupportedEncryptedFixture(t, "08-15-2024")

	validateOut := captureStdout(t, func() error {
		return run([]string{"validate", path, "--format", "pdf", "--password", "user", "--json", "--fail-on-invalid"})
	})
	var validation validationResult
	if err := json.Unmarshal([]byte(validateOut), &validation); err != nil {
		t.Fatal(err)
	}
	if !validation.Valid || len(validation.Errors) != 0 {
		t.Fatalf("validation = %+v, want valid encrypted PDF with password", validation)
	}

	inspectOut := captureStdout(t, func() error {
		return run([]string{"inspect", path, "--format", "pdf", "--password", "user", "--json"})
	})
	var inspect struct {
		Nodes int `json:"nodes"`
	}
	if err := json.Unmarshal([]byte(inspectOut), &inspect); err != nil {
		t.Fatal(err)
	}
	if inspect.Nodes == 0 {
		t.Fatalf("inspect nodes = %d, want parsed encrypted tree", inspect.Nodes)
	}

	stdout := captureStdout(t, func() error {
		return run([]string{"query", path, "--format", "pdf", "--password", "user", "--kind", "pdf.content.text_show", "--text", "08-15-2024", "--json"})
	})
	assertCLIQueryCount(t, stdout, 1)

	_, err := captureStdoutAndError(t, func() error {
		return run([]string{"query", path, "--format", "pdf", "--kind", "pdf.content.text_show", "--text", "08-15-2024", "--json"})
	})
	if err == nil || !strings.Contains(err.Error(), "encrypted PDFs require an explicit password") {
		t.Fatalf("query without password error = %v, want encrypted-PDF refusal", err)
	}
}

func TestCLIEditEncryptedPDFWithPasswordCanonicalReencryptsAndVerifies(t *testing.T) {
	path := writeSupportedEncryptedFixture(t, "08-15-2024")
	out := filepath.Join(t.TempDir(), "encrypted-out.pdf")

	stdout := captureStdout(t, func() error {
		return run([]string{
			"edit", path,
			"--format", "pdf",
			"--password", "user",
			"--rewrite", "auto",
			"--kind", "pdf.content.text_show",
			"--text", "08-15-2024",
			"--replace", "05-05-2026",
			"-o", out,
			"--json",
		})
	})
	var result struct {
		Report struct {
			Edit          string `json:"edit"`
			NodesModified int    `json:"nodes_modified"`
			FallbackUsed  bool   `json:"fallback_used"`
			OutputPath    string `json:"output_path"`
		} `json:"report"`
		Verification struct {
			ReparseOK      bool `json:"reparse_ok"`
			OldTextRemoved bool `json:"old_text_removed"`
			NewSelectable  bool `json:"new_text_selectable"`
			PageUnchanged  bool `json:"page_count_unchanged"`
		} `json:"verification"`
	}
	if err := json.Unmarshal([]byte(stdout), &result); err != nil {
		t.Fatal(err)
	}
	if result.Report.Edit != "pdf.canonical_content_stream_text_rewrite" || result.Report.NodesModified != 1 || result.Report.FallbackUsed || result.Report.OutputPath != out {
		t.Fatalf("report = %+v, want canonical encrypted edit", result.Report)
	}
	if !result.Verification.ReparseOK || !result.Verification.OldTextRemoved || !result.Verification.NewSelectable || !result.Verification.PageUnchanged {
		t.Fatalf("verification = %+v, want all text invariants true", result.Verification)
	}
	output, err := os.ReadFile(out)
	if err != nil {
		t.Fatal(err)
	}
	if bytes.Contains(output, []byte("05-05-2026")) {
		t.Fatal("output contains plaintext replacement; want re-encrypted content")
	}

	oldQueryOut := captureStdout(t, func() error {
		return run([]string{"query", out, "--format", "pdf", "--password", "user", "--kind", "pdf.content.text_show", "--text", "08-15-2024", "--json"})
	})
	assertCLIQueryCount(t, oldQueryOut, 0)

	newQueryOut := captureStdout(t, func() error {
		return run([]string{"query", out, "--format", "pdf", "--password", "user", "--kind", "pdf.content.text_show", "--text", "05-05-2026", "--json"})
	})
	assertCLIQueryCount(t, newQueryOut, 1)
}

func TestCLIEditAESV2EncryptedPDFWithPasswordCanonicalReencryptsAndVerifies(t *testing.T) {
	path := writeSupportedAESV2EncryptedFixture(t, "08-15-2024")
	out := filepath.Join(t.TempDir(), "encrypted-aes-out.pdf")

	initialQueryOut := captureStdout(t, func() error {
		return run([]string{"query", path, "--format", "pdf", "--password", "user", "--kind", "pdf.content.text_show", "--text", "08-15-2024", "--json"})
	})
	assertCLIQueryCount(t, initialQueryOut, 1)

	stdout := captureStdout(t, func() error {
		return run([]string{
			"edit", path,
			"--format", "pdf",
			"--password", "user",
			"--rewrite", "auto",
			"--kind", "pdf.content.text_show",
			"--text", "08-15-2024",
			"--replace", "05-05-2026",
			"-o", out,
			"--json",
		})
	})
	var result struct {
		Report struct {
			Edit          string `json:"edit"`
			NodesModified int    `json:"nodes_modified"`
			FallbackUsed  bool   `json:"fallback_used"`
			OutputPath    string `json:"output_path"`
		} `json:"report"`
		Verification struct {
			ReparseOK      bool `json:"reparse_ok"`
			OldTextRemoved bool `json:"old_text_removed"`
			NewSelectable  bool `json:"new_text_selectable"`
			PageUnchanged  bool `json:"page_count_unchanged"`
		} `json:"verification"`
	}
	if err := json.Unmarshal([]byte(stdout), &result); err != nil {
		t.Fatal(err)
	}
	if result.Report.Edit != "pdf.canonical_content_stream_text_rewrite" || result.Report.NodesModified != 1 || result.Report.FallbackUsed || result.Report.OutputPath != out {
		t.Fatalf("report = %+v, want canonical AESV2 encrypted edit", result.Report)
	}
	if !result.Verification.ReparseOK || !result.Verification.OldTextRemoved || !result.Verification.NewSelectable || !result.Verification.PageUnchanged {
		t.Fatalf("verification = %+v, want all text invariants true", result.Verification)
	}
	output, err := os.ReadFile(out)
	if err != nil {
		t.Fatal(err)
	}
	if bytes.Contains(output, []byte("05-05-2026")) {
		t.Fatal("AESV2 output contains plaintext replacement; want re-encrypted content")
	}

	newQueryOut := captureStdout(t, func() error {
		return run([]string{"query", out, "--format", "pdf", "--password", "user", "--kind", "pdf.content.text_show", "--text", "05-05-2026", "--json"})
	})
	assertCLIQueryCount(t, newQueryOut, 1)
}

func TestCLIEditAESV2EncryptedPDFStreamLevelCryptWithPasswordCanonicalReencryptsAndVerifies(t *testing.T) {
	path := writeSupportedAESV2StreamCryptEncryptedFixture(t, "08-15-2024")
	out := filepath.Join(t.TempDir(), "encrypted-aes-crypt-out.pdf")

	initialQueryOut := captureStdout(t, func() error {
		return run([]string{"query", path, "--format", "pdf", "--password", "user", "--kind", "pdf.content.text_show", "--text", "08-15-2024", "--json"})
	})
	assertCLIQueryCount(t, initialQueryOut, 1)

	stdout := captureStdout(t, func() error {
		return run([]string{
			"edit", path,
			"--format", "pdf",
			"--password", "user",
			"--rewrite", "auto",
			"--kind", "pdf.content.text_show",
			"--text", "08-15-2024",
			"--replace", "05-05-2026",
			"-o", out,
			"--json",
		})
	})
	var result struct {
		Report struct {
			Edit          string `json:"edit"`
			NodesModified int    `json:"nodes_modified"`
			FallbackUsed  bool   `json:"fallback_used"`
			OutputPath    string `json:"output_path"`
		} `json:"report"`
		Verification struct {
			ReparseOK      bool `json:"reparse_ok"`
			OldTextRemoved bool `json:"old_text_removed"`
			NewSelectable  bool `json:"new_text_selectable"`
			PageUnchanged  bool `json:"page_count_unchanged"`
		} `json:"verification"`
	}
	if err := json.Unmarshal([]byte(stdout), &result); err != nil {
		t.Fatal(err)
	}
	if result.Report.Edit != "pdf.canonical_content_stream_text_rewrite" || result.Report.NodesModified != 1 || result.Report.FallbackUsed || result.Report.OutputPath != out {
		t.Fatalf("report = %+v, want canonical AESV2 stream-level /Crypt edit", result.Report)
	}
	if !result.Verification.ReparseOK || !result.Verification.OldTextRemoved || !result.Verification.NewSelectable || !result.Verification.PageUnchanged {
		t.Fatalf("verification = %+v, want all text invariants true", result.Verification)
	}
	output, err := os.ReadFile(out)
	if err != nil {
		t.Fatal(err)
	}
	if bytes.Contains(output, []byte("05-05-2026")) {
		t.Fatal("AESV2 stream-level /Crypt output contains plaintext replacement; want re-encrypted content")
	}

	oldQueryOut := captureStdout(t, func() error {
		return run([]string{"query", out, "--format", "pdf", "--password", "user", "--kind", "pdf.content.text_show", "--text", "08-15-2024", "--json"})
	})
	assertCLIQueryCount(t, oldQueryOut, 0)

	newQueryOut := captureStdout(t, func() error {
		return run([]string{"query", out, "--format", "pdf", "--password", "user", "--kind", "pdf.content.text_show", "--text", "05-05-2026", "--json"})
	})
	assertCLIQueryCount(t, newQueryOut, 1)
}

func TestCLIEditEncryptedObjectStreamPDFWithPasswordCanonicalInflatesReencryptsAndVerifies(t *testing.T) {
	path := writeSupportedEncryptedObjectStreamFixture(t, "08-15-2024")
	out := filepath.Join(t.TempDir(), "encrypted-object-stream-out.pdf")

	initialQueryOut := captureStdout(t, func() error {
		return run([]string{"query", path, "--format", "pdf", "--password", "user", "--kind", "pdf.content.text_show", "--text", "08-15-2024", "--json"})
	})
	assertCLIQueryCount(t, initialQueryOut, 1)

	stdout := captureStdout(t, func() error {
		return run([]string{
			"edit", path,
			"--format", "pdf",
			"--password", "user",
			"--rewrite", "auto",
			"--kind", "pdf.content.text_show",
			"--text", "08-15-2024",
			"--replace", "05-05-2026",
			"-o", out,
			"--json",
		})
	})
	var result struct {
		Report struct {
			Edit          string `json:"edit"`
			NodesModified int    `json:"nodes_modified"`
			FallbackUsed  bool   `json:"fallback_used"`
			OutputPath    string `json:"output_path"`
		} `json:"report"`
		Verification struct {
			ReparseOK      bool `json:"reparse_ok"`
			OldTextRemoved bool `json:"old_text_removed"`
			NewSelectable  bool `json:"new_text_selectable"`
			PageUnchanged  bool `json:"page_count_unchanged"`
		} `json:"verification"`
	}
	if err := json.Unmarshal([]byte(stdout), &result); err != nil {
		t.Fatal(err)
	}
	if result.Report.Edit != "pdf.canonical_content_stream_text_rewrite" || result.Report.NodesModified != 1 || result.Report.FallbackUsed || result.Report.OutputPath != out {
		t.Fatalf("report = %+v, want canonical encrypted object-stream edit", result.Report)
	}
	if !result.Verification.ReparseOK || !result.Verification.OldTextRemoved || !result.Verification.NewSelectable || !result.Verification.PageUnchanged {
		t.Fatalf("verification = %+v, want all text invariants true", result.Verification)
	}
	output, err := os.ReadFile(out)
	if err != nil {
		t.Fatal(err)
	}
	if bytes.Contains(output, []byte("/Type /ObjStm")) {
		t.Fatal("canonical encrypted output preserved object stream container")
	}
	if bytes.Contains(output, []byte("05-05-2026")) {
		t.Fatal("encrypted object-stream output contains plaintext replacement")
	}

	oldQueryOut := captureStdout(t, func() error {
		return run([]string{"query", out, "--format", "pdf", "--password", "user", "--kind", "pdf.content.text_show", "--text", "08-15-2024", "--json"})
	})
	assertCLIQueryCount(t, oldQueryOut, 0)

	newQueryOut := captureStdout(t, func() error {
		return run([]string{"query", out, "--format", "pdf", "--password", "user", "--kind", "pdf.content.text_show", "--text", "05-05-2026", "--json"})
	})
	assertCLIQueryCount(t, newQueryOut, 1)
}

func TestCLIEditEncryptedXrefStreamPDFWithPasswordCanonicalInflatesReencryptsAndVerifies(t *testing.T) {
	path := writeSupportedEncryptedXrefStreamFixture(t, "08-15-2024")
	out := filepath.Join(t.TempDir(), "encrypted-xref-stream-out.pdf")

	initialQueryOut := captureStdout(t, func() error {
		return run([]string{"query", path, "--format", "pdf", "--password", "user", "--kind", "pdf.content.text_show", "--text", "08-15-2024", "--json"})
	})
	assertCLIQueryCount(t, initialQueryOut, 1)

	stdout := captureStdout(t, func() error {
		return run([]string{
			"edit", path,
			"--format", "pdf",
			"--password", "user",
			"--rewrite", "auto",
			"--kind", "pdf.content.text_show",
			"--text", "08-15-2024",
			"--replace", "05-05-2026",
			"-o", out,
			"--json",
		})
	})
	var result struct {
		Report struct {
			Edit          string `json:"edit"`
			NodesModified int    `json:"nodes_modified"`
			FallbackUsed  bool   `json:"fallback_used"`
			OutputPath    string `json:"output_path"`
		} `json:"report"`
		Verification struct {
			ReparseOK      bool `json:"reparse_ok"`
			OldTextRemoved bool `json:"old_text_removed"`
			NewSelectable  bool `json:"new_text_selectable"`
			PageUnchanged  bool `json:"page_count_unchanged"`
		} `json:"verification"`
	}
	if err := json.Unmarshal([]byte(stdout), &result); err != nil {
		t.Fatal(err)
	}
	if result.Report.Edit != "pdf.canonical_content_stream_text_rewrite" || result.Report.NodesModified != 1 || result.Report.FallbackUsed || result.Report.OutputPath != out {
		t.Fatalf("report = %+v, want canonical encrypted xref-stream edit", result.Report)
	}
	if !result.Verification.ReparseOK || !result.Verification.OldTextRemoved || !result.Verification.NewSelectable || !result.Verification.PageUnchanged {
		t.Fatalf("verification = %+v, want all text invariants true", result.Verification)
	}
	output, err := os.ReadFile(out)
	if err != nil {
		t.Fatal(err)
	}
	if bytes.Contains(output, []byte("/Type /XRef")) {
		t.Fatal("canonical encrypted output preserved xref stream container")
	}
	if bytes.Contains(output, []byte("05-05-2026")) {
		t.Fatal("encrypted xref-stream output contains plaintext replacement")
	}

	oldQueryOut := captureStdout(t, func() error {
		return run([]string{"query", out, "--format", "pdf", "--password", "user", "--kind", "pdf.content.text_show", "--text", "08-15-2024", "--json"})
	})
	assertCLIQueryCount(t, oldQueryOut, 0)

	newQueryOut := captureStdout(t, func() error {
		return run([]string{"query", out, "--format", "pdf", "--password", "user", "--kind", "pdf.content.text_show", "--text", "05-05-2026", "--json"})
	})
	assertCLIQueryCount(t, newQueryOut, 1)
}

func TestCLIEditAESV3EncryptedPDFsWithPasswordCanonicalReencryptsAndVerifies(t *testing.T) {
	cases := []struct {
		name       string
		fixture    func(*testing.T, string) string
		outputName string
	}{
		{
			name:       "normal-content",
			fixture:    writeSupportedAESV3EncryptedFixture,
			outputName: "encrypted-aesv3-out.pdf",
		},
		{
			name:       "object-stream",
			fixture:    writeSupportedAESV3EncryptedObjectStreamFixture,
			outputName: "encrypted-aesv3-object-stream-out.pdf",
		},
		{
			name:       "xref-stream",
			fixture:    writeSupportedAESV3EncryptedXrefStreamFixture,
			outputName: "encrypted-aesv3-xref-stream-out.pdf",
		},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			path := tc.fixture(t, "08-15-2024")
			out := filepath.Join(t.TempDir(), tc.outputName)

			validateOut := captureStdout(t, func() error {
				return run([]string{"validate", path, "--format", "pdf", "--password", "user", "--json", "--fail-on-invalid"})
			})
			var validation validationResult
			if err := json.Unmarshal([]byte(validateOut), &validation); err != nil {
				t.Fatal(err)
			}
			if !validation.Valid || len(validation.Errors) != 0 {
				t.Fatalf("AESV3 validation = %+v, want valid encrypted PDF with password", validation)
			}

			inspectOut := captureStdout(t, func() error {
				return run([]string{"inspect", path, "--format", "pdf", "--password", "user", "--json"})
			})
			var inspect struct {
				Nodes int    `json:"nodes"`
				Error string `json:"parse_error"`
			}
			if err := json.Unmarshal([]byte(inspectOut), &inspect); err != nil {
				t.Fatal(err)
			}
			if inspect.Nodes == 0 || inspect.Error != "" {
				t.Fatalf("AESV3 inspect = %+v, want parsed encrypted tree", inspect)
			}

			initialQueryOut := captureStdout(t, func() error {
				return run([]string{"query", path, "--format", "pdf", "--password", "user", "--kind", "pdf.content.text_show", "--text", "08-15-2024", "--json"})
			})
			assertCLIQueryCount(t, initialQueryOut, 1)

			_, err := captureStdoutAndError(t, func() error {
				return run([]string{"query", path, "--format", "pdf", "--password", "wrong", "--kind", "pdf.content.text_show", "--text", "08-15-2024", "--json"})
			})
			if err == nil || !strings.Contains(err.Error(), "supplied password did not authenticate") || strings.Contains(err.Error(), "wrong") {
				t.Fatalf("wrong-password query error = %v, want sanitized authentication failure", err)
			}

			stdout := captureStdout(t, func() error {
				return run([]string{
					"edit", path,
					"--format", "pdf",
					"--password", "user",
					"--rewrite", "auto",
					"--kind", "pdf.content.text_show",
					"--text", "08-15-2024",
					"--replace", "05-05-2026",
					"-o", out,
					"--json",
				})
			})
			assertCanonicalCLIEditResult(t, stdout)
			output, err := os.ReadFile(out)
			if err != nil {
				t.Fatal(err)
			}
			if bytes.Contains(output, []byte("08-15-2024")) || bytes.Contains(output, []byte("05-05-2026")) {
				t.Fatal("AESV3 output contains plaintext old/new text; want encrypted content")
			}

			_, err = captureStdoutAndError(t, func() error {
				return run([]string{"query", out, "--format", "pdf", "--kind", "pdf.content.text_show", "--text", "05-05-2026", "--json"})
			})
			if err == nil || !strings.Contains(err.Error(), "encrypted PDFs require an explicit password") {
				t.Fatalf("query without password on AESV3 output error = %v, want encrypted-PDF refusal", err)
			}

			oldQueryOut := captureStdout(t, func() error {
				return run([]string{"query", out, "--format", "pdf", "--password", "user", "--kind", "pdf.content.text_show", "--text", "08-15-2024", "--json"})
			})
			assertCLIQueryCount(t, oldQueryOut, 0)

			newQueryOut := captureStdout(t, func() error {
				return run([]string{"query", out, "--format", "pdf", "--password", "user", "--kind", "pdf.content.text_show", "--text", "05-05-2026", "--json"})
			})
			assertCLIQueryCount(t, newQueryOut, 1)

			_, err = captureStdoutAndError(t, func() error {
				return run([]string{"query", out, "--format", "pdf", "--password", "wrong", "--kind", "pdf.content.text_show", "--text", "05-05-2026", "--json"})
			})
			if err == nil || !strings.Contains(err.Error(), "supplied password did not authenticate") || strings.Contains(err.Error(), "wrong") {
				t.Fatalf("wrong-password output query error = %v, want sanitized authentication failure", err)
			}
		})
	}
}

func TestCLIEditEncryptedPDFSurgicalPasswordFailsClosed(t *testing.T) {
	path := writeSupportedEncryptedFixture(t, "08-15-2024")
	out := filepath.Join(t.TempDir(), "encrypted-out.pdf")

	_, err := captureStdoutAndError(t, func() error {
		return run([]string{
			"edit", path,
			"--format", "pdf",
			"--password", "user",
			"--rewrite", "surgical",
			"--kind", "pdf.content.text_show",
			"--text", "08-15-2024",
			"--replace", "05-05-2026",
			"-o", out,
			"--json",
		})
	})
	if err == nil || !strings.Contains(err.Error(), "surgical rewrite does not support encrypted PDFs") {
		t.Fatalf("error = %v, want surgical encrypted-PDF refusal", err)
	}
}

func TestCLIFormSetWritesVerifiedFieldUpdate(t *testing.T) {
	path := filepath.Join(t.TempDir(), "form.pdf")
	input := pdfFixture(
		"<< /Type /Catalog /AcroForm 2 0 R >>",
		"<< /Fields [3 0 R] >>",
		"<< /FT /Tx /T (payer.name) /V (Old Name) >>",
	)
	if err := os.WriteFile(path, input, 0644); err != nil {
		t.Fatal(err)
	}
	out := filepath.Join(t.TempDir(), "form-out.pdf")

	stdout := captureStdout(t, func() error {
		return run([]string{
			"form", "set", path,
			"--format", "pdf",
			"--field", "payer.name",
			"--value", "New (Name)",
			"-o", out,
			"--json",
		})
	})
	var result struct {
		Report struct {
			Edit             string `json:"edit"`
			NodesModified    int    `json:"nodes_modified"`
			OutputPath       string `json:"output_path"`
			AppearanceStatus string `json:"appearance_status"`
		} `json:"report"`
		Verification struct {
			ReparseOK bool `json:"reparse_ok"`
		} `json:"verification"`
	}
	if err := json.Unmarshal([]byte(stdout), &result); err != nil {
		t.Fatal(err)
	}
	if result.Report.Edit != "pdf.acroform_field_value_update" || result.Report.NodesModified != 1 || result.Report.OutputPath != out {
		t.Fatalf("result = %+v", result)
	}
	if result.Report.AppearanceStatus != "preserved" {
		t.Fatalf("appearance_status = %q, want preserved", result.Report.AppearanceStatus)
	}
	if !result.Verification.ReparseOK {
		t.Fatalf("verification = %+v", result.Verification)
	}
	written, err := os.ReadFile(out)
	if err != nil {
		t.Fatal(err)
	}
	if bytes.Contains(written, []byte("(Old Name)")) {
		t.Fatalf("old field value remains:\n%s", written)
	}
	if !bytes.Contains(written, []byte(`/V (New \(Name\))`)) {
		t.Fatalf("new field value missing:\n%s", written)
	}
	if !bytes.Contains(written, []byte("/NeedAppearances true")) {
		t.Fatalf("NeedAppearances was not set:\n%s", written)
	}
}

func TestCLIFormSetRegeneratesTextWidgetAppearance(t *testing.T) {
	path := filepath.Join(t.TempDir(), "form.pdf")
	input := pdfFixture(
		"<< /Type /Catalog /AcroForm 2 0 R >>",
		"<< /Fields [3 0 R] >>",
		"<< /FT /Tx /T (payer.name) /V (Old Name) /Rect [0 0 120 20] >>",
	)
	if err := os.WriteFile(path, input, 0644); err != nil {
		t.Fatal(err)
	}
	out := filepath.Join(t.TempDir(), "form-out.pdf")

	stdout := captureStdout(t, func() error {
		return run([]string{
			"form", "set", path,
			"--format", "pdf",
			"--field", "payer.name",
			"--value", "New Name",
			"--regenerate-appearance",
			"-o", out,
			"--json",
		})
	})
	var result struct {
		Report struct {
			AppearanceRegenerated bool   `json:"appearance_regenerated"`
			AppearanceStatus      string `json:"appearance_status"`
		} `json:"report"`
		Verification struct {
			ReparseOK             bool `json:"reparse_ok"`
			AppearanceRegenerated bool `json:"appearance_regenerated"`
		} `json:"verification"`
	}
	if err := json.Unmarshal([]byte(stdout), &result); err != nil {
		t.Fatal(err)
	}
	if !result.Report.AppearanceRegenerated || !result.Verification.ReparseOK || !result.Verification.AppearanceRegenerated {
		t.Fatalf("result = %+v", result)
	}
	if result.Report.AppearanceStatus != "regenerated" {
		t.Fatalf("appearance_status = %q, want regenerated", result.Report.AppearanceStatus)
	}
	written, err := os.ReadFile(out)
	if err != nil {
		t.Fatal(err)
	}
	for _, required := range [][]byte{[]byte("/AP << /N 4 0 R >>"), []byte("/Subtype /Form"), []byte("/Helv 10 Tf"), []byte("(New Name) Tj")} {
		if !bytes.Contains(written, required) {
			t.Fatalf("expected %q in regenerated output:\n%s", required, written)
		}
	}
}

func TestCLIFormSetUpdatesChoiceWithRegeneratedAppearance(t *testing.T) {
	path := filepath.Join(t.TempDir(), "choice-form.pdf")
	input := pdfFixture(
		"<< /Type /Catalog /AcroForm 2 0 R >>",
		"<< /Fields [3 0 R] >>",
		"<< /FT /Ch /T (payer.plan) /V (Basic) /Opt [(Basic) [(pro) (Pro Plan)]] /Rect [0 0 120 20] >>",
	)
	if err := os.WriteFile(path, input, 0644); err != nil {
		t.Fatal(err)
	}
	out := filepath.Join(t.TempDir(), "choice-form-out.pdf")

	stdout := captureStdout(t, func() error {
		return run([]string{
			"form", "set", path,
			"--field", "payer.plan",
			"--value", "Pro Plan",
			"--regenerate-appearance",
			"-o", out,
			"--json",
		})
	})
	var result struct {
		Report struct {
			AppearanceStatus      string `json:"appearance_status"`
			AppearanceRegenerated bool   `json:"appearance_regenerated"`
		} `json:"report"`
		Verification struct {
			FieldValueSet         bool `json:"field_value_set"`
			AppearanceRegenerated bool `json:"appearance_regenerated"`
		} `json:"verification"`
	}
	if err := json.Unmarshal([]byte(stdout), &result); err != nil {
		t.Fatal(err)
	}
	if result.Report.AppearanceStatus != "regenerated" || !result.Report.AppearanceRegenerated || !result.Verification.FieldValueSet || !result.Verification.AppearanceRegenerated {
		t.Fatalf("result = %+v, want regenerated choice appearance and verified value", result)
	}
	written, err := os.ReadFile(out)
	if err != nil {
		t.Fatal(err)
	}
	for _, want := range [][]byte{[]byte("/V (pro)"), []byte("/I [1]"), []byte("(Pro Plan) Tj")} {
		if !bytes.Contains(written, want) {
			t.Fatalf("choice output missing %q:\n%s", want, written)
		}
	}
}

func TestCLIFormSetUpdatesCheckboxAndReportsPreservedAppearance(t *testing.T) {
	path := filepath.Join(t.TempDir(), "checkbox-form.pdf")
	input := pdfFixture(
		"<< /Type /Catalog /AcroForm 2 0 R >>",
		"<< /Fields [3 0 R] >>",
		"<< /FT /Btn /T (payer.accept) /V /Off /AS /Off /AP << /N << /Off <<>> /Yes <<>> >> >> >>",
	)
	if err := os.WriteFile(path, input, 0644); err != nil {
		t.Fatal(err)
	}
	out := filepath.Join(t.TempDir(), "checkbox-form-out.pdf")

	stdout := captureStdout(t, func() error {
		return run([]string{"form", "set", path, "--field", "payer.accept", "--value", "true", "-o", out, "--json"})
	})
	var result struct {
		Report struct {
			AppearanceStatus      string `json:"appearance_status"`
			AppearanceRegenerated bool   `json:"appearance_regenerated"`
		} `json:"report"`
		Verification struct {
			FieldValueSet bool `json:"field_value_set"`
		} `json:"verification"`
	}
	if err := json.Unmarshal([]byte(stdout), &result); err != nil {
		t.Fatal(err)
	}
	if result.Report.AppearanceStatus != "preserved" || result.Report.AppearanceRegenerated || !result.Verification.FieldValueSet {
		t.Fatalf("result = %+v, want preserved checkbox appearance and verified value", result)
	}
	written, err := os.ReadFile(out)
	if err != nil {
		t.Fatal(err)
	}
	for _, want := range [][]byte{[]byte("/V /Yes"), []byte("/AS /Yes")} {
		if !bytes.Contains(written, want) {
			t.Fatalf("checkbox output missing %q:\n%s", want, written)
		}
	}
}

func TestCLIFormSetUpdatesRadioAndReportsPreservedAppearance(t *testing.T) {
	path := filepath.Join(t.TempDir(), "radio-form.pdf")
	input := pdfFixture(
		"<< /Type /Catalog /AcroForm 2 0 R >>",
		"<< /Fields [3 0 R] >>",
		"<< /FT /Btn /T (payer.plan) /V /Off /Kids [4 0 R 5 0 R] >>",
		"<< /AP << /N << /Off <<>> /Basic <<>> >> >> /AS /Off /Parent 3 0 R >>",
		"<< /AP << /N << /Off <<>> /Pro <<>> >> >> /AS /Off /Parent 3 0 R >>",
	)
	if err := os.WriteFile(path, input, 0644); err != nil {
		t.Fatal(err)
	}
	out := filepath.Join(t.TempDir(), "radio-form-out.pdf")

	stdout := captureStdout(t, func() error {
		return run([]string{"form", "set", path, "--field", "payer.plan", "--value", "Pro", "-o", out, "--json"})
	})
	var result struct {
		Report struct {
			AppearanceStatus string `json:"appearance_status"`
		} `json:"report"`
		Verification struct {
			FieldValueSet bool `json:"field_value_set"`
		} `json:"verification"`
	}
	if err := json.Unmarshal([]byte(stdout), &result); err != nil {
		t.Fatal(err)
	}
	if result.Report.AppearanceStatus != "preserved" || !result.Verification.FieldValueSet {
		t.Fatalf("result = %+v, want preserved radio appearance and verified value", result)
	}
	written, err := os.ReadFile(out)
	if err != nil {
		t.Fatal(err)
	}
	for _, want := range [][]byte{[]byte("/V /Pro"), []byte("/AS /Off"), []byte("/AS /Pro")} {
		if !bytes.Contains(written, want) {
			t.Fatalf("radio output missing %q:\n%s", want, written)
		}
	}
}

func TestCLIFormSetReportsUnsupportedRichWidgetAppearanceReasonAsJSON(t *testing.T) {
	path := filepath.Join(t.TempDir(), "rich-widget-regenerate-form.pdf")
	input := pdfFixture(
		"<< /Type /Catalog /AcroForm 2 0 R >>",
		"<< /Fields [3 0 R] >>",
		"<< /FT /Tx /Ff 33554432 /T (payer.name) /V (Old Name) /RV (<b>Old Name</b>) /Rect [0 0 120 20] >>",
	)
	if err := os.WriteFile(path, input, 0644); err != nil {
		t.Fatal(err)
	}
	out := filepath.Join(t.TempDir(), "rich-widget-regenerate-form-out.pdf")

	stdout, err := captureStdoutAndError(t, func() error {
		return run([]string{"form", "set", path, "--field", "payer.name", "--value", "New Name", "--regenerate-appearance", "-o", out, "--json"})
	})
	if err == nil {
		t.Fatal("form set succeeded, want unsupported rich-widget appearance regeneration error")
	}
	wantReason := "unsupported AcroForm appearance regeneration: rich text/default appearance cannot be regenerated safely"
	if err.Error() != wantReason {
		t.Fatalf("error = %q, want %q", err.Error(), wantReason)
	}
	var result struct {
		Error             string `json:"error"`
		AppearanceStatus  string `json:"appearance_status"`
		UnsupportedReason string `json:"unsupported_reason"`
	}
	if err := json.Unmarshal([]byte(stdout), &result); err != nil {
		t.Fatalf("json.Unmarshal(%q): %v", stdout, err)
	}
	if result.Error != wantReason || result.AppearanceStatus != "unsupported" || result.UnsupportedReason != wantReason {
		t.Fatalf("unsupported JSON = %+v, want exact reason", result)
	}
}

func TestCLIFormListEmitsStableJSONFields(t *testing.T) {
	path := filepath.Join(t.TempDir(), "form-list.pdf")
	input := pdfFixture(
		"<< /Type /Catalog /AcroForm 2 0 R >>",
		"<< /Fields [3 0 R] >>",
		"<< /FT /Btn /T (payer) /Kids [4 0 R] >>",
		"<< /AP << /N << /Off <<>> /Yes <<>> >> >> /AS /Off /T (choice) /V /Off /Parent 3 0 R >>",
	)
	if err := os.WriteFile(path, input, 0644); err != nil {
		t.Fatal(err)
	}

	stdout := captureStdout(t, func() error {
		return run([]string{"form", "list", path, "--format", "pdf", "--json"})
	})
	var result struct {
		Count  int `json:"count"`
		Fields []struct {
			Index                       int     `json:"index"`
			Name                        string  `json:"name"`
			ObjectNumber                *int    `json:"object_number"`
			ObjectGeneration            *int    `json:"object_generation"`
			FieldType                   string  `json:"field_type"`
			Value                       *string `json:"value"`
			KidCount                    int     `json:"kid_count"`
			ButtonWidgetAppearanceProof bool    `json:"button_widget_appearance_proof"`
		} `json:"fields"`
	}
	if err := json.Unmarshal([]byte(stdout), &result); err != nil {
		t.Fatal(err)
	}
	if result.Count != 2 || len(result.Fields) != 2 {
		t.Fatalf("result count = %+v, want two fields", result)
	}
	parent := result.Fields[0]
	if parent.Index != 0 || parent.Name != "payer" || parent.FieldType != "Btn" || parent.KidCount != 1 || !parent.ButtonWidgetAppearanceProof {
		t.Fatalf("parent field metadata = %+v", parent)
	}
	child := result.Fields[1]
	if child.Index != 1 || child.Name != "payer.choice" || child.ObjectNumber == nil || *child.ObjectNumber != 4 || child.ObjectGeneration == nil || *child.ObjectGeneration != 0 {
		t.Fatalf("child object metadata = %+v, want index 1 object 4 0", child)
	}
	if child.FieldType != "Btn" || child.Value == nil || *child.Value != "Off" || child.KidCount != 0 || !child.ButtonWidgetAppearanceProof {
		t.Fatalf("child field metadata = %+v", child)
	}
}

func TestCLIFormListPlainTextIncludesSemanticMetadata(t *testing.T) {
	path := filepath.Join(t.TempDir(), "form-list-text.pdf")
	input := pdfFixture(
		"<< /Type /Catalog /AcroForm 2 0 R >>",
		"<< /Fields [3 0 R 4 0 R] >>",
		"<< /FT /Ch /T (payer.plan) /TU (Payer Plan) /TM (payer_plan_export) /V (Basic) /DV (Basic Default) /Ff 131075 /Opt [(Basic) [(pro) (Pro Plan)]] >>",
		"<< /FT /Btn /T (payer.choice) /Kids [5 0 R] >>",
		"<< /AP << /N << /Off <<>> /Yes <<>> >> >> /AS /Off /T (accept) /V /Off /Parent 4 0 R >>",
	)
	if err := os.WriteFile(path, input, 0644); err != nil {
		t.Fatal(err)
	}

	stdout := captureStdout(t, func() error {
		return run([]string{"form", "list", path, "--format", "pdf"})
	})

	for _, want := range []string{
		`name="payer.plan"`,
		`flags=131075`,
		`flag_names=["read_only" "required"]`,
		`type_flag_names=["combo"]`,
		`alternate_name="Payer Plan"`,
		`mapping_name="payer_plan_export"`,
		`default_value="Basic Default"`,
		`options_count=2`,
		`options=["Basic" "Pro Plan"]`,
		`name="payer.choice.accept"`,
		`button_states=["Off" "Yes"]`,
	} {
		if !strings.Contains(stdout, want) {
			t.Fatalf("stdout missing %q:\n%s", want, stdout)
		}
	}
}

func TestCLIAnnotSetContentsWritesVerifiedUpdate(t *testing.T) {
	path := filepath.Join(t.TempDir(), "annot.pdf")
	input := pdfFixture(
		"<< /Type /Catalog /Pages 2 0 R >>",
		"<< /Type /Page /Annots [3 0 R] >>",
		"<< /Type /Annot /Subtype /Text /Rect [0 0 10 10] /Contents (old note) >>",
	)
	if err := os.WriteFile(path, input, 0644); err != nil {
		t.Fatal(err)
	}
	out := filepath.Join(t.TempDir(), "annot-out.pdf")

	stdout := captureStdout(t, func() error {
		return run([]string{
			"annot", "set-contents", path,
			"--format", "pdf",
			"--index", "0",
			"--contents", "new note",
			"-o", out,
			"--json",
		})
	})
	var result struct {
		Report struct {
			Edit                  string `json:"edit"`
			AnnotationIndex       int    `json:"annotation_index"`
			AppearanceRegenerated bool   `json:"appearance_regenerated"`
			AppearanceStatus      string `json:"appearance_status"`
			OutputPath            string `json:"output_path"`
		} `json:"report"`
		Verification struct {
			ReparseOK             bool `json:"reparse_ok"`
			ContentsUpdated       bool `json:"contents_updated"`
			PageUnchanged         bool `json:"page_count_unchanged"`
			AppearanceRegenerated bool `json:"appearance_regenerated"`
		} `json:"verification"`
	}
	if err := json.Unmarshal([]byte(stdout), &result); err != nil {
		t.Fatal(err)
	}
	if result.Report.Edit != "pdf.annotation_contents_update" || result.Report.AnnotationIndex != 0 || result.Report.OutputPath != out {
		t.Fatalf("result = %+v", result)
	}
	if result.Report.AppearanceRegenerated {
		t.Fatalf("appearance regeneration should remain explicit false: %+v", result.Report)
	}
	if result.Report.AppearanceStatus != "preserved" {
		t.Fatalf("appearance_status = %q, want preserved", result.Report.AppearanceStatus)
	}
	if !result.Verification.ReparseOK || !result.Verification.ContentsUpdated || !result.Verification.PageUnchanged || result.Verification.AppearanceRegenerated {
		t.Fatalf("verification = %+v", result.Verification)
	}
	written, err := os.ReadFile(out)
	if err != nil {
		t.Fatal(err)
	}
	if bytes.Contains(written, []byte("(old note)")) {
		t.Fatalf("old annotation contents remain:\n%s", written)
	}
	if !bytes.Contains(written, []byte("/Contents (new note)")) {
		t.Fatalf("new annotation contents missing:\n%s", written)
	}
}

func TestCLIAnnotListEmitsStableJSONCandidates(t *testing.T) {
	path := filepath.Join(t.TempDir(), "annot-list.pdf")
	input := pdfFixture(
		"<< /Type /Catalog /Pages 2 0 R >>",
		"<< /Type /Page /Annots [3 0 R << /Subtype /FreeText /Rect [0 0 20 20] /Contents (inline note) >>] >>",
		"<< /Type /Annot /Subtype /Text /Rect [0 0 10 10] /Contents (indirect note) /AP << /N 4 0 R >> >>",
		"<< /Length 0 >>\nstream\n\nendstream",
	)
	if err := os.WriteFile(path, input, 0644); err != nil {
		t.Fatal(err)
	}

	stdout := captureStdout(t, func() error {
		return run([]string{"annot", "list", path, "--format", "pdf", "--json"})
	})
	var result struct {
		Count       int `json:"count"`
		Annotations []struct {
			Index                int    `json:"index"`
			ObjectNumber         *int   `json:"object_number"`
			ObjectGeneration     *int   `json:"object_generation"`
			PageIndex            *int   `json:"page_index"`
			PageObjectNumber     *int   `json:"page_object_number"`
			PageObjectGeneration *int   `json:"page_object_generation"`
			Subtype              string `json:"subtype"`
			Contents             string `json:"contents"`
			HasAppearance        bool   `json:"has_appearance"`
		} `json:"annotations"`
	}
	if err := json.Unmarshal([]byte(stdout), &result); err != nil {
		t.Fatal(err)
	}
	if result.Count != 2 || len(result.Annotations) != 2 {
		t.Fatalf("result count = %+v, want two annotations", result)
	}
	first := result.Annotations[0]
	if first.Index != 0 || first.ObjectNumber == nil || *first.ObjectNumber != 3 || first.ObjectGeneration == nil || *first.ObjectGeneration != 0 {
		t.Fatalf("first object metadata = %+v, want index 0 object 3 0", first)
	}
	if first.PageIndex == nil || *first.PageIndex != 0 || first.PageObjectNumber == nil || *first.PageObjectNumber != 2 || first.PageObjectGeneration == nil || *first.PageObjectGeneration != 0 {
		t.Fatalf("first page metadata = %+v, want page index 0 object 2 0", first)
	}
	if first.Subtype != "Text" || first.Contents != "indirect note" || !first.HasAppearance {
		t.Fatalf("first annotation metadata = %+v", first)
	}
	second := result.Annotations[1]
	if second.Index != 1 || second.ObjectNumber != nil || second.ObjectGeneration != nil {
		t.Fatalf("second object metadata = %+v, want direct index 1", second)
	}
	if second.PageIndex == nil || *second.PageIndex != 0 || second.PageObjectNumber == nil || *second.PageObjectNumber != 2 || second.PageObjectGeneration == nil || *second.PageObjectGeneration != 0 {
		t.Fatalf("second page metadata = %+v, want page index 0 object 2 0", second)
	}
	if second.Subtype != "FreeText" || second.Contents != "inline note" || second.HasAppearance {
		t.Fatalf("second annotation metadata = %+v", second)
	}
}

func TestCLIAnnotListPlainTextIncludesSemanticMetadata(t *testing.T) {
	path := filepath.Join(t.TempDir(), "annot-list-text.pdf")
	input := pdfFixture(
		"<< /Type /Catalog /Pages 2 0 R >>",
		"<< /Type /Page /Annots [3 0 R] >>",
		"<< /Type /Annot /Subtype /Text /Rect [0.5 -1 20 20.25] /Contents (flagged note) /NM (note-001) /M (D:20260505090100-08'00') /T (David) /F 628 /AP << /N 4 0 R >> /C [1 0.5 0] /Border [0 0 2] /QuadPoints [0 0 10 0 10 10 0 10 1 1 11 1 11 11 1 11] >>",
		"<< /Length 0 >>\nstream\n\nendstream",
	)
	if err := os.WriteFile(path, input, 0644); err != nil {
		t.Fatal(err)
	}

	stdout := captureStdout(t, func() error {
		return run([]string{"annot", "list", path, "--format", "pdf"})
	})

	for _, want := range []string{
		`subtype=Text`,
		`contents="flagged note"`,
		`page_index=0`,
		`page_object=2 0 R`,
		`rect=[0.5 -1 20 20.25]`,
		`color=[1 0.5 0]`,
		`border=[0 0 2]`,
		`quad_points_count=2`,
		`name="note-001"`,
		`modified="D:20260505090100-08'00'"`,
		`title="David"`,
		`flags=628`,
		`flag_names=["print" "no_rotate" "no_view" "read_only" "locked_contents"]`,
	} {
		if !strings.Contains(stdout, want) {
			t.Fatalf("stdout missing %q:\n%s", want, stdout)
		}
	}
}

func TestCLIAnnotSetContentsCanRemoveAppearance(t *testing.T) {
	path := filepath.Join(t.TempDir(), "annot-remove-ap.pdf")
	input := pdfFixture(
		"<< /Type /Catalog /Pages 2 0 R >>",
		"<< /Type /Page /Annots [3 0 R] >>",
		"<< /Type /Annot /Subtype /Text /Rect [0 0 10 10] /Contents (old note) /AP << /N 4 0 R >> >>",
		"<< /Length 0 >>\nstream\n\nendstream",
	)
	if err := os.WriteFile(path, input, 0644); err != nil {
		t.Fatal(err)
	}
	out := filepath.Join(t.TempDir(), "annot-remove-ap-out.pdf")

	stdout := captureStdout(t, func() error {
		return run([]string{
			"annot", "set-contents", path,
			"--format", "pdf",
			"--index", "0",
			"--contents", "new note",
			"--remove-appearance",
			"-o", out,
			"--json",
		})
	})
	var result struct {
		Report struct {
			AppearanceRegenerated bool   `json:"appearance_regenerated"`
			AppearanceInvalidated bool   `json:"appearance_invalidated"`
			AppearanceRemoved     bool   `json:"appearance_removed"`
			AppearanceStatus      string `json:"appearance_status"`
		} `json:"report"`
		Verification struct {
			ReparseOK             bool `json:"reparse_ok"`
			AppearanceRegenerated bool `json:"appearance_regenerated"`
			AppearanceInvalidated bool `json:"appearance_invalidated"`
			AppearanceRemoved     bool `json:"appearance_removed"`
		} `json:"verification"`
	}
	if err := json.Unmarshal([]byte(stdout), &result); err != nil {
		t.Fatal(err)
	}
	if result.Report.AppearanceRegenerated || !result.Report.AppearanceInvalidated || !result.Report.AppearanceRemoved {
		t.Fatalf("report = %+v", result.Report)
	}
	if result.Report.AppearanceStatus != "removed" {
		t.Fatalf("appearance_status = %q, want removed", result.Report.AppearanceStatus)
	}
	if !result.Verification.ReparseOK || result.Verification.AppearanceRegenerated || !result.Verification.AppearanceInvalidated || !result.Verification.AppearanceRemoved {
		t.Fatalf("verification = %+v", result.Verification)
	}
	written, err := os.ReadFile(out)
	if err != nil {
		t.Fatal(err)
	}
	if bytes.Contains(written, []byte("/AP ")) {
		t.Fatalf("annotation appearance remains:\n%s", written)
	}
}

func TestCLIAnnotSetContentsCanRegenerateAppearance(t *testing.T) {
	path := filepath.Join(t.TempDir(), "annot-regenerate-ap.pdf")
	input := pdfFixture(
		"<< /Type /Catalog /Pages 2 0 R >>",
		"<< /Type /Page /Annots [3 0 R] >>",
		"<< /Type /Annot /Subtype /FreeText /Rect [0 0 80 20] /Contents (old note) >>",
	)
	if err := os.WriteFile(path, input, 0644); err != nil {
		t.Fatal(err)
	}
	out := filepath.Join(t.TempDir(), "annot-regenerate-ap-out.pdf")

	stdout := captureStdout(t, func() error {
		return run([]string{
			"annot", "set-contents", path,
			"--format", "pdf",
			"--index", "0",
			"--contents", "visible note",
			"--regenerate-appearance",
			"-o", out,
			"--json",
		})
	})
	var result struct {
		Report struct {
			AppearanceRegenerated bool   `json:"appearance_regenerated"`
			AppearanceStatus      string `json:"appearance_status"`
		} `json:"report"`
		Verification struct {
			ReparseOK             bool `json:"reparse_ok"`
			AppearanceRegenerated bool `json:"appearance_regenerated"`
		} `json:"verification"`
	}
	if err := json.Unmarshal([]byte(stdout), &result); err != nil {
		t.Fatal(err)
	}
	if !result.Report.AppearanceRegenerated {
		t.Fatalf("report = %+v, want regenerated appearance", result.Report)
	}
	if result.Report.AppearanceStatus != "regenerated" {
		t.Fatalf("appearance_status = %q, want regenerated", result.Report.AppearanceStatus)
	}
	if !result.Verification.ReparseOK || !result.Verification.AppearanceRegenerated {
		t.Fatalf("verification = %+v", result.Verification)
	}
	written, err := os.ReadFile(out)
	if err != nil {
		t.Fatal(err)
	}
	for _, want := range [][]byte{
		[]byte("/AP << /N 4 0 R >>"),
		[]byte("/Subtype /Form"),
		[]byte("/BBox [0 0 80 20]"),
		[]byte("/BaseFont /Helvetica"),
		[]byte("(visible note) Tj"),
	} {
		if !bytes.Contains(written, want) {
			t.Fatalf("written PDF missing %q:\n%s", want, written)
		}
	}
}

func TestCLIAnnotSetContentsReportsUnsupportedAppearanceReasonAsJSON(t *testing.T) {
	path := filepath.Join(t.TempDir(), "annot-unsupported-regenerate.pdf")
	input := pdfFixture(
		"<< /Type /Catalog /Pages 2 0 R >>",
		"<< /Type /Page /Annots [3 0 R] >>",
		"<< /Type /Annot /Subtype /Stamp /Rect [0 0 80 20] /Contents (old note) >>",
	)
	if err := os.WriteFile(path, input, 0644); err != nil {
		t.Fatal(err)
	}
	out := filepath.Join(t.TempDir(), "annot-unsupported-regenerate-out.pdf")

	stdout, err := captureStdoutAndError(t, func() error {
		return run([]string{
			"annot", "set-contents", path,
			"--index", "0",
			"--contents", "visible note",
			"--regenerate-appearance",
			"-o", out,
			"--json",
		})
	})
	if err == nil {
		t.Fatal("annot set-contents succeeded, want unsupported appearance regeneration error")
	}
	wantReason := `cannot regenerate annotation appearance: unsupported annotation subtype "Stamp"`
	if err.Error() != wantReason {
		t.Fatalf("error = %q, want %q", err.Error(), wantReason)
	}
	var result struct {
		Error             string `json:"error"`
		AppearanceStatus  string `json:"appearance_status"`
		UnsupportedReason string `json:"unsupported_reason"`
	}
	if err := json.Unmarshal([]byte(stdout), &result); err != nil {
		t.Fatalf("json.Unmarshal(%q): %v", stdout, err)
	}
	if result.Error != wantReason || result.AppearanceStatus != "unsupported" || result.UnsupportedReason != wantReason {
		t.Fatalf("unsupported JSON = %+v, want exact reason", result)
	}
}

func TestCLIAnnotSetContentsRejectsAppearanceFlagConflict(t *testing.T) {
	path := filepath.Join(t.TempDir(), "annot-flag-conflict.pdf")
	input := pdfFixture(
		"<< /Type /Catalog /Pages 2 0 R >>",
		"<< /Type /Page /Annots [3 0 R] >>",
		"<< /Type /Annot /Subtype /Text /Rect [0 0 10 10] /Contents (old note) /AP << /N 4 0 R >> >>",
		"<< /Length 0 >>\nstream\n\nendstream",
	)
	if err := os.WriteFile(path, input, 0644); err != nil {
		t.Fatal(err)
	}
	out := filepath.Join(t.TempDir(), "annot-conflict-out.pdf")

	_, err := captureStdoutAndError(t, func() error {
		return run([]string{
			"annot", "set-contents", path,
			"--contents", "new note",
			"--remove-appearance",
			"--regenerate-appearance",
			"-o", out,
		})
	})
	if err == nil {
		t.Fatal("expected conflicting appearance flags to fail")
	}
	if err.Error() != "use only one of --remove-appearance or --regenerate-appearance" {
		t.Fatalf("error = %q", err)
	}
	if _, statErr := os.Stat(out); !os.IsNotExist(statErr) {
		t.Fatalf("output path exists after flag conflict, stat err = %v", statErr)
	}
}

func TestCLIXFAReplaceWritesVerifiedPacketUpdate(t *testing.T) {
	path := filepath.Join(t.TempDir(), "xfa.pdf")
	input := pdfFixture("<< /Type /Catalog /AcroForm << /XFA (<template>old</template>) >> >>")
	if err := os.WriteFile(path, input, 0644); err != nil {
		t.Fatal(err)
	}
	out := filepath.Join(t.TempDir(), "xfa-out.pdf")

	stdout := captureStdout(t, func() error {
		return run([]string{
			"xfa", "replace", path,
			"--format", "pdf",
			"--text", "<template>old</template>",
			"--replace", "<template>new</template>",
			"-o", out,
			"--json",
		})
	})
	var result struct {
		Report struct {
			Edit          string `json:"edit"`
			NodesModified int    `json:"nodes_modified"`
			OutputPath    string `json:"output_path"`
		} `json:"report"`
		Verification struct {
			ReparseOK      bool `json:"reparse_ok"`
			OldTextRemoved bool `json:"old_text_removed"`
			NewSelectable  bool `json:"new_text_selectable"`
		} `json:"verification"`
	}
	if err := json.Unmarshal([]byte(stdout), &result); err != nil {
		t.Fatal(err)
	}
	if result.Report.Edit != "pdf.xfa_replace" || result.Report.NodesModified != 1 || result.Report.OutputPath != out {
		t.Fatalf("result = %+v", result)
	}
	if !result.Verification.ReparseOK || !result.Verification.OldTextRemoved || !result.Verification.NewSelectable {
		t.Fatalf("verification = %+v", result.Verification)
	}
	written, err := os.ReadFile(out)
	if err != nil {
		t.Fatal(err)
	}
	if bytes.Contains(written, []byte("<template>old</template>")) {
		t.Fatalf("old XFA packet remains:\n%s", written)
	}
	if !bytes.Contains(written, []byte("<template>new</template>")) {
		t.Fatalf("new XFA packet missing:\n%s", written)
	}
}

func TestCLIXFAReplaceMatchIndexSelectsPacket(t *testing.T) {
	path := filepath.Join(t.TempDir(), "xfa-index.pdf")
	input := pdfFixture("<< /Type /Catalog /AcroForm << /XFA [(template) (<template>old</template>) (datasets) (<datasets>old</datasets>)] >> >>")
	if err := os.WriteFile(path, input, 0644); err != nil {
		t.Fatal(err)
	}
	out := filepath.Join(t.TempDir(), "xfa-index-out.pdf")

	stdout := captureStdout(t, func() error {
		return run([]string{
			"xfa", "replace", path,
			"--format", "pdf",
			"--text", "old",
			"--replace", "new",
			"--match-index", "1",
			"-o", out,
			"--json",
		})
	})
	var result struct {
		Report struct {
			Edit       string `json:"edit"`
			MatchIndex int    `json:"match_index"`
		} `json:"report"`
		Verification struct {
			ReparseOK bool `json:"reparse_ok"`
		} `json:"verification"`
	}
	if err := json.Unmarshal([]byte(stdout), &result); err != nil {
		t.Fatal(err)
	}
	if result.Report.Edit != "pdf.xfa_replace" || result.Report.MatchIndex != 1 {
		t.Fatalf("result = %+v", result)
	}
	if !result.Verification.ReparseOK {
		t.Fatalf("verification = %+v", result.Verification)
	}
	written, err := os.ReadFile(out)
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Contains(written, []byte("<template>old</template>")) {
		t.Fatalf("unselected XFA packet changed:\n%s", written)
	}
	if !bytes.Contains(written, []byte("<datasets>new</datasets>")) || bytes.Contains(written, []byte("<datasets>old</datasets>")) {
		t.Fatalf("selected XFA packet was not updated:\n%s", written)
	}
}

func TestCLIXFAReplaceSelectorSelectsPacketKind(t *testing.T) {
	path := filepath.Join(t.TempDir(), "xfa-kind.pdf")
	input := pdfFixture("<< /Type /Catalog /AcroForm << /XFA [(template) (<template>old</template>) (datasets) (<datasets>old</datasets>)] >> >>")
	if err := os.WriteFile(path, input, 0644); err != nil {
		t.Fatal(err)
	}
	out := filepath.Join(t.TempDir(), "xfa-kind-out.pdf")

	stdout := captureStdout(t, func() error {
		return run([]string{
			"xfa", "replace", path,
			"--format", "pdf",
			"--text", "old",
			"--replace", "new",
			"--packet-kind", "datasets",
			"-o", out,
			"--json",
		})
	})
	var result struct {
		Report struct {
			Edit string `json:"edit"`
		} `json:"report"`
		Verification struct {
			ReparseOK bool `json:"reparse_ok"`
		} `json:"verification"`
	}
	if err := json.Unmarshal([]byte(stdout), &result); err != nil {
		t.Fatal(err)
	}
	if result.Report.Edit != "pdf.xfa_replace" || !result.Verification.ReparseOK {
		t.Fatalf("result = %+v", result)
	}
	written, err := os.ReadFile(out)
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Contains(written, []byte("<template>old</template>")) {
		t.Fatalf("unselected XFA packet changed:\n%s", written)
	}
	if !bytes.Contains(written, []byte("<datasets>new</datasets>")) || bytes.Contains(written, []byte("<datasets>old</datasets>")) {
		t.Fatalf("selected XFA packet was not updated:\n%s", written)
	}
}

func TestCLIXFAListEmitsStableJSONPackets(t *testing.T) {
	path := filepath.Join(t.TempDir(), "xfa-list.pdf")
	input := pdfFixture(
		"<< /Type /Catalog /AcroForm << /XFA [(template) 2 0 R (datasets) (<datasets>direct</datasets>)] >> >>",
		"<< /Length 27 >>\nstream\n<template>stream</template>\nendstream",
	)
	if err := os.WriteFile(path, input, 0644); err != nil {
		t.Fatal(err)
	}

	stdout := captureStdout(t, func() error {
		return run([]string{"xfa", "list", path, "--format", "pdf", "--json"})
	})
	var result struct {
		Count   int `json:"count"`
		Packets []struct {
			Index            int    `json:"index"`
			Label            string `json:"label"`
			ObjectNumber     *int   `json:"object_number"`
			ObjectGeneration *int   `json:"object_generation"`
			IsStream         bool   `json:"is_stream"`
			TextLength       int    `json:"text_length"`
			Preview          string `json:"preview"`
		} `json:"packets"`
	}
	if err := json.Unmarshal([]byte(stdout), &result); err != nil {
		t.Fatal(err)
	}
	if result.Count != 2 || len(result.Packets) != 2 {
		t.Fatalf("result = %+v, want two XFA packets", result)
	}
	first := result.Packets[0]
	if first.Index != 0 || first.Label != "template" || first.ObjectNumber == nil || *first.ObjectNumber != 2 || first.ObjectGeneration == nil || *first.ObjectGeneration != 0 || !first.IsStream {
		t.Fatalf("first packet = %+v", first)
	}
	if first.TextLength != len("<template>stream</template>") || first.Preview != "<template>stream</template>" {
		t.Fatalf("first packet text = %+v", first)
	}
	second := result.Packets[1]
	if second.Index != 1 || second.Label != "datasets" || second.ObjectNumber != nil || second.ObjectGeneration != nil || second.IsStream {
		t.Fatalf("second packet = %+v", second)
	}
	if second.TextLength != len("<datasets>direct</datasets>") || second.Preview != "<datasets>direct</datasets>" {
		t.Fatalf("second packet text = %+v", second)
	}
}

func TestCLIXFAListSelectorFiltersJSONPackets(t *testing.T) {
	path := filepath.Join(t.TempDir(), "xfa-list-filtered.pdf")
	input := pdfFixture("<< /Type /Catalog /AcroForm << /XFA [(template) (<template>direct</template>) /datasets (<datasets>direct</datasets>)] >> >>")
	if err := os.WriteFile(path, input, 0644); err != nil {
		t.Fatal(err)
	}

	stdout := captureStdout(t, func() error {
		return run([]string{"xfa", "list", path, "--format", "pdf", "--packet-kind", "datasets", "--label", "datasets", "--json"})
	})
	var result struct {
		Count   int `json:"count"`
		Packets []struct {
			Index      int    `json:"index"`
			Label      string `json:"label"`
			PacketKind string `json:"packet_kind"`
			Preview    string `json:"preview"`
		} `json:"packets"`
	}
	if err := json.Unmarshal([]byte(stdout), &result); err != nil {
		t.Fatal(err)
	}
	if result.Count != 1 || len(result.Packets) != 1 {
		t.Fatalf("result = %+v, want one filtered packet", result)
	}
	if result.Packets[0].Index != 1 || result.Packets[0].Label != "datasets" || result.Packets[0].PacketKind != "datasets" || result.Packets[0].Preview != "<datasets>direct</datasets>" {
		t.Fatalf("filtered packet = %+v", result.Packets[0])
	}
}

func TestCLIXFAListPlainTextIncludesSemanticMetadata(t *testing.T) {
	path := filepath.Join(t.TempDir(), "xfa-list-text.pdf")
	input := pdfFixture(
		"<< /Type /Catalog /AcroForm << /XFA [(template) 2 0 R (datasets) (<datasets>direct</datasets>)] >> >>",
		"<< /Length 6 /Filter /FlateDecode /DecodeParms 12 >>\nstream\nabcdef\nendstream",
	)
	if err := os.WriteFile(path, input, 0644); err != nil {
		t.Fatal(err)
	}

	stdout := captureStdout(t, func() error {
		return run([]string{"xfa", "list", path, "--format", "pdf"})
	})

	for _, want := range []string{
		`label=template`,
		`stream=true`,
		`byte_length=0`,
		`packet_kind=template`,
		`filter=/FlateDecode`,
		`decode_parms="12"`,
		`decode_error="unsupported stream: /DecodeParms must be a dictionary, array, or null"`,
		`label=datasets`,
		`stream=false`,
		`length=27`,
		`byte_length=27`,
		`preview="<datasets>direct</datasets>"`,
		`packet_kind=datasets`,
	} {
		if !strings.Contains(stdout, want) {
			t.Fatalf("stdout missing %q:\n%s", want, stdout)
		}
	}
}

func TestCLIXFADatasetsEmitsJSONFields(t *testing.T) {
	path := filepath.Join(t.TempDir(), "xfa-datasets.pdf")
	input := pdfFixture(
		"<< /Type /Catalog /AcroForm << /XFA [(template) (<template>ignored</template>) (datasets) (<xfa:datasets xmlns:xfa=\"http://www.xfa.org/schema/xfa-data/1.0/\"><xfa:data><form><name>Alice</name><address><city>Tijuana</city></address></form></xfa:data></xfa:datasets>)] >> >>",
	)
	if err := os.WriteFile(path, input, 0644); err != nil {
		t.Fatal(err)
	}

	stdout := captureStdout(t, func() error {
		return run([]string{"xfa", "datasets", path, "--format", "pdf", "--json"})
	})
	var result struct {
		Count  int `json:"count"`
		Fields []struct {
			PacketIndex int    `json:"packet_index"`
			Label       string `json:"label"`
			Path        string `json:"path"`
			Value       string `json:"value"`
		} `json:"fields"`
	}
	if err := json.Unmarshal([]byte(stdout), &result); err != nil {
		t.Fatal(err)
	}
	if result.Count != 2 || len(result.Fields) != 2 {
		t.Fatalf("result = %+v, want two dataset fields", result)
	}
	first := result.Fields[0]
	if first.PacketIndex != 1 || first.Label != "datasets" || first.Path != "form.name" || first.Value != "Alice" {
		t.Fatalf("first field = %+v", first)
	}
	second := result.Fields[1]
	if second.PacketIndex != 1 || second.Label != "datasets" || second.Path != "form.address.city" || second.Value != "Tijuana" {
		t.Fatalf("second field = %+v", second)
	}
}

func TestCLIXFADatasetsSelectorFiltersJSONFields(t *testing.T) {
	path := filepath.Join(t.TempDir(), "xfa-datasets-filtered.pdf")
	input := pdfFixture(
		"<< /Type /Catalog /AcroForm << /XFA [(left) (<datasets><data><form><name>Left</name></form></data></datasets>) (datasets) (<datasets><data><form><name>Datasets</name></form></data></datasets>) (xdp) (<xdp:xdp xmlns:xdp=\"http://ns.adobe.com/xdp/\"><datasets><data><form><name>XDP</name></form></data></datasets></xdp:xdp>)] >> >>",
	)
	if err := os.WriteFile(path, input, 0644); err != nil {
		t.Fatal(err)
	}

	stdout := captureStdout(t, func() error {
		return run([]string{"xfa", "datasets", path, "--format", "pdf", "--packet-kind", "datasets", "--label", "datasets", "--json"})
	})
	var result struct {
		Count  int `json:"count"`
		Fields []struct {
			PacketIndex int    `json:"packet_index"`
			Label       string `json:"label"`
			Path        string `json:"path"`
			Value       string `json:"value"`
		} `json:"fields"`
	}
	if err := json.Unmarshal([]byte(stdout), &result); err != nil {
		t.Fatal(err)
	}
	if result.Count != 1 || len(result.Fields) != 1 {
		t.Fatalf("result = %+v, want one filtered field", result)
	}
	field := result.Fields[0]
	if field.PacketIndex != 1 || field.Label != "datasets" || field.Path != "form.name" || field.Value != "Datasets" {
		t.Fatalf("filtered field = %+v", field)
	}
}

func TestCLIXFADatasetsPlainTextPrintsPathValueLines(t *testing.T) {
	path := filepath.Join(t.TempDir(), "xfa-datasets-text.pdf")
	input := pdfFixture(
		"<< /Type /Catalog /AcroForm << /XFA [(datasets) (<datasets><data><invoice><number>INV-7</number><total>42</total></invoice></data></datasets>)] >> >>",
	)
	if err := os.WriteFile(path, input, 0644); err != nil {
		t.Fatal(err)
	}

	stdout := captureStdout(t, func() error {
		return run([]string{"xfa", "datasets", path, "--format", "pdf"})
	})
	want := "invoice.number=INV-7\ninvoice.total=42\n"
	if stdout != want {
		t.Fatalf("stdout = %q, want %q", stdout, want)
	}
}

func TestCLIXFAMappingsEmitsJSONMappings(t *testing.T) {
	path := filepath.Join(t.TempDir(), "xfa-mappings.pdf")
	input := pdfFixture(
		"<< /Type /Catalog /AcroForm << /XFA [(template) (<template><field name=\"form.name\"/></template>) (datasets) (<datasets><data><form><name>Alice</name></form></data></datasets>)] >> >>",
	)
	if err := os.WriteFile(path, input, 0644); err != nil {
		t.Fatal(err)
	}

	stdout := captureStdout(t, func() error {
		return run([]string{"xfa", "mappings", path, "--format", "pdf", "--json"})
	})
	var result struct {
		Count    int `json:"count"`
		Mappings []struct {
			FieldName           string `json:"field_name"`
			DatasetPath         string `json:"dataset_path"`
			Value               string `json:"value"`
			TemplatePacketIndex int    `json:"template_packet_index"`
			DatasetPacketIndex  int    `json:"dataset_packet_index"`
			Label               string `json:"label"`
		} `json:"mappings"`
	}
	if err := json.Unmarshal([]byte(stdout), &result); err != nil {
		t.Fatal(err)
	}
	if result.Count != 1 || len(result.Mappings) != 1 {
		t.Fatalf("result = %+v, want one mapping", result)
	}
	mapping := result.Mappings[0]
	if mapping.FieldName != "form.name" || mapping.DatasetPath != "form.name" || mapping.Value != "Alice" {
		t.Fatalf("mapping value = %+v", mapping)
	}
	if mapping.TemplatePacketIndex != 0 || mapping.DatasetPacketIndex != 1 || mapping.Label != "template" {
		t.Fatalf("mapping metadata = %+v", mapping)
	}
}

func TestCLIXFAMappingsRejectsNonPDFFormat(t *testing.T) {
	path := filepath.Join(t.TempDir(), "xfa-mappings.txt")
	if err := os.WriteFile(path, []byte("not a pdf"), 0644); err != nil {
		t.Fatal(err)
	}

	_, err := captureStdoutAndError(t, func() error {
		return run([]string{"xfa", "mappings", path, "--format", "txt", "--json"})
	})
	if err == nil {
		t.Fatal("xfa mappings succeeded, want unsupported format error")
	}
	if err.Error() != `xfa mappings is unsupported for format "txt"` {
		t.Fatalf("error = %q", err)
	}
}

func TestCLIXFADatasetSetWritesVerifiedFieldUpdate(t *testing.T) {
	path := filepath.Join(t.TempDir(), "xfa-dataset-set.pdf")
	input := pdfFixture(
		"<< /Type /Catalog /AcroForm << /XFA [(template) (<template>ignored</template>) (datasets) (<xfa:datasets xmlns:xfa=\"http://www.xfa.org/schema/xfa-data/1.0/\"><xfa:data><form><name>Alice</name><address><city>Tijuana</city></address></form></xfa:data></xfa:datasets>)] >> >>",
	)
	if err := os.WriteFile(path, input, 0644); err != nil {
		t.Fatal(err)
	}
	out := filepath.Join(t.TempDir(), "xfa-dataset-set-out.pdf")

	stdout := captureStdout(t, func() error {
		return run([]string{
			"xfa", "dataset-set", path,
			"--format", "pdf",
			"--path", "form.name",
			"--value", "Ana & Co",
			"-o", out,
			"--json",
		})
	})
	var result struct {
		Report struct {
			Edit          string `json:"edit"`
			NodesModified int    `json:"nodes_modified"`
			OutputPath    string `json:"output_path"`
		} `json:"report"`
		Verification struct {
			ReparseOK      bool `json:"reparse_ok"`
			OldTextRemoved bool `json:"old_text_removed"`
			NewSelectable  bool `json:"new_text_selectable"`
		} `json:"verification"`
	}
	if err := json.Unmarshal([]byte(stdout), &result); err != nil {
		t.Fatal(err)
	}
	if result.Report.Edit != "pdf.xfa_dataset_field_update" || result.Report.NodesModified != 1 || result.Report.OutputPath != out {
		t.Fatalf("result = %+v", result)
	}
	if !result.Verification.ReparseOK || !result.Verification.OldTextRemoved || !result.Verification.NewSelectable {
		t.Fatalf("verification = %+v", result.Verification)
	}
	written, err := os.ReadFile(out)
	if err != nil {
		t.Fatal(err)
	}
	if bytes.Contains(written, []byte("Alice")) {
		t.Fatalf("old dataset value remains:\n%s", written)
	}
	if !bytes.Contains(written, []byte("Ana &amp; Co")) {
		t.Fatalf("escaped dataset value missing:\n%s", written)
	}
	if !bytes.Contains(written, []byte("<template>ignored</template>")) {
		t.Fatalf("unselected XFA packet changed:\n%s", written)
	}
}

func TestCLIXFADatasetSetSelectorSelectsLabel(t *testing.T) {
	path := filepath.Join(t.TempDir(), "xfa-dataset-set-label.pdf")
	input := pdfFixture(
		"<< /Type /Catalog /AcroForm << /XFA [(left) (<datasets><data><form><name>Left</name></form></data></datasets>) (right) (<datasets><data><form><name>Right</name></form></data></datasets>)] >> >>",
	)
	if err := os.WriteFile(path, input, 0644); err != nil {
		t.Fatal(err)
	}
	out := filepath.Join(t.TempDir(), "xfa-dataset-set-label-out.pdf")

	stdout := captureStdout(t, func() error {
		return run([]string{
			"xfa", "dataset-set", path,
			"--format", "pdf",
			"--label", "right",
			"--path", "form.name",
			"--value", "Updated",
			"-o", out,
			"--json",
		})
	})
	var result struct {
		Report struct {
			Edit string `json:"edit"`
		} `json:"report"`
		Verification struct {
			ReparseOK     bool `json:"reparse_ok"`
			NewSelectable bool `json:"new_text_selectable"`
		} `json:"verification"`
	}
	if err := json.Unmarshal([]byte(stdout), &result); err != nil {
		t.Fatal(err)
	}
	if result.Report.Edit != "pdf.xfa_dataset_field_update" || !result.Verification.ReparseOK || !result.Verification.NewSelectable {
		t.Fatalf("result = %+v", result)
	}
	written, err := os.ReadFile(out)
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Contains(written, []byte("<name>Left</name>")) {
		t.Fatalf("unselected dataset packet changed:\n%s", written)
	}
	if bytes.Contains(written, []byte("<name>Right</name>")) || !bytes.Contains(written, []byte("<name>Updated</name>")) {
		t.Fatalf("selected dataset packet was not updated:\n%s", written)
	}
}

func TestCLIXFADatasetsRejectsNonPDFFormat(t *testing.T) {
	path := filepath.Join(t.TempDir(), "xfa-datasets.txt")
	if err := os.WriteFile(path, []byte("not a pdf"), 0644); err != nil {
		t.Fatal(err)
	}

	_, err := captureStdoutAndError(t, func() error {
		return run([]string{"xfa", "datasets", path, "--format", "txt", "--json"})
	})
	if err == nil {
		t.Fatal("xfa datasets succeeded, want unsupported format error")
	}
	if err.Error() != `xfa datasets is unsupported for format "txt"` {
		t.Fatalf("error = %q", err)
	}
}

func TestCLIEditWritesVerifiedPDF(t *testing.T) {
	path := writeFixture(t)
	out := filepath.Join(t.TempDir(), "out.pdf")
	stdout := captureStdout(t, func() error {
		return run([]string{
			"edit", path,
			"--format", "pdf",
			"--kind", "pdf.content.text_show",
			"--text", "08-15-2024",
			"--replace", "May 5, 2026",
			"--verify", "reparse,old-gone,new-selectable",
			"-o", out,
			"--json",
		})
	})
	var result struct {
		Verification struct {
			OldTextRemoved bool `json:"old_text_removed"`
			NewSelectable  bool `json:"new_text_selectable"`
		} `json:"verification"`
	}
	if err := json.Unmarshal([]byte(stdout), &result); err != nil {
		t.Fatal(err)
	}
	if !result.Verification.OldTextRemoved || !result.Verification.NewSelectable {
		t.Fatalf("verification = %+v", result.Verification)
	}
	written, err := os.ReadFile(out)
	if err != nil {
		t.Fatal(err)
	}
	if bytes.Contains(written, []byte(`08\05515\0552024`)) {
		t.Fatal("old encoded date remains")
	}
	if !bytes.Contains(written, []byte(`May 5, 2026`)) {
		t.Fatal("new encoded date missing")
	}
}

func TestCLIEditSimpleFontPDFPreservesSelectableText(t *testing.T) {
	path := writeSimpleFontDifferencesFixture(t)
	out := filepath.Join(t.TempDir(), "simple-font-out.pdf")

	stdout := captureStdout(t, func() error {
		return run([]string{
			"edit", path,
			"--format", "pdf",
			"--kind", "pdf.content.text_show",
			"--text", "\u20ac\u00c1",
			"--replace", "\u00c1\u20ac",
			"--verify", "reparse,old-gone,new-selectable,page-count",
			"-o", out,
			"--json",
		})
	})
	assertCLISelectableEditResult(t, stdout)

	written, err := os.ReadFile(out)
	if err != nil {
		t.Fatal(err)
	}
	if bytes.Contains(written, []byte("<4142> Tj")) {
		t.Fatal("old simple-font encoded operand remains")
	}
	if !bytes.Contains(written, []byte("<4241> Tj")) {
		t.Fatalf("new simple-font encoded operand missing:\n%s", written)
	}
	oldQueryOut := captureStdout(t, func() error {
		return run([]string{"query", out, "--format", "pdf", "--kind", "pdf.content.text_show", "--text", "\u20ac\u00c1", "--json"})
	})
	assertCLIQueryCount(t, oldQueryOut, 0)
	newQueryOut := captureStdout(t, func() error {
		return run([]string{"query", out, "--format", "pdf", "--kind", "pdf.content.text_show", "--text", "\u00c1\u20ac", "--json"})
	})
	assertCLIQueryCount(t, newQueryOut, 1)
}

func TestCLIEditType0CMapPDFPreservesSelectableText(t *testing.T) {
	path := writeType0CMapFixture(t)
	out := filepath.Join(t.TempDir(), "type0-cmap-out.pdf")

	stdout := captureStdout(t, func() error {
		return run([]string{
			"edit", path,
			"--format", "pdf",
			"--kind", "pdf.content.text_show",
			"--text", "AB",
			"--replace", "BA",
			"--layout-mode", "preserve-width",
			"--verify", "reparse,old-gone,new-selectable,page-count",
			"-o", out,
			"--json",
		})
	})
	assertCLISelectableEditResult(t, stdout)

	written, err := os.ReadFile(out)
	if err != nil {
		t.Fatal(err)
	}
	if bytes.Contains(written, []byte("<00010002> Tj")) {
		t.Fatal("old Type0/CMap encoded operand remains")
	}
	if !bytes.Contains(written, []byte("<00020001> Tj")) {
		t.Fatalf("new Type0/CMap encoded operand missing:\n%s", written)
	}
	oldQueryOut := captureStdout(t, func() error {
		return run([]string{"query", out, "--format", "pdf", "--kind", "pdf.content.text_show", "--text", "AB", "--json"})
	})
	assertCLIQueryCount(t, oldQueryOut, 0)
	newQueryOut := captureStdout(t, func() error {
		return run([]string{"query", out, "--format", "pdf", "--kind", "pdf.content.text_show", "--text", "BA", "--json"})
	})
	assertCLIQueryCount(t, newQueryOut, 1)
}

func TestCLIEditLayoutModePreserveWidthAllowsWidthProvenPlan(t *testing.T) {
	path := writeLayoutProofFixture(t, "AB")
	out := filepath.Join(t.TempDir(), "out.pdf")

	stdout := captureStdout(t, func() error {
		return run([]string{
			"edit", path,
			"--format", "pdf",
			"--kind", "pdf.content.text_show",
			"--text", "AB",
			"--replace", "BA",
			"--layout-mode", "preserve-width",
			"--verify", "reparse,old-gone,new-selectable",
			"-o", out,
			"--json",
		})
	})
	var result struct {
		LayoutMode string `json:"layout_mode"`
		Report     struct {
			Meta map[string]any `json:"meta"`
		} `json:"report"`
		Verification struct {
			ReparseOK     bool `json:"reparse_ok"`
			NewSelectable bool `json:"new_text_selectable"`
		} `json:"verification"`
	}
	if err := json.Unmarshal([]byte(stdout), &result); err != nil {
		t.Fatal(err)
	}
	if result.LayoutMode != "preserve-width" {
		t.Fatalf("layout_mode = %q, want preserve-width", result.LayoutMode)
	}
	if result.Report.Meta["layout_mode"] != "preserve-width" || result.Report.Meta["layout_proof"] != "width_proven" {
		t.Fatalf("report meta = %+v, want preserve-width width_proven", result.Report.Meta)
	}
	for key, want := range map[string]any{
		"encoding":           "literal",
		"encoding_path":      "text_show/literal",
		"text_decode_source": "pdf_literal_string",
		"font_id":            "F1",
		"old_width_units":    float64(1210),
		"new_width_units":    float64(1210),
	} {
		if result.Report.Meta[key] != want {
			t.Fatalf("report meta[%q] = %v, want %v; meta=%+v", key, result.Report.Meta[key], want, result.Report.Meta)
		}
	}
	if result.Report.Meta["width_delta_units"] != float64(0) {
		t.Fatalf("width_delta_units = %v, want 0", result.Report.Meta["width_delta_units"])
	}
	if !result.Verification.ReparseOK || !result.Verification.NewSelectable {
		t.Fatalf("verification = %+v", result.Verification)
	}
	if _, err := os.Stat(out); err != nil {
		t.Fatalf("output file was not written: %v", err)
	}
}

func TestCLIEditLayoutModePreserveWidthRejectsReflowRequiredPlan(t *testing.T) {
	path := writeLayoutProofFixture(t, "AA")
	out := filepath.Join(t.TempDir(), "out.pdf")

	_, err := captureStdoutAndError(t, func() error {
		return run([]string{
			"edit", path,
			"--format", "pdf",
			"--kind", "pdf.content.text_show",
			"--text", "AA",
			"--replace", "BB",
			"--layout-mode", "preserve-width",
			"-o", out,
			"--json",
		})
	})
	if err == nil {
		t.Fatal("edit succeeded, want preserve-width layout refusal")
	}
	for _, want := range []string{
		"unsupported PDF text replacement",
		"layout_proof=reflow_required",
		"width_delta_units=20",
	} {
		if !strings.Contains(err.Error(), want) {
			t.Fatalf("error = %q, want substring %q", err, want)
		}
	}
	if _, statErr := os.Stat(out); !os.IsNotExist(statErr) {
		t.Fatalf("output file exists after refused preserve-width edit: %v", statErr)
	}
}

func TestCLIEditRewriteAutoAndSurgicalUseSurgicalPath(t *testing.T) {
	for _, mode := range []string{"auto", "surgical"} {
		t.Run(mode, func(t *testing.T) {
			path := writeFixture(t)
			out := filepath.Join(t.TempDir(), mode+".pdf")
			stdout := captureStdout(t, func() error {
				return run([]string{
					"edit", path,
					"--format", "pdf",
					"--kind", "pdf.content.text_show",
					"--text", "08-15-2024",
					"--replace", "May 5, 2026",
					"--rewrite", mode,
					"--verify", "reparse,old-gone,new-selectable",
					"-o", out,
					"--json",
				})
			})
			var result struct {
				Report struct {
					Edit          string `json:"edit"`
					FallbackUsed  bool   `json:"fallback_used"`
					NodesModified int    `json:"nodes_modified"`
				} `json:"report"`
				Verification struct {
					ReparseOK     bool `json:"reparse_ok"`
					NewSelectable bool `json:"new_text_selectable"`
				} `json:"verification"`
			}
			if err := json.Unmarshal([]byte(stdout), &result); err != nil {
				t.Fatal(err)
			}
			if result.Report.Edit != "pdf.content_stream_text_rewrite" {
				t.Fatalf("edit = %q, want surgical content stream rewrite", result.Report.Edit)
			}
			if result.Report.FallbackUsed {
				t.Fatal("edit used fallback path")
			}
			if result.Report.NodesModified != 1 {
				t.Fatalf("nodes modified = %d, want 1", result.Report.NodesModified)
			}
			if !result.Verification.ReparseOK || !result.Verification.NewSelectable {
				t.Fatalf("verification = %+v", result.Verification)
			}
		})
	}
}

func TestCLIEditRejectsUnknownRewriteMode(t *testing.T) {
	path := writeFixture(t)
	err := run([]string{
		"edit", path,
		"--format", "pdf",
		"--kind", "pdf.content.text_show",
		"--text", "08-15-2024",
		"--replace", "May 5, 2026",
		"--rewrite", "overlay",
		"-o", filepath.Join(t.TempDir(), "out.pdf"),
		"--json",
	})
	if err == nil {
		t.Fatal("edit succeeded, want rewrite mode validation error")
	}
	want := `unsupported rewrite mode "overlay" (expected auto, surgical, canonical, or preserve-structure)`
	if err.Error() != want {
		t.Fatalf("error = %q, want %q", err, want)
	}
}

func TestCLIOverlayTextWritesExplicitFallbackJSON(t *testing.T) {
	path := writeFixture(t)
	out := filepath.Join(t.TempDir(), "overlay.pdf")

	stdout := captureStdout(t, func() error {
		return run([]string{
			"overlay", "text", path,
			"--format", "pdf",
			"--page-index", "0",
			"--text", "APPROVED",
			"--x", "72",
			"--y", "144",
			"-o", out,
			"--json",
		})
	})
	var result struct {
		Operation string `json:"operation"`
		Report    struct {
			Edit           string `json:"edit"`
			FallbackUsed   bool   `json:"fallback_used"`
			FallbackKind   string `json:"fallback_kind"`
			FallbackPolicy struct {
				Fallback string `json:"fallback"`
				Mode     string `json:"mode"`
			} `json:"fallback_policy"`
			NodesModified int    `json:"nodes_modified"`
			OutputPath    string `json:"output_path"`
			Meta          struct {
				PageIndex int    `json:"page_index"`
				Operation string `json:"operation"`
			} `json:"meta"`
		} `json:"report"`
		Verification struct {
			ReparseOK     bool `json:"reparse_ok"`
			NewSelectable bool `json:"new_text_selectable"`
			PageUnchanged bool `json:"page_count_unchanged"`
		} `json:"verification"`
	}
	if err := json.Unmarshal([]byte(stdout), &result); err != nil {
		t.Fatal(err)
	}
	if result.Operation != "pdf.overlay_explicit_stamp" || result.Report.Edit != "pdf.overlay_explicit_stamp" {
		t.Fatalf("operation = %q report edit = %q, want explicit overlay stamp", result.Operation, result.Report.Edit)
	}
	if !result.Report.FallbackUsed || result.Report.FallbackKind != "overlay" || result.Report.FallbackPolicy.Fallback != "overlay" || result.Report.FallbackPolicy.Mode != "explicit" {
		t.Fatalf("fallback = used %v policy %+v, want overlay/explicit", result.Report.FallbackUsed, result.Report.FallbackPolicy)
	}
	if result.Report.Meta.Operation != "overlay_stamp" || result.Report.Meta.PageIndex != 0 {
		t.Fatalf("meta = %+v, want overlay operation metadata", result.Report.Meta)
	}
	if result.Report.NodesModified != 1 || result.Report.OutputPath != out {
		t.Fatalf("report = %+v, want one modified node and output path", result.Report)
	}
	if !result.Verification.ReparseOK || !result.Verification.NewSelectable || !result.Verification.PageUnchanged {
		t.Fatalf("verification = %+v, want reparse/selectable/page unchanged", result.Verification)
	}

	queryOut := captureStdout(t, func() error {
		return run([]string{"query", out, "--format", "pdf", "--kind", "pdf.content.text_show", "--text", "APPROVED", "--json"})
	})
	assertCLIQueryCount(t, queryOut, 1)
}

func TestCLIOCRTextLayerPlanWritesExplicitFallbackJSON(t *testing.T) {
	path := filepath.Join(t.TempDir(), "ocr-plan.pdf")
	input := pdfFixture(
		"<< /Type /Catalog /Pages 2 0 R >>",
		"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
		"<< /Type /Page /Parent 2 0 R /Resources << >> >>",
	)
	if err := os.WriteFile(path, input, 0644); err != nil {
		t.Fatal(err)
	}

	stdout := captureStdout(t, func() error {
		return run([]string{
			"ocr", "text-layer-plan", path,
			"--format", "pdf",
			"--page-index", "0",
			"--text", "External OCR text",
			"--x-min", "1",
			"--y-min", "2",
			"--x-max", "3",
			"--y-max", "4",
			"--confidence", "0.9",
			"--json",
		})
	})
	var result struct {
		Operation  string  `json:"operation"`
		PageIndex  int     `json:"page_index"`
		Text       string  `json:"text"`
		Confidence float64 `json:"confidence"`
		Box        struct {
			XMin float64 `json:"x_min"`
			YMin float64 `json:"y_min"`
			XMax float64 `json:"x_max"`
			YMax float64 `json:"y_max"`
		} `json:"box"`
		Policy struct {
			Fallback string `json:"fallback"`
			Mode     string `json:"mode"`
		} `json:"policy"`
		Report struct {
			Edit           string `json:"edit"`
			FallbackUsed   bool   `json:"fallback_used"`
			FallbackKind   string `json:"fallback_kind"`
			FallbackPolicy struct {
				Fallback string `json:"fallback"`
				Mode     string `json:"mode"`
			} `json:"fallback_policy"`
			NodesModified int `json:"nodes_modified"`
			MatchIndex    int `json:"match_index"`
			Meta          struct {
				Operation   string  `json:"operation"`
				PlannedOnly bool    `json:"planned_only"`
				PageIndex   int     `json:"page_index"`
				Text        string  `json:"text"`
				Confidence  float64 `json:"confidence"`
			} `json:"meta"`
		} `json:"report"`
	}
	if err := json.Unmarshal([]byte(stdout), &result); err != nil {
		t.Fatal(err)
	}
	if result.Operation != "pdf.ocr_text_layer_explicit_plan" || result.Report.Edit != result.Operation {
		t.Fatalf("operation = %q report edit = %q, want OCR text-layer plan", result.Operation, result.Report.Edit)
	}
	if result.PageIndex != 0 || result.Text != "External OCR text" || result.Confidence != 0.9 {
		t.Fatalf("plan = %+v, want caller-provided OCR options", result)
	}
	if result.Box.XMin != 1 || result.Box.YMin != 2 || result.Box.XMax != 3 || result.Box.YMax != 4 {
		t.Fatalf("box = %+v, want caller-provided bounds", result.Box)
	}
	if result.Policy.Fallback != "ocr_text_layer" || result.Policy.Mode != "explicit" {
		t.Fatalf("policy = %+v, want ocr_text_layer/explicit", result.Policy)
	}
	if !result.Report.FallbackUsed || result.Report.FallbackKind != "ocr_text_layer" || result.Report.FallbackPolicy.Fallback != "ocr_text_layer" || result.Report.FallbackPolicy.Mode != "explicit" {
		t.Fatalf("report fallback = used %v kind %q policy %+v, want ocr_text_layer/explicit", result.Report.FallbackUsed, result.Report.FallbackKind, result.Report.FallbackPolicy)
	}
	if result.Report.NodesModified != 0 || result.Report.MatchIndex != 0 {
		t.Fatalf("report = %+v, want planning-only zero modifications", result.Report)
	}
	if result.Report.Meta.Operation != "ocr_text_layer" || !result.Report.Meta.PlannedOnly || result.Report.Meta.PageIndex != 0 || result.Report.Meta.Text != "External OCR text" || result.Report.Meta.Confidence != 0.9 {
		t.Fatalf("meta = %+v, want planning-only OCR metadata", result.Report.Meta)
	}
}

func TestCLIOCRTextLayerPlanFailsClosedForInvalidInput(t *testing.T) {
	path := filepath.Join(t.TempDir(), "ocr-plan.pdf")
	input := pdfFixture(
		"<< /Type /Catalog /Pages 2 0 R >>",
		"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
		"<< /Type /Page /Parent 2 0 R /Resources << >> >>",
	)
	if err := os.WriteFile(path, input, 0644); err != nil {
		t.Fatal(err)
	}

	stdout, err := captureStdoutAndError(t, func() error {
		return run([]string{
			"ocr", "text-layer-plan", path,
			"--format", "pdf",
			"--page-index", "0",
			"--text", "External OCR text",
			"--x-min", "1",
			"--y-min", "2",
			"--x-max", "1",
			"--y-max", "4",
			"--confidence", "0.9",
			"--json",
		})
	})
	if err == nil {
		t.Fatal("ocr text-layer-plan succeeded, want invalid box error")
	}
	if err.Error() != "ocr text-layer bounding box must have positive width and height" {
		t.Fatalf("error = %q, want invalid bounding box error", err)
	}
	if stdout != "" {
		t.Fatalf("stdout = %q, want no JSON on invalid OCR plan input", stdout)
	}
}

func TestCLIEditPreserveStructureTableXrefUsesAdapterWriterMode(t *testing.T) {
	path := writeFixture(t)
	out := filepath.Join(t.TempDir(), "preserve-structure.pdf")
	stdout := captureStdout(t, func() error {
		return run([]string{
			"edit", path,
			"--format", "pdf",
			"--kind", "pdf.content.text_show",
			"--text", "08-15-2024",
			"--replace", "May 5, 2026",
			"--rewrite", "preserve-structure",
			"--verify", "reparse,old-gone,new-selectable",
			"-o", out,
			"--json",
		})
	})
	assertCanonicalCLIEditResult(t, stdout)
	result := decodePreserveStructureCLIEditResult(t, stdout)
	assertPreserveStructureCLIReportMeta(t, result.Report.Meta, "canonical", true, map[string]any{
		"has_table_xref":         true,
		"has_xref_stream":        false,
		"has_hybrid_xref":        false,
		"object_stream_objects":  0,
		"xref_stream_objects":    0,
		"requires_packed_writer": false,
	})
}

func TestCLIEditCanonicalRewriteWritesVerifiedPDF(t *testing.T) {
	path := writeFixture(t)
	out := filepath.Join(t.TempDir(), "out.pdf")
	stdout := captureStdout(t, func() error {
		return run([]string{
			"edit", path,
			"--format", "pdf",
			"--kind", "pdf.content.text_show",
			"--text", "08-15-2024",
			"--replace", "May 5, 2026",
			"--rewrite", "canonical",
			"--verify", "reparse,old-gone,new-selectable",
			"-o", out,
			"--json",
		})
	})
	var result struct {
		Report struct {
			Edit string `json:"edit"`
		} `json:"report"`
		Verification struct {
			ReparseOK      bool `json:"reparse_ok"`
			OldTextRemoved bool `json:"old_text_removed"`
			NewSelectable  bool `json:"new_text_selectable"`
		} `json:"verification"`
	}
	if err := json.Unmarshal([]byte(stdout), &result); err != nil {
		t.Fatal(err)
	}
	if result.Report.Edit != "pdf.canonical_content_stream_text_rewrite" {
		t.Fatalf("edit = %q, want canonical rewrite", result.Report.Edit)
	}
	if !result.Verification.ReparseOK || !result.Verification.OldTextRemoved || !result.Verification.NewSelectable {
		t.Fatalf("verification = %+v", result.Verification)
	}
	written, err := os.ReadFile(out)
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Contains(written, []byte("xref\n")) || !bytes.Contains(written, []byte("trailer\n")) {
		t.Fatalf("canonical output missing table xref/trailer:\n%s", written)
	}
}

func TestCLIEditSignedPDFDefaultsToRefuseSignatureInvalidation(t *testing.T) {
	path := writeSignedTextFixture(t)
	out := filepath.Join(t.TempDir(), "out.pdf")

	err := run([]string{
		"edit", path,
		"--format", "pdf",
		"--kind", "pdf.content.text_show",
		"--text", "08-15-2024",
		"--replace", "May 5, 2026",
		"--rewrite", "canonical",
		"-o", out,
		"--json",
	})
	if err == nil {
		t.Fatal("edit succeeded, want signed PDF invalidation refusal")
	}
	want := "unsupported PDF: digital signatures require explicit invalidation mode; Adapter.Parse refuses silent signature invalidation"
	if err.Error() != want {
		t.Fatalf("error = %q, want %q", err, want)
	}
	if _, statErr := os.Stat(out); !os.IsNotExist(statErr) {
		t.Fatalf("output file exists after refused signed edit: %v", statErr)
	}
}

func TestCLISignatureInspectReportsExistingSignatureMetadataAsJSON(t *testing.T) {
	path := writeSignedTextFixture(t)

	stdout := captureStdout(t, func() error {
		return run([]string{"signature", "inspect", path, "--format", "pdf", "--json"})
	})
	var result struct {
		Present                       bool   `json:"present"`
		ByteRangeCount                int    `json:"byte_range_count"`
		ByteRangeTotalRanges          int    `json:"byte_range_total_ranges"`
		ByteRangeCoveredBytes         int    `json:"byte_range_covered_bytes"`
		ByteRangeStatus               string `json:"byte_range_status"`
		ContentsByteLength            *int   `json:"contents_byte_length"`
		ObjectNumber                  *int   `json:"object_number"`
		ObjectGeneration              *int   `json:"object_generation"`
		Filter                        string `json:"filter"`
		SubFilter                     string `json:"sub_filter"`
		SignatureContainer            string `json:"signature_container"`
		DigestAlgorithm               string `json:"digest_algorithm"`
		DigestAlgorithmStatus         string `json:"digest_algorithm_status"`
		CryptographicValidation       bool   `json:"cryptographic_validation"`
		CryptographicValidationStatus string `json:"cryptographic_validation_status"`
	}
	if err := json.Unmarshal([]byte(stdout), &result); err != nil {
		t.Fatal(err)
	}
	if !result.Present || result.ByteRangeCount != 2 {
		t.Fatalf("signature metadata = %+v, want present with two byte ranges", result)
	}
	if result.ByteRangeTotalRanges != 2 || result.ByteRangeCoveredBytes != 4 || result.ByteRangeStatus != "valid" {
		t.Fatalf("byte range summary = count %d covered %d status %q, want 2/4/valid", result.ByteRangeTotalRanges, result.ByteRangeCoveredBytes, result.ByteRangeStatus)
	}
	if result.ObjectNumber == nil || *result.ObjectNumber != 5 || result.ObjectGeneration == nil || *result.ObjectGeneration != 0 {
		t.Fatalf("signature object = %v %v, want 5 0", result.ObjectNumber, result.ObjectGeneration)
	}
	if result.ContentsByteLength == nil || *result.ContentsByteLength != 1 {
		t.Fatalf("contents byte length = %v, want 1", result.ContentsByteLength)
	}
	if result.Filter != "Adobe.PPKLite" || result.SubFilter != "adbe.pkcs7.detached" {
		t.Fatalf("signature filter metadata = %+v", result)
	}
	if result.SignatureContainer != "pkcs7" || result.DigestAlgorithm != "unknown" || result.DigestAlgorithmStatus != "not_parsed" {
		t.Fatalf("signature diagnostics = %+v, want pkcs7/unknown/not_parsed", result)
	}
	if result.CryptographicValidation || result.CryptographicValidationStatus != "not_performed" {
		t.Fatalf("cryptographic validation = %t/%q, want false/not_performed", result.CryptographicValidation, result.CryptographicValidationStatus)
	}
}

func TestCLISignatureInspectPlainTextIncludesNonCryptographicHints(t *testing.T) {
	path := writeSignedTextFixture(t)

	stdout := captureStdout(t, func() error {
		return run([]string{"signature", "inspect", path, "--format", "pdf"})
	})

	for _, want := range []string{
		"signature present=true",
		"byte_range_status=valid",
		"byte_range_total_ranges=2",
		"byte_range_covered_bytes=4",
		"signature_container=pkcs7",
		"digest_algorithm=unknown",
		"digest_algorithm_status=not_parsed",
		"cryptographic_validation_status=not_performed",
		"object=5 0 R",
		"filter=Adobe.PPKLite",
		"sub_filter=adbe.pkcs7.detached",
		"contents_byte_length=1",
	} {
		if !strings.Contains(stdout, want) {
			t.Fatalf("signature inspect output = %q, want %q", stdout, want)
		}
	}
}

func TestCLISignatureInspectPlainTextReportsMalformedByteRangeWithoutValidationClaim(t *testing.T) {
	path := filepath.Join(t.TempDir(), "malformed-signed.pdf")
	input := pdfFixture(
		"<< /Type /Catalog /SigFlags 3 >>",
		"<< /Type /Sig /Filter /Adobe.PPKLite /SubFilter /ETSI.RFC3161 /ByteRange [0 1 999999 10] /Contents <00> >>",
	)
	if err := os.WriteFile(path, input, 0644); err != nil {
		t.Fatal(err)
	}

	stdout := captureStdout(t, func() error {
		return run([]string{"signature", "inspect", path, "--format", "pdf"})
	})

	for _, want := range []string{
		"signature present=true",
		"byte_range_status=malformed",
		"byte_range_count=0",
		"signature_container=unknown",
		"digest_algorithm=unknown",
		"cryptographic_validation_status=not_performed",
		"object=2 0 R",
		"sub_filter=ETSI.RFC3161",
	} {
		if !strings.Contains(stdout, want) {
			t.Fatalf("signature inspect output = %q, want %q", stdout, want)
		}
	}
}

func TestCLISignatureInspectRejectsUnsupportedFormat(t *testing.T) {
	path := writeSignedTextFixture(t)

	_, err := captureStdoutAndError(t, func() error {
		return run([]string{"signature", "inspect", path, "--format", "png", "--json"})
	})
	if err == nil {
		t.Fatal("signature inspect succeeded, want unsupported format error")
	}
	if err.Error() != `signature inspect is unsupported for format "png"` {
		t.Fatalf("error = %q", err)
	}
}

func TestCLIEditSignedPDFAllowsExplicitCanonicalSignatureInvalidation(t *testing.T) {
	path := writeSignedTextFixture(t)
	out := filepath.Join(t.TempDir(), "out.pdf")

	stdout := captureStdout(t, func() error {
		return run([]string{
			"edit", path,
			"--format", "pdf",
			"--kind", "pdf.content.text_show",
			"--text", "08-15-2024",
			"--replace", "May 5, 2026",
			"--rewrite", "canonical",
			"--allow-signature-invalidation",
			"--verify", "reparse,old-gone,new-selectable",
			"-o", out,
			"--json",
		})
	})
	var result struct {
		Report struct {
			Edit          string `json:"edit"`
			NodesModified int    `json:"nodes_modified"`
			OutputPath    string `json:"output_path"`
		} `json:"report"`
		SignatureInvalidation string `json:"signature_invalidation"`
		Verification          struct {
			ReparseOK      bool `json:"reparse_ok"`
			OldTextRemoved bool `json:"old_text_removed"`
			NewSelectable  bool `json:"new_text_selectable"`
		} `json:"verification"`
	}
	if err := json.Unmarshal([]byte(stdout), &result); err != nil {
		t.Fatal(err)
	}
	if result.Report.Edit != "pdf.canonical_content_stream_text_rewrite" || result.Report.NodesModified != 1 || result.Report.OutputPath != out {
		t.Fatalf("report = %+v", result.Report)
	}
	wantInvalidation := "digital signatures invalidated; not preserved or re-signed"
	if result.SignatureInvalidation != wantInvalidation {
		t.Fatalf("signature_invalidation = %q, want %q", result.SignatureInvalidation, wantInvalidation)
	}
	if !result.Verification.ReparseOK || !result.Verification.OldTextRemoved || !result.Verification.NewSelectable {
		t.Fatalf("verification = %+v", result.Verification)
	}
	written, err := os.ReadFile(out)
	if err != nil {
		t.Fatal(err)
	}
	if bytes.Contains(written, []byte(`08\05515\0552024`)) {
		t.Fatal("old encoded date remains")
	}
	if !bytes.Contains(written, []byte(`May 5, 2026`)) {
		t.Fatal("new encoded date missing")
	}
}

func TestCLIEditSignedPDFAllowsSignatureModeInvalidate(t *testing.T) {
	path := writeSignedTextFixture(t)
	out := filepath.Join(t.TempDir(), "out.pdf")

	stdout := captureStdout(t, func() error {
		return run([]string{
			"edit", path,
			"--format", "pdf",
			"--kind", "pdf.content.text_show",
			"--text", "08-15-2024",
			"--replace", "May 5, 2026",
			"--rewrite", "canonical",
			"--signature-mode", "invalidate",
			"--verify", "reparse,old-gone,new-selectable",
			"-o", out,
			"--json",
		})
	})
	assertCanonicalCLIEditResult(t, stdout)
}

func TestCLIEditSignedPDFPreserveIncrementalAppendsUpdateAndPreservesByteRanges(t *testing.T) {
	path := writeSignedTextFixture(t)
	out := filepath.Join(t.TempDir(), "out.pdf")
	original, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}

	stdout := captureStdout(t, func() error {
		return run([]string{
			"edit", path,
			"--format", "pdf",
			"--kind", "pdf.content.text_show",
			"--text", "08-15-2024",
			"--replace", "May 5, 2026",
			"--signature-mode", "preserve-incremental",
			"--verify", "reparse,old-gone,new-selectable,page-count-unchanged",
			"-o", out,
			"--json",
		})
	})
	var result struct {
		Report struct {
			Edit          string `json:"edit"`
			NodesModified int    `json:"nodes_modified"`
			OutputPath    string `json:"output_path"`
		} `json:"report"`
		Verification struct {
			ReparseOK      bool `json:"reparse_ok"`
			OldTextRemoved bool `json:"old_text_removed"`
			NewSelectable  bool `json:"new_text_selectable"`
			PageUnchanged  bool `json:"page_count_unchanged"`
		} `json:"verification"`
		SignaturePreservation struct {
			IncrementalUpdate         bool `json:"incremental_update"`
			OriginalBytesPreserved    bool `json:"original_bytes_preserved"`
			ByteRangeProof            bool `json:"byte_range_proof"`
			ByteRangesChecked         int  `json:"byte_ranges_checked"`
			SignedByteRangesUnchanged bool `json:"signed_byte_ranges_unchanged"`
			CryptographicValidation   bool `json:"cryptographic_validation"`
		} `json:"signature_preservation"`
	}
	if err := json.Unmarshal([]byte(stdout), &result); err != nil {
		t.Fatal(err)
	}
	if result.Report.Edit != "pdf.incremental_content_stream_text_rewrite" || result.Report.NodesModified != 1 || result.Report.OutputPath != out {
		t.Fatalf("report = %+v", result.Report)
	}
	if !result.Verification.ReparseOK || !result.Verification.OldTextRemoved || !result.Verification.NewSelectable || !result.Verification.PageUnchanged {
		t.Fatalf("verification = %+v", result.Verification)
	}
	if !result.SignaturePreservation.IncrementalUpdate ||
		!result.SignaturePreservation.OriginalBytesPreserved ||
		!result.SignaturePreservation.ByteRangeProof ||
		result.SignaturePreservation.ByteRangesChecked != 2 ||
		!result.SignaturePreservation.SignedByteRangesUnchanged ||
		result.SignaturePreservation.CryptographicValidation {
		t.Fatalf("signature preservation = %+v", result.SignaturePreservation)
	}
	written, err := os.ReadFile(out)
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.HasPrefix(written, original) {
		t.Fatal("incremental output does not preserve original signed bytes as prefix")
	}
	appended := written[len(original):]
	if !bytes.Contains(appended, []byte("(May 5, 2026) Tj")) || !bytes.Contains(appended, []byte("/Prev ")) {
		t.Fatalf("incremental append missing replacement or /Prev:\n%s", appended)
	}
}

func TestCLIEditSignedPDFPreserveIncrementalRequiresAutoRewrite(t *testing.T) {
	path := writeSignedTextFixture(t)
	out := filepath.Join(t.TempDir(), "out.pdf")

	err := run([]string{
		"edit", path,
		"--format", "pdf",
		"--kind", "pdf.content.text_show",
		"--text", "08-15-2024",
		"--replace", "May 5, 2026",
		"--rewrite", "canonical",
		"--signature-mode", "preserve-incremental",
		"-o", out,
		"--json",
	})
	if err == nil {
		t.Fatal("edit succeeded, want rewrite mode conflict")
	}
	want := "signature preservation requires --rewrite auto because it uses append-only incremental updates"
	if err.Error() != want {
		t.Fatalf("error = %q, want %q", err, want)
	}
	if _, statErr := os.Stat(out); !os.IsNotExist(statErr) {
		t.Fatalf("output file exists after preserve-incremental rewrite conflict: %v", statErr)
	}
}

func TestCLIEditRejectsSignatureInvalidationFlagForSurgicalRewrite(t *testing.T) {
	path := writeFixture(t)
	err := run([]string{
		"edit", path,
		"--format", "pdf",
		"--kind", "pdf.content.text_show",
		"--text", "08-15-2024",
		"--replace", "May 5, 2026",
		"--rewrite", "surgical",
		"--allow-signature-invalidation",
		"-o", filepath.Join(t.TempDir(), "out.pdf"),
		"--json",
	})
	if err == nil {
		t.Fatal("edit succeeded, want invalid flag/rewrite combination error")
	}
	want := "--allow-signature-invalidation requires --rewrite canonical or --rewrite auto selecting the canonical path"
	if err.Error() != want {
		t.Fatalf("error = %q, want %q", err, want)
	}
}

func TestCLIEditObjectStreamPDFUsesCanonicalForAutoAndRejectsSurgical(t *testing.T) {
	path := writeObjectStreamContentFixture(t)
	surgicalOut := filepath.Join(t.TempDir(), "surgical.pdf")
	err := run([]string{
		"edit", path,
		"--format", "pdf",
		"--kind", "pdf.content.text_show",
		"--text", "08-15-2024",
		"--replace", "May 5, 2026",
		"--rewrite", "surgical",
		"-o", surgicalOut,
		"--json",
	})
	if err == nil {
		t.Fatal("surgical edit succeeded, want object-stream refusal")
	}
	want := "surgical rewrite does not support PDFs with xref streams or object streams; use --rewrite auto or --rewrite canonical"
	if err.Error() != want {
		t.Fatalf("error = %q, want %q", err, want)
	}

	out := filepath.Join(t.TempDir(), "auto.pdf")
	stdout := captureStdout(t, func() error {
		return run([]string{
			"edit", path,
			"--format", "pdf",
			"--kind", "pdf.content.text_show",
			"--text", "08-15-2024",
			"--replace", "May 5, 2026",
			"--rewrite", "auto",
			"--verify", "reparse,old-gone,new-selectable",
			"-o", out,
			"--json",
		})
	})
	assertCanonicalCLIEditResult(t, stdout)
	written, err := os.ReadFile(out)
	if err != nil {
		t.Fatal(err)
	}
	if bytes.Contains(written, []byte("/Type /ObjStm")) {
		t.Fatalf("canonical output preserved object stream container:\n%s", written)
	}
}

func TestCLIEditPreserveStructurePackedPDFsUsePreserveWriter(t *testing.T) {
	cases := []struct {
		name           string
		fixture        func(*testing.T) string
		writerPath     string
		wantStructure  map[string]any
		wantOutputByte []byte
	}{
		{
			name:       "object-stream",
			fixture:    writeObjectStreamContentFixture,
			writerPath: "preserve-packed",
			wantStructure: map[string]any{
				"has_table_xref":         true,
				"has_xref_stream":        false,
				"has_hybrid_xref":        false,
				"object_stream_objects":  1,
				"xref_stream_objects":    0,
				"requires_packed_writer": true,
			},
			wantOutputByte: []byte("/Type /ObjStm"),
		},
		{
			name:       "xref-stream",
			fixture:    writeXrefStreamContentFixture,
			writerPath: "xref_stream",
			wantStructure: map[string]any{
				"has_table_xref":         false,
				"has_xref_stream":        true,
				"has_hybrid_xref":        false,
				"object_stream_objects":  0,
				"xref_stream_objects":    1,
				"requires_packed_writer": true,
			},
			wantOutputByte: []byte("/Type /XRef"),
		},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			path := tc.fixture(t)
			out := filepath.Join(t.TempDir(), "out.pdf")
			stdout := captureStdout(t, func() error {
				return run([]string{
					"edit", path,
					"--format", "pdf",
					"--kind", "pdf.content.text_show",
					"--text", "08-15-2024",
					"--replace", "May 5, 2026",
					"--rewrite", "preserve-structure",
					"--verify", "reparse,old-gone,new-selectable",
					"-o", out,
					"--json",
				})
			})
			result := decodePreserveStructureCLIEditResult(t, stdout)
			if result.Report.OutputPath != out {
				t.Fatalf("output_path = %q, want %q", result.Report.OutputPath, out)
			}
			if result.Report.Edit != "pdf.canonical_content_stream_text_rewrite" || result.Report.FallbackUsed || result.Report.NodesModified != 1 {
				t.Fatalf("report = %+v, want preserve-structure edit without fallback", result.Report)
			}
			if !result.Verification.ReparseOK || !result.Verification.OldTextRemoved || !result.Verification.NewSelectable {
				t.Fatalf("verification = %+v, want reparse/old-gone/new-selectable", result.Verification)
			}
			assertPreserveStructureCLIReportMeta(t, result.Report.Meta, tc.writerPath, false, tc.wantStructure)
			written, err := os.ReadFile(out)
			if err != nil {
				t.Fatal(err)
			}
			if !bytes.Contains(written, tc.wantOutputByte) {
				t.Fatalf("preserve-structure output missing %q:\n%s", tc.wantOutputByte, written)
			}
		})
	}
}

func TestCLIEditXrefStreamPDFUsesCanonicalForAuto(t *testing.T) {
	path := writeXrefStreamContentFixture(t)
	out := filepath.Join(t.TempDir(), "xref-auto.pdf")
	stdout := captureStdout(t, func() error {
		return run([]string{
			"edit", path,
			"--format", "pdf",
			"--kind", "pdf.content.text_show",
			"--text", "08-15-2024",
			"--replace", "May 5, 2026",
			"--rewrite", "auto",
			"--verify", "reparse,old-gone,new-selectable",
			"-o", out,
			"--json",
		})
	})
	assertCanonicalCLIEditResult(t, stdout)
	written, err := os.ReadFile(out)
	if err != nil {
		t.Fatal(err)
	}
	if bytes.Contains(written, []byte("/Type /XRef")) {
		t.Fatalf("canonical output preserved xref stream container:\n%s", written)
	}
	if !bytes.Contains(written, []byte("xref\n")) {
		t.Fatalf("canonical output missing table xref:\n%s", written)
	}
}

func TestCLIEditRequiresMatchIndexForMultipleMatches(t *testing.T) {
	path := writeDuplicateTextFixture(t)
	want := "selector matched 2 nodes; pass --match-index N (zero-based, 0..1) to choose one"

	err := run([]string{
		"edit", path,
		"--format", "pdf",
		"--kind", "pdf.content.text_show",
		"--text", "08-15-2024",
		"--replace", "May 5, 2026",
		"-o", filepath.Join(t.TempDir(), "omitted.pdf"),
		"--json",
	})
	if err == nil {
		t.Fatal("edit succeeded, want multiple-match index error")
	}
	if !strings.Contains(err.Error(), want) {
		t.Fatalf("error = %q, want to contain %q", err, want)
	}
}

func TestCLIEditMatchIndexOutOfRangeReturnsClearError(t *testing.T) {
	path := writeDuplicateTextFixture(t)
	err := run([]string{
		"edit", path,
		"--format", "pdf",
		"--kind", "pdf.content.text_show",
		"--text", "08-15-2024",
		"--replace", "May 5, 2026",
		"--match-index", "2",
		"-o", filepath.Join(t.TempDir(), "out.pdf"),
		"--json",
	})
	if err == nil {
		t.Fatal("edit succeeded, want out-of-range error")
	}
	want := "match index 2 out of range for 2 matches (zero-based)"
	if err.Error() != want {
		t.Fatalf("error = %q, want %q", err, want)
	}
}

func TestCLIEditMatchIndexZeroEditsFirstMatch(t *testing.T) {
	path := writeDuplicateTextFixture(t)
	out := filepath.Join(t.TempDir(), "out.pdf")
	stdout := captureStdout(t, func() error {
		return run([]string{
			"edit", path,
			"--format", "pdf",
			"--kind", "pdf.content.text_show",
			"--text", "08-15-2024",
			"--replace", "May 5, 2026",
			"--match-index", "0",
			"--verify", "reparse,new-selectable",
			"-o", out,
			"--json",
		})
	})
	var result struct {
		Report struct {
			MatchIndex int `json:"match_index"`
		} `json:"report"`
	}
	if err := json.Unmarshal([]byte(stdout), &result); err != nil {
		t.Fatal(err)
	}
	if result.Report.MatchIndex != 0 {
		t.Fatalf("match_index = %d, want 0", result.Report.MatchIndex)
	}
	written, err := os.ReadFile(out)
	if err != nil {
		t.Fatal(err)
	}
	replacement := bytes.Index(written, []byte(`May 5, 2026`))
	remainingOriginal := bytes.Index(written, []byte(`08\05515\0552024`))
	if replacement == -1 {
		t.Fatal("replacement text missing")
	}
	if remainingOriginal == -1 {
		t.Fatal("second matching text node was changed; want it left unchanged")
	}
	if replacement > remainingOriginal {
		t.Fatalf("replacement occurred after remaining original: replacement at %d, original at %d", replacement, remainingOriginal)
	}
}

func TestCLIEditMatchIndexOneEditsSecondMatch(t *testing.T) {
	path := writeDuplicateTextFixture(t)
	out := filepath.Join(t.TempDir(), "out.pdf")
	stdout := captureStdout(t, func() error {
		return run([]string{
			"edit", path,
			"--format", "pdf",
			"--kind", "pdf.content.text_show",
			"--text", "08-15-2024",
			"--replace", "May 5, 2026",
			"--match-index", "1",
			"--verify", "reparse,new-selectable",
			"-o", out,
			"--json",
		})
	})
	var result struct {
		Report struct {
			Target     int `json:"target"`
			MatchIndex int `json:"match_index"`
		} `json:"report"`
		Verification struct {
			ReparseOK     bool `json:"reparse_ok"`
			NewSelectable bool `json:"new_text_selectable"`
		} `json:"verification"`
	}
	if err := json.Unmarshal([]byte(stdout), &result); err != nil {
		t.Fatal(err)
	}
	if !result.Verification.ReparseOK || !result.Verification.NewSelectable {
		t.Fatalf("verification = %+v", result.Verification)
	}
	if result.Report.MatchIndex != 1 {
		t.Fatalf("match_index = %d, want 1", result.Report.MatchIndex)
	}
	written, err := os.ReadFile(out)
	if err != nil {
		t.Fatal(err)
	}
	first := bytes.Index(written, []byte(`08\05515\0552024`))
	second := bytes.Index(written, []byte(`May 5, 2026`))
	if first == -1 {
		t.Fatal("first matching text node was changed; want it left unchanged")
	}
	if second == -1 {
		t.Fatal("replacement text missing")
	}
	if first > second {
		t.Fatalf("replacement occurred before remaining original: original at %d, replacement at %d", first, second)
	}
}

func TestCLIInspectValidateQueryAndEditProveImagePassThroughDoesNotBlockTextEdit(t *testing.T) {
	path, imageBytes := writeImagePassThroughAndFlateTextFixture(t)
	out := filepath.Join(t.TempDir(), "out.pdf")

	inspectOut := captureStdout(t, func() error {
		return run([]string{"inspect", path, "--format", "pdf", "--json"})
	})
	var inspect struct {
		Streams       []coreNodeJSON         `json:"streams"`
		StreamFilters streamFilterReportJSON `json:"stream_filters"`
	}
	if err := json.Unmarshal([]byte(inspectOut), &inspect); err != nil {
		t.Fatal(err)
	}
	assertCLIStreamFilterSummary(t, inspect.StreamFilters, 2, 1, 1, 0)
	imageMeta := requireCLIStreamMeta(t, inspect.Streams, "filter_capability", "pass_through_image")
	if imageMeta["image_xobject"] != true || imageMeta["filter_editable"] != false || imageMeta["filter_pass_through"] != true || imageMeta["filter_target"] != false {
		t.Fatalf("image stream metadata = %+v, want pass-through non-target image boundary", imageMeta)
	}
	flateMeta := requireCLIStreamMeta(t, inspect.Streams, "filter_capability", "editable_reversible")
	if flateMeta["filter_editable"] != true || flateMeta["filter_pass_through"] != false || flateMeta["filter_target"] != true {
		t.Fatalf("Flate stream metadata = %+v, want editable target", flateMeta)
	}

	validateOut := captureStdout(t, func() error {
		return run([]string{"validate", path, "--format", "pdf", "--json"})
	})
	var validation struct {
		Valid         bool                   `json:"valid"`
		Streams       []coreNodeJSON         `json:"streams"`
		StreamFilters streamFilterReportJSON `json:"stream_filters"`
	}
	if err := json.Unmarshal([]byte(validateOut), &validation); err != nil {
		t.Fatal(err)
	}
	if !validation.Valid {
		t.Fatalf("validation valid = false, want true")
	}
	assertCLIStreamFilterSummary(t, validation.StreamFilters, 2, 1, 1, 0)
	requireCLIStreamMeta(t, validation.Streams, "filter_capability", "pass_through_image")

	queryOut := captureStdout(t, func() error {
		return run([]string{"query", path, "--format", "pdf", "--kind", "pdf.stream", "--meta", "filter_capability=pass_through_image", "--json"})
	})
	var query struct {
		Count   int            `json:"count"`
		Matches []coreNodeJSON `json:"matches"`
	}
	if err := json.Unmarshal([]byte(queryOut), &query); err != nil {
		t.Fatal(err)
	}
	if query.Count != 1 || len(query.Matches) != 1 {
		t.Fatalf("query pass-through stream count = %d/%d, want 1", query.Count, len(query.Matches))
	}
	if query.Matches[0].Meta["filter_pass_through"] != true || query.Matches[0].Meta["filter_target"] != false {
		t.Fatalf("query stream metadata = %+v, want pass-through non-target", query.Matches[0].Meta)
	}

	stdout := captureStdout(t, func() error {
		return run([]string{
			"edit", path,
			"--format", "pdf",
			"--kind", "pdf.content.text_show",
			"--text", "FLATE-TEXT",
			"--replace", "EDITED-TXT",
			"-o", out,
			"--json",
		})
	})
	assertCLISelectableEditResult(t, stdout)
	written, err := os.ReadFile(out)
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Contains(written, imageBytes) {
		t.Fatal("pass-through image stream bytes changed during unrelated edit")
	}
	oldQueryOut := captureStdout(t, func() error {
		return run([]string{"query", out, "--format", "pdf", "--kind", "pdf.content.text_show", "--text", "FLATE-TEXT", "--json"})
	})
	assertCLIQueryCount(t, oldQueryOut, 0)
	newQueryOut := captureStdout(t, func() error {
		return run([]string{"query", out, "--format", "pdf", "--kind", "pdf.content.text_show", "--text", "EDITED-TXT", "--json"})
	})
	assertCLIQueryCount(t, newQueryOut, 1)
}

func TestCLIUnsupportedTargetFilterJSONReportsBoundaryAndNoFallback(t *testing.T) {
	path := writeUnsupportedTargetAndFlateTextFixture(t)
	out := filepath.Join(t.TempDir(), "out.pdf")

	validateOut := captureStdout(t, func() error {
		return run([]string{"validate", path, "--format", "pdf", "--json"})
	})
	var validation struct {
		Valid         bool                   `json:"valid"`
		Streams       []coreNodeJSON         `json:"streams"`
		StreamFilters streamFilterReportJSON `json:"stream_filters"`
	}
	if err := json.Unmarshal([]byte(validateOut), &validation); err != nil {
		t.Fatal(err)
	}
	if !validation.Valid {
		t.Fatalf("validation valid = false, want true parse with explicit unsupported stream metadata")
	}
	assertCLIStreamFilterSummary(t, validation.StreamFilters, 2, 1, 0, 1)
	unsupportedMeta := requireCLIStreamMeta(t, validation.Streams, "filter_capability", "unsupported_target")
	if unsupportedMeta["unsupported"] != `unsupported PDF stream filter "FooDecode"` || unsupportedMeta["filter_editable"] != false || unsupportedMeta["filter_target"] != true {
		t.Fatalf("unsupported stream metadata = %+v, want explicit FooDecode target boundary", unsupportedMeta)
	}

	queryOut := captureStdout(t, func() error {
		return run([]string{"query", path, "--format", "pdf", "--kind", "pdf.stream", "--meta", "filter_capability=unsupported_target", "--json"})
	})
	var query struct {
		Count   int            `json:"count"`
		Matches []coreNodeJSON `json:"matches"`
	}
	if err := json.Unmarshal([]byte(queryOut), &query); err != nil {
		t.Fatal(err)
	}
	if query.Count != 1 || len(query.Matches) != 1 || query.Matches[0].Meta["unsupported"] != `unsupported PDF stream filter "FooDecode"` {
		t.Fatalf("unsupported stream query = %+v, want one explicit FooDecode stream", query)
	}

	stdout, err := captureStdoutAndError(t, func() error {
		return run([]string{
			"edit", path,
			"--format", "pdf",
			"--kind", "pdf.content.text_show",
			"--text", "UNSUPPORTED-TEXT",
			"--replace", "EDITED-UNSUPPORTED",
			"-o", out,
			"--json",
		})
	})
	if err == nil {
		t.Fatal("edit succeeded, want unsupported target boundary error")
	}
	if !strings.Contains(err.Error(), `unsupported stream filter targets present: unsupported PDF stream filter "FooDecode"`) {
		t.Fatalf("error = %q, want unsupported filter boundary", err)
	}
	var result struct {
		Error              string                 `json:"error"`
		EditStatus         string                 `json:"edit_status"`
		FallbackUsed       bool                   `json:"fallback_used"`
		UnsupportedStreams []coreNodeJSON         `json:"unsupported_streams"`
		StreamFilters      streamFilterReportJSON `json:"stream_filters"`
	}
	if err := json.Unmarshal([]byte(stdout), &result); err != nil {
		t.Fatal(err)
	}
	if result.EditStatus != "unsupported" || result.FallbackUsed || len(result.UnsupportedStreams) != 1 {
		t.Fatalf("edit error JSON = %+v, want unsupported/no-fallback with one stream", result)
	}
	if !strings.Contains(result.Error, `unsupported PDF stream filter "FooDecode"`) {
		t.Fatalf("error JSON = %q, want FooDecode reason", result.Error)
	}
	assertCLIStreamFilterSummary(t, result.StreamFilters, 2, 1, 0, 1)
	if _, statErr := os.Stat(out); !errors.Is(statErr, os.ErrNotExist) {
		t.Fatalf("output stat error = %v, want no output file", statErr)
	}
}

func TestCLIEditASCII85FlateFilterArrayWritesVerifiedPDF(t *testing.T) {
	path := writeASCII85FlateFilterArrayFixture(t)
	out := filepath.Join(t.TempDir(), "out.pdf")

	stdout := captureStdout(t, func() error {
		return run([]string{
			"edit", path,
			"--format", "pdf",
			"--kind", "pdf.content.text_show",
			"--text", "08-15-2024",
			"--replace", "May 5, 2026",
			"--verify", "reparse,old-gone,new-selectable",
			"-o", out,
			"--json",
		})
	})
	var result struct {
		Report struct {
			FallbackUsed  bool `json:"fallback_used"`
			NodesModified int  `json:"nodes_modified"`
		} `json:"report"`
		Verification struct {
			ReparseOK      bool `json:"reparse_ok"`
			OldTextRemoved bool `json:"old_text_removed"`
			NewSelectable  bool `json:"new_text_selectable"`
		} `json:"verification"`
	}
	if err := json.Unmarshal([]byte(stdout), &result); err != nil {
		t.Fatal(err)
	}
	if result.Report.FallbackUsed {
		t.Fatal("edit used fallback path")
	}
	if result.Report.NodesModified != 1 {
		t.Fatalf("nodes modified = %d, want 1", result.Report.NodesModified)
	}
	if !result.Verification.ReparseOK || !result.Verification.OldTextRemoved || !result.Verification.NewSelectable {
		t.Fatalf("verification = %+v", result.Verification)
	}

	oldQueryOut := captureStdout(t, func() error {
		return run([]string{"query", out, "--format", "pdf", "--kind", "pdf.content.text_show", "--text", "08-15-2024", "--json"})
	})
	assertCLIQueryCount(t, oldQueryOut, 0)

	newQueryOut := captureStdout(t, func() error {
		return run([]string{"query", out, "--format", "pdf", "--kind", "pdf.content.text_show", "--text", "May 5, 2026", "--json"})
	})
	assertCLIQueryCount(t, newQueryOut, 1)
}

func TestCLIEditFlateDecodeParmsPredictor12WritesVerifiedPDF(t *testing.T) {
	path := writeFlateDecodeParmsPredictor12Fixture(t)
	out := filepath.Join(t.TempDir(), "out.pdf")
	const decodeParms = "<< /Predictor 12 /Columns 1 >>"

	inspectOut := captureStdout(t, func() error {
		return run([]string{"inspect", path, "--format", "pdf", "--json"})
	})
	var inspect struct {
		Streams       []coreNodeJSON         `json:"streams"`
		StreamFilters streamFilterReportJSON `json:"stream_filters"`
	}
	if err := json.Unmarshal([]byte(inspectOut), &inspect); err != nil {
		t.Fatal(err)
	}
	assertCLIStreamFilterSummary(t, inspect.StreamFilters, 1, 1, 0, 0)
	streamMeta := requireCLIStreamMeta(t, inspect.Streams, "decode_parms", decodeParms)
	if streamMeta["filter_capability"] != "editable_reversible" || streamMeta["filter_editable"] != true || streamMeta["filter_target"] != true || streamMeta["unsupported"] != nil {
		t.Fatalf("DecodeParms stream metadata = %+v, want editable reversible target with no unsupported reason", streamMeta)
	}

	stdout := captureStdout(t, func() error {
		return run([]string{
			"edit", path,
			"--format", "pdf",
			"--kind", "pdf.content.text_show",
			"--text", "08-15-2024",
			"--replace", "May 5, 2026",
			"--verify", "reparse,old-gone,new-selectable",
			"-o", out,
			"--json",
		})
	})
	var result struct {
		Report struct {
			FallbackUsed  bool   `json:"fallback_used"`
			NodesModified int    `json:"nodes_modified"`
			OutputPath    string `json:"output_path"`
		} `json:"report"`
		Verification struct {
			ReparseOK      bool `json:"reparse_ok"`
			OldTextRemoved bool `json:"old_text_removed"`
			NewSelectable  bool `json:"new_text_selectable"`
		} `json:"verification"`
	}
	if err := json.Unmarshal([]byte(stdout), &result); err != nil {
		t.Fatal(err)
	}
	if result.Report.FallbackUsed {
		t.Fatal("edit used fallback path")
	}
	if result.Report.NodesModified != 1 {
		t.Fatalf("nodes modified = %d, want 1", result.Report.NodesModified)
	}
	if result.Report.OutputPath != out {
		t.Fatalf("output path = %q, want %q", result.Report.OutputPath, out)
	}
	if !result.Verification.ReparseOK || !result.Verification.OldTextRemoved || !result.Verification.NewSelectable {
		t.Fatalf("verification = %+v", result.Verification)
	}

	validateOut := captureStdout(t, func() error {
		return run([]string{"validate", out, "--format", "pdf", "--json"})
	})
	var validation struct {
		Valid         bool                   `json:"valid"`
		Streams       []coreNodeJSON         `json:"streams"`
		StreamFilters streamFilterReportJSON `json:"stream_filters"`
	}
	if err := json.Unmarshal([]byte(validateOut), &validation); err != nil {
		t.Fatal(err)
	}
	if !validation.Valid {
		t.Fatal("edited DecodeParms output is not valid")
	}
	assertCLIStreamFilterSummary(t, validation.StreamFilters, 1, 1, 0, 0)
	requireCLIStreamMeta(t, validation.Streams, "decode_parms", decodeParms)

	oldQueryOut := captureStdout(t, func() error {
		return run([]string{"query", out, "--format", "pdf", "--kind", "pdf.content.text_show", "--text", "08-15-2024", "--json"})
	})
	assertCLIQueryCount(t, oldQueryOut, 0)

	newQueryOut := captureStdout(t, func() error {
		return run([]string{"query", out, "--format", "pdf", "--kind", "pdf.content.text_show", "--text", "May 5, 2026", "--json"})
	})
	assertCLIQueryCount(t, newQueryOut, 1)
}

func TestCLIUnsupportedDecodeParmsJSONReportsExactReasonAndNoOutput(t *testing.T) {
	path := writeUnsupportedDecodeParmsTargetFixture(t)
	out := filepath.Join(t.TempDir(), "out.pdf")
	const wantReason = "unsupported stream: /DecodeParms /Predictor 9 is not supported"

	validateOut := captureStdout(t, func() error {
		return run([]string{"validate", path, "--format", "pdf", "--json"})
	})
	var validation struct {
		Valid         bool                   `json:"valid"`
		Streams       []coreNodeJSON         `json:"streams"`
		StreamFilters streamFilterReportJSON `json:"stream_filters"`
	}
	if err := json.Unmarshal([]byte(validateOut), &validation); err != nil {
		t.Fatal(err)
	}
	if !validation.Valid {
		t.Fatalf("validation valid = false, want true parse with explicit unsupported DecodeParms metadata")
	}
	assertCLIStreamFilterSummary(t, validation.StreamFilters, 1, 0, 0, 1)
	unsupportedMeta := requireCLIStreamMeta(t, validation.Streams, "unsupported", wantReason)
	if unsupportedMeta["filter"] != "FlateDecode" || unsupportedMeta["decode_parms"] != "<< /Predictor 9 /Columns 1 >>" || unsupportedMeta["filter_target"] != true {
		t.Fatalf("unsupported DecodeParms metadata = %+v, want exact reason on Flate target", unsupportedMeta)
	}

	stdout, err := captureStdoutAndError(t, func() error {
		return run([]string{
			"edit", path,
			"--format", "pdf",
			"--kind", "pdf.content.text_show",
			"--text", "08-15-2024",
			"--replace", "May 5, 2026",
			"-o", out,
			"--json",
		})
	})
	if err == nil {
		t.Fatal("edit succeeded, want unsupported DecodeParms boundary error")
	}
	if !strings.Contains(err.Error(), wantReason) {
		t.Fatalf("error = %q, want unsupported DecodeParms reason", err)
	}
	var result struct {
		Error              string                 `json:"error"`
		EditStatus         string                 `json:"edit_status"`
		FallbackUsed       bool                   `json:"fallback_used"`
		UnsupportedStreams []coreNodeJSON         `json:"unsupported_streams"`
		StreamFilters      streamFilterReportJSON `json:"stream_filters"`
	}
	if err := json.Unmarshal([]byte(stdout), &result); err != nil {
		t.Fatal(err)
	}
	if result.EditStatus != "unsupported" || result.FallbackUsed || len(result.UnsupportedStreams) != 1 {
		t.Fatalf("edit error JSON = %+v, want unsupported/no-fallback with one stream", result)
	}
	if !strings.Contains(result.Error, wantReason) || result.UnsupportedStreams[0].Meta["unsupported"] != wantReason {
		t.Fatalf("unsupported JSON = %+v, want exact DecodeParms reason", result)
	}
	assertCLIStreamFilterSummary(t, result.StreamFilters, 1, 0, 0, 1)
	if _, statErr := os.Stat(out); !errors.Is(statErr, os.ErrNotExist) {
		t.Fatalf("output stat error = %v, want no output file", statErr)
	}
}

func TestCLIEditFlateDecodeParmsPredictor12Columns4WritesVerifiedPDF(t *testing.T) {
	path := writeFlateDecodeParmsPredictor12Columns4Fixture(t)
	out := filepath.Join(t.TempDir(), "out.pdf")

	stdout, err := captureStdoutAndError(t, func() error {
		return run([]string{
			"edit", path,
			"--format", "pdf",
			"--kind", "pdf.content.text_show",
			"--text", "08-15-2024",
			"--replace", "May 05, 2026",
			"--verify", "reparse,old-gone,new-selectable",
			"-o", out,
			"--json",
		})
	})
	if err != nil {
		t.Fatalf("edit failed: %v\nstdout: %s", err, stdout)
	}
	var result struct {
		Report struct {
			FallbackUsed  bool   `json:"fallback_used"`
			NodesModified int    `json:"nodes_modified"`
			OutputPath    string `json:"output_path"`
		} `json:"report"`
		Verification struct {
			ReparseOK      bool `json:"reparse_ok"`
			OldTextRemoved bool `json:"old_text_removed"`
			NewSelectable  bool `json:"new_text_selectable"`
		} `json:"verification"`
	}
	if err := json.Unmarshal([]byte(stdout), &result); err != nil {
		t.Fatal(err)
	}
	if result.Report.FallbackUsed {
		t.Fatal("edit used fallback path")
	}
	if result.Report.NodesModified != 1 {
		t.Fatalf("nodes modified = %d, want 1", result.Report.NodesModified)
	}
	if result.Report.OutputPath != out {
		t.Fatalf("output path = %q, want %q", result.Report.OutputPath, out)
	}
	if !result.Verification.ReparseOK || !result.Verification.OldTextRemoved || !result.Verification.NewSelectable {
		t.Fatalf("verification = %+v", result.Verification)
	}

	oldQueryOut := captureStdout(t, func() error {
		return run([]string{"query", out, "--format", "pdf", "--kind", "pdf.content.text_show", "--text", "08-15-2024", "--json"})
	})
	assertCLIQueryCount(t, oldQueryOut, 0)

	newQueryOut := captureStdout(t, func() error {
		return run([]string{"query", out, "--format", "pdf", "--kind", "pdf.content.text_show", "--text", "May 05, 2026", "--json"})
	})
	assertCLIQueryCount(t, newQueryOut, 1)
}

func TestCLIEditFlateDecodeParmsPredictor12Columns4Colors3WritesVerifiedPDF(t *testing.T) {
	path := writeFlateDecodeParmsPredictor12Columns4Colors3Fixture(t)
	out := filepath.Join(t.TempDir(), "out.pdf")
	const replacement = "May 5 2026"

	stdout, err := captureStdoutAndError(t, func() error {
		return run([]string{
			"edit", path,
			"--format", "pdf",
			"--kind", "pdf.content.text_show",
			"--text", "08-15-2024",
			"--replace", replacement,
			"--verify", "reparse,old-gone,new-selectable",
			"-o", out,
			"--json",
		})
	})
	if err != nil {
		t.Fatalf("edit failed: %v\nstdout: %s", err, stdout)
	}
	var result struct {
		Report struct {
			FallbackUsed  bool   `json:"fallback_used"`
			NodesModified int    `json:"nodes_modified"`
			OutputPath    string `json:"output_path"`
		} `json:"report"`
		Verification struct {
			ReparseOK      bool `json:"reparse_ok"`
			OldTextRemoved bool `json:"old_text_removed"`
			NewSelectable  bool `json:"new_text_selectable"`
		} `json:"verification"`
	}
	if err := json.Unmarshal([]byte(stdout), &result); err != nil {
		t.Fatal(err)
	}
	if result.Report.FallbackUsed {
		t.Fatal("edit used fallback path")
	}
	if result.Report.NodesModified != 1 {
		t.Fatalf("nodes modified = %d, want 1", result.Report.NodesModified)
	}
	if result.Report.OutputPath != out {
		t.Fatalf("output path = %q, want %q", result.Report.OutputPath, out)
	}
	if !result.Verification.ReparseOK || !result.Verification.OldTextRemoved || !result.Verification.NewSelectable {
		t.Fatalf("verification = %+v", result.Verification)
	}

	oldQueryOut := captureStdout(t, func() error {
		return run([]string{"query", out, "--format", "pdf", "--kind", "pdf.content.text_show", "--text", "08-15-2024", "--json"})
	})
	assertCLIQueryCount(t, oldQueryOut, 0)

	newQueryOut := captureStdout(t, func() error {
		return run([]string{"query", out, "--format", "pdf", "--kind", "pdf.content.text_show", "--text", replacement, "--json"})
	})
	assertCLIQueryCount(t, newQueryOut, 1)
}

func TestCLIEditFlateDecodeParmsPredictor12Columns4BitsPerComponent16WritesVerifiedPDF(t *testing.T) {
	path := writeFlateDecodeParmsPredictor12Columns4BitsPerComponent16Fixture(t)
	out := filepath.Join(t.TempDir(), "out.pdf")
	const replacement = "May 5 2026"

	stdout, err := captureStdoutAndError(t, func() error {
		return run([]string{
			"edit", path,
			"--format", "pdf",
			"--kind", "pdf.content.text_show",
			"--text", "08-15-2024",
			"--replace", replacement,
			"--verify", "reparse,old-gone,new-selectable",
			"-o", out,
			"--json",
		})
	})
	if err != nil {
		t.Fatalf("edit failed: %v\nstdout: %s", err, stdout)
	}
	var result struct {
		Report struct {
			FallbackUsed  bool   `json:"fallback_used"`
			NodesModified int    `json:"nodes_modified"`
			OutputPath    string `json:"output_path"`
		} `json:"report"`
		Verification struct {
			ReparseOK      bool `json:"reparse_ok"`
			OldTextRemoved bool `json:"old_text_removed"`
			NewSelectable  bool `json:"new_text_selectable"`
		} `json:"verification"`
	}
	if err := json.Unmarshal([]byte(stdout), &result); err != nil {
		t.Fatal(err)
	}
	if result.Report.FallbackUsed {
		t.Fatal("edit used fallback path")
	}
	if result.Report.NodesModified != 1 {
		t.Fatalf("nodes modified = %d, want 1", result.Report.NodesModified)
	}
	if result.Report.OutputPath != out {
		t.Fatalf("output path = %q, want %q", result.Report.OutputPath, out)
	}
	if !result.Verification.ReparseOK || !result.Verification.OldTextRemoved || !result.Verification.NewSelectable {
		t.Fatalf("verification = %+v", result.Verification)
	}

	oldQueryOut := captureStdout(t, func() error {
		return run([]string{"query", out, "--format", "pdf", "--kind", "pdf.content.text_show", "--text", "08-15-2024", "--json"})
	})
	assertCLIQueryCount(t, oldQueryOut, 0)

	newQueryOut := captureStdout(t, func() error {
		return run([]string{"query", out, "--format", "pdf", "--kind", "pdf.content.text_show", "--text", replacement, "--json"})
	})
	assertCLIQueryCount(t, newQueryOut, 1)
}

func TestCLIEditFlateDecodeParmsPredictor2BitPackedWritesVerifiedPDF(t *testing.T) {
	path := writeFlateDecodeParmsPredictor2BitPackedFixture(t)
	out := filepath.Join(t.TempDir(), "out.pdf")
	const replacement = "May 6, 2026"

	stdout, err := captureStdoutAndError(t, func() error {
		return run([]string{
			"edit", path,
			"--format", "pdf",
			"--kind", "pdf.content.text_show",
			"--text", "08-15-2024",
			"--replace", replacement,
			"--verify", "reparse,old-gone,new-selectable",
			"-o", out,
			"--json",
		})
	})
	if err != nil {
		t.Fatalf("edit failed: %v\nstdout: %s", err, stdout)
	}
	var result struct {
		Report struct {
			FallbackUsed  bool   `json:"fallback_used"`
			NodesModified int    `json:"nodes_modified"`
			OutputPath    string `json:"output_path"`
		} `json:"report"`
		Verification struct {
			ReparseOK      bool `json:"reparse_ok"`
			OldTextRemoved bool `json:"old_text_removed"`
			NewSelectable  bool `json:"new_text_selectable"`
		} `json:"verification"`
	}
	if err := json.Unmarshal([]byte(stdout), &result); err != nil {
		t.Fatal(err)
	}
	if result.Report.FallbackUsed {
		t.Fatal("edit used fallback path")
	}
	if result.Report.NodesModified != 1 {
		t.Fatalf("nodes modified = %d, want 1", result.Report.NodesModified)
	}
	if result.Report.OutputPath != out {
		t.Fatalf("output path = %q, want %q", result.Report.OutputPath, out)
	}
	if !result.Verification.ReparseOK || !result.Verification.OldTextRemoved || !result.Verification.NewSelectable {
		t.Fatalf("verification = %+v", result.Verification)
	}

	oldQueryOut := captureStdout(t, func() error {
		return run([]string{"query", out, "--format", "pdf", "--kind", "pdf.content.text_show", "--text", "08-15-2024", "--json"})
	})
	assertCLIQueryCount(t, oldQueryOut, 0)

	newQueryOut := captureStdout(t, func() error {
		return run([]string{"query", out, "--format", "pdf", "--kind", "pdf.content.text_show", "--text", replacement, "--json"})
	})
	assertCLIQueryCount(t, newQueryOut, 1)
}

func writeFixture(t *testing.T) string {
	t.Helper()
	path := filepath.Join(t.TempDir(), "fixture.pdf")
	input := []byte("%PDF-1.3\n1 0 obj\n<< /Type /Page /Length 27 >>\nstream\nBT\n(08\\05515\\0552024) Tj\nET\nendstream\nendobj\nxref\n0 2\n0000000000 65535 f \n0000000009 00000 n \ntrailer\n<< /Size 2 /Root 1 0 R >>\nstartxref\n100\n%%EOF\n")
	if err := os.WriteFile(path, input, 0644); err != nil {
		t.Fatal(err)
	}
	return path
}

func writeLayoutProofFixture(t *testing.T, text string) string {
	t.Helper()
	content := []byte(fmt.Sprintf("BT\n/F1 12 Tf\n(%s) Tj\nET\n", text))
	input := pdfFixture(
		"<< /Type /Catalog >>",
		"<< /Type /Page /Contents 4 0 R /Resources << /Font << /F1 3 0 R >> >> >>",
		"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /FirstChar 65 /Widths [600 610] >>",
		fmt.Sprintf("<< /Length %d >>\nstream\n%sendstream", len(content), content),
	)
	path := filepath.Join(t.TempDir(), "layout-proof.pdf")
	if err := os.WriteFile(path, input, 0644); err != nil {
		t.Fatal(err)
	}
	return path
}

func writeSimpleFontDifferencesFixture(t *testing.T) string {
	t.Helper()
	content := []byte("BT\n/F1 12 Tf\n<4142> Tj\nET\n")
	input := pdfFixture(
		"<< /Type /Catalog >>",
		"<< /Type /Page /Contents 4 0 R /Resources << /Font << /F1 3 0 R >> >> >>",
		"<< /Type /Font /Subtype /Type1 /Encoding << /BaseEncoding /WinAnsiEncoding /Differences [65 /Euro /Aacute] >> >>",
		fmt.Sprintf("<< /Length %d >>\nstream\n%sendstream", len(content), content),
	)
	path := filepath.Join(t.TempDir(), "simple-font-differences.pdf")
	if err := os.WriteFile(path, input, 0644); err != nil {
		t.Fatal(err)
	}
	return path
}

func writeType0CMapFixture(t *testing.T) string {
	t.Helper()
	content := []byte("BT\n/F1 12 Tf\n<00010002> Tj\nET\n")
	input := pdfFixture(
		"<< /Type /Catalog >>",
		"<< /Type /Page /Contents 4 0 R /Resources << /Font << /F1 3 0 R >> >> >>",
		"<< /Type /Font /Subtype /Type0 /Encoding /Identity-H /DescendantFonts [6 0 R] /ToUnicode 5 0 R >>",
		fmt.Sprintf("<< /Length %d >>\nstream\n%sendstream", len(content), content),
		cliTwoCIDToUnicodeCMapStream(),
		"<< /Type /Font /Subtype /CIDFontType2 /BaseFont /CIDFixture /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> /W [1 [500 610]] >>",
	)
	path := filepath.Join(t.TempDir(), "type0-cmap.pdf")
	if err := os.WriteFile(path, input, 0644); err != nil {
		t.Fatal(err)
	}
	return path
}

func cliTwoCIDToUnicodeCMapStream() string {
	cmap := []byte(`/CIDInit /ProcSet findresource begin
12 dict begin
begincmap
2 beginbfchar
<0001> <0041>
<0002> <0042>
endbfchar
endcmap
CMapName currentdict /CMap defineresource pop
end
end`)
	return fmt.Sprintf("<< /Length %d >>\nstream\n%s\nendstream", len(cmap)+1, cmap)
}

func writeSignedTextFixture(t *testing.T) string {
	t.Helper()
	content := []byte("BT\n(08\\05515\\0552024) Tj\nET\n")
	input := pdfFixture(
		"<< /Type /Catalog /Pages 2 0 R /SigFlags 3 >>",
		"<< /Type /Page /Contents 3 0 R >>",
		fmt.Sprintf("<< /Length %d >>\nstream\n%sendstream", len(content), content),
		"<< /Type /Annot /Subtype /Widget /FT /Sig /T (Approval) /V 5 0 R >>",
		"<< /Type /Sig /Filter /Adobe.PPKLite /SubFilter /adbe.pkcs7.detached /ByteRange [0 1 2 3] /Contents <00> >>",
	)
	path := filepath.Join(t.TempDir(), "signed-text.pdf")
	if err := os.WriteFile(path, input, 0644); err != nil {
		t.Fatal(err)
	}
	return path
}

func writeEncryptedFixture(t *testing.T) string {
	t.Helper()
	var input bytes.Buffer
	input.WriteString("%PDF-1.3\n")
	offsets := make([]int, 0, 3)
	writeObject := func(number int, body []byte) {
		t.Helper()
		offsets = append(offsets, input.Len())
		fmt.Fprintf(&input, "%d 0 obj\n", number)
		input.Write(body)
		input.WriteString("\nendobj\n")
	}
	writeObject(1, []byte("<< /Type /Catalog /Pages 3 0 R >>"))
	writeObject(2, []byte("<< /Filter /Standard /SubFilter /adbe.pkcs7.s5 /V 4 /R 4 /Length 128 /EncryptMetadata false >>"))
	writeObject(3, []byte("<< /Type /Page >>"))
	xrefOffset := input.Len()
	input.WriteString("xref\n0 4\n")
	input.WriteString("0000000000 65535 f \n")
	for _, offset := range offsets {
		fmt.Fprintf(&input, "%010d 00000 n \n", offset)
	}
	fmt.Fprintf(&input, "trailer\n<< /Size 4 /Root 1 0 R /Encrypt 2 0 R >>\nstartxref\n%d\n%%%%EOF\n", xrefOffset)
	path := filepath.Join(t.TempDir(), "encrypted.pdf")
	if err := os.WriteFile(path, input.Bytes(), 0644); err != nil {
		t.Fatal(err)
	}
	return path
}

func writePublicKeyEncryptedFixture(t *testing.T, recipient string) string {
	t.Helper()
	var input bytes.Buffer
	input.WriteString("%PDF-1.4\n")
	offsets := make([]int, 0, 3)
	writeObject := func(number int, body []byte) {
		t.Helper()
		offsets = append(offsets, input.Len())
		fmt.Fprintf(&input, "%d 0 obj\n", number)
		input.Write(body)
		input.WriteString("\nendobj\n")
	}
	writeObject(1, []byte("<< /Type /Catalog /Pages 3 0 R >>"))
	writeObject(2, []byte(fmt.Sprintf("<< /Filter /Adobe.PubSec /SubFilter /adbe.pkcs7.s5 /V 4 /R 4 /Length 128 /Recipients [<%s>] >>", recipient)))
	writeObject(3, []byte("<< /Type /Page >>"))
	xrefOffset := input.Len()
	input.WriteString("xref\n0 4\n")
	input.WriteString("0000000000 65535 f \n")
	for _, offset := range offsets {
		fmt.Fprintf(&input, "%010d 00000 n \n", offset)
	}
	fmt.Fprintf(&input, "trailer\n<< /Size 4 /Root 1 0 R /Encrypt 2 0 R >>\nstartxref\n%d\n%%%%EOF\n", xrefOffset)
	path := filepath.Join(t.TempDir(), "public-key-encrypted.pdf")
	if err := os.WriteFile(path, input.Bytes(), 0644); err != nil {
		t.Fatal(err)
	}
	return path
}

func writeSupportedEncryptedFixture(t *testing.T, text string) string {
	t.Helper()
	fileID := []byte("fixture-file-id1")
	fileKey := cliStandardR2FileKey(t, "user", fileID)
	content := []byte(fmt.Sprintf("BT\n(%s) Tj\nET\n", strings.ReplaceAll(text, "-", `\055`)))
	encryptedContent := cliRC4Object(t, fileKey, 4, 0, content)
	encryptedTitle := cliRC4Object(t, fileKey, 5, 0, []byte("Sensitive Title"))

	var input bytes.Buffer
	input.WriteString("%PDF-1.3\n")
	offsets := make([]int, 0, 6)
	writeObject := func(number int, body []byte) {
		t.Helper()
		offsets = append(offsets, input.Len())
		fmt.Fprintf(&input, "%d 0 obj\n", number)
		input.Write(body)
		input.WriteString("\nendobj\n")
	}
	writeObject(1, []byte("<< /Type /Catalog /Pages 2 0 R >>"))
	writeObject(2, []byte("<< /Type /Pages /Kids [3 0 R] /Count 1 >>"))
	writeObject(3, []byte("<< /Type /Page /Parent 2 0 R /Contents 4 0 R >>"))
	offsets = append(offsets, input.Len())
	input.WriteString("4 0 obj\n")
	fmt.Fprintf(&input, "<< /Length %d >>\nstream\n", len(encryptedContent))
	input.Write(encryptedContent)
	input.WriteString("\nendstream\nendobj\n")
	writeObject(5, []byte(fmt.Sprintf("<< /Title <%s> >>", hex.EncodeToString(encryptedTitle))))
	writeObject(6, []byte(`<<
/Filter /Standard
/V 1
/R 2
/O <000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f>
/U <f2e39758794025127846c07006961602075fb11d4b5c227e0b27c9a2abb73e61>
/P -44
>>`))

	xrefOffset := input.Len()
	input.WriteString("xref\n0 7\n")
	input.WriteString("0000000000 65535 f \n")
	for _, offset := range offsets {
		fmt.Fprintf(&input, "%010d 00000 n \n", offset)
	}
	fmt.Fprintf(&input, "trailer\n<< /Size 7 /Root 1 0 R /Info 5 0 R /Encrypt 6 0 R /ID [<%s> <%s>] >>\nstartxref\n%d\n%%%%EOF\n",
		hex.EncodeToString(fileID),
		hex.EncodeToString(fileID),
		xrefOffset,
	)
	path := filepath.Join(t.TempDir(), "supported-encrypted.pdf")
	if err := os.WriteFile(path, input.Bytes(), 0644); err != nil {
		t.Fatal(err)
	}
	return path
}

func writeSupportedEncryptedObjectStreamFixture(t *testing.T, text string) string {
	t.Helper()
	fileID := []byte("fixture-file-id1")
	fileKey := cliStandardR2FileKey(t, "user", fileID)
	content := []byte(fmt.Sprintf("BT\n(%s) Tj\nET\n", strings.ReplaceAll(text, "-", `\055`)))
	encryptedContent := cliRC4Object(t, fileKey, 4, 0, content)
	encryptedTitle := cliRC4Object(t, fileKey, 5, 0, []byte("Sensitive Title"))
	objectStreamData := cliObjectStreamData(
		cliObjectStreamEntry{number: 1, value: "<< /Type /Catalog /Pages 2 0 R >>"},
		cliObjectStreamEntry{number: 2, value: "<< /Type /Pages /Kids [3 0 R] /Count 1 >>"},
		cliObjectStreamEntry{number: 3, value: "<< /Type /Page /Parent 2 0 R /Contents 4 0 R >>"},
	)
	encryptedObjectStream := cliRC4Object(t, fileKey, 7, 0, objectStreamData)
	first := bytes.IndexByte(objectStreamData, '\n') + 1

	var input bytes.Buffer
	input.WriteString("%PDF-1.5\n")
	offsets := map[int]int{}
	writeObject := func(number int, body []byte) {
		t.Helper()
		offsets[number] = input.Len()
		fmt.Fprintf(&input, "%d 0 obj\n", number)
		input.Write(body)
		input.WriteString("\nendobj\n")
	}
	offsets[4] = input.Len()
	input.WriteString("4 0 obj\n")
	fmt.Fprintf(&input, "<< /Length %d >>\nstream\n", len(encryptedContent))
	input.Write(encryptedContent)
	input.WriteString("\nendstream\nendobj\n")
	writeObject(5, []byte(fmt.Sprintf("<< /Title <%s> >>", hex.EncodeToString(encryptedTitle))))
	writeObject(6, []byte(`<<
/Filter /Standard
/V 1
/R 2
/O <000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f>
/U <f2e39758794025127846c07006961602075fb11d4b5c227e0b27c9a2abb73e61>
/P -44
>>`))
	offsets[7] = input.Len()
	input.WriteString("7 0 obj\n")
	fmt.Fprintf(&input, "<< /Type /ObjStm /N 3 /First %d /Length %d >>\nstream\n", first, len(encryptedObjectStream))
	input.Write(encryptedObjectStream)
	input.WriteString("\nendstream\nendobj\n")

	xrefOffset := input.Len()
	input.WriteString("xref\n0 8\n")
	input.WriteString("0000000000 65535 f \n")
	for number := 1; number <= 7; number++ {
		if offset, ok := offsets[number]; ok {
			fmt.Fprintf(&input, "%010d 00000 n \n", offset)
			continue
		}
		input.WriteString("0000000000 65535 f \n")
	}
	fmt.Fprintf(&input, "trailer\n<< /Size 8 /Root 1 0 R /Info 5 0 R /Encrypt 6 0 R /ID [<%s> <%s>] >>\nstartxref\n%d\n%%%%EOF\n",
		hex.EncodeToString(fileID),
		hex.EncodeToString(fileID),
		xrefOffset,
	)
	path := filepath.Join(t.TempDir(), "supported-encrypted-object-stream.pdf")
	if err := os.WriteFile(path, input.Bytes(), 0644); err != nil {
		t.Fatal(err)
	}
	return path
}

func writeSupportedEncryptedXrefStreamFixture(t *testing.T, text string) string {
	t.Helper()
	fileID := []byte("fixture-file-id1")
	fileKey := cliStandardR2FileKey(t, "user", fileID)
	content := []byte(fmt.Sprintf("BT\n(%s) Tj\nET\n", strings.ReplaceAll(text, "-", `\055`)))
	encryptedContent := cliRC4Object(t, fileKey, 4, 0, content)
	encryptedTitle := cliRC4Object(t, fileKey, 5, 0, []byte("Sensitive Title"))

	var input bytes.Buffer
	input.WriteString("%PDF-1.5\n")
	offsets := map[int]int{}
	writeObject := func(number int, body []byte) {
		t.Helper()
		offsets[number] = input.Len()
		fmt.Fprintf(&input, "%d 0 obj\n", number)
		input.Write(body)
		input.WriteString("\nendobj\n")
	}
	writeObject(1, []byte("<< /Type /Catalog /Pages 2 0 R >>"))
	writeObject(2, []byte("<< /Type /Pages /Kids [3 0 R] /Count 1 >>"))
	writeObject(3, []byte("<< /Type /Page /Parent 2 0 R /Contents 4 0 R >>"))
	offsets[4] = input.Len()
	input.WriteString("4 0 obj\n")
	fmt.Fprintf(&input, "<< /Length %d >>\nstream\n", len(encryptedContent))
	input.Write(encryptedContent)
	input.WriteString("\nendstream\nendobj\n")
	writeObject(5, []byte(fmt.Sprintf("<< /Title <%s> >>", hex.EncodeToString(encryptedTitle))))
	writeObject(6, []byte(`<<
/Filter /Standard
/V 1
/R 2
/O <000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f>
/U <f2e39758794025127846c07006961602075fb11d4b5c227e0b27c9a2abb73e61>
/P -44
>>`))

	xrefOffset := input.Len()
	offsets[8] = xrefOffset
	xrefData := make([]byte, 0, 9*6)
	for number := 0; number <= 8; number++ {
		offset, ok := offsets[number]
		if !ok {
			xrefData = appendXrefStreamEntry(xrefData, 0, 0, 0)
			continue
		}
		xrefData = appendXrefStreamEntry(xrefData, 1, offset, 0)
	}
	encryptedXrefData := cliRC4Object(t, fileKey, 8, 0, xrefData)
	input.WriteString("8 0 obj\n")
	fmt.Fprintf(&input, "<< /Type /XRef /Size 9 /Root 1 0 R /Info 5 0 R /Encrypt 6 0 R /ID [<%s> <%s>] /W [1 4 1] /Length %d >>\nstream\n",
		hex.EncodeToString(fileID),
		hex.EncodeToString(fileID),
		len(encryptedXrefData),
	)
	input.Write(encryptedXrefData)
	input.WriteString("\nendstream\nendobj\n")
	fmt.Fprintf(&input, "startxref\n%d\n%%%%EOF\n", xrefOffset)

	path := filepath.Join(t.TempDir(), "supported-encrypted-xref-stream.pdf")
	if err := os.WriteFile(path, input.Bytes(), 0644); err != nil {
		t.Fatal(err)
	}
	return path
}

func writeSupportedAESV2EncryptedFixture(t *testing.T, text string) string {
	t.Helper()
	fileID := []byte("revision4-aes-id")
	ownerKey := mustDecodeHex(t, "00112233445566778899aabbccddeeff102132435465768798a9babbdcddfeff")
	fileKey := cliStandardR4FileKey(t, "user", fileID, ownerKey)
	userEntry := cliStandardR4UserEntry(t, fileKey, fileID)
	content := []byte(fmt.Sprintf("BT\n(%s) Tj\nET\n", strings.ReplaceAll(text, "-", `\055`)))
	encryptedContent := cliAESV2Object(t, fileKey, 4, 0, content)
	encryptedTitle := cliAESV2Object(t, fileKey, 5, 0, []byte("Sensitive Title"))

	encryptObject := fmt.Sprintf(`<<
/CF << /StdCF << /CFM /AESV2 /Length 128 >> >>
/Filter /Standard
/Length 128
/O <%s>
/P -1028
/R 4
/StmF /StdCF
/StrF /StdCF
/U <%s>
/V 4
>>`, hex.EncodeToString(ownerKey), hex.EncodeToString(userEntry))

	var input bytes.Buffer
	input.WriteString("%PDF-1.6\n")
	offsets := make([]int, 0, 6)
	writeObject := func(number int, body []byte) {
		t.Helper()
		offsets = append(offsets, input.Len())
		fmt.Fprintf(&input, "%d 0 obj\n", number)
		input.Write(body)
		input.WriteString("\nendobj\n")
	}
	writeObject(1, []byte("<< /Type /Catalog /Pages 2 0 R >>"))
	writeObject(2, []byte("<< /Type /Pages /Kids [3 0 R] /Count 1 >>"))
	writeObject(3, []byte("<< /Type /Page /Parent 2 0 R /Contents 4 0 R >>"))
	offsets = append(offsets, input.Len())
	input.WriteString("4 0 obj\n")
	fmt.Fprintf(&input, "<< /Length %d >>\nstream\n", len(encryptedContent))
	input.Write(encryptedContent)
	input.WriteString("\nendstream\nendobj\n")
	writeObject(5, []byte(fmt.Sprintf("<< /Title <%s> >>", hex.EncodeToString(encryptedTitle))))
	writeObject(6, []byte(encryptObject))

	xrefOffset := input.Len()
	input.WriteString("xref\n0 7\n")
	input.WriteString("0000000000 65535 f \n")
	for _, offset := range offsets {
		fmt.Fprintf(&input, "%010d 00000 n \n", offset)
	}
	fmt.Fprintf(&input, "trailer\n<< /Size 7 /Root 1 0 R /Info 5 0 R /Encrypt 6 0 R /ID [<%s> <%s>] >>\nstartxref\n%d\n%%%%EOF\n",
		hex.EncodeToString(fileID),
		hex.EncodeToString(fileID),
		xrefOffset,
	)
	path := filepath.Join(t.TempDir(), "supported-aesv2-encrypted.pdf")
	if err := os.WriteFile(path, input.Bytes(), 0644); err != nil {
		t.Fatal(err)
	}
	return path
}

func writeSupportedAESV2StreamCryptEncryptedFixture(t *testing.T, text string) string {
	t.Helper()
	fileID := []byte("revision4-cryptf")
	ownerKey := mustDecodeHex(t, "00112233445566778899aabbccddeeff102132435465768798a9babbdcddfeff")
	fileKey := cliStandardR4FileKey(t, "user", fileID, ownerKey)
	userEntry := cliStandardR4UserEntry(t, fileKey, fileID)
	content := []byte(fmt.Sprintf("BT\n(%s) Tj\nET\n", strings.ReplaceAll(text, "-", `\055`)))
	filteredContent := cliFlate(t, content)
	encryptedContent := cliAESV2Object(t, fileKey, 4, 0, filteredContent)
	encryptedTitle := cliAESV2Object(t, fileKey, 5, 0, []byte("Sensitive Title"))

	encryptObject := fmt.Sprintf(`<<
/CF << /StdCF << /CFM /AESV2 /Length 128 >> >>
/Filter /Standard
/Length 128
/O <%s>
/P -1028
/R 4
/StmF /StdCF
/StrF /StdCF
/U <%s>
/V 4
>>`, hex.EncodeToString(ownerKey), hex.EncodeToString(userEntry))

	var input bytes.Buffer
	input.WriteString("%PDF-1.6\n")
	offsets := make([]int, 0, 6)
	writeObject := func(number int, body []byte) {
		t.Helper()
		offsets = append(offsets, input.Len())
		fmt.Fprintf(&input, "%d 0 obj\n", number)
		input.Write(body)
		input.WriteString("\nendobj\n")
	}
	writeObject(1, []byte("<< /Type /Catalog /Pages 2 0 R >>"))
	writeObject(2, []byte("<< /Type /Pages /Kids [3 0 R] /Count 1 >>"))
	writeObject(3, []byte("<< /Type /Page /Parent 2 0 R /Contents 4 0 R >>"))
	offsets = append(offsets, input.Len())
	input.WriteString("4 0 obj\n")
	fmt.Fprintf(&input, "<< /Length %d /Filter [/Crypt /FlateDecode] /DecodeParms [<< /Name /StdCF >> null] >>\nstream\n", len(encryptedContent))
	input.Write(encryptedContent)
	input.WriteString("\nendstream\nendobj\n")
	writeObject(5, []byte(fmt.Sprintf("<< /Title <%s> >>", hex.EncodeToString(encryptedTitle))))
	writeObject(6, []byte(encryptObject))

	xrefOffset := input.Len()
	input.WriteString("xref\n0 7\n")
	input.WriteString("0000000000 65535 f \n")
	for _, offset := range offsets {
		fmt.Fprintf(&input, "%010d 00000 n \n", offset)
	}
	fmt.Fprintf(&input, "trailer\n<< /Size 7 /Root 1 0 R /Info 5 0 R /Encrypt 6 0 R /ID [<%s> <%s>] >>\nstartxref\n%d\n%%%%EOF\n",
		hex.EncodeToString(fileID),
		hex.EncodeToString(fileID),
		xrefOffset,
	)
	path := filepath.Join(t.TempDir(), "supported-aesv2-stream-crypt-encrypted.pdf")
	if err := os.WriteFile(path, input.Bytes(), 0644); err != nil {
		t.Fatal(err)
	}
	return path
}

func writeSupportedAESV3EncryptedFixture(t *testing.T, text string) string {
	t.Helper()
	fileID := []byte("revision5-aesv3")
	fileKey, encryptObject := cliStandardAESV3EncryptionObject(t)
	content := []byte(fmt.Sprintf("BT\n(%s) Tj\nET\n", strings.ReplaceAll(text, "-", `\055`)))
	encryptedContent := cliAESV3Object(t, fileKey, content)
	encryptedTitle := cliAESV3Object(t, fileKey, []byte("Sensitive Title"))

	var input bytes.Buffer
	input.WriteString("%PDF-1.7\n")
	offsets := make([]int, 0, 6)
	writeObject := func(number int, body []byte) {
		t.Helper()
		offsets = append(offsets, input.Len())
		fmt.Fprintf(&input, "%d 0 obj\n", number)
		input.Write(body)
		input.WriteString("\nendobj\n")
	}
	writeObject(1, []byte("<< /Type /Catalog /Pages 2 0 R >>"))
	writeObject(2, []byte("<< /Type /Pages /Kids [3 0 R] /Count 1 >>"))
	writeObject(3, []byte("<< /Type /Page /Parent 2 0 R /Contents 4 0 R >>"))
	offsets = append(offsets, input.Len())
	input.WriteString("4 0 obj\n")
	fmt.Fprintf(&input, "<< /Length %d >>\nstream\n", len(encryptedContent))
	input.Write(encryptedContent)
	input.WriteString("\nendstream\nendobj\n")
	writeObject(5, []byte(fmt.Sprintf("<< /Title <%s> >>", hex.EncodeToString(encryptedTitle))))
	writeObject(6, []byte(encryptObject))

	xrefOffset := input.Len()
	input.WriteString("xref\n0 7\n")
	input.WriteString("0000000000 65535 f \n")
	for _, offset := range offsets {
		fmt.Fprintf(&input, "%010d 00000 n \n", offset)
	}
	fmt.Fprintf(&input, "trailer\n<< /Size 7 /Root 1 0 R /Info 5 0 R /Encrypt 6 0 R /ID [<%s> <%s>] >>\nstartxref\n%d\n%%%%EOF\n",
		hex.EncodeToString(fileID),
		hex.EncodeToString(fileID),
		xrefOffset,
	)
	path := filepath.Join(t.TempDir(), "supported-aesv3-encrypted.pdf")
	if err := os.WriteFile(path, input.Bytes(), 0644); err != nil {
		t.Fatal(err)
	}
	return path
}

func writeSupportedAESV3EncryptedObjectStreamFixture(t *testing.T, text string) string {
	t.Helper()
	fileID := []byte("revision5-aesv3")
	fileKey, encryptObject := cliStandardAESV3EncryptionObject(t)
	content := []byte(fmt.Sprintf("BT\n(%s) Tj\nET\n", strings.ReplaceAll(text, "-", `\055`)))
	encryptedContent := cliAESV3Object(t, fileKey, content)
	encryptedTitle := cliAESV3Object(t, fileKey, []byte("Sensitive Title"))
	objectStreamData := cliObjectStreamData(
		cliObjectStreamEntry{number: 1, value: "<< /Type /Catalog /Pages 2 0 R >>"},
		cliObjectStreamEntry{number: 2, value: "<< /Type /Pages /Kids [3 0 R] /Count 1 >>"},
		cliObjectStreamEntry{number: 3, value: "<< /Type /Page /Parent 2 0 R /Contents 4 0 R >>"},
	)
	encryptedObjectStream := cliAESV3Object(t, fileKey, objectStreamData)
	first := bytes.IndexByte(objectStreamData, '\n') + 1

	var input bytes.Buffer
	input.WriteString("%PDF-1.7\n")
	offsets := map[int]int{}
	writeObject := func(number int, body []byte) {
		t.Helper()
		offsets[number] = input.Len()
		fmt.Fprintf(&input, "%d 0 obj\n", number)
		input.Write(body)
		input.WriteString("\nendobj\n")
	}
	offsets[4] = input.Len()
	input.WriteString("4 0 obj\n")
	fmt.Fprintf(&input, "<< /Length %d >>\nstream\n", len(encryptedContent))
	input.Write(encryptedContent)
	input.WriteString("\nendstream\nendobj\n")
	writeObject(5, []byte(fmt.Sprintf("<< /Title <%s> >>", hex.EncodeToString(encryptedTitle))))
	writeObject(6, []byte(encryptObject))
	offsets[7] = input.Len()
	input.WriteString("7 0 obj\n")
	fmt.Fprintf(&input, "<< /Type /ObjStm /N 3 /First %d /Length %d >>\nstream\n", first, len(encryptedObjectStream))
	input.Write(encryptedObjectStream)
	input.WriteString("\nendstream\nendobj\n")

	xrefOffset := input.Len()
	input.WriteString("xref\n0 8\n")
	input.WriteString("0000000000 65535 f \n")
	for number := 1; number <= 7; number++ {
		if offset, ok := offsets[number]; ok {
			fmt.Fprintf(&input, "%010d 00000 n \n", offset)
			continue
		}
		input.WriteString("0000000000 65535 f \n")
	}
	fmt.Fprintf(&input, "trailer\n<< /Size 8 /Root 1 0 R /Info 5 0 R /Encrypt 6 0 R /ID [<%s> <%s>] >>\nstartxref\n%d\n%%%%EOF\n",
		hex.EncodeToString(fileID),
		hex.EncodeToString(fileID),
		xrefOffset,
	)
	path := filepath.Join(t.TempDir(), "supported-aesv3-encrypted-object-stream.pdf")
	if err := os.WriteFile(path, input.Bytes(), 0644); err != nil {
		t.Fatal(err)
	}
	return path
}

func writeSupportedAESV3EncryptedXrefStreamFixture(t *testing.T, text string) string {
	t.Helper()
	fileID := []byte("revision5-aesv3")
	fileKey, encryptObject := cliStandardAESV3EncryptionObject(t)
	content := []byte(fmt.Sprintf("BT\n(%s) Tj\nET\n", strings.ReplaceAll(text, "-", `\055`)))
	encryptedContent := cliAESV3Object(t, fileKey, content)
	encryptedTitle := cliAESV3Object(t, fileKey, []byte("Sensitive Title"))

	var input bytes.Buffer
	input.WriteString("%PDF-1.7\n")
	offsets := map[int]int{}
	writeObject := func(number int, body []byte) {
		t.Helper()
		offsets[number] = input.Len()
		fmt.Fprintf(&input, "%d 0 obj\n", number)
		input.Write(body)
		input.WriteString("\nendobj\n")
	}
	writeObject(1, []byte("<< /Type /Catalog /Pages 2 0 R >>"))
	writeObject(2, []byte("<< /Type /Pages /Kids [3 0 R] /Count 1 >>"))
	writeObject(3, []byte("<< /Type /Page /Parent 2 0 R /Contents 4 0 R >>"))
	offsets[4] = input.Len()
	input.WriteString("4 0 obj\n")
	fmt.Fprintf(&input, "<< /Length %d >>\nstream\n", len(encryptedContent))
	input.Write(encryptedContent)
	input.WriteString("\nendstream\nendobj\n")
	writeObject(5, []byte(fmt.Sprintf("<< /Title <%s> >>", hex.EncodeToString(encryptedTitle))))
	writeObject(6, []byte(encryptObject))

	xrefOffset := input.Len()
	offsets[8] = xrefOffset
	xrefData := make([]byte, 0, 9*6)
	for number := 0; number <= 8; number++ {
		offset, ok := offsets[number]
		if !ok {
			xrefData = appendXrefStreamEntry(xrefData, 0, 0, 0)
			continue
		}
		xrefData = appendXrefStreamEntry(xrefData, 1, offset, 0)
	}
	xrefStream := xrefData
	input.WriteString("8 0 obj\n")
	fmt.Fprintf(&input, "<< /Type /XRef /Size 9 /Root 1 0 R /Info 5 0 R /Encrypt 6 0 R /ID [<%s> <%s>] /W [1 4 1] /Length %d >>\nstream\n",
		hex.EncodeToString(fileID),
		hex.EncodeToString(fileID),
		len(xrefStream),
	)
	input.Write(xrefStream)
	input.WriteString("\nendstream\nendobj\n")
	fmt.Fprintf(&input, "startxref\n%d\n%%%%EOF\n", xrefOffset)

	path := filepath.Join(t.TempDir(), "supported-aesv3-encrypted-xref-stream.pdf")
	if err := os.WriteFile(path, input.Bytes(), 0644); err != nil {
		t.Fatal(err)
	}
	return path
}

var cliStandardPadding = []byte{
	0x28, 0xbf, 0x4e, 0x5e, 0x4e, 0x75, 0x8a, 0x41,
	0x64, 0x00, 0x4e, 0x56, 0xff, 0xfa, 0x01, 0x08,
	0x2e, 0x2e, 0x00, 0xb6, 0xd0, 0x68, 0x3e, 0x80,
	0x2f, 0x0c, 0xa9, 0xfe, 0x64, 0x53, 0x69, 0x7a,
}

func cliStandardR2FileKey(t *testing.T, password string, fileID []byte) []byte {
	t.Helper()
	ownerKey, err := hex.DecodeString("000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f")
	if err != nil {
		t.Fatal(err)
	}
	padded := make([]byte, 32)
	copy(padded, []byte(password))
	copy(padded[len(password):], cliStandardPadding)
	h := md5.New()
	h.Write(padded)
	h.Write(ownerKey)
	var permissions [4]byte
	permissionValue := int32(-44)
	binary.LittleEndian.PutUint32(permissions[:], uint32(permissionValue))
	h.Write(permissions[:])
	h.Write(fileID)
	return h.Sum(nil)[:5]
}

func cliStandardR4FileKey(t *testing.T, password string, fileID, ownerKey []byte) []byte {
	t.Helper()
	padded := make([]byte, 32)
	copy(padded, []byte(password))
	copy(padded[len(password):], cliStandardPadding)
	h := md5.New()
	h.Write(padded)
	h.Write(ownerKey)
	var permissions [4]byte
	permissionValue := int32(-1028)
	binary.LittleEndian.PutUint32(permissions[:], uint32(permissionValue))
	h.Write(permissions[:])
	h.Write(fileID)
	digest := h.Sum(nil)
	for i := 0; i < 50; i++ {
		next := md5.Sum(digest[:16])
		digest = next[:]
	}
	return bytes.Clone(digest[:16])
}

func cliStandardR4UserEntry(t *testing.T, fileKey, fileID []byte) []byte {
	t.Helper()
	h := md5.New()
	h.Write(cliStandardPadding)
	h.Write(fileID)
	digest := h.Sum(nil)
	out := cliRC4Crypt(t, fileKey, digest)
	for i := 1; i <= 19; i++ {
		key := bytes.Clone(fileKey)
		for j := range key {
			key[j] ^= byte(i)
		}
		out = cliRC4Crypt(t, key, out)
	}
	entry := make([]byte, 32)
	copy(entry, out)
	copy(entry[16:], []byte("binas-aes-fixture"))
	return entry
}

func cliRC4Object(t *testing.T, fileKey []byte, number, generation int, input []byte) []byte {
	t.Helper()
	keyInput := append([]byte{}, fileKey...)
	keyInput = append(keyInput, byte(number), byte(number>>8), byte(number>>16), byte(generation), byte(generation>>8))
	sum := md5.Sum(keyInput)
	objectKey := sum[:min(len(fileKey)+5, 16)]
	return cliRC4Crypt(t, objectKey, input)
}

func cliRC4Crypt(t *testing.T, key, input []byte) []byte {
	t.Helper()
	cipher, err := rc4.NewCipher(key)
	if err != nil {
		t.Fatal(err)
	}
	out := bytes.Clone(input)
	cipher.XORKeyStream(out, out)
	return out
}

type cliObjectStreamEntry struct {
	number int
	value  string
}

func cliObjectStreamData(entries ...cliObjectStreamEntry) []byte {
	var body bytes.Buffer
	offsets := make([]int, 0, len(entries))
	for _, entry := range entries {
		offsets = append(offsets, body.Len())
		body.WriteString(entry.value)
		body.WriteByte('\n')
	}
	var header bytes.Buffer
	for i, entry := range entries {
		fmt.Fprintf(&header, "%d %d ", entry.number, offsets[i])
	}
	header.WriteByte('\n')
	out := append(bytes.Clone(header.Bytes()), body.Bytes()...)
	return out
}

func cliAESV2Object(t *testing.T, fileKey []byte, number, generation int, input []byte) []byte {
	t.Helper()
	keyInput := append([]byte{}, fileKey...)
	keyInput = append(keyInput, byte(number), byte(number>>8), byte(number>>16), byte(generation), byte(generation>>8))
	keyInput = append(keyInput, 's', 'A', 'l', 'T')
	sum := md5.Sum(keyInput)
	objectKey := sum[:min(len(fileKey)+5, 16)]
	block, err := aes.NewCipher(objectKey)
	if err != nil {
		t.Fatal(err)
	}
	iv := []byte("binas-aes-iv-000")
	data := cliPKCS7Pad(input, aes.BlockSize)
	cipher.NewCBCEncrypter(block, iv).CryptBlocks(data, data)
	out := make([]byte, 0, len(iv)+len(data))
	out = append(out, iv...)
	out = append(out, data...)
	return out
}

func cliStandardAESV3EncryptionObject(t *testing.T) ([]byte, string) {
	t.Helper()
	fileKey := []byte("0123456789abcdef0123456789abcdef")
	userPassword := []byte("user")
	ownerPassword := []byte("owner")
	userValidationSalt := []byte("uvalsalt")
	userKeySalt := []byte("ukeysalt")
	ownerValidationSalt := []byte("ovalsalt")
	ownerKeySalt := []byte("okeysalt")

	userHashInput := append(bytes.Clone(userPassword), userValidationSalt...)
	userHash := sha256.Sum256(userHashInput)
	userKeyHashInput := append(bytes.Clone(userPassword), userKeySalt...)
	userKeyHash := sha256.Sum256(userKeyHashInput)
	userEntry := append(append(bytes.Clone(userHash[:]), userValidationSalt...), userKeySalt...)
	userEncryptedFileKey := cliAESCBCNoPadding(t, userKeyHash[:], bytes.Repeat([]byte{0}, aes.BlockSize), fileKey, true)

	ownerHashInput := append(bytes.Clone(ownerPassword), ownerValidationSalt...)
	ownerHashInput = append(ownerHashInput, userEntry[:48]...)
	ownerHash := sha256.Sum256(ownerHashInput)
	ownerKeyHashInput := append(bytes.Clone(ownerPassword), ownerKeySalt...)
	ownerKeyHashInput = append(ownerKeyHashInput, userEntry[:48]...)
	ownerKeyHash := sha256.Sum256(ownerKeyHashInput)
	ownerEntry := append(append(bytes.Clone(ownerHash[:]), ownerValidationSalt...), ownerKeySalt...)
	ownerEncryptedFileKey := cliAESCBCNoPadding(t, ownerKeyHash[:], bytes.Repeat([]byte{0}, aes.BlockSize), fileKey, true)

	perms := cliAESV3Perms(t, fileKey, -1028, true)
	encryptObject := fmt.Sprintf(`<<
/CF << /StdCF << /CFM /AESV3 /Length 256 >> >>
/EncryptMetadata true
/Filter /Standard
/Length 256
/O <%s>
/OE <%s>
/P -1028
/Perms <%s>
/R 5
/StmF /StdCF
/StrF /StdCF
/U <%s>
/UE <%s>
/V 5
>>`,
		hex.EncodeToString(ownerEntry),
		hex.EncodeToString(ownerEncryptedFileKey),
		hex.EncodeToString(perms),
		hex.EncodeToString(userEntry),
		hex.EncodeToString(userEncryptedFileKey),
	)
	return fileKey, encryptObject
}

func cliAESV3Perms(t *testing.T, fileKey []byte, permissions int, encryptMetadata bool) []byte {
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
	blockCipher, err := aes.NewCipher(fileKey)
	if err != nil {
		t.Fatal(err)
	}
	out := bytes.Clone(block)
	blockCipher.Encrypt(out, out)
	return out
}

func cliAESV3Object(t *testing.T, fileKey, input []byte) []byte {
	t.Helper()
	iv := bytes.Repeat([]byte{0x5a}, aes.BlockSize)
	encrypted := cliAESCBCNoPadding(t, fileKey, iv, cliPKCS7Pad(input, aes.BlockSize), true)
	return append(iv, encrypted...)
}

func cliAESCBCNoPadding(t *testing.T, key, iv, input []byte, encrypt bool) []byte {
	t.Helper()
	if len(iv) != aes.BlockSize || len(input)%aes.BlockSize != 0 {
		t.Fatalf("invalid AES-CBC input: iv=%d input=%d", len(iv), len(input))
	}
	block, err := aes.NewCipher(key)
	if err != nil {
		t.Fatal(err)
	}
	out := bytes.Clone(input)
	if encrypt {
		cipher.NewCBCEncrypter(block, iv).CryptBlocks(out, out)
	} else {
		cipher.NewCBCDecrypter(block, iv).CryptBlocks(out, out)
	}
	return out
}

func cliFlate(t *testing.T, input []byte) []byte {
	t.Helper()
	var output bytes.Buffer
	writer := zlib.NewWriter(&output)
	if _, err := writer.Write(input); err != nil {
		writer.Close()
		t.Fatal(err)
	}
	if err := writer.Close(); err != nil {
		t.Fatal(err)
	}
	return output.Bytes()
}

func cliPKCS7Pad(input []byte, blockSize int) []byte {
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

func mustDecodeHex(t *testing.T, input string) []byte {
	t.Helper()
	out, err := hex.DecodeString(input)
	if err != nil {
		t.Fatal(err)
	}
	return out
}

func writeResidualBoundaryFixture(t *testing.T) string {
	t.Helper()
	input := pdfFixture(
		"<< /Type /Catalog /AcroForm 2 0 R >>",
		"<< /Type /Page /Annots [ ] /Resources << /Font << /F1 3 0 R >> >> >>",
		"<< /Type /Font /Subtype /Type0 /ToUnicode 4 0 R >>",
		"<< /Length 9 >>\nstream\nbegincmap\nendstream",
	)
	path := filepath.Join(t.TempDir(), "residual-boundaries.pdf")
	if err := os.WriteFile(path, input, 0644); err != nil {
		t.Fatal(err)
	}
	return path
}

func writeUnsupportedBoundaryFixture(t *testing.T, object string) string {
	t.Helper()
	path := filepath.Join(t.TempDir(), "unsupported-boundary.pdf")
	if err := os.WriteFile(path, pdfFixture(object, "<<>>"), 0644); err != nil {
		t.Fatal(err)
	}
	return path
}

func writeASCII85FlateFilterArrayFixture(t *testing.T) string {
	t.Helper()
	decoded := []byte("BT\n(08\\05515\\0552024) Tj\nET\n")
	encoded := encodeASCII85FlateStream(t, decoded)
	input := pdfFixture(
		"<< /Type /Page >>",
		fmt.Sprintf("<< /Length %d /Filter [/ASCII85Decode /FlateDecode] >>\nstream\n%sendstream", len(encoded), encoded),
	)
	path := filepath.Join(t.TempDir(), "ascii85-flate-filter-array.pdf")
	if err := os.WriteFile(path, input, 0644); err != nil {
		t.Fatal(err)
	}
	return path
}

func writeImagePassThroughAndFlateTextFixture(t *testing.T) (string, []byte) {
	t.Helper()
	imageBytes := []byte("opaque DCT image bytes BT\n(IMAGE-ONLY) Tj\nET\n")
	decoded := []byte("BT\n(FLATE-TEXT) Tj\nET\n")
	encoded := cliFlate(t, decoded)
	input := pdfFixture(
		"<< /Type /Page >>",
		fmt.Sprintf("<< /Type /XObject /Subtype /Image /Width 1 /Height 1 /ColorSpace /DeviceGray /BitsPerComponent 8 /Length %d /Filter /DCTDecode >>\nstream\n%sendstream", len(imageBytes), imageBytes),
		fmt.Sprintf("<< /Length %d /Filter /FlateDecode >>\nstream\n%sendstream", len(encoded), encoded),
	)
	path := filepath.Join(t.TempDir(), "image-pass-through-flate-text.pdf")
	if err := os.WriteFile(path, input, 0644); err != nil {
		t.Fatal(err)
	}
	return path, imageBytes
}

func writeUnsupportedTargetAndFlateTextFixture(t *testing.T) string {
	t.Helper()
	unsupported := []byte("BT\n(UNSUPPORTED-TEXT) Tj\nET\n")
	decoded := []byte("BT\n(FLATE-TEXT) Tj\nET\n")
	encoded := cliFlate(t, decoded)
	input := pdfFixture(
		"<< /Type /Page >>",
		fmt.Sprintf("<< /Length %d /Filter /FooDecode >>\nstream\n%sendstream", len(unsupported), unsupported),
		fmt.Sprintf("<< /Length %d /Filter /FlateDecode >>\nstream\n%sendstream", len(encoded), encoded),
	)
	path := filepath.Join(t.TempDir(), "unsupported-target-flate-text.pdf")
	if err := os.WriteFile(path, input, 0644); err != nil {
		t.Fatal(err)
	}
	return path
}

func writeFlateDecodeParmsPredictor12Fixture(t *testing.T) string {
	t.Helper()
	decoded := []byte("BT\n(08\\05515\\0552024) Tj\nET\n")
	encoded := encodeFlatePredictor12Columns1Stream(t, decoded)
	input := pdfFixture(
		"<< /Type /Page >>",
		fmt.Sprintf("<< /Length %d /Filter /FlateDecode /DecodeParms << /Predictor 12 /Columns 1 >> >>\nstream\n%sendstream", len(encoded), encoded),
	)
	path := filepath.Join(t.TempDir(), "flate-decodeparms-predictor12.pdf")
	if err := os.WriteFile(path, input, 0644); err != nil {
		t.Fatal(err)
	}
	return path
}

func writeUnsupportedDecodeParmsTargetFixture(t *testing.T) string {
	t.Helper()
	decoded := []byte("BT\n(08\\05515\\0552024) Tj\nET\n")
	encoded := cliFlate(t, decoded)
	input := pdfFixture(
		"<< /Type /Page >>",
		fmt.Sprintf("<< /Length %d /Filter /FlateDecode /DecodeParms << /Predictor 9 /Columns 1 >> >>\nstream\n%sendstream", len(encoded), encoded),
	)
	path := filepath.Join(t.TempDir(), "unsupported-decodeparms.pdf")
	if err := os.WriteFile(path, input, 0644); err != nil {
		t.Fatal(err)
	}
	return path
}

func writeFlateDecodeParmsPredictor12Columns4Fixture(t *testing.T) string {
	t.Helper()
	decoded := []byte("BT\n(08\\05515\\0552024) Tj\nET\n")
	encoded := encodeFlatePredictor12Columns4Stream(t, decoded)
	input := pdfFixture(
		"<< /Type /Page >>",
		fmt.Sprintf("<< /Length %d /Filter /FlateDecode /DecodeParms << /Predictor 12 /Columns 4 >> >>\nstream\n%sendstream", len(encoded), encoded),
	)
	path := filepath.Join(t.TempDir(), "flate-decodeparms-predictor12-columns4.pdf")
	if err := os.WriteFile(path, input, 0644); err != nil {
		t.Fatal(err)
	}
	return path
}

func writeFlateDecodeParmsPredictor12Columns4Colors3Fixture(t *testing.T) string {
	t.Helper()
	decoded := []byte("BT\n(08-15-2024) Tj\nET\n  ")
	encoded := encodeFlatePredictor12RowWidthStream(t, decoded, 4*3)
	input := pdfFixture(
		"<< /Type /Page >>",
		fmt.Sprintf("<< /Length %d /Filter /FlateDecode /DecodeParms << /Predictor 12 /Columns 4 /Colors 3 /BitsPerComponent 8 >> >>\nstream\n%sendstream", len(encoded), encoded),
	)
	path := filepath.Join(t.TempDir(), "flate-decodeparms-predictor12-columns4-colors3.pdf")
	if err := os.WriteFile(path, input, 0644); err != nil {
		t.Fatal(err)
	}
	return path
}

func writeFlateDecodeParmsPredictor12Columns4BitsPerComponent16Fixture(t *testing.T) string {
	t.Helper()
	decoded := []byte("BT\n(08-15-2024) Tj\nET\n  ")
	encoded := encodeFlatePredictor12RowWidthStream(t, decoded, 4*2)
	input := pdfFixture(
		"<< /Type /Page >>",
		fmt.Sprintf("<< /Length %d /Filter /FlateDecode /DecodeParms << /Predictor 12 /Columns 4 /BitsPerComponent 16 >> >>\nstream\n%sendstream", len(encoded), encoded),
	)
	path := filepath.Join(t.TempDir(), "flate-decodeparms-predictor12-columns4-bpc16.pdf")
	if err := os.WriteFile(path, input, 0644); err != nil {
		t.Fatal(err)
	}
	return path
}

func writeFlateDecodeParmsPredictor2BitPackedFixture(t *testing.T) string {
	t.Helper()
	decoded := []byte("BT\n(08-15-2024) Tj\nET\n")
	encoded := encodeFlatePredictor2PackedStream(t, decoded, 7, 1, 1)
	input := pdfFixture(
		"<< /Type /Page >>",
		fmt.Sprintf("<< /Length %d /Filter /FlateDecode /DecodeParms << /Predictor 2 /Columns 7 /Colors 1 /BitsPerComponent 1 >> >>\nstream\n%sendstream", len(encoded), encoded),
	)
	path := filepath.Join(t.TempDir(), "flate-decodeparms-predictor2-bitpacked.pdf")
	if err := os.WriteFile(path, input, 0644); err != nil {
		t.Fatal(err)
	}
	return path
}

func encodeFlatePredictor12Columns1Stream(t *testing.T, decoded []byte) []byte {
	t.Helper()
	predicted := make([]byte, 0, len(decoded)*2)
	var previous byte
	for _, current := range decoded {
		predicted = append(predicted, 2, current-previous)
		previous = current
	}
	var compressed bytes.Buffer
	writer := zlib.NewWriter(&compressed)
	if _, err := writer.Write(predicted); err != nil {
		writer.Close()
		t.Fatal(err)
	}
	if err := writer.Close(); err != nil {
		t.Fatal(err)
	}
	return compressed.Bytes()
}

func encodeFlatePredictor2PackedStream(t *testing.T, decoded []byte, columns, colors, bitsPerComponent int) []byte {
	t.Helper()
	rowBits := columns * colors * bitsPerComponent
	rowBytes := (rowBits + 7) / 8
	sampleCount := columns * colors
	mask := (uint64(1) << bitsPerComponent) - 1
	predicted := make([]byte, 0, len(decoded))
	for rowStart := 0; rowStart < len(decoded); rowStart += rowBytes {
		rowEnd := rowStart + rowBytes
		if rowEnd > len(decoded) {
			t.Fatalf("decoded stream has partial TIFF predictor row: len=%d rowBytes=%d", len(decoded), rowBytes)
		}
		decodedRow := decoded[rowStart:rowEnd]
		predictedRow := bytes.Clone(decodedRow)
		for sample := 0; sample < sampleCount; sample++ {
			value := readPackedPredictorSampleForTest(decodedRow, sample, bitsPerComponent)
			if sample >= colors {
				left := readPackedPredictorSampleForTest(decodedRow, sample-colors, bitsPerComponent)
				value = (value - left) & mask
			}
			writePackedPredictorSampleForTest(predictedRow, sample, bitsPerComponent, value)
		}
		predicted = append(predicted, predictedRow...)
	}
	var compressed bytes.Buffer
	writer := zlib.NewWriter(&compressed)
	if _, err := writer.Write(predicted); err != nil {
		writer.Close()
		t.Fatal(err)
	}
	if err := writer.Close(); err != nil {
		t.Fatal(err)
	}
	return compressed.Bytes()
}

func readPackedPredictorSampleForTest(row []byte, sample, bitsPerComponent int) uint64 {
	bitOffset := sample * bitsPerComponent
	var value uint64
	for bit := 0; bit < bitsPerComponent; bit++ {
		position := bitOffset + bit
		value = value<<1 | uint64((row[position/8]>>(7-position%8))&1)
	}
	return value
}

func writePackedPredictorSampleForTest(row []byte, sample, bitsPerComponent int, value uint64) {
	bitOffset := sample * bitsPerComponent
	for bit := 0; bit < bitsPerComponent; bit++ {
		position := bitOffset + bit
		mask := byte(1 << (7 - position%8))
		if (value>>(bitsPerComponent-1-bit))&1 == 1 {
			row[position/8] |= mask
			continue
		}
		row[position/8] &^= mask
	}
}

func encodeFlatePredictor12RowWidthStream(t *testing.T, decoded []byte, rowWidth int) []byte {
	t.Helper()
	predicted := make([]byte, 0, len(decoded)+len(decoded)/rowWidth+1)
	for rowStart := 0; rowStart < len(decoded); rowStart += rowWidth {
		rowEnd := rowStart + rowWidth
		if rowEnd > len(decoded) {
			rowEnd = len(decoded)
		}
		predicted = append(predicted, 2)
		for i := rowStart; i < rowEnd; i++ {
			var previous byte
			if previousIndex := i - rowWidth; previousIndex >= 0 {
				previous = decoded[previousIndex]
			}
			predicted = append(predicted, decoded[i]-previous)
		}
	}
	var compressed bytes.Buffer
	writer := zlib.NewWriter(&compressed)
	if _, err := writer.Write(predicted); err != nil {
		writer.Close()
		t.Fatal(err)
	}
	if err := writer.Close(); err != nil {
		t.Fatal(err)
	}
	return compressed.Bytes()
}

func encodeFlatePredictor12Columns4Stream(t *testing.T, decoded []byte) []byte {
	t.Helper()
	const columns = 4
	predicted := make([]byte, 0, len(decoded)+len(decoded)/columns+1)
	for rowStart := 0; rowStart < len(decoded); rowStart += columns {
		rowEnd := rowStart + columns
		if rowEnd > len(decoded) {
			rowEnd = len(decoded)
		}
		predicted = append(predicted, 2)
		for i := rowStart; i < rowEnd; i++ {
			var previous byte
			if previousIndex := i - columns; previousIndex >= 0 {
				previous = decoded[previousIndex]
			}
			predicted = append(predicted, decoded[i]-previous)
		}
	}
	var compressed bytes.Buffer
	writer := zlib.NewWriter(&compressed)
	if _, err := writer.Write(predicted); err != nil {
		writer.Close()
		t.Fatal(err)
	}
	if err := writer.Close(); err != nil {
		t.Fatal(err)
	}
	return compressed.Bytes()
}

func encodeASCII85FlateStream(t *testing.T, decoded []byte) []byte {
	t.Helper()
	var compressed bytes.Buffer
	writer := zlib.NewWriter(&compressed)
	if _, err := writer.Write(decoded); err != nil {
		writer.Close()
		t.Fatal(err)
	}
	if err := writer.Close(); err != nil {
		t.Fatal(err)
	}
	encoded := make([]byte, ascii85.MaxEncodedLen(compressed.Len()))
	n := ascii85.Encode(encoded, compressed.Bytes())
	return append(encoded[:n], '~', '>')
}

func pdfFixture(objects ...string) []byte {
	var out bytes.Buffer
	out.WriteString("%PDF-1.3\n")
	offsets := make([]int, 0, len(objects))
	for i, object := range objects {
		offsets = append(offsets, out.Len())
		fmt.Fprintf(&out, "%d 0 obj\n%s\nendobj\n", i+1, object)
	}
	xrefOffset := out.Len()
	fmt.Fprintf(&out, "xref\n0 %d\n", len(objects)+1)
	out.WriteString("0000000000 65535 f \n")
	for _, offset := range offsets {
		fmt.Fprintf(&out, "%010d 00000 n \n", offset)
	}
	fmt.Fprintf(&out, "trailer\n<< /Size %d /Root 1 0 R >>\nstartxref\n%d\n%%%%EOF\n", len(objects)+1, xrefOffset)
	return out.Bytes()
}

func assertCLIQueryCount(t *testing.T, stdout string, want int) {
	t.Helper()
	var query struct {
		Count int `json:"count"`
	}
	if err := json.Unmarshal([]byte(stdout), &query); err != nil {
		t.Fatal(err)
	}
	if query.Count != want {
		t.Fatalf("query count = %d, want %d", query.Count, want)
	}
}

func assertCanonicalCLIEditResult(t *testing.T, stdout string) {
	t.Helper()
	var result struct {
		Report struct {
			Edit          string `json:"edit"`
			FallbackUsed  bool   `json:"fallback_used"`
			NodesModified int    `json:"nodes_modified"`
		} `json:"report"`
		Verification struct {
			ReparseOK      bool `json:"reparse_ok"`
			OldTextRemoved bool `json:"old_text_removed"`
			NewSelectable  bool `json:"new_text_selectable"`
		} `json:"verification"`
	}
	if err := json.Unmarshal([]byte(stdout), &result); err != nil {
		t.Fatal(err)
	}
	if result.Report.Edit != "pdf.canonical_content_stream_text_rewrite" {
		t.Fatalf("edit = %q, want canonical rewrite", result.Report.Edit)
	}
	if result.Report.FallbackUsed {
		t.Fatal("edit used fallback path")
	}
	if result.Report.NodesModified != 1 {
		t.Fatalf("nodes modified = %d, want 1", result.Report.NodesModified)
	}
	if !result.Verification.ReparseOK || !result.Verification.OldTextRemoved || !result.Verification.NewSelectable {
		t.Fatalf("verification = %+v", result.Verification)
	}
}

func assertStringDoesNotContain(t *testing.T, got, label string, values ...string) {
	t.Helper()
	for _, value := range values {
		if value == "" {
			continue
		}
		if strings.Contains(got, value) {
			t.Fatalf("%s leaked %q in %q", label, value, got)
		}
	}
}

type preserveStructureCLIEditResult struct {
	Report struct {
		Edit          string         `json:"edit"`
		FallbackUsed  bool           `json:"fallback_used"`
		NodesModified int            `json:"nodes_modified"`
		OutputPath    string         `json:"output_path"`
		Meta          map[string]any `json:"meta"`
	} `json:"report"`
	Verification struct {
		ReparseOK      bool `json:"reparse_ok"`
		OldTextRemoved bool `json:"old_text_removed"`
		NewSelectable  bool `json:"new_text_selectable"`
	} `json:"verification"`
}

func decodePreserveStructureCLIEditResult(t *testing.T, stdout string) preserveStructureCLIEditResult {
	t.Helper()
	var result preserveStructureCLIEditResult
	if err := json.Unmarshal([]byte(stdout), &result); err != nil {
		t.Fatal(err)
	}
	return result
}

func assertPreserveStructureCLIReportMeta(t *testing.T, meta map[string]any, wantPath string, wantCanonicalPath bool, wantStructure map[string]any) {
	t.Helper()
	if meta["writer_mode"] != "preserve-structure" {
		t.Fatalf("writer_mode = %v, want preserve-structure; meta=%+v", meta["writer_mode"], meta)
	}
	if meta["writer_path"] != wantPath || meta["used_canonical_writer_path"] != wantCanonicalPath {
		t.Fatalf("writer metadata = %+v, want writer_path=%q used_canonical_writer_path=%t", meta, wantPath, wantCanonicalPath)
	}
	structure, ok := meta["structure_plan"].(map[string]any)
	if !ok {
		t.Fatalf("structure_plan = %#v, want map", meta["structure_plan"])
	}
	for key, want := range wantStructure {
		if got := structure[key]; fmt.Sprint(got) != fmt.Sprint(want) {
			t.Fatalf("structure_plan[%q] = %v, want %v; structure=%+v", key, got, want, structure)
		}
	}
}

func assertCLISelectableEditResult(t *testing.T, stdout string) {
	t.Helper()
	var result struct {
		Report struct {
			Edit          string `json:"edit"`
			FallbackUsed  bool   `json:"fallback_used"`
			NodesModified int    `json:"nodes_modified"`
		} `json:"report"`
		Verification struct {
			ReparseOK      bool `json:"reparse_ok"`
			OldTextRemoved bool `json:"old_text_removed"`
			NewSelectable  bool `json:"new_text_selectable"`
			PageUnchanged  bool `json:"page_count_unchanged"`
		} `json:"verification"`
	}
	if err := json.Unmarshal([]byte(stdout), &result); err != nil {
		t.Fatal(err)
	}
	if result.Report.Edit != "pdf.content_stream_text_rewrite" {
		t.Fatalf("edit = %q, want surgical content stream rewrite", result.Report.Edit)
	}
	if result.Report.FallbackUsed {
		t.Fatal("edit used fallback path")
	}
	if result.Report.NodesModified != 1 {
		t.Fatalf("nodes modified = %d, want 1", result.Report.NodesModified)
	}
	if !result.Verification.ReparseOK || !result.Verification.OldTextRemoved || !result.Verification.NewSelectable || !result.Verification.PageUnchanged {
		t.Fatalf("verification = %+v", result.Verification)
	}
}

type coreNodeJSON struct {
	Kind string         `json:"kind"`
	Meta map[string]any `json:"meta"`
}

type streamFilterReportJSON struct {
	Total              int `json:"total"`
	EditableTargets    int `json:"editable_targets"`
	PassThroughStreams int `json:"pass_through_streams"`
	UnsupportedTargets int `json:"unsupported_targets"`
}

func assertCLIStreamFilterSummary(t *testing.T, got streamFilterReportJSON, total, editable, passThrough, unsupported int) {
	t.Helper()
	if got.Total != total || got.EditableTargets != editable || got.PassThroughStreams != passThrough || got.UnsupportedTargets != unsupported {
		t.Fatalf("stream filter summary = %+v, want total=%d editable=%d pass_through=%d unsupported=%d", got, total, editable, passThrough, unsupported)
	}
}

func requireCLIStreamMeta(t *testing.T, streams []coreNodeJSON, key string, value any) map[string]any {
	t.Helper()
	for _, stream := range streams {
		if stream.Kind != "pdf.stream" {
			continue
		}
		if fmt.Sprint(stream.Meta[key]) == fmt.Sprint(value) {
			return stream.Meta
		}
	}
	t.Fatalf("missing stream with %s=%v in %+v", key, value, streams)
	return nil
}

func containsString(values []string, want string) bool {
	for _, value := range values {
		if value == want {
			return true
		}
	}
	return false
}

func writeDuplicateTextFixture(t *testing.T) string {
	t.Helper()
	path := filepath.Join(t.TempDir(), "duplicate.pdf")
	input := []byte("%PDF-1.3\n1 0 obj\n<< /Type /Page /Length 59 >>\nstream\nBT\n(08\\05515\\0552024) Tj\n0 -12 Td\n(08\\05515\\0552024) Tj\nET\nendstream\nendobj\nxref\n0 2\n0000000000 65535 f \n0000000009 00000 n \ntrailer\n<< /Size 2 /Root 1 0 R >>\nstartxref\n127\n%%EOF\n")
	if err := os.WriteFile(path, input, 0644); err != nil {
		t.Fatal(err)
	}
	return path
}

func writeTextOperatorsFixture(t *testing.T) string {
	t.Helper()
	content := []byte("BT\n(08\\05515\\0552024) Tj\n0 -12 Td\n[(08) (-) (15) (-) (2024)] TJ\nET\n")
	path := filepath.Join(t.TempDir(), "text-operators.pdf")
	input := pdfFixture(fmt.Sprintf("<< /Type /Page /Length %d >>\nstream\n%sendstream", len(content), content))
	if err := os.WriteFile(path, input, 0644); err != nil {
		t.Fatal(err)
	}
	return path
}

func writeMalformedFixture(t *testing.T) string {
	t.Helper()
	path := filepath.Join(t.TempDir(), "malformed.pdf")
	input := []byte("%PDF-1.3\n1 0 obj\n<< /Type /Page /Length 27 >>\nstream\nBT\n(08\\05515\\0552024) Tj\nET\nendstream\nendobj\n")
	if err := os.WriteFile(path, input, 0644); err != nil {
		t.Fatal(err)
	}
	return path
}

func writeObjectStreamFixture(t *testing.T) string {
	t.Helper()
	path := filepath.Join(t.TempDir(), "object-stream.pdf")
	input := []byte("%PDF-1.5\n1 0 obj\n<< /Type /Catalog >>\nendobj\n3 0 obj\n<< /Length 4 >>\nstream\nwxyz\nendstream\nendobj\n7 0 obj\n<< /Type /ObjStm /N 1 /First 4 /Length 8 >>\nstream\n0 0 \nendstream\nendobj\nxref\n0 8\n0000000000 65535 f \ntrailer\n<< /Size 8 >>\nstartxref\n145\n%%EOF\n")
	if err := os.WriteFile(path, input, 0644); err != nil {
		t.Fatal(err)
	}
	return path
}

func writeObjectStreamContentFixture(t *testing.T) string {
	t.Helper()
	content := []byte("BT\n(08\\05515\\0552024) Tj\nET\n")
	var input bytes.Buffer
	input.WriteString("%PDF-1.5\n")
	catalogOffset := input.Len()
	input.WriteString("1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n")
	contentOffset := input.Len()
	fmt.Fprintf(&input, "3 0 obj\n<< /Length %d >>\nstream\n%sendstream\nendobj\n", len(content), content)
	page := []byte("<< /Type /Page /Contents 3 0 R >>")
	objectStreamBody := append([]byte("2 0 "), page...)
	objectStreamOffset := input.Len()
	fmt.Fprintf(&input, "4 0 obj\n<< /Type /ObjStm /N 1 /First 4 /Length %d >>\nstream\n%sendstream\nendobj\n", len(objectStreamBody), objectStreamBody)
	xrefOffset := input.Len()
	input.WriteString("xref\n0 5\n")
	input.WriteString("0000000000 65535 f \n")
	fmt.Fprintf(&input, "%010d 00000 n \n", catalogOffset)
	input.WriteString("0000000000 65535 f \n")
	fmt.Fprintf(&input, "%010d 00000 n \n", contentOffset)
	fmt.Fprintf(&input, "%010d 00000 n \n", objectStreamOffset)
	fmt.Fprintf(&input, "trailer\n<< /Size 5 /Root 1 0 R >>\nstartxref\n%d\n%%%%EOF\n", xrefOffset)
	path := filepath.Join(t.TempDir(), "object-stream-content.pdf")
	if err := os.WriteFile(path, input.Bytes(), 0644); err != nil {
		t.Fatal(err)
	}
	return path
}

func writeXrefStreamFixture(t *testing.T) string {
	t.Helper()
	path := filepath.Join(t.TempDir(), "xref-stream.pdf")
	input := []byte("%PDF-1.5\n1 0 obj\n<< /Type /Catalog >>\nendobj\n8 0 obj\n<< /Type /XRef /Length 4 /Size 9 /W [1 2 1] >>\nstream\nabcd\nendstream\nendobj\nstartxref\n45\n%%EOF\n")
	if err := os.WriteFile(path, input, 0644); err != nil {
		t.Fatal(err)
	}
	return path
}

func writeXrefStreamContentFixture(t *testing.T) string {
	t.Helper()
	content := []byte("BT\n(08\\05515\\0552024) Tj\nET\n")
	var out bytes.Buffer
	out.WriteString("%PDF-1.5\n")
	offsets := []int{0}
	offsets = append(offsets, out.Len())
	out.WriteString("1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n")
	offsets = append(offsets, out.Len())
	out.WriteString("2 0 obj\n<< /Type /Page /Contents 3 0 R >>\nendobj\n")
	offsets = append(offsets, out.Len())
	fmt.Fprintf(&out, "3 0 obj\n<< /Length %d >>\nstream\n%sendstream\nendobj\n", len(content), content)
	xrefOffset := out.Len()
	offsets = append(offsets, xrefOffset)
	xrefData := make([]byte, 0, 5*6)
	xrefData = appendXrefStreamEntry(xrefData, 0, 0, 255)
	for objectNumber := 1; objectNumber <= 4; objectNumber++ {
		xrefData = appendXrefStreamEntry(xrefData, 1, offsets[objectNumber], 0)
	}
	fmt.Fprintf(&out, "4 0 obj\n<< /Type /XRef /Size 5 /Root 1 0 R /W [1 4 1] /Length %d >>\nstream\n", len(xrefData))
	out.Write(xrefData)
	out.WriteString("\nendstream\nendobj\nstartxref\n")
	out.WriteString(strconv.Itoa(xrefOffset))
	out.WriteString("\n%%EOF\n")
	path := filepath.Join(t.TempDir(), "xref-stream-content.pdf")
	if err := os.WriteFile(path, out.Bytes(), 0644); err != nil {
		t.Fatal(err)
	}
	return path
}

func appendXrefStreamEntry(out []byte, entryType, field2, field3 int) []byte {
	out = append(out, byte(entryType))
	out = append(out, byte(field2>>24), byte(field2>>16), byte(field2>>8), byte(field2))
	out = append(out, byte(field3))
	return out
}

func captureStdout(t *testing.T, fn func() error) string {
	t.Helper()
	out, err := captureStdoutAndError(t, fn)
	if err != nil {
		t.Fatal(err)
	}
	return out
}

func captureStdoutAndError(t *testing.T, fn func() error) (string, error) {
	t.Helper()
	old := os.Stdout
	r, w, err := os.Pipe()
	if err != nil {
		t.Fatal(err)
	}
	os.Stdout = w
	runErr := fn()
	_ = w.Close()
	os.Stdout = old
	out, err := os.ReadFile(r.Name())
	if err == nil && len(out) > 0 {
		return string(out), runErr
	}
	buf := new(bytes.Buffer)
	_, _ = buf.ReadFrom(r)
	return buf.String(), runErr
}
