package pdf

import (
	"bytes"
	"strings"
	"testing"
)

func TestApplyAnnotationContentsEditUpdatesContentsAndCanonicalWrites(t *testing.T) {
	input := testPDF(
		"<< /Type /Catalog /Pages 2 0 R >>",
		"<< /Type /Page /Annots [3 0 R] >>",
		"<< /Type /Annot /Subtype /Text /Rect [0 0 10 10] /Contents (old note) /AP << /N 4 0 R >> >>",
		"<< /Length 0 >>\nstream\n\nendstream",
	)

	output, report, verification, err := ApplyAnnotationContentsEdit(input, 0, "new (note) - ok")
	if err != nil {
		t.Fatal(err)
	}

	if report.Edit != annotationContentsEditOperation {
		t.Fatalf("report edit = %q", report.Edit)
	}
	if report.FallbackUsed {
		t.Fatal("unexpected fallback")
	}
	if report.NodesModified != 1 {
		t.Fatalf("nodes modified = %d, want 1", report.NodesModified)
	}
	if report.ObjectNumber != 3 || report.ObjectGeneration != 0 {
		t.Fatalf("object = %d %d, want 3 0", report.ObjectNumber, report.ObjectGeneration)
	}
	if report.OldContents != "old note" || report.NewContents != "new (note) - ok" {
		t.Fatalf("contents report = old %q new %q", report.OldContents, report.NewContents)
	}
	if report.AppearanceRegenerated || !strings.Contains(report.AppearanceNote, "not implemented") {
		t.Fatalf("appearance report = regenerated %v note %q", report.AppearanceRegenerated, report.AppearanceNote)
	}
	if report.AppearanceInvalidated || report.AppearanceRemoved {
		t.Fatalf("appearance invalidation report = invalidated %v removed %v", report.AppearanceInvalidated, report.AppearanceRemoved)
	}
	if !verification.ReparseOK || !verification.ContentsUpdated || !verification.PageUnchanged || verification.AppearanceRegenerated || verification.AppearanceInvalidated || verification.AppearanceRemoved {
		t.Fatalf("verification = %+v", verification)
	}
	if bytes.Contains(output, []byte("(old note)")) {
		t.Fatalf("old contents remain:\n%s", output)
	}
	if !bytes.Contains(output, []byte(`/Contents (new \(note\) \055 ok)`)) {
		t.Fatalf("new escaped contents missing:\n%s", output)
	}
	if !bytes.Contains(output, []byte("xref\n0 5\n")) {
		t.Fatalf("xref table was not rebuilt:\n%s", output)
	}
	if !bytes.Contains(output, []byte("/AP ")) {
		t.Fatalf("appearance dictionary was unexpectedly removed:\n%s", output)
	}
	if !annotationCandidateHasAP(t, output, 0) {
		t.Fatalf("updated annotation no longer has /AP:\n%s", output)
	}
}

func TestApplyAnnotationContentsEditCanRemoveStaleAppearance(t *testing.T) {
	input := testPDF(
		"<< /Type /Catalog /Pages 2 0 R >>",
		"<< /Type /Page /Annots [3 0 R] >>",
		"<< /Type /Annot /Subtype /Text /Rect [0 0 10 10] /Contents (old note) /AP << /N 4 0 R >> >>",
		"<< /Length 0 >>\nstream\n\nendstream",
	)

	output, report, verification, err := ApplyAnnotationContentsEdit(input, 0, "fresh note", AnnotationContentsEditOptions{
		RemoveAppearance: true,
	})
	if err != nil {
		t.Fatal(err)
	}

	if report.AppearanceRegenerated {
		t.Fatalf("appearance regeneration should remain false: %+v", report)
	}
	if !report.AppearanceInvalidated || !report.AppearanceRemoved {
		t.Fatalf("appearance invalidation report = invalidated %v removed %v", report.AppearanceInvalidated, report.AppearanceRemoved)
	}
	if !strings.Contains(report.AppearanceNote, "removed") {
		t.Fatalf("appearance note = %q, want removal note", report.AppearanceNote)
	}
	if !verification.ReparseOK || !verification.ContentsUpdated || !verification.PageUnchanged || verification.AppearanceRegenerated || !verification.AppearanceInvalidated || !verification.AppearanceRemoved {
		t.Fatalf("verification = %+v", verification)
	}
	if annotationCandidateHasAP(t, output, 0) {
		t.Fatalf("updated annotation still has /AP:\n%s", output)
	}
}

func TestApplyAnnotationContentsEditUsesZeroBasedIndex(t *testing.T) {
	input := testPDF(
		"<< /Type /Catalog /Pages 2 0 R >>",
		"<< /Type /Page /Annots [3 0 R 4 0 R] >>",
		"<< /Type /Annot /Subtype /Text /Rect [0 0 10 10] /Contents (first) >>",
		"<< /Type /Annot /Subtype /Text /Rect [0 0 10 10] /Contents (second) >>",
	)

	output, report, verification, err := ApplyAnnotationContentsEdit(input, 1, "updated second")
	if err != nil {
		t.Fatal(err)
	}

	if report.ObjectNumber != 4 {
		t.Fatalf("object number = %d, want 4", report.ObjectNumber)
	}
	if !verification.ContentsUpdated {
		t.Fatalf("verification = %+v", verification)
	}
	if !bytes.Contains(output, []byte("/Contents (first)")) {
		t.Fatalf("first annotation changed unexpectedly:\n%s", output)
	}
	if !bytes.Contains(output, []byte("/Contents (updated second)")) {
		t.Fatalf("second annotation was not updated:\n%s", output)
	}
}

func TestListAnnotationCandidatesIncludesStableMetadata(t *testing.T) {
	input := testPDF(
		"<< /Type /Catalog /Pages 2 0 R >>",
		"<< /Type /Page /Annots [3 0 R << /Subtype /FreeText /Rect [0.5 -1 20 20.25] /Contents <FEFF0069006E006C0069006E0065> /F 513 >> << /Subtype /Square /Rect [0 0 10] /Contents (unsupported rect) >>] >>",
		"<< /Type /Annot /Subtype /Text /Rect [0 0 10 10] /Contents (indirect note) /F 628 /AP << /N 4 0 R >> >>",
		"<< /Length 0 >>\nstream\n\nendstream",
	)

	candidates, err := ListAnnotationCandidates(input)
	if err != nil {
		t.Fatal(err)
	}

	if len(candidates) != 3 {
		t.Fatalf("candidate count = %d, want 3: %+v", len(candidates), candidates)
	}
	first := candidates[0]
	if first.Index != 0 || first.ObjectNumber == nil || *first.ObjectNumber != 3 || first.ObjectGeneration == nil || *first.ObjectGeneration != 0 {
		t.Fatalf("first object metadata = %+v, want index 0 object 3 0", first)
	}
	if first.PageIndex == nil || *first.PageIndex != 0 || first.PageObjectNumber == nil || *first.PageObjectNumber != 2 || first.PageObjectGeneration == nil || *first.PageObjectGeneration != 0 {
		t.Fatalf("first page metadata = %+v, want page index 0 object 2 0", first)
	}
	if first.Subtype != "Text" || first.Contents != "indirect note" || !first.HasAppearance {
		t.Fatalf("first annotation metadata = %+v", first)
	}
	assertFloat64SliceEqual(t, first.Rect, []float64{0, 0, 10, 10})
	if first.Flags != 628 {
		t.Fatalf("first flags = %d, want 628", first.Flags)
	}
	assertStringSliceEqual(t, first.FlagNames, []string{"print", "no_rotate", "no_view", "read_only", "locked_contents"})
	if first.Invisible || first.Hidden || !first.Print || first.NoZoom || !first.NoRotate || !first.NoView || !first.ReadOnly || first.Locked || first.ToggleNoView || !first.LockedContents {
		t.Fatalf("first flag booleans = %+v", first)
	}
	second := candidates[1]
	if second.Index != 1 || second.ObjectNumber != nil || second.ObjectGeneration != nil {
		t.Fatalf("second object metadata = %+v, want direct index 1", second)
	}
	if second.PageIndex == nil || *second.PageIndex != 0 || second.PageObjectNumber == nil || *second.PageObjectNumber != 2 || second.PageObjectGeneration == nil || *second.PageObjectGeneration != 0 {
		t.Fatalf("second page metadata = %+v, want page index 0 object 2 0", second)
	}
	if second.Subtype != "FreeText" || second.Contents != "inline" || second.HasAppearance {
		t.Fatalf("second annotation metadata = %+v", second)
	}
	assertFloat64SliceEqual(t, second.Rect, []float64{0.5, -1, 20, 20.25})
	if second.Flags != 513 {
		t.Fatalf("second flags = %d, want 513", second.Flags)
	}
	assertStringSliceEqual(t, second.FlagNames, []string{"invisible", "locked_contents"})
	if !second.Invisible || second.Hidden || second.Print || second.NoZoom || second.NoRotate || second.NoView || second.ReadOnly || second.Locked || second.ToggleNoView || !second.LockedContents {
		t.Fatalf("second flag booleans = %+v", second)
	}
	third := candidates[2]
	if third.Index != 2 || third.ObjectNumber != nil || third.ObjectGeneration != nil {
		t.Fatalf("third object metadata = %+v, want direct index 2", third)
	}
	if third.Subtype != "Square" || third.Contents != "unsupported rect" || third.HasAppearance {
		t.Fatalf("third annotation metadata = %+v", third)
	}
	if third.Rect != nil {
		t.Fatalf("third rect = %+v, want omitted", third.Rect)
	}
	if third.Flags != 0 || len(third.FlagNames) != 0 || third.Invisible || third.Hidden || third.Print || third.NoZoom || third.NoRotate || third.NoView || third.ReadOnly || third.Locked || third.ToggleNoView || third.LockedContents {
		t.Fatalf("third flags = %+v, want zero-value flags", third)
	}
}

func TestListAnnotationCandidatesReportsPageTreeIndex(t *testing.T) {
	input := testPDF(
		"<< /Type /Catalog /Pages 2 0 R >>",
		"<< /Type /Pages /Kids [3 0 R 5 0 R] /Count 2 >>",
		"<< /Type /Page /Annots [4 0 R] >>",
		"<< /Type /Annot /Subtype /Text /Rect [0 0 10 10] /Contents (first page note) >>",
		"<< /Type /Page /Annots [6 0 R] >>",
		"<< /Type /Annot /Subtype /Text /Rect [0 0 10 10] /Contents (second page note) >>",
	)

	candidates, err := ListAnnotationCandidates(input)
	if err != nil {
		t.Fatal(err)
	}

	if len(candidates) != 2 {
		t.Fatalf("candidate count = %d, want 2: %+v", len(candidates), candidates)
	}
	first := candidates[0]
	if first.Index != 0 || first.PageIndex == nil || *first.PageIndex != 0 || first.PageObjectNumber == nil || *first.PageObjectNumber != 3 {
		t.Fatalf("first page metadata = %+v, want page index 0 object 3", first)
	}
	second := candidates[1]
	if second.Index != 1 || second.PageIndex == nil || *second.PageIndex != 1 || second.PageObjectNumber == nil || *second.PageObjectNumber != 5 {
		t.Fatalf("second page metadata = %+v, want page index 1 object 5", second)
	}
}

func TestApplyAnnotationContentsEditAddsMissingContents(t *testing.T) {
	input := testPDF(
		"<< /Type /Catalog /Pages 2 0 R >>",
		"<< /Type /Page /Annots [<< /Subtype /Text /Rect [0 0 10 10] >>] >>",
	)

	output, report, verification, err := ApplyAnnotationContentsEdit(input, 0, "inline note")
	if err != nil {
		t.Fatal(err)
	}

	if report.ObjectNumber != 0 {
		t.Fatalf("inline annotation should not report an indirect object: %+v", report)
	}
	if !verification.ContentsUpdated {
		t.Fatalf("verification = %+v", verification)
	}
	if !bytes.Contains(output, []byte("/Contents (inline note)")) {
		t.Fatalf("inline annotation contents missing:\n%s", output)
	}
}

func TestApplyAnnotationContentsEditRejectsOutOfRangeIndex(t *testing.T) {
	input := testPDF(
		"<< /Type /Catalog /Pages 2 0 R >>",
		"<< /Type /Page /Annots [3 0 R] >>",
		"<< /Type /Annot /Subtype /Text /Rect [0 0 10 10] /Contents (only) >>",
	)

	_, _, _, err := ApplyAnnotationContentsEdit(input, 1, "nope")
	if err == nil {
		t.Fatal("expected out-of-range error")
	}
	if got := err.Error(); got != "annotation index 1 out of range for 1 annotations (zero-based)" {
		t.Fatalf("error = %q", got)
	}
}

func annotationCandidateHasAP(t *testing.T, input []byte, index int) bool {
	t.Helper()

	graph, err := parsePDFGraph(input)
	if err != nil {
		t.Fatal(err)
	}
	candidates := graph.annotationCandidates()
	if index < 0 || index >= len(candidates) {
		t.Fatalf("annotation index %d out of range for %d annotations", index, len(candidates))
	}
	_, ok := candidates[index].Dict["AP"]
	return ok
}

func assertFloat64SliceEqual(t *testing.T, got, want []float64) {
	t.Helper()

	if len(got) != len(want) {
		t.Fatalf("slice length = %d, want %d: got %+v want %+v", len(got), len(want), got, want)
	}
	for i := range want {
		if got[i] != want[i] {
			t.Fatalf("slice[%d] = %v, want %v: got %+v want %+v", i, got[i], want[i], got, want)
		}
	}
}

func assertStringSliceEqual(t *testing.T, got, want []string) {
	t.Helper()

	if len(got) != len(want) {
		t.Fatalf("slice length = %d, want %d: got %+v want %+v", len(got), len(want), got, want)
	}
	for i := range want {
		if got[i] != want[i] {
			t.Fatalf("slice[%d] = %q, want %q: got %+v want %+v", i, got[i], want[i], got, want)
		}
	}
}
