package main

import (
	"encoding/json"
	"os"
	"strings"
	"testing"
)

func TestInspectPDFJSONReportsRootMetadata(t *testing.T) {
	result := decodeJSONResult(t, inspectPDFJSON(wasmTestPDF("08-15-2024")))

	if result["ok"] != true {
		t.Fatalf("ok = %v, want true: %+v", result["ok"], result)
	}
	if result["format"] != "pdf" {
		t.Fatalf("format = %v, want pdf", result["format"])
	}
	if result["nodes"].(float64) < 2 {
		t.Fatalf("nodes = %v, want at least document and text nodes", result["nodes"])
	}
	if _, ok := result["root"].(map[string]any); !ok {
		t.Fatalf("root = %T, want object", result["root"])
	}
}

func TestQueryPDFJSONReportsTextMatches(t *testing.T) {
	result := decodeJSONResult(t, queryPDFJSON(wasmTestPDF("08-15-2024"), "08-15-2024"))

	if result["ok"] != true {
		t.Fatalf("ok = %v, want true: %+v", result["ok"], result)
	}
	if result["count"] != float64(1) {
		t.Fatalf("count = %v, want 1", result["count"])
	}
	matches := result["matches"].([]any)
	match := matches[0].(map[string]any)
	if match["value"] != "08-15-2024" {
		t.Fatalf("match value = %v, want original text", match["value"])
	}
}

func TestEditPDFTextReturnsBytesReportAndVerification(t *testing.T) {
	result := editPDFText(wasmTestPDF("08-15-2024"), "08-15-2024", "May 5, 2026")

	if !result.OK {
		t.Fatalf("OK = false, error = %q", result.Error)
	}
	if len(result.Bytes) == 0 {
		t.Fatal("Bytes is empty")
	}
	if result.Report.Format != "pdf" || result.Report.NodesModified != 1 {
		t.Fatalf("report = %+v, want one pdf node modified", result.Report)
	}
	if result.Verification == nil || !result.Verification.ReparseOK || !result.Verification.OldTextRemoved || !result.Verification.NewSelectable {
		t.Fatalf("verification = %+v, want reparse/old-gone/new-selectable", result.Verification)
	}
	edited := decodeJSONResult(t, queryPDFJSON(result.Bytes, "May 5, 2026"))
	if edited["count"] != float64(1) {
		t.Fatalf("edited query count = %v, want 1: %+v", edited["count"], edited)
	}
}

func TestEditorHTMLIncludesProductionRenderingSurface(t *testing.T) {
	raw, err := os.ReadFile("editor.html")
	if err != nil {
		t.Fatal(err)
	}
	html := string(raw)
	for _, want := range []string{
		"id=\"pdf-canvas\"",
		"id=\"highlight-layer\"",
		"id=\"prev-page\"",
		"id=\"next-page\"",
		"id=\"zoom-in\"",
		"id=\"pdfjs-file\"",
		"id=\"pdfjs-worker-file\"",
		"pdfjs-dist@5.7.284",
		"pdfjsLib.getDocument",
		"page.getTextContent",
		"renderHighlights",
		"findFirstVisualMatchPage",
	} {
		if !strings.Contains(html, want) {
			t.Fatalf("editor.html missing %q", want)
		}
	}
	if strings.Contains(html, "This is not a production editor") {
		t.Fatal("editor.html still labels the rendering surface as non-production")
	}
}

func decodeJSONResult(t *testing.T, raw string) map[string]any {
	t.Helper()
	var result map[string]any
	if err := json.Unmarshal([]byte(raw), &result); err != nil {
		t.Fatalf("json.Unmarshal(%q): %v", raw, err)
	}
	return result
}

func wasmTestPDF(text string) []byte {
	stream := "BT (" + text + ") Tj ET"
	objects := []string{
		"<< /Type /Catalog /Pages 2 0 R >>",
		"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
		"<< /Type /Page /Parent 2 0 R /Contents 4 0 R >>",
		"<< /Length " + itoa(len(stream)) + " >>\nstream\n" + stream + "\nendstream",
	}
	out := []byte("%PDF-1.7\n")
	offsets := make([]int, 0, len(objects)+1)
	offsets = append(offsets, 0)
	for i, object := range objects {
		offsets = append(offsets, len(out))
		out = append(out, []byte(itoa(i+1)+" 0 obj\n"+object+"\nendobj\n")...)
	}
	xrefOffset := len(out)
	out = append(out, []byte("xref\n0 "+itoa(len(objects)+1)+"\n0000000000 65535 f \n")...)
	for i := 1; i < len(offsets); i++ {
		out = append(out, []byte(leftPad10(offsets[i])+" 00000 n \n")...)
	}
	out = append(out, []byte("trailer\n<< /Root 1 0 R /Size "+itoa(len(objects)+1)+" >>\nstartxref\n"+itoa(xrefOffset)+"\n%%EOF\n")...)
	return out
}

func itoa(value int) string {
	if value == 0 {
		return "0"
	}
	var digits [20]byte
	i := len(digits)
	for value > 0 {
		i--
		digits[i] = byte('0' + value%10)
		value /= 10
	}
	return string(digits[i:])
}

func leftPad10(value int) string {
	raw := itoa(value)
	for len(raw) < 10 {
		raw = "0" + raw
	}
	return raw
}
