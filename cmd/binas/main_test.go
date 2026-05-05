package main

import (
	"bytes"
	"compress/zlib"
	"encoding/ascii85"
	"encoding/json"
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
		"PDF boundary detected: AcroForm field values require form set; widget appearances are not regenerated",
		"PDF boundary detected: annotations can update /Contents only; appearance regeneration is not implemented",
		"PDF boundary detected: font/CMap support is limited to page font-scoped ToUnicode CMaps for simple Tf flows plus one unambiguous fallback; glyph metrics and layout are not verified",
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
			Edit          string `json:"edit"`
			NodesModified int    `json:"nodes_modified"`
			OutputPath    string `json:"output_path"`
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
			AppearanceRegenerated bool `json:"appearance_regenerated"`
			AppearanceInvalidated bool `json:"appearance_invalidated"`
			AppearanceRemoved     bool `json:"appearance_removed"`
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
	want := `unsupported rewrite mode "overlay" (expected auto, surgical, or canonical)`
	if err.Error() != want {
		t.Fatalf("error = %q, want %q", err, want)
	}
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

	oldQueryOut := captureStdout(t, func() error {
		return run([]string{"query", out, "--format", "pdf", "--kind", "pdf.content.text_show", "--text", "08-15-2024", "--json"})
	})
	assertCLIQueryCount(t, oldQueryOut, 0)

	newQueryOut := captureStdout(t, func() error {
		return run([]string{"query", out, "--format", "pdf", "--kind", "pdf.content.text_show", "--text", "May 5, 2026", "--json"})
	})
	assertCLIQueryCount(t, newQueryOut, 1)
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

func writeFixture(t *testing.T) string {
	t.Helper()
	path := filepath.Join(t.TempDir(), "fixture.pdf")
	input := []byte("%PDF-1.3\n1 0 obj\n<< /Type /Page /Length 27 >>\nstream\nBT\n(08\\05515\\0552024) Tj\nET\nendstream\nendobj\nxref\n0 2\n0000000000 65535 f \n0000000009 00000 n \ntrailer\n<< /Size 2 /Root 1 0 R >>\nstartxref\n100\n%%EOF\n")
	if err := os.WriteFile(path, input, 0644); err != nil {
		t.Fatal(err)
	}
	return path
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
	page := []byte("<< /Type /Page /Contents 3 0 R >>")
	objectStreamBody := append([]byte("2 0 "), page...)
	input := pdfFixture(
		"<< /Type /Catalog /Pages 2 0 R >>",
		fmt.Sprintf("<< /Type /ObjStm /N 1 /First 4 /Length %d >>\nstream\n%sendstream", len(objectStreamBody), objectStreamBody),
		fmt.Sprintf("<< /Length %d >>\nstream\n%sendstream", len(content), content),
	)
	path := filepath.Join(t.TempDir(), "object-stream-content.pdf")
	if err := os.WriteFile(path, input, 0644); err != nil {
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
