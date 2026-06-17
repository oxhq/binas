package pdf

import (
	"bytes"
	"fmt"
	"testing"

	"github.com/oxhq/binas/pkg/core"
)

func TestExtractPagesSelectsZeroBasedPages(t *testing.T) {
	input := pageOpsTestPDF("FIRST", "SECOND")

	output, report, verification, err := ExtractPages(input, []int{1})
	if err != nil {
		t.Fatalf("ExtractPages: %v", err)
	}

	assertPageOpsOutput(t, output, 1, []string{"SECOND"}, []string{"FIRST"})
	if report.Operation != "pdf.extract_pages" || report.InputPages != 2 || report.OutputPages != 1 {
		t.Fatalf("report = %+v", report)
	}
	if !verification.ReparseOK || !verification.PageCountOK || verification.ActualPageCount != 1 {
		t.Fatalf("verification = %+v", verification)
	}
}

func TestCopyPagesCopiesSelectedPages(t *testing.T) {
	input := pageOpsTestPDF("ONE", "TWO", "THREE")

	output, report, verification, err := CopyPages(input, []int{0, 2})
	if err != nil {
		t.Fatalf("CopyPages: %v", err)
	}

	assertPageOpsOutput(t, output, 2, []string{"ONE", "THREE"}, []string{"TWO"})
	if report.Operation != "pdf.copy_pages" || report.InputPages != 3 || report.OutputPages != 2 {
		t.Fatalf("report = %+v", report)
	}
	if !verification.ReparseOK || !verification.PageCountOK || verification.ActualPageCount != 2 {
		t.Fatalf("verification = %+v", verification)
	}
}

func TestMergeRenumbersConflictingSimplePDFObjects(t *testing.T) {
	first := pageOpsTestPDF("ALPHA")
	second := pageOpsTestPDF("BETA")

	output, report, verification, err := Merge([][]byte{first, second})
	if err != nil {
		t.Fatalf("Merge: %v", err)
	}

	assertPageOpsOutput(t, output, 2, []string{"ALPHA", "BETA"}, nil)
	if report.Operation != "pdf.merge" || report.InputDocuments != 2 || report.OutputPages != 2 {
		t.Fatalf("report = %+v", report)
	}
	if !verification.ReparseOK || !verification.PageCountOK || verification.ActualPageCount != 2 {
		t.Fatalf("verification = %+v", verification)
	}
}

func TestInsertPagesSupportsBeginningMiddleAndEnd(t *testing.T) {
	base := pageOpsTestPDF("BASE-1", "BASE-2")
	source := pageOpsTestPDF("SRC-1", "SRC-2")

	for _, tc := range []struct {
		name  string
		index int
		want  []string
	}{
		{name: "beginning", index: 0, want: []string{"SRC-2", "BASE-1", "BASE-2"}},
		{name: "middle", index: 1, want: []string{"BASE-1", "SRC-2", "BASE-2"}},
		{name: "end", index: 2, want: []string{"BASE-1", "BASE-2", "SRC-2"}},
	} {
		t.Run(tc.name, func(t *testing.T) {
			output, report, verification, err := InsertPages(base, tc.index, []PageSource{{Input: source, Pages: []int{1}}})
			if err != nil {
				t.Fatalf("InsertPages: %v", err)
			}
			assertPageOpsOutput(t, output, len(tc.want), tc.want, []string{"SRC-1"})
			assertPageOpsTextOrder(t, output, tc.want)
			if report.Operation != "pdf.insert_pages" || report.InputDocuments != 2 || report.OutputPages != len(tc.want) {
				t.Fatalf("report = %+v", report)
			}
			if !verification.ReparseOK || !verification.PageCountOK || verification.ActualPageCount != len(tc.want) || !verification.NoDanglingRefs {
				t.Fatalf("verification = %+v", verification)
			}
		})
	}
}

func TestInsertPagesDefaultsSourceToAllPages(t *testing.T) {
	base := pageOpsTestPDF("BASE")
	source := pageOpsTestPDF("SRC-1", "SRC-2")

	output, _, verification, err := InsertPages(base, 1, []PageSource{{Input: source}})
	if err != nil {
		t.Fatalf("InsertPages: %v", err)
	}

	assertPageOpsTextOrder(t, output, []string{"BASE", "SRC-1", "SRC-2"})
	if verification.ActualPageCount != 3 {
		t.Fatalf("verification = %+v, want 3 pages", verification)
	}
}

func TestInsertPagesRejectsInvalidInputs(t *testing.T) {
	base := pageOpsTestPDF("BASE")

	if _, _, _, err := InsertPages(base, 2, []PageSource{{Input: base}}); err == nil {
		t.Fatal("InsertPages out-of-range index error = nil")
	}
	if _, _, _, err := InsertPages(base, 0, nil); err == nil {
		t.Fatal("InsertPages without sources error = nil")
	}
}

func pageOpsTestPDF(labels ...string) []byte {
	objects := make([]string, 0, 2+len(labels)*2)
	kids := make([]string, 0, len(labels))
	objects = append(objects, "<< /Type /Catalog /Pages 2 0 R >>")
	for i := range labels {
		pageObject := 3 + i*2
		contentObject := pageObject + 1
		kids = append(kids, fmt.Sprintf("%d 0 R", pageObject))
		objects = append(objects,
			fmt.Sprintf("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Resources <<>> /Contents %d 0 R >>", contentObject),
			pageOpsContentStream(labels[i]),
		)
	}
	pages := fmt.Sprintf("<< /Type /Pages /Kids [%s] /Count %d >>", joinPageOpsTokens(kids), len(labels))
	objects = append(objects[:1], append([]string{pages}, objects[1:]...)...)
	return testPDF(objects...)
}

func pageOpsContentStream(label string) string {
	content := fmt.Sprintf("BT\n(%s) Tj\nET\n", label)
	return fmt.Sprintf("<< /Length %d >>\nstream\n%sendstream", len(content), content)
}

func joinPageOpsTokens(values []string) string {
	var out bytes.Buffer
	for i, value := range values {
		if i > 0 {
			out.WriteByte(' ')
		}
		out.WriteString(value)
	}
	return out.String()
}

func assertPageOpsOutput(t *testing.T, output []byte, pageCount int, wantText, absentText []string) {
	t.Helper()
	graph, err := parsePDFGraph(output)
	if err != nil {
		t.Fatalf("reparse output: %v\n%s", err, output)
	}
	if got := graph.pageCount(); got != pageCount {
		t.Fatalf("page count = %d, want %d\n%s", got, pageCount, output)
	}
	tree, err := NewAdapter().Parse(output, core.ParseOptions{Strict: true})
	if err != nil {
		t.Fatalf("parse output tree: %v\n%s", err, output)
	}
	for _, text := range wantText {
		if matches := tree.Query(core.Match{Kind: KindTextShow, Text: text}); len(matches) != 1 {
			t.Fatalf("matches for %q = %d, want 1\n%s", text, len(matches), output)
		}
	}
	for _, text := range absentText {
		if matches := tree.Query(core.Match{Kind: KindTextShow, Text: text}); len(matches) != 0 {
			t.Fatalf("matches for absent %q = %d, want 0\n%s", text, len(matches), output)
		}
	}
}

func assertPageOpsTextOrder(t *testing.T, output []byte, want []string) {
	t.Helper()
	tree, err := NewAdapter().Parse(output, core.ParseOptions{Strict: true})
	if err != nil {
		t.Fatalf("parse output tree: %v\n%s", err, output)
	}
	nodes := tree.Query(core.Match{Kind: KindTextShow})
	if len(nodes) != len(want) {
		t.Fatalf("text node count = %d, want %d\nnodes=%+v\n%s", len(nodes), len(want), nodes, output)
	}
	for i, text := range want {
		if got := fmt.Sprint(nodes[i].Value); got != text {
			t.Fatalf("text node %d = %q, want %q\nnodes=%+v\n%s", i, got, text, nodes, output)
		}
	}
}
