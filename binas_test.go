package binas

import (
	"bytes"
	"fmt"
	"testing"

	"github.com/oxhq/binas/pkg/core"
)

func TestOpenDetectsPDFAndEditsThroughGenericAPI(t *testing.T) {
	input := testPDFFile("<< /Type /Page >>", testStreamObject("old"))

	matches, err := Open(input).Query(core.Selector{Text: "old"})
	if err != nil {
		t.Fatal(err)
	}
	if len(matches) != 1 || matches[0].Kind != "pdf.content.text_show" {
		t.Fatalf("matches = %+v, want one PDF text node", matches)
	}

	output, report, verification, err := Open(input).
		Text("old").
		Replace("new").
		Verify(core.InvariantReparse, core.InvariantOldGone, core.InvariantNewSelectable, core.InvariantPageUnchanged, core.InvariantNoFallbackUsed).
		Bytes()
	if err != nil {
		t.Fatal(err)
	}
	if bytes.Equal(output, input) || report.Format != "pdf" || !verification.ReparseOK || !verification.NewSelectable {
		t.Fatalf("report=%+v verification=%+v output_changed=%v", report, verification, !bytes.Equal(output, input))
	}

	rewritten, err := Open(output, WithFormat(PDF)).Text("new").FindOne()
	if err != nil {
		t.Fatal(err)
	}
	if rewritten.Kind != "pdf.content.text_show" {
		t.Fatalf("rewritten kind = %q, want PDF text node", rewritten.Kind)
	}
}

func TestOpenWorksWithCustomAdapterForFutureFormats(t *testing.T) {
	doc := Open([]byte("ELF"), WithAdapter(fakeAdapter{}))

	node, err := doc.Kind("elf.symbol").Text("main").FindOne()
	if err != nil {
		t.Fatal(err)
	}
	if node.Name != "main" {
		t.Fatalf("node = %+v, want main symbol", node)
	}

	output, report, verification, err := doc.Replace("start").Bytes()
	if err != nil {
		t.Fatal(err)
	}
	if string(output) != "patched:start" || report.Format != "elf" || !verification.ReparseOK {
		t.Fatalf("output=%q report=%+v verification=%+v", output, report, verification)
	}
}

type fakeAdapter struct{}

func (fakeAdapter) Detect([]byte) (core.Confidence, error) {
	return 1, nil
}

func (fakeAdapter) Parse(input []byte, opts core.ParseOptions) (*core.Tree, error) {
	tree := &core.Tree{Format: "elf"}
	tree.Root = tree.AddNode(core.Node{Kind: "elf.file", Span: core.Span{Start: 0, End: int64(len(input))}})
	symbol := tree.AddNode(core.Node{
		Kind:  "elf.symbol",
		Name:  "main",
		Span:  core.Span{Start: 0, End: int64(len(input))},
		Value: "main",
	})
	tree.Nodes[tree.Root].Children = append(tree.Nodes[tree.Root].Children, symbol)
	return tree, nil
}

func (fakeAdapter) PlanEdit(tree *core.Tree, selector core.Match, mutation core.Mutation) (*core.EditPlan, error) {
	node, err := tree.FindOne(selector)
	if err != nil {
		return nil, err
	}
	return &core.EditPlan{
		Target:    node.ID,
		Operation: "elf.symbol_replace",
		OldText:   fmt.Sprint(node.Value),
		NewText:   mutation.Replace,
		Span:      node.Span,
	}, nil
}

func (fakeAdapter) Apply(input []byte, plan *core.EditPlan) ([]byte, core.Report, error) {
	return []byte("patched:" + plan.NewText), core.Report{
		Format:        "elf",
		Edit:          plan.Operation,
		NodesModified: 1,
	}, nil
}

func (fakeAdapter) Verify(output []byte, plan *core.EditPlan) (core.Verification, error) {
	return core.Verification{ReparseOK: true}, nil
}

func testStreamObject(text string) string {
	content := fmt.Sprintf("BT\n(%s) Tj\nET\n", text)
	return fmt.Sprintf("<< /Length %d >>\nstream\n%sendstream", len(content), content)
}

func testPDFFile(objects ...string) []byte {
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
