package pdf

import (
	"bytes"
	"fmt"
	"testing"

	"github.com/oxhq/binas/pkg/core"
)

func TestFormXObjectUsesOwnFontResourcesForCMapTextShow(t *testing.T) {
	pageContent := []byte("q\n/Fm1 Do\nQ\n")
	formContent := []byte("BT\n/F1 12 Tf\n<01> Tj\nET\n")
	input := testPDF(
		"<< /Type /Catalog /Pages 2 0 R >>",
		"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
		"<< /Type /Page /Parent 2 0 R /Contents 6 0 R /Resources << /Font << /F1 4 0 R >> /XObject << /Fm1 7 0 R >> >> >>",
		"<< /Type /Font /Subtype /Type0 /ToUnicode 5 0 R >>",
		testTwoCharToUnicodeCMapStream("0050", "0051"),
		fmt.Sprintf("<< /Length %d >>\nstream\n%sendstream", len(pageContent), pageContent),
		fmt.Sprintf("<< /Type /XObject /Subtype /Form /Resources << /Font << /F1 8 0 R >> >> /Length %d >>\nstream\n%sendstream", len(formContent), formContent),
		"<< /Type /Font /Subtype /Type0 /ToUnicode 9 0 R >>",
		testTwoCharToUnicodeCMapStream("0046", "0047"),
	)

	tree, err := NewAdapter().Parse(input, core.ParseOptions{Strict: true})
	if err != nil {
		t.Fatal(err)
	}
	if matches := tree.Query(core.Match{Kind: KindTextShow, Text: "P"}); len(matches) != 0 {
		t.Fatalf("page font CMap matches = %d, want 0", len(matches))
	}
	matches := tree.Query(core.Match{Kind: KindTextShow, Text: "F"})
	if len(matches) != 1 {
		t.Fatalf("form font CMap matches = %d, want 1", len(matches))
	}
	if matches[0].Meta["encoding"] != "hex-cmap" {
		t.Fatalf("encoding = %v, want hex-cmap", matches[0].Meta["encoding"])
	}
	if matches[0].Meta["font"] != "F1" {
		t.Fatalf("font = %v, want F1", matches[0].Meta["font"])
	}
	if matches[0].Meta["font_context"] != "form_xobject" {
		t.Fatalf("font_context = %v, want form_xobject", matches[0].Meta["font_context"])
	}
	if matches[0].Meta["page_object_number"] != 3 {
		t.Fatalf("page_object_number = %v, want 3", matches[0].Meta["page_object_number"])
	}
	if matches[0].Meta["form_object_number"] != 7 {
		t.Fatalf("form_object_number = %v, want 7", matches[0].Meta["form_object_number"])
	}
}

func TestPageResourceContextKeepsSameFontNamePerPage(t *testing.T) {
	pageOneContent := []byte("BT\n/F1 12 Tf\n<01> Tj\nET\n")
	pageTwoContent := []byte("BT\n/F1 12 Tf\n<01> Tj\nET\n")
	input := testPDF(
		"<< /Type /Catalog /Pages 2 0 R >>",
		"<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>",
		"<< /Type /Page /Parent 2 0 R /Contents 7 0 R /Resources << /Font << /F1 5 0 R >> >> >>",
		"<< /Type /Page /Parent 2 0 R /Contents 8 0 R /Resources << /Font << /F1 6 0 R >> >> >>",
		"<< /Type /Font /Subtype /Type0 /ToUnicode 9 0 R >>",
		"<< /Type /Font /Subtype /Type0 /ToUnicode 10 0 R >>",
		fmt.Sprintf("<< /Length %d >>\nstream\n%sendstream", len(pageOneContent), pageOneContent),
		fmt.Sprintf("<< /Length %d >>\nstream\n%sendstream", len(pageTwoContent), pageTwoContent),
		testTwoCharToUnicodeCMapStream("0041", "0043"),
		testTwoCharToUnicodeCMapStream("0042", "0044"),
	)

	adapter := NewAdapter()
	tree, err := adapter.Parse(input, core.ParseOptions{Strict: true})
	if err != nil {
		t.Fatal(err)
	}
	assertOneTextShow(t, tree, "A", "page", 3, 0, false)
	assertOneTextShow(t, tree, "B", "page", 4, 0, false)

	plan, err := adapter.PlanEdit(tree, core.Match{Kind: KindTextShow, Text: "B"}, core.Mutation{Replace: "D"})
	if err != nil {
		t.Fatal(err)
	}
	output, _, err := adapter.Apply(input, plan)
	if err != nil {
		t.Fatal(err)
	}
	if bytes.Count(output, []byte("<01> Tj")) != 1 {
		t.Fatalf("page-one encoded operand should remain exactly once:\n%s", output)
	}
	if !bytes.Contains(output, []byte("<02> Tj")) {
		t.Fatalf("page-two replacement did not use page-two font CMap:\n%s", output)
	}
}

func TestInheritedPageResourceContextMetadata(t *testing.T) {
	content := []byte("BT\n/F1 12 Tf\n<01> Tj\nET\n")
	input := testPDF(
		"<< /Type /Catalog /Pages 2 0 R >>",
		"<< /Type /Pages /Kids [3 0 R] /Count 1 /Resources << /Font << /F1 4 0 R >> >> >>",
		"<< /Type /Page /Parent 2 0 R /Contents 5 0 R >>",
		"<< /Type /Font /Subtype /Type0 /ToUnicode 6 0 R >>",
		fmt.Sprintf("<< /Length %d >>\nstream\n%sendstream", len(content), content),
		testTwoCharToUnicodeCMapStream("0049", "004A"),
	)

	tree, err := NewAdapter().Parse(input, core.ParseOptions{Strict: true})
	if err != nil {
		t.Fatal(err)
	}
	assertOneTextShow(t, tree, "I", "page", 3, 0, true)
}

func TestNestedFormXObjectUsesNestedOwnFontResources(t *testing.T) {
	pageContent := []byte("q\n/Fm1 Do\nQ\n")
	formOneContent := []byte("q\n/Fm2 Do\nQ\n")
	formTwoContent := []byte("BT\n/F1 12 Tf\n<01> Tj\nET\n")
	input := testPDF(
		"<< /Type /Catalog /Pages 2 0 R >>",
		"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
		"<< /Type /Page /Parent 2 0 R /Contents 8 0 R /Resources << /Font << /F1 4 0 R >> /XObject << /Fm1 9 0 R >> >> >>",
		"<< /Type /Font /Subtype /Type0 /ToUnicode 5 0 R >>",
		testTwoCharToUnicodeCMapStream("0050", "0051"),
		"<< /Type /Font /Subtype /Type0 /ToUnicode 7 0 R >>",
		testTwoCharToUnicodeCMapStream("004E", "004F"),
		fmt.Sprintf("<< /Length %d >>\nstream\n%sendstream", len(pageContent), pageContent),
		fmt.Sprintf("<< /Type /XObject /Subtype /Form /Resources << /XObject << /Fm2 10 0 R >> >> /Length %d >>\nstream\n%sendstream", len(formOneContent), formOneContent),
		fmt.Sprintf("<< /Type /XObject /Subtype /Form /Resources << /Font << /F1 6 0 R >> >> /Length %d >>\nstream\n%sendstream", len(formTwoContent), formTwoContent),
	)

	tree, err := NewAdapter().Parse(input, core.ParseOptions{Strict: true})
	if err != nil {
		t.Fatal(err)
	}
	if matches := tree.Query(core.Match{Kind: KindTextShow, Text: "P"}); len(matches) != 0 {
		t.Fatalf("page font CMap matches = %d, want 0", len(matches))
	}
	assertOneTextShow(t, tree, "N", "form_xobject", 3, 10, false)
}

func TestSharedFormXObjectInheritsInvokingPageResourceContext(t *testing.T) {
	pageContent := []byte("q\n/Fm1 Do\nQ\n")
	formContent := []byte("BT\n/F1 12 Tf\n<01> Tj\nET\n")
	input := testPDF(
		"<< /Type /Catalog /Pages 2 0 R >>",
		"<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>",
		"<< /Type /Page /Parent 2 0 R /Contents 8 0 R /Resources << /Font << /F1 5 0 R >> /XObject << /Fm1 7 0 R >> >> >>",
		"<< /Type /Page /Parent 2 0 R /Contents 9 0 R /Resources << /Font << /F1 6 0 R >> /XObject << /Fm1 7 0 R >> >> >>",
		"<< /Type /Font /Subtype /Type0 /ToUnicode 10 0 R >>",
		"<< /Type /Font /Subtype /Type0 /ToUnicode 11 0 R >>",
		fmt.Sprintf("<< /Type /XObject /Subtype /Form /Length %d >>\nstream\n%sendstream", len(formContent), formContent),
		fmt.Sprintf("<< /Length %d >>\nstream\n%sendstream", len(pageContent), pageContent),
		fmt.Sprintf("<< /Length %d >>\nstream\n%sendstream", len(pageContent), pageContent),
		testTwoCharToUnicodeCMapStream("0041", "0043"),
		testTwoCharToUnicodeCMapStream("0042", "0044"),
	)

	tree, err := NewAdapter().Parse(input, core.ParseOptions{Strict: true})
	if err != nil {
		t.Fatal(err)
	}
	assertOneTextShow(t, tree, "A", "form_xobject", 3, 7, true)
	assertOneTextShow(t, tree, "B", "form_xobject", 4, 7, true)
}

func assertOneTextShow(t *testing.T, tree *core.Tree, text, fontContext string, pageObject, formObject int, inherited bool) {
	t.Helper()
	matches := tree.Query(core.Match{Kind: KindTextShow, Text: text})
	if len(matches) != 1 {
		t.Fatalf("%q matches = %d, want 1", text, len(matches))
	}
	if matches[0].Meta["font_context"] != fontContext {
		t.Fatalf("%q font_context = %v, want %s", text, matches[0].Meta["font_context"], fontContext)
	}
	if matches[0].Meta["page_object_number"] != pageObject {
		t.Fatalf("%q page_object_number = %v, want %d", text, matches[0].Meta["page_object_number"], pageObject)
	}
	if formObject == 0 {
		if _, ok := matches[0].Meta["form_object_number"]; ok {
			t.Fatalf("%q form_object_number = %v, want absent", text, matches[0].Meta["form_object_number"])
		}
	} else if matches[0].Meta["form_object_number"] != formObject {
		t.Fatalf("%q form_object_number = %v, want %d", text, matches[0].Meta["form_object_number"], formObject)
	}
	if got := matches[0].Meta["inherited_resources"] == true; got != inherited {
		t.Fatalf("%q inherited_resources = %v, want %v", text, got, inherited)
	}
}
