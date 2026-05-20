package pdf

import (
	"bytes"
	"errors"
	"fmt"
	"strings"
	"testing"

	"github.com/oxhq/binas/pkg/core"
)

func TestPackedWriterModeNormalizesPreserveStructure(t *testing.T) {
	mode, err := NormalizePDFWriterMode("preserve-structure")
	if err != nil {
		t.Fatalf("NormalizePDFWriterMode: %v", err)
	}
	if mode != PDFWriterModePreserveStructure {
		t.Fatalf("mode = %q, want %q", mode, PDFWriterModePreserveStructure)
	}
}

func TestPackedWriterPreserveStructureObjectStreamFailsClosedWhileCanonicalWorks(t *testing.T) {
	input := testObjectStreamStructurePDF()

	output, report, verification, err := ApplyCanonicalEdit(
		input,
		core.Match{Kind: KindTextShow, Text: "08-15-2024"},
		core.Mutation{Replace: "05-04-2026"},
		nil,
	)
	if err != nil {
		t.Fatalf("ApplyCanonicalEdit: %v", err)
	}
	if report.Edit != "pdf.canonical_content_stream_text_rewrite" || report.NodesModified != 1 || report.FallbackUsed {
		t.Fatalf("report = %+v, want canonical edit without fallback", report)
	}
	if !verification.ReparseOK || !verification.OldTextRemoved || !verification.NewSelectable {
		t.Fatalf("verification = %+v, want canonical proof", verification)
	}
	if bytes.Contains(output, []byte("/ObjStm")) {
		t.Fatalf("canonical output preserved object stream container:\n%s", output)
	}

	_, _, _, err = ApplyCanonicalEditWithWriterMode(
		input,
		PDFWriterModePreserveStructure,
		core.Match{Kind: KindTextShow, Text: "08-15-2024"},
		core.Mutation{Replace: "05-04-2026"},
		nil,
	)
	assertPreserveStructureUnsupported(t, err, "object_stream_objects=1", "xref_stream_objects=0")
}

func TestPackedWriterPreserveStructureNormalTableXrefUsesCanonicalWriterPath(t *testing.T) {
	content := "BT\n(08\\05515\\0552024) Tj\nET\n"
	input := testPDF(
		"<< /Type /Catalog /Pages 2 0 R >>",
		"<< /Type /Page /Contents 3 0 R >>",
		fmt.Sprintf("<< /Length %d >>\nstream\n%sendstream", len(content), content),
	)

	output, report, verification, err := ApplyCanonicalEditWithWriterMode(
		input,
		PDFWriterModePreserveStructure,
		core.Match{Kind: KindTextShow, Text: "08-15-2024"},
		core.Mutation{Replace: "05-04-2026"},
		nil,
	)
	if err != nil {
		t.Fatalf("ApplyCanonicalEditWithWriterMode: %v", err)
	}
	if report.Edit != "pdf.canonical_content_stream_text_rewrite" || report.NodesModified != 1 || report.FallbackUsed {
		t.Fatalf("report = %+v, want canonical writer path without fallback", report)
	}
	assertPreserveStructureReportMeta(t, report.Meta, map[string]any{
		"has_table_xref":         true,
		"has_xref_stream":        false,
		"has_hybrid_xref":        false,
		"object_stream_objects":  0,
		"xref_stream_objects":    0,
		"requires_packed_writer": false,
	})
	if !verification.ReparseOK || !verification.OldTextRemoved || !verification.NewSelectable {
		t.Fatalf("verification = %+v, want canonical proof", verification)
	}
	if !bytes.Contains(output, []byte("xref\n0 4\n")) {
		t.Fatalf("preserve-structure normal table xref output did not rebuild table xref:\n%s", output)
	}
	if bytes.Contains(output, []byte("/ObjStm")) || bytes.Contains(output, []byte("/Type /XRef")) || bytes.Contains(output, []byte("/XRefStm")) {
		t.Fatalf("normal table xref preserve-structure output unexpectedly contains packed structure:\n%s", output)
	}
}

func TestPackedWriterPreserveStructureXrefStreamFailsClosedWhileCanonicalWorks(t *testing.T) {
	input := testXrefStreamPDF(t)

	output, report, verification, err := ApplyCanonicalEdit(
		input,
		core.Match{Kind: KindTextShow, Text: "08-15-2024"},
		core.Mutation{Replace: "05-04-2026"},
		nil,
	)
	if err != nil {
		t.Fatalf("ApplyCanonicalEdit: %v", err)
	}
	if report.Edit != "pdf.canonical_content_stream_text_rewrite" || report.NodesModified != 1 || report.FallbackUsed {
		t.Fatalf("report = %+v, want canonical edit without fallback", report)
	}
	if !verification.ReparseOK || !verification.OldTextRemoved || !verification.NewSelectable {
		t.Fatalf("verification = %+v, want canonical proof", verification)
	}
	if bytes.Contains(output, []byte("/Type /XRef")) {
		t.Fatalf("canonical output preserved xref stream container:\n%s", output)
	}

	_, _, _, err = ApplyCanonicalEditWithWriterMode(
		input,
		PDFWriterModePreserveStructure,
		core.Match{Kind: KindTextShow, Text: "08-15-2024"},
		core.Mutation{Replace: "05-04-2026"},
		nil,
	)
	assertPreserveStructureUnsupported(t, err, "object_stream_objects=0", "xref_stream_objects=1")
}

func TestPackedWriterPreserveStructureHybridXrefFailsClosedWithMetadata(t *testing.T) {
	input := buildHybridXrefPDFFixture(t, validHybridXrefStreamData).input

	_, _, _, err := ApplyCanonicalEditWithWriterMode(
		input,
		PDFWriterModePreserveStructure,
		core.Match{Kind: KindTextShow, Text: "08-15-2024"},
		core.Mutation{Replace: "05-04-2026"},
		nil,
	)
	assertPreserveStructureUnsupported(
		t,
		err,
		"has_table_xref=true",
		"has_xref_stream=true",
		"has_hybrid_xref=true",
		"object_stream_objects=0",
		"xref_stream_objects=1",
	)
}

func testObjectStreamStructurePDF() []byte {
	content := []byte("BT\n(08\\05515\\0552024) Tj\nET\n")
	objectStreamData := "5 0 << /Fixture true >>"
	return testPDF(
		"<< /Type /Catalog /Pages 2 0 R >>",
		"<< /Type /Page /Contents 3 0 R >>",
		fmt.Sprintf("<< /Length %d >>\nstream\n%sendstream", len(content), content),
		fmt.Sprintf("<< /Type /ObjStm /N 1 /First 4 /Length %d >>\nstream\n%sendstream", len(objectStreamData), objectStreamData),
	)
}

func assertPreserveStructureUnsupported(t *testing.T, err error, details ...string) {
	t.Helper()
	if !errors.Is(err, ErrPreserveStructureRepackUnsupported) {
		t.Fatalf("error = %v, want ErrPreserveStructureRepackUnsupported", err)
	}
	var structureErr *PreserveStructureUnsupportedError
	if !errors.As(err, &structureErr) {
		t.Fatalf("error = %T, want PreserveStructureUnsupportedError", err)
	}
	structureDetails := structureErr.StructureDetails()
	for _, detail := range details {
		if !strings.Contains(err.Error(), detail) {
			t.Fatalf("error = %q, want detail %q", err, detail)
		}
	}
	if structureDetails["requires_packed_writer"] != true {
		t.Fatalf("structure details = %+v, want requires_packed_writer=true", structureDetails)
	}
}

func assertPreserveStructureReportMeta(t *testing.T, meta map[string]any, wantStructure map[string]any) {
	t.Helper()
	if meta["writer_mode"] != string(PDFWriterModePreserveStructure) {
		t.Fatalf("writer_mode = %v, want %q; meta=%+v", meta["writer_mode"], PDFWriterModePreserveStructure, meta)
	}
	if meta["writer_path"] != "canonical" || meta["used_canonical_writer_path"] != true {
		t.Fatalf("writer path metadata = %+v, want canonical path proof", meta)
	}
	structure, ok := meta["structure_plan"].(map[string]any)
	if !ok {
		t.Fatalf("structure_plan = %#v, want map", meta["structure_plan"])
	}
	for key, want := range wantStructure {
		if got := structure[key]; got != want {
			t.Fatalf("structure_plan[%q] = %v, want %v; structure=%+v", key, got, want, structure)
		}
	}
}
