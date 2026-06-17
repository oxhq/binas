package pdf

import (
	"strings"
	"testing"
)

func TestTransformPagesRotatesAndCropsSelectedZeroBasedPage(t *testing.T) {
	input := pageOpsTestPDF("FIRST", "SECOND")
	rotate := 90
	crop := PageBox{Left: 10, Bottom: 20, Right: 190, Top: 180}

	output, report, verification, err := TransformPages(input, PageSelector{Indexes: []int{1}}, PageTransform{
		Rotate:  &rotate,
		CropBox: &crop,
	}, PageWriteOptions{})
	if err != nil {
		t.Fatalf("TransformPages: %v", err)
	}

	assertPageOpsOutput(t, output, 2, []string{"FIRST", "SECOND"}, nil)
	if report.Operation != "pdf.transform_pages" || report.InputPages != 2 || report.OutputPages != 2 {
		t.Fatalf("report = %+v", report)
	}
	if !verification.ReparseOK || !verification.PageCountOK || verification.ActualPageCount != 2 {
		t.Fatalf("verification = %+v", verification)
	}

	graph, err := parsePDFGraph(output)
	if err != nil {
		t.Fatalf("reparse transformed output: %v", err)
	}
	pages, err := graph.orderedPages()
	if err != nil {
		t.Fatalf("ordered pages: %v", err)
	}
	if _, ok := pages[0].Dict["Rotate"]; ok {
		t.Fatalf("unselected page has /Rotate: %+v", pages[0].Dict)
	}
	if _, ok := pages[0].Dict["CropBox"]; ok {
		t.Fatalf("unselected page has /CropBox: %+v", pages[0].Dict)
	}
	if got, ok := dictInt(pages[1].Dict, "Rotate"); !ok || got != 90 {
		t.Fatalf("selected page /Rotate = %v ok=%t, want 90", got, ok)
	}
	if got, ok := dictIntArray(pages[1].Dict, "CropBox"); !ok || !sameInts(got, []int{10, 20, 190, 180}) {
		t.Fatalf("selected page /CropBox = %v ok=%t, want [10 20 190 180]", got, ok)
	}
}

func TestTransformPagesRejectsOutOfRangeZeroBasedPage(t *testing.T) {
	input := pageOpsTestPDF("ONLY")
	rotate := 90

	_, _, _, err := TransformPages(input, PageSelector{Indexes: []int{1}}, PageTransform{Rotate: &rotate}, PageWriteOptions{})
	if err == nil {
		t.Fatal("TransformPages succeeded with an out-of-range page index")
	}
	if !strings.Contains(err.Error(), "zero-based") {
		t.Fatalf("error = %v, want zero-based page index detail", err)
	}
}

func TestTransformPagesScalesSelectedPageBoxesAndContent(t *testing.T) {
	input := pageOpsTestPDF("SCALE")
	scale := PageScale{X: 2, Y: 3}

	output, _, verification, err := TransformPages(input, PageSelector{}, PageTransform{Scale: &scale}, PageWriteOptions{})
	if err != nil {
		t.Fatalf("TransformPages scale: %v", err)
	}
	if !verification.ReparseOK || !verification.PageCountOK || !verification.NoDanglingRefs {
		t.Fatalf("verification = %+v", verification)
	}
	graph, err := parsePDFGraph(output)
	if err != nil {
		t.Fatalf("reparse scaled output: %v", err)
	}
	pages, err := graph.orderedPages()
	if err != nil {
		t.Fatalf("ordered pages: %v", err)
	}
	if got, ok := dictIntArray(pages[0].Dict, "MediaBox"); !ok || !sameInts(got, []int{0, 0, 400, 600}) {
		t.Fatalf("scaled MediaBox = %v ok=%t, want [0 0 400 600]", got, ok)
	}
	contents, ok := pages[0].Dict["Contents"].(pdfRef)
	if !ok {
		t.Fatalf("Contents = %#v, want ref", pages[0].Dict["Contents"])
	}
	stream, ok := graph.Objects[contents.ID].Value.(pdfStreamObject)
	if !ok {
		t.Fatalf("content stream = %#v, want stream", graph.Objects[contents.ID])
	}
	if !strings.Contains(string(stream.Data), "2 0 0 3 0 0 cm") || !strings.Contains(string(stream.Data), "(SCALE) Tj") {
		t.Fatalf("scaled content stream = %q, want scale matrix wrapping original text", stream.Data)
	}
}

func sameInts(got, want []int) bool {
	if len(got) != len(want) {
		return false
	}
	for i := range got {
		if got[i] != want[i] {
			return false
		}
	}
	return true
}
