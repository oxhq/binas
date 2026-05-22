package pdf

import (
	"bytes"
	"encoding/json"
	"fmt"
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
	if report.AppearanceRegenerated || !strings.Contains(report.AppearanceNote, "left unchanged") {
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

func TestApplyAnnotationContentsEditCanRegenerateBasicTextAppearance(t *testing.T) {
	input := testPDF(
		"<< /Type /Catalog /Pages 2 0 R >>",
		"<< /Type /Page /Annots [3 0 R] >>",
		"<< /Type /Annot /Subtype /FreeText /Rect [10 20 90 50] /Contents (old note) /AP << /N 4 0 R >> >>",
		"<< /Length 0 >>\nstream\n\nendstream",
	)

	output, report, verification, err := ApplyAnnotationContentsEdit(input, 0, "fresh (note) - ok", AnnotationContentsEditOptions{
		RegenerateAppearance: true,
	})
	if err != nil {
		t.Fatal(err)
	}

	if !report.AppearanceRegenerated || report.AppearanceInvalidated || report.AppearanceRemoved {
		t.Fatalf("appearance report = regenerated %v invalidated %v removed %v note %q", report.AppearanceRegenerated, report.AppearanceInvalidated, report.AppearanceRemoved, report.AppearanceNote)
	}
	if !strings.Contains(report.AppearanceNote, "basic") {
		t.Fatalf("appearance note = %q, want basic regeneration note", report.AppearanceNote)
	}
	if !verification.ReparseOK || !verification.ContentsUpdated || !verification.PageUnchanged || !verification.AppearanceRegenerated || verification.AppearanceInvalidated || verification.AppearanceRemoved {
		t.Fatalf("verification = %+v", verification)
	}
	if !annotationCandidateHasNormalAppearanceStream(t, output, 0) {
		t.Fatalf("updated annotation missing reparsed /AP /N stream:\n%s", output)
	}
	for _, want := range [][]byte{
		[]byte("/AP << /N 5 0 R >>"),
		[]byte("/Subtype /Form"),
		[]byte("/BBox [0 0 80 30]"),
		[]byte("/BaseFont /Helvetica"),
		[]byte(`/Helv 10 Tf`),
		[]byte(`(fresh \(note\) \055) Tj`),
		[]byte(`(ok) Tj`),
	} {
		if !bytes.Contains(output, want) {
			t.Fatalf("output missing %q:\n%s", want, output)
		}
	}
}

func TestApplyAnnotationContentsEditCanRegenerateTextAnnotationAppearance(t *testing.T) {
	input := testPDF(
		"<< /Type /Catalog /Pages 2 0 R >>",
		"<< /Type /Page /Annots [3 0 R] >>",
		"<< /Type /Annot /Subtype /Text /Rect [10 20 110 48] /Contents (old sticky note) >>",
	)

	output, report, verification, err := ApplyAnnotationContentsEdit(input, 0, "fresh sticky note", AnnotationContentsEditOptions{
		RegenerateAppearance: true,
	})
	if err != nil {
		t.Fatal(err)
	}
	if !report.AppearanceRegenerated || !verification.AppearanceRegenerated {
		t.Fatalf("appearance was not regenerated: report %+v verification %+v", report, verification)
	}

	stream := annotationCandidateNormalAppearanceStream(t, output, 0)
	got := string(stream.Data)
	for _, want := range []string{
		"0 0 100 28 re W n",
		"BT\n/Helv 10 Tf",
		"0 0 0 rg",
		"(fresh sticky note) Tj",
		"ET\nQ",
	} {
		if !strings.Contains(got, want) {
			t.Fatalf("text annotation appearance missing %q in %q", want, got)
		}
	}
}

func TestApplyAnnotationContentsEditCanRegenerateSquareAppearance(t *testing.T) {
	input := testPDF(
		"<< /Type /Catalog /Pages 2 0 R >>",
		"<< /Type /Page /Annots [3 0 R] >>",
		"<< /Type /Annot /Subtype /Square /Rect [10 20 50 45] /Contents (old square) /C [1 0.5 0] >>",
	)

	output, report, verification, err := ApplyAnnotationContentsEdit(input, 0, "fresh square", AnnotationContentsEditOptions{
		RegenerateAppearance: true,
	})
	if err != nil {
		t.Fatal(err)
	}

	if !report.AppearanceRegenerated || report.AppearanceInvalidated || report.AppearanceRemoved {
		t.Fatalf("appearance report = regenerated %v invalidated %v removed %v note %q", report.AppearanceRegenerated, report.AppearanceInvalidated, report.AppearanceRemoved, report.AppearanceNote)
	}
	if !strings.Contains(report.AppearanceNote, "basic") {
		t.Fatalf("appearance note = %q, want basic regeneration note", report.AppearanceNote)
	}
	if !verification.ReparseOK || !verification.ContentsUpdated || !verification.PageUnchanged || !verification.AppearanceRegenerated || verification.AppearanceInvalidated || verification.AppearanceRemoved {
		t.Fatalf("verification = %+v", verification)
	}

	stream := annotationCandidateNormalAppearanceStream(t, output, 0)
	if got := string(stream.Data); !strings.Contains(got, "1 0.5 0 RG") || !strings.Contains(got, "0.5 0.5 39 24 re S") {
		t.Fatalf("square appearance data = %q", got)
	}
	if bbox := stream.Dict["BBox"].(pdfArray); len(bbox) != 4 || fmt.Sprint(bbox[2]) != "40" || fmt.Sprint(bbox[3]) != "25" {
		t.Fatalf("square BBox = %+v, want width 40 height 25", bbox)
	}
}

func TestApplyAnnotationContentsEditCanRegenerateSquareAppearanceWithBSBorderStyle(t *testing.T) {
	input := testPDF(
		"<< /Type /Catalog /Pages 2 0 R >>",
		"<< /Type /Page /Annots [3 0 R] >>",
		"<< /Type /Annot /Subtype /Square /Rect [10 20 50 45] /Contents (old square) /C [1 0.5 0] /BS << /W 2 /S /D /D [4 2] >> >>",
	)

	output, report, verification, err := ApplyAnnotationContentsEdit(input, 0, "fresh square", AnnotationContentsEditOptions{
		RegenerateAppearance: true,
	})
	if err != nil {
		t.Fatal(err)
	}
	if !report.AppearanceRegenerated || !verification.AppearanceRegenerated {
		t.Fatalf("appearance was not regenerated: report %+v verification %+v", report, verification)
	}

	stream := annotationCandidateNormalAppearanceStream(t, output, 0)
	got := string(stream.Data)
	for _, want := range []string{
		"2 w",
		"[4 2] 0 d",
		"1 0.5 0 RG",
		"1 1 38 23 re S",
	} {
		if !strings.Contains(got, want) {
			t.Fatalf("square /BS appearance missing %q in %q", want, got)
		}
	}
}

func TestApplyAnnotationContentsEditRegenerateAppearanceFailsClosedForUnsupportedAnnotationInputs(t *testing.T) {
	tests := []struct {
		name    string
		annot   string
		wantErr string
	}{
		{
			name:    "rich text content",
			annot:   "<< /Type /Annot /Subtype /FreeText /Rect [0 0 80 30] /Contents (old note) /RC (<b>old note</b>) >>",
			wantErr: "cannot regenerate annotation appearance: annotation 0 has unsupported rich text content",
		},
		{
			name:    "javascript action",
			annot:   "<< /Type /Annot /Subtype /Text /Rect [0 0 80 30] /Contents (old note) /A << /S /JavaScript /JS (app.alert('x')) >> >>",
			wantErr: "cannot regenerate annotation appearance: annotation 0 has unsupported JavaScript action",
		},
		{
			name:    "additional actions",
			annot:   "<< /Type /Annot /Subtype /Square /Rect [0 0 80 30] /Contents (old square) /AA << /E << /S /URI /URI (https://example.test) >> >> >>",
			wantErr: "cannot regenerate annotation appearance: annotation 0 has unsupported additional actions",
		},
		{
			name:    "unsupported subtype",
			annot:   "<< /Type /Annot /Subtype /Popup /Rect [0 0 80 30] /Contents (old popup) >>",
			wantErr: `cannot regenerate annotation appearance: unsupported annotation subtype "Popup"`,
		},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			input := testPDF(
				"<< /Type /Catalog /Pages 2 0 R >>",
				"<< /Type /Page /Annots [3 0 R] >>",
				tt.annot,
			)

			_, _, _, err := ApplyAnnotationContentsEdit(input, 0, "new note", AnnotationContentsEditOptions{
				RegenerateAppearance: true,
			})
			if err == nil {
				t.Fatal("expected regenerate appearance to fail")
			}
			if got := err.Error(); got != tt.wantErr {
				t.Fatalf("error = %q, want %q", got, tt.wantErr)
			}
		})
	}
}

func TestApplyAnnotationContentsEditCanRegenerateCircleAppearance(t *testing.T) {
	input := testPDF(
		"<< /Type /Catalog /Pages 2 0 R >>",
		"<< /Type /Page /Annots [3 0 R] >>",
		"<< /Type /Annot /Subtype /Circle /Rect [0 0 20 10] /Contents (old circle) /C [0.25] >>",
	)

	output, report, verification, err := ApplyAnnotationContentsEdit(input, 0, "fresh circle", AnnotationContentsEditOptions{
		RegenerateAppearance: true,
	})
	if err != nil {
		t.Fatal(err)
	}

	if !report.AppearanceRegenerated || report.AppearanceInvalidated || report.AppearanceRemoved {
		t.Fatalf("appearance report = regenerated %v invalidated %v removed %v note %q", report.AppearanceRegenerated, report.AppearanceInvalidated, report.AppearanceRemoved, report.AppearanceNote)
	}
	if !verification.ReparseOK || !verification.ContentsUpdated || !verification.PageUnchanged || !verification.AppearanceRegenerated || verification.AppearanceInvalidated || verification.AppearanceRemoved {
		t.Fatalf("verification = %+v", verification)
	}

	stream := annotationCandidateNormalAppearanceStream(t, output, 0)
	got := string(stream.Data)
	for _, want := range []string{
		"0.25 G",
		"19.5 5 m",
		" c\n",
		" c S\n",
	} {
		if !strings.Contains(got, want) {
			t.Fatalf("circle appearance missing %q in %q", want, got)
		}
	}
	if bbox := stream.Dict["BBox"].(pdfArray); len(bbox) != 4 || fmt.Sprint(bbox[2]) != "20" || fmt.Sprint(bbox[3]) != "10" {
		t.Fatalf("circle BBox = %+v, want width 20 height 10", bbox)
	}
}

func TestApplyAnnotationContentsEditCanRegenerateLinkBorderAppearance(t *testing.T) {
	input := testPDF(
		"<< /Type /Catalog /Pages 2 0 R >>",
		"<< /Type /Page /Annots [3 0 R] >>",
		"<< /Type /Annot /Subtype /Link /Rect [10 20 50 40] /Contents (old link) /Border [0 0 2.5] /C [0 0 1] /A << /S /URI /URI (https://example.test) >> >>",
	)

	output, report, verification, err := ApplyAnnotationContentsEdit(input, 0, "fresh link", AnnotationContentsEditOptions{
		RegenerateAppearance: true,
	})
	if err != nil {
		t.Fatal(err)
	}

	if !report.AppearanceRegenerated || report.AppearanceInvalidated || report.AppearanceRemoved {
		t.Fatalf("appearance report = regenerated %v invalidated %v removed %v note %q", report.AppearanceRegenerated, report.AppearanceInvalidated, report.AppearanceRemoved, report.AppearanceNote)
	}
	if !verification.ReparseOK || !verification.ContentsUpdated || !verification.PageUnchanged || !verification.AppearanceRegenerated || verification.AppearanceInvalidated || verification.AppearanceRemoved {
		t.Fatalf("verification = %+v", verification)
	}

	stream := annotationCandidateNormalAppearanceStream(t, output, 0)
	got := string(stream.Data)
	for _, want := range []string{
		"2.5 w",
		"0 0 1 RG",
		"1.25 1.25 37.5 17.5 re S",
	} {
		if !strings.Contains(got, want) {
			t.Fatalf("link appearance missing %q in %q", want, got)
		}
	}
	if bbox := stream.Dict["BBox"].(pdfArray); len(bbox) != 4 || fmt.Sprint(bbox[2]) != "40" || fmt.Sprint(bbox[3]) != "20" {
		t.Fatalf("link BBox = %+v, want width 40 height 20", bbox)
	}
	if !bytes.Contains(output, []byte("/A << /S /URI /URI (https://example.test) >>")) {
		t.Fatalf("link URI action was not preserved:\n%s", output)
	}
}

func TestApplyAnnotationContentsEditCanRegenerateLinkBSBorderAppearance(t *testing.T) {
	tests := []struct {
		name    string
		bs      string
		wantOps []string
	}{
		{
			name: "solid",
			bs:   "<< /W 3 /S /S >>",
			wantOps: []string{
				"3 w",
				"0 0 1 RG",
				"1.5 1.5 37 17 re S",
			},
		},
		{
			name: "dashed",
			bs:   "<< /W 2 /S /D /D [4 1.5] >>",
			wantOps: []string{
				"2 w",
				"[4 1.5] 0 d",
				"1 1 38 18 re S",
			},
		},
		{
			name: "underline",
			bs:   "<< /W 2 /S /U >>",
			wantOps: []string{
				"2 w",
				"1 1 m",
				"39 1 l S",
			},
		},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			input := testPDF(
				"<< /Type /Catalog /Pages 2 0 R >>",
				"<< /Type /Page /Annots [3 0 R] >>",
				fmt.Sprintf("<< /Type /Annot /Subtype /Link /Rect [10 20 50 40] /Contents (old link) /Border [0 0 9] /BS %s /C [0 0 1] >>", tt.bs),
			)

			output, report, verification, err := ApplyAnnotationContentsEdit(input, 0, "fresh link", AnnotationContentsEditOptions{
				RegenerateAppearance: true,
			})
			if err != nil {
				t.Fatal(err)
			}
			if !report.AppearanceRegenerated || !verification.AppearanceRegenerated {
				t.Fatalf("appearance was not regenerated: report %+v verification %+v", report, verification)
			}

			stream := annotationCandidateNormalAppearanceStream(t, output, 0)
			got := string(stream.Data)
			for _, want := range tt.wantOps {
				if !strings.Contains(got, want) {
					t.Fatalf("link /BS appearance missing %q in %q", want, got)
				}
			}
		})
	}
}

func TestApplyAnnotationContentsEditRegenerateLinkAppearanceFailsClosedForUnsupportedVisualInputs(t *testing.T) {
	tests := []struct {
		name    string
		annot   string
		wantErr string
	}{
		{
			name:    "unsupported border style dictionary style",
			annot:   "<< /Type /Annot /Subtype /Link /Rect [0 0 20 10] /Contents (old link) /Border [0 0 1] /BS << /W 1 /S /B >> >>",
			wantErr: "unsupported /BS border style /S /B",
		},
		{
			name:    "indirect border style dictionary",
			annot:   "<< /Type /Annot /Subtype /Link /Rect [0 0 20 10] /Contents (old link) /Border [0 0 1] /BS 9 0 R >>",
			wantErr: "unsupported /BS border style",
		},
		{
			name:    "malformed border style width",
			annot:   "<< /Type /Annot /Subtype /Link /Rect [0 0 20 10] /Contents (old link) /BS << /W /bad /S /S >> >>",
			wantErr: "unsupported /BS /W border width",
		},
		{
			name:    "unsupported dash array",
			annot:   "<< /Type /Annot /Subtype /Link /Rect [0 0 20 10] /Contents (old link) /BS << /W 1 /S /D /D [0 0] >> >>",
			wantErr: "unsupported /BS /D dash array",
		},
		{
			name:    "dashed border array",
			annot:   "<< /Type /Annot /Subtype /Link /Rect [0 0 20 10] /Contents (old link) /Border [0 0 1 [3]] >>",
			wantErr: "unsupported /Border",
		},
		{
			name:    "javascript action",
			annot:   "<< /Type /Annot /Subtype /Link /Rect [0 0 20 10] /Contents (old link) /Border [0 0 1] /A << /S /JavaScript /JS (app.alert('x')) >> >>",
			wantErr: "unsupported JavaScript action",
		},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			input := testPDF(
				"<< /Type /Catalog /Pages 2 0 R >>",
				"<< /Type /Page /Annots [3 0 R] >>",
				tt.annot,
			)

			_, _, _, err := ApplyAnnotationContentsEdit(input, 0, "fresh link", AnnotationContentsEditOptions{
				RegenerateAppearance: true,
			})
			if err == nil {
				t.Fatal("expected regenerate appearance to fail")
			}
			if !strings.Contains(err.Error(), tt.wantErr) {
				t.Fatalf("error = %q, want substring %q", err, tt.wantErr)
			}
		})
	}
}

func TestApplyAnnotationContentsEditRegenerateShapeAppearanceFailsClosedForUnsupportedBS(t *testing.T) {
	tests := []struct {
		name    string
		annot   string
		wantErr string
	}{
		{
			name:    "underline square",
			annot:   "<< /Type /Annot /Subtype /Square /Rect [0 0 20 10] /Contents (old square) /BS << /W 1 /S /U >> >>",
			wantErr: "unsupported /BS border style /S /U for /Square",
		},
		{
			name:    "unsupported dashed square dash array",
			annot:   "<< /Type /Annot /Subtype /Square /Rect [0 0 20 10] /Contents (old square) /BS << /W 1 /S /D /D [-1 2] >> >>",
			wantErr: "unsupported /BS /D dash array",
		},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			input := testPDF(
				"<< /Type /Catalog /Pages 2 0 R >>",
				"<< /Type /Page /Annots [3 0 R] >>",
				tt.annot,
			)

			_, _, _, err := ApplyAnnotationContentsEdit(input, 0, "fresh square", AnnotationContentsEditOptions{
				RegenerateAppearance: true,
			})
			if err == nil {
				t.Fatal("expected regenerate appearance to fail")
			}
			if !strings.Contains(err.Error(), tt.wantErr) {
				t.Fatalf("error = %q, want substring %q", err, tt.wantErr)
			}
		})
	}
}

func TestApplyAnnotationContentsEditCanRegenerateHighlightAppearance(t *testing.T) {
	input := testPDF(
		"<< /Type /Catalog /Pages 2 0 R >>",
		"<< /Type /Page /Annots [3 0 R] >>",
		"<< /Type /Annot /Subtype /Highlight /Rect [10 20 80 40] /Contents (old highlight) /C [0.5 0.75 0.25] /QuadPoints [12 36 72 36 12 24 72 24] >>",
	)

	output, report, verification, err := ApplyAnnotationContentsEdit(input, 0, "fresh highlight", AnnotationContentsEditOptions{
		RegenerateAppearance: true,
	})
	if err != nil {
		t.Fatal(err)
	}

	if !report.AppearanceRegenerated || report.AppearanceInvalidated || report.AppearanceRemoved {
		t.Fatalf("appearance report = regenerated %v invalidated %v removed %v note %q", report.AppearanceRegenerated, report.AppearanceInvalidated, report.AppearanceRemoved, report.AppearanceNote)
	}
	if !strings.Contains(report.AppearanceNote, "basic") {
		t.Fatalf("appearance note = %q, want basic regeneration note", report.AppearanceNote)
	}
	if !verification.ReparseOK || !verification.ContentsUpdated || !verification.PageUnchanged || !verification.AppearanceRegenerated || verification.AppearanceInvalidated || verification.AppearanceRemoved {
		t.Fatalf("verification = %+v", verification)
	}

	stream := annotationCandidateNormalAppearanceStream(t, output, 0)
	got := string(stream.Data)
	for _, want := range []string{
		"0.5 0.75 0.25 rg",
		"2 16 m",
		"62 16 l",
		"62 4 l",
		"2 4 l f",
	} {
		if !strings.Contains(got, want) {
			t.Fatalf("highlight appearance missing %q in %q", want, got)
		}
	}
	if bbox := stream.Dict["BBox"].(pdfArray); len(bbox) != 4 || fmt.Sprint(bbox[2]) != "70" || fmt.Sprint(bbox[3]) != "20" {
		t.Fatalf("highlight BBox = %+v, want width 70 height 20", bbox)
	}
}

func TestApplyAnnotationContentsEditCanRegenerateUnderlineAndStrikeOutAppearance(t *testing.T) {
	tests := []struct {
		name          string
		subtype       string
		color         string
		wantColor     string
		wantFirstLine string
		wantSecond    string
	}{
		{
			name:          "underline",
			subtype:       "Underline",
			color:         "[0 0 1]",
			wantColor:     "0 0 1 RG",
			wantFirstLine: "4 3.2 m",
			wantSecond:    "54 3.2 l S",
		},
		{
			name:          "strikeout",
			subtype:       "StrikeOut",
			color:         "[0.25]",
			wantColor:     "0.25 G",
			wantFirstLine: "4 8 m",
			wantSecond:    "54 8 l S",
		},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			input := testPDF(
				"<< /Type /Catalog /Pages 2 0 R >>",
				"<< /Type /Page /Annots [3 0 R] >>",
				fmt.Sprintf("<< /Type /Annot /Subtype /%s /Rect [10 20 80 40] /Contents (old markup) /C %s /QuadPoints [14 34 64 34 14 22 64 22] >>", tt.subtype, tt.color),
			)

			output, report, verification, err := ApplyAnnotationContentsEdit(input, 0, "fresh markup", AnnotationContentsEditOptions{
				RegenerateAppearance: true,
			})
			if err != nil {
				t.Fatal(err)
			}
			if !report.AppearanceRegenerated || !verification.AppearanceRegenerated {
				t.Fatalf("appearance was not regenerated: report %+v verification %+v", report, verification)
			}

			stream := annotationCandidateNormalAppearanceStream(t, output, 0)
			got := string(stream.Data)
			for _, want := range []string{tt.wantColor, tt.wantFirstLine, tt.wantSecond} {
				if !strings.Contains(got, want) {
					t.Fatalf("%s appearance missing %q in %q", tt.subtype, want, got)
				}
			}
		})
	}
}

func TestApplyAnnotationContentsEditRegenerateMarkupAppearanceRequiresDirectUsableQuadPoints(t *testing.T) {
	tests := []struct {
		name    string
		annot   string
		wantErr string
	}{
		{
			name:    "missing",
			annot:   "<< /Type /Annot /Subtype /Highlight /Rect [0 0 10 10] /Contents (old note) >>",
			wantErr: "has no /QuadPoints",
		},
		{
			name:    "wrong length",
			annot:   "<< /Type /Annot /Subtype /Underline /Rect [0 0 10 10] /Contents (old note) /QuadPoints [0 0 10 10] >>",
			wantErr: "has malformed /QuadPoints",
		},
		{
			name:    "non numeric",
			annot:   "<< /Type /Annot /Subtype /StrikeOut /Rect [0 0 10 10] /Contents (old note) /QuadPoints [0 0 10 10 0 5 /bad 5] >>",
			wantErr: "has malformed /QuadPoints",
		},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			input := testPDF(
				"<< /Type /Catalog /Pages 2 0 R >>",
				"<< /Type /Page /Annots [3 0 R] >>",
				tt.annot,
			)

			_, _, _, err := ApplyAnnotationContentsEdit(input, 0, "new note", AnnotationContentsEditOptions{
				RegenerateAppearance: true,
			})
			if err == nil {
				t.Fatal("expected regenerate appearance to fail")
			}
			if !strings.Contains(err.Error(), tt.wantErr) {
				t.Fatalf("error = %q, want substring %q", err, tt.wantErr)
			}
		})
	}
}

func TestApplyAnnotationContentsEditRegeneratesMultilineWrappedAppearance(t *testing.T) {
	input := testPDF(
		"<< /Type /Catalog /Pages 2 0 R >>",
		"<< /Type /Page /Annots [3 0 R] >>",
		"<< /Type /Annot /Subtype /FreeText /Rect [10 20 74 60] /Contents (old note) >>",
	)

	output, _, verification, err := ApplyAnnotationContentsEdit(input, 0, "first (line)\nsecond wraps here", AnnotationContentsEditOptions{
		RegenerateAppearance: true,
	})
	if err != nil {
		t.Fatal(err)
	}
	if !verification.AppearanceRegenerated {
		t.Fatalf("verification = %+v", verification)
	}
	for _, want := range [][]byte{
		[]byte("0 0 64 40 re W n"),
		[]byte(`(first \(line\)) Tj`),
		[]byte("(second wraps) Tj"),
		[]byte("(here) Tj"),
		[]byte("0 -12 Td"),
	} {
		if !bytes.Contains(output, want) {
			t.Fatalf("output missing %q:\n%s", want, output)
		}
	}
}

func TestApplyAnnotationContentsEditRegenerateAppearanceTruncatesToRectHeight(t *testing.T) {
	input := testPDF(
		"<< /Type /Catalog /Pages 2 0 R >>",
		"<< /Type /Page /Annots [3 0 R] >>",
		"<< /Type /Annot /Subtype /Text /Rect [0 0 100 16] /Contents (old note) >>",
	)

	output, _, verification, err := ApplyAnnotationContentsEdit(input, 0, "visible\ntruncated", AnnotationContentsEditOptions{
		RegenerateAppearance: true,
	})
	if err != nil {
		t.Fatal(err)
	}
	if !verification.AppearanceRegenerated {
		t.Fatalf("verification = %+v", verification)
	}
	if !bytes.Contains(output, []byte("(visible) Tj")) {
		t.Fatalf("visible line missing:\n%s", output)
	}
	if bytes.Contains(output, []byte("(truncated) Tj")) {
		t.Fatalf("line outside rectangle was not truncated:\n%s", output)
	}
}

func TestApplyAnnotationContentsEditRegenerateAppearanceFailsWithoutUsableRect(t *testing.T) {
	input := testPDF(
		"<< /Type /Catalog /Pages 2 0 R >>",
		"<< /Type /Page /Annots [3 0 R] >>",
		"<< /Type /Annot /Subtype /Text /Contents (old note) >>",
	)

	_, _, _, err := ApplyAnnotationContentsEdit(input, 0, "new note", AnnotationContentsEditOptions{
		RegenerateAppearance: true,
	})
	if err == nil {
		t.Fatal("expected regenerate appearance to fail without a usable Rect")
	}
	if !strings.Contains(err.Error(), "no usable /Rect") {
		t.Fatalf("error = %q", err)
	}
}

func TestApplyAnnotationContentsEditRegenerateAppearanceFailsForUnsupportedSubtype(t *testing.T) {
	input := testPDF(
		"<< /Type /Catalog /Pages 2 0 R >>",
		"<< /Type /Page /Annots [3 0 R] >>",
		"<< /Type /Annot /Subtype /Popup /Rect [0 0 10 10] /Contents (old note) >>",
	)

	_, _, _, err := ApplyAnnotationContentsEdit(input, 0, "new note", AnnotationContentsEditOptions{
		RegenerateAppearance: true,
	})
	if err == nil {
		t.Fatal("expected regenerate appearance to fail for unsupported subtype")
	}
	if !strings.Contains(err.Error(), `unsupported annotation subtype "Popup"`) {
		t.Fatalf("error = %q", err)
	}
}

func TestVerifyAnnotationContentsEditRejectsMismatchedRegeneratedAppearance(t *testing.T) {
	input := testPDF(
		"<< /Type /Catalog /Pages 2 0 R >>",
		"<< /Type /Page /Annots [3 0 R] >>",
		"<< /Type /Annot /Subtype /FreeText /Rect [10 20 90 50] /Contents (old note) >>",
	)

	output, _, verification, err := ApplyAnnotationContentsEdit(input, 0, "fresh note", AnnotationContentsEditOptions{
		RegenerateAppearance: true,
	})
	if err != nil {
		t.Fatal(err)
	}
	if !verification.AppearanceRegenerated {
		t.Fatalf("verification = %+v", verification)
	}

	graph, err := parsePDFGraph(output)
	if err != nil {
		t.Fatal(err)
	}
	candidates := graph.annotationCandidates()
	stream, ok := annotationNormalAppearanceStream(graph, candidates[0].Dict)
	if !ok {
		t.Fatal("missing regenerated normal appearance stream")
	}
	stream.Data = []byte("q\nQ")
	ap := candidates[0].Dict["AP"].(pdfDict)
	ref := ap["N"].(pdfRef)
	graph.Objects[ref.ID].Value = stream
	tampered, err := writeCanonicalPDF(graph)
	if err != nil {
		t.Fatal(err)
	}

	verification, err = verifyAnnotationContentsEdit(tampered, 0, "fresh note", graph.pageCount(), false, true)
	if err == nil {
		t.Fatalf("expected mismatched appearance verification to fail: %+v", verification)
	}
	if got := err.Error(); got != "verification failed: annotation /AP /N appearance stream was not regenerated" {
		t.Fatalf("error = %q", got)
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
		"<< /Type /Page /Annots [3 0 R << /Subtype /FreeText /Rect [0.5 -1 20 20.25] /Contents <FEFF0069006E006C0069006E0065> /NM <696E6C696E652D6E616D65> /M (D:20260505090200-08'00') /T (Inline Reviewer) /F 513 /C [0.25 0.5 0.75] /Border [1 2 3.5] /QuadPoints [0 0 20 0 20 20 0 20] >> << /Subtype /Square /Rect [0 0 10] /Contents (unsupported rect) /NM /not-text /M 42 /T [ (bad) ] /C /Red /Border [0 /solid 1] /QuadPoints [0 0 1 1] >>] >>",
		"<< /Type /Annot /Subtype /Text /Rect [0 0 10 10] /Contents (indirect note) /NM (note-001) /M (D:20260505090100-08'00') /T <FEFF00440061007600690064> /F 628 /AP << /N 4 0 R >> /C [1 0.5 0] /Border [0 0 2] /QuadPoints [0 0 10 0 10 10 0 10 1 1 11 1 11 11 1 11] >>",
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
	if first.Name != "note-001" || first.Modified != "D:20260505090100-08'00'" || first.Title != "David" {
		t.Fatalf("first annotation common fields = name %q modified %q title %q", first.Name, first.Modified, first.Title)
	}
	assertFloat64SliceEqual(t, first.Rect, []float64{0, 0, 10, 10})
	assertFloat64SliceEqual(t, first.Color, []float64{1, 0.5, 0})
	assertFloat64SliceEqual(t, first.Border, []float64{0, 0, 2})
	if first.QuadPointsCount != 2 {
		t.Fatalf("first quad points count = %d, want 2", first.QuadPointsCount)
	}
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
	if second.Name != "inline-name" || second.Modified != "D:20260505090200-08'00'" || second.Title != "Inline Reviewer" {
		t.Fatalf("second annotation common fields = name %q modified %q title %q", second.Name, second.Modified, second.Title)
	}
	assertFloat64SliceEqual(t, second.Rect, []float64{0.5, -1, 20, 20.25})
	assertFloat64SliceEqual(t, second.Color, []float64{0.25, 0.5, 0.75})
	assertFloat64SliceEqual(t, second.Border, []float64{1, 2, 3.5})
	if second.QuadPointsCount != 1 {
		t.Fatalf("second quad points count = %d, want 1", second.QuadPointsCount)
	}
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
	if third.Name != "" || third.Modified != "" || third.Title != "" {
		t.Fatalf("third unsupported common fields = name %q modified %q title %q, want empty", third.Name, third.Modified, third.Title)
	}
	if third.Rect != nil {
		t.Fatalf("third rect = %+v, want omitted", third.Rect)
	}
	if third.Color != nil || third.Border != nil || third.QuadPointsCount != 0 {
		t.Fatalf("third style metadata = color %+v border %+v quad count %d, want omitted/zero", third.Color, third.Border, third.QuadPointsCount)
	}
	if third.Flags != 0 || len(third.FlagNames) != 0 || third.Invisible || third.Hidden || third.Print || third.NoZoom || third.NoRotate || third.NoView || third.ReadOnly || third.Locked || third.ToggleNoView || third.LockedContents {
		t.Fatalf("third flags = %+v, want zero-value flags", third)
	}

	encoded, err := json.Marshal(second)
	if err != nil {
		t.Fatal(err)
	}
	for _, key := range []string{`"color"`, `"border"`, `"quad_points_count"`} {
		if !bytes.Contains(encoded, []byte(key)) {
			t.Fatalf("encoded metadata %s missing key %s", encoded, key)
		}
	}
}

func TestListAnnotationCandidatesReportsUnsafeAppearanceGenerationStatus(t *testing.T) {
	input := testPDF(
		"<< /Type /Catalog /Pages 2 0 R >>",
		"<< /Type /Page /Annots [3 0 R 4 0 R 5 0 R 6 0 R] >>",
		"<< /Type /Annot /Subtype /Text /Rect [0 0 80 30] /Contents (plain note) >>",
		"<< /Type /Annot /Subtype /FreeText /Rect [0 0 80 30] /Contents (rich note) /RC (<b>rich note</b>) >>",
		"<< /Type /Annot /Subtype /Text /Rect [0 0 80 30] /Contents (script note) /A << /S /JavaScript /JS (app.alert('x')) >> >>",
		"<< /Type /Annot /Subtype /Square /Rect [0 0 80 30] /Contents (actions) /AA << /E << /S /URI /URI (https://example.test) >> >> >>",
	)

	candidates, err := ListAnnotationCandidates(input)
	if err != nil {
		t.Fatal(err)
	}
	if len(candidates) != 4 {
		t.Fatalf("candidate count = %d, want 4: %+v", len(candidates), candidates)
	}
	if candidates[0].AppearanceGenerationStatus != "approximate_supported" || len(candidates[0].AppearanceGenerationBlockers) != 0 {
		t.Fatalf("plain annotation appearance status = %q blockers %+v", candidates[0].AppearanceGenerationStatus, candidates[0].AppearanceGenerationBlockers)
	}
	for i, want := range [][]string{
		{"rich_text_content"},
		{"javascript_action"},
		{"additional_actions"},
	} {
		candidate := candidates[i+1]
		if candidate.AppearanceGenerationStatus != "unsafe" {
			t.Fatalf("candidate %d appearance status = %q, want unsafe", i+1, candidate.AppearanceGenerationStatus)
		}
		assertStringSliceEqual(t, candidate.AppearanceGenerationBlockers, want)
	}

	encoded, err := json.Marshal(candidates[2])
	if err != nil {
		t.Fatal(err)
	}
	for _, key := range []string{`"appearance_generation_status":"unsafe"`, `"appearance_generation_blockers":["javascript_action"]`} {
		if !bytes.Contains(encoded, []byte(key)) {
			t.Fatalf("encoded metadata missing %s: %s", key, encoded)
		}
	}
}

func TestListAnnotationCandidatesReportsUnsupportedBSAppearanceGenerationStatus(t *testing.T) {
	input := testPDF(
		"<< /Type /Catalog /Pages 2 0 R >>",
		"<< /Type /Page /Annots [3 0 R 4 0 R 5 0 R] >>",
		"<< /Type /Annot /Subtype /Link /Rect [0 0 80 30] /Contents (solid link) /BS << /W 2 /S /S >> >>",
		"<< /Type /Annot /Subtype /Link /Rect [0 0 80 30] /Contents (beveled link) /BS << /W 2 /S /B >> >>",
		"<< /Type /Annot /Subtype /Square /Rect [0 0 80 30] /Contents (underline square) /BS << /W 1 /S /U >> >>",
	)

	candidates, err := ListAnnotationCandidates(input)
	if err != nil {
		t.Fatal(err)
	}
	if len(candidates) != 3 {
		t.Fatalf("candidate count = %d, want 3: %+v", len(candidates), candidates)
	}
	if candidates[0].AppearanceGenerationStatus != "approximate_supported" || len(candidates[0].AppearanceGenerationBlockers) != 0 {
		t.Fatalf("solid /BS link appearance status = %q blockers %+v", candidates[0].AppearanceGenerationStatus, candidates[0].AppearanceGenerationBlockers)
	}
	for i := 1; i < len(candidates); i++ {
		if candidates[i].AppearanceGenerationStatus != "unsupported" {
			t.Fatalf("candidate %d appearance status = %q, want unsupported", i, candidates[i].AppearanceGenerationStatus)
		}
		assertStringSliceEqual(t, candidates[i].AppearanceGenerationBlockers, []string{"unsupported_bs_border_style"})
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

func annotationCandidateHasNormalAppearanceStream(t *testing.T, input []byte, index int) bool {
	t.Helper()

	_, ok := annotationCandidateNormalAppearanceStreamIfPresent(t, input, index)
	return ok
}

func annotationCandidateNormalAppearanceStream(t *testing.T, input []byte, index int) pdfStreamObject {
	t.Helper()

	stream, ok := annotationCandidateNormalAppearanceStreamIfPresent(t, input, index)
	if !ok {
		t.Fatalf("updated annotation missing normal appearance stream:\n%s", input)
	}
	return stream
}

func annotationCandidateNormalAppearanceStreamIfPresent(t *testing.T, input []byte, index int) (pdfStreamObject, bool) {
	t.Helper()

	graph, err := parsePDFGraph(input)
	if err != nil {
		t.Fatal(err)
	}
	candidates := graph.annotationCandidates()
	if index < 0 || index >= len(candidates) {
		t.Fatalf("annotation index %d out of range for %d annotations", index, len(candidates))
	}
	ap, ok := candidates[index].Dict["AP"].(pdfDict)
	if !ok {
		return pdfStreamObject{}, false
	}
	switch normal := ap["N"].(type) {
	case pdfRef:
		object := graph.Objects[normal.ID]
		if object == nil {
			return pdfStreamObject{}, false
		}
		stream, ok := object.Value.(pdfStreamObject)
		return stream, ok
	case pdfStreamObject:
		return normal, true
	default:
		return pdfStreamObject{}, false
	}
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
