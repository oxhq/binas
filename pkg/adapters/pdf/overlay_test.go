package pdf

import (
	"bytes"
	"encoding/json"
	"testing"

	"github.com/oxhq/binas/pkg/core"
)

func TestApplyExplicitOverlayStampAddsSelectableOverlayAndReportsFallback(t *testing.T) {
	input := testPDF(
		"<< /Type /Catalog /Pages 2 0 R >>",
		"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
		"<< /Type /Page /Parent 2 0 R /Resources << >> /Contents 4 0 R >>",
		"<< /Length 26 >>\nstream\nBT\n(Original) Tj\nET\nendstream",
	)

	output, report, verification, err := ApplyExplicitOverlayStamp(input, ExplicitOverlayStampOptions{
		PageIndex: 0,
		Text:      "APPROVED",
		X:         72,
		Y:         144,
		FontSize:  14,
	})
	if err != nil {
		t.Fatal(err)
	}

	if report.Edit != explicitOverlayStampOperation {
		t.Fatalf("edit = %q, want %q", report.Edit, explicitOverlayStampOperation)
	}
	if !report.FallbackUsed {
		t.Fatal("fallback_used = false, want true")
	}
	if report.FallbackPolicy == nil || report.FallbackPolicy.Fallback != "overlay" || report.FallbackPolicy.Mode != "explicit" {
		t.Fatalf("fallback policy = %+v, want overlay/explicit", report.FallbackPolicy)
	}
	if report.FallbackKind != "overlay" {
		t.Fatalf("fallback_kind = %q, want overlay", report.FallbackKind)
	}
	if hasCoreInvariant(report.Invariants, core.InvariantNoFallbackUsed) {
		t.Fatalf("overlay report must not claim %s: %+v", core.InvariantNoFallbackUsed, report.Invariants)
	}
	if !verification.ReparseOK || !verification.NewSelectable || !verification.PageUnchanged {
		t.Fatalf("verification = %+v, want reparse/selectable/page unchanged", verification)
	}
	if report.Verification == nil || !report.Verification.NewSelectable {
		t.Fatalf("report verification = %+v, want selectable overlay proof", report.Verification)
	}

	reparsed, err := NewAdapter().Parse(output, core.ParseOptions{Strict: true})
	if err != nil {
		t.Fatal(err)
	}
	if matches := reparsed.Query(core.Match{Kind: KindTextShow, Text: "APPROVED"}); len(matches) != 1 {
		t.Fatalf("overlay selectable matches = %d, want 1", len(matches))
	}
	if matches := reparsed.Query(core.Match{Kind: KindTextShow, Text: "Original"}); len(matches) != 1 {
		t.Fatalf("original selectable matches = %d, want 1", len(matches))
	}
	if !bytes.Contains(output, []byte("/BaseFont /Helvetica")) {
		t.Fatalf("overlay font resource missing:\n%s", output)
	}
	if !bytes.Contains(output, []byte("/Contents [4 0 R 6 0 R]")) {
		t.Fatalf("overlay content stream was not appended to page contents:\n%s", output)
	}
	if bytes.Count(output, []byte("(Original) Tj")) != 1 {
		t.Fatalf("true text stream was unexpectedly rewritten:\n%s", output)
	}

	encoded, err := json.Marshal(report)
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Contains(encoded, []byte(`"fallback_used":true`)) || !bytes.Contains(encoded, []byte(`"fallback_kind":"overlay"`)) || !bytes.Contains(encoded, []byte(`"fallback_policy":{"fallback":"overlay","mode":"explicit"}`)) {
		t.Fatalf("report JSON missing explicit overlay fallback contract: %s", encoded)
	}
}

func TestApplyExplicitOverlayStampCanAddMissingContents(t *testing.T) {
	input := testPDF(
		"<< /Type /Catalog /Pages 2 0 R >>",
		"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
		"<< /Type /Page /Parent 2 0 R /Resources << >> >>",
	)

	output, report, verification, err := ApplyExplicitOverlayStamp(input, ExplicitOverlayStampOptions{
		PageIndex: 0,
		Text:      "STAMP",
	})
	if err != nil {
		t.Fatal(err)
	}
	if !report.FallbackUsed || report.FallbackPolicy == nil || report.FallbackPolicy.Fallback != string(FallbackOverlay) || report.FallbackPolicy.Mode != string(FallbackModeExplicit) {
		t.Fatalf("report fallback = used %v policy %+v, want overlay/explicit", report.FallbackUsed, report.FallbackPolicy)
	}
	if !verification.ReparseOK || !verification.NewSelectable || !verification.PageUnchanged {
		t.Fatalf("verification = %+v, want reparse/selectable/page unchanged", verification)
	}
	if !bytes.Contains(output, []byte("/Contents 5 0 R")) {
		t.Fatalf("missing page contents did not become the overlay stream ref:\n%s", output)
	}
}

func TestApplyExplicitOverlayStampRejectsInvalidPolicyInputs(t *testing.T) {
	input := testPDF(
		"<< /Type /Catalog /Pages 2 0 R >>",
		"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
		"<< /Type /Page /Parent 2 0 R /Resources << >> >>",
	)

	if _, _, _, err := ApplyExplicitOverlayStamp(input, ExplicitOverlayStampOptions{PageIndex: -1, Text: "STAMP"}); err == nil {
		t.Fatal("expected negative page index to fail")
	}
	if _, _, _, err := ApplyExplicitOverlayStamp(input, ExplicitOverlayStampOptions{PageIndex: 1, Text: "STAMP"}); err == nil {
		t.Fatal("expected out-of-range page index to fail")
	}
	if _, _, _, err := ApplyExplicitOverlayStamp(input, ExplicitOverlayStampOptions{PageIndex: 0}); err == nil {
		t.Fatal("expected empty text to fail")
	}
}
