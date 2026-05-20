package pdf

import (
	"bytes"
	"fmt"
	"strings"
	"testing"

	"github.com/oxhq/binas/pkg/core"
)

func TestCMapParsesBFCharAndBFRangeMappings(t *testing.T) {
	cmap, ok := parseToUnicodeCMap([]byte(`
begincmap
2 beginbfchar
<01> <0048>
<02> <0069>
endbfchar
2 beginbfrange
<03> <05> [<0020> <0057> <0069>]
<10> <12> <0061>
endbfrange
endcmap
`))
	if !ok {
		t.Fatal("expected CMap to parse")
	}
	got, ok := cmap.DecodeHex([]byte("0102030405101112"))
	if !ok {
		t.Fatal("expected mapped text to decode")
	}
	if got != "Hi Wiabc" {
		t.Fatalf("decoded text = %q", got)
	}
}

func TestCMapSingleToUnicodeMapDecodesAndEditsHexTextShow(t *testing.T) {
	content := []byte("BT\n<0102030405> Tj\nET\n")
	input := testPDF(
		"<< /Type /Catalog >>",
		"<< /Type /Page /Resources << /Font << /F1 3 0 R >> >> >>",
		"<< /Type /Font /Subtype /Type0 /ToUnicode 5 0 R >>",
		fmt.Sprintf("<< /Length %d >>\nstream\n%sendstream", len(content), content),
		testToUnicodeCMapStream(),
	)
	adapter := NewAdapter()
	tree, err := adapter.Parse(input, core.ParseOptions{Strict: true})
	if err != nil {
		t.Fatal(err)
	}
	matches := tree.Query(core.Match{Kind: KindTextShow, Text: "HELLO"})
	if len(matches) != 1 {
		t.Fatalf("HELLO matches = %d, want 1", len(matches))
	}
	if matches[0].Meta["encoding"] != "hex-cmap" {
		t.Fatalf("encoding meta = %v, want hex-cmap", matches[0].Meta["encoding"])
	}

	plan, err := adapter.PlanEdit(tree, core.Match{Kind: KindTextShow, Text: "HELLO"}, core.Mutation{Replace: "OLE"})
	if err != nil {
		t.Fatal(err)
	}
	output, _, err := adapter.Apply(input, plan)
	if err != nil {
		t.Fatal(err)
	}
	if bytes.Contains(output, []byte("<0102030405> Tj")) {
		t.Fatal("old mapped text operand remains")
	}
	if !bytes.Contains(output, []byte("<050302> Tj")) {
		t.Fatalf("new mapped operand missing:\n%s", output)
	}
	verification, err := adapter.Verify(output, plan)
	if err != nil {
		t.Fatal(err)
	}
	if !verification.ReparseOK || !verification.OldTextRemoved || !verification.NewSelectable || !verification.PageUnchanged {
		t.Fatalf("verification failed: %+v", verification)
	}

	_, err = adapter.PlanEdit(tree, core.Match{Kind: KindTextShow, Text: "HELLO"}, core.Mutation{Replace: "HELLO!"})
	if err == nil {
		t.Fatal("expected unrepresentable CMap replacement to fail closed")
	}
	if !strings.Contains(err.Error(), "not representable by the CMap") {
		t.Fatalf("error = %q, want CMap representability refusal", err)
	}
}

func TestCMapAmbiguousToUnicodeMapsFallBackToSimpleHexDecoding(t *testing.T) {
	content := []byte("BT\n<3031> Tj\nET\n")
	input := testPDF(
		"<< /Type /Catalog >>",
		"<< /Type /Page /Contents 5 0 R /Resources << /Font << /F1 3 0 R /F2 4 0 R >> >> >>",
		"<< /Type /Font /Subtype /Type0 /ToUnicode 6 0 R >>",
		"<< /Type /Font /Subtype /Type0 /ToUnicode 7 0 R >>",
		fmt.Sprintf("<< /Length %d >>\nstream\n%sendstream", len(content), content),
		testToUnicodeCMapStream(),
		testToUnicodeCMapStream(),
	)
	tree, err := NewAdapter().Parse(input, core.ParseOptions{Strict: true})
	if err != nil {
		t.Fatal(err)
	}
	if matches := tree.Query(core.Match{Kind: KindTextShow, Text: "AB"}); len(matches) != 0 {
		t.Fatalf("mapped matches = %d, want fallback with no mapped text", len(matches))
	}
	matches := tree.Query(core.Match{Kind: KindTextShow, Text: "01"})
	if len(matches) != 1 {
		t.Fatalf("simple hex matches = %d, want 1", len(matches))
	}
	if matches[0].Meta["encoding"] != "hex" {
		t.Fatalf("encoding meta = %v, want hex", matches[0].Meta["encoding"])
	}
}

func TestCMapUsesActiveTfFontResourceForHexTextShow(t *testing.T) {
	content := []byte("BT\n/F1 12 Tf\n<01> Tj\n/F2 12 Tf\n<01> Tj\nET\n")
	input := testPDF(
		"<< /Type /Catalog >>",
		"<< /Type /Page /Contents 5 0 R /Resources << /Font << /F1 3 0 R /F2 4 0 R >> >> >>",
		"<< /Type /Font /Subtype /Type0 /ToUnicode 6 0 R >>",
		"<< /Type /Font /Subtype /Type0 /ToUnicode 7 0 R >>",
		fmt.Sprintf("<< /Length %d >>\nstream\n%sendstream", len(content), content),
		testTwoCharToUnicodeCMapStream("0041", "0043"),
		testTwoCharToUnicodeCMapStream("0042", "0044"),
	)
	adapter := NewAdapter()
	tree, err := adapter.Parse(input, core.ParseOptions{Strict: true})
	if err != nil {
		t.Fatal(err)
	}
	if matches := tree.Query(core.Match{Kind: KindTextShow, Text: "A"}); len(matches) != 1 {
		t.Fatalf("A matches = %d, want 1", len(matches))
	}
	if matches := tree.Query(core.Match{Kind: KindTextShow, Text: "B"}); len(matches) != 1 {
		t.Fatalf("B matches = %d, want 1", len(matches))
	}

	plan, err := adapter.PlanEdit(tree, core.Match{Kind: KindTextShow, Text: "B"}, core.Mutation{Replace: "D"})
	if err != nil {
		t.Fatal(err)
	}
	output, _, err := adapter.Apply(input, plan)
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Contains(output, []byte("/F1 12 Tf\n<01> Tj\n/F2 12 Tf\n<02> Tj")) {
		t.Fatalf("expected F2 replacement to use its font-specific encoded byte:\n%s", output)
	}
	verification, err := adapter.Verify(output, plan)
	if err != nil {
		t.Fatal(err)
	}
	if !verification.ReparseOK || !verification.OldTextRemoved || !verification.NewSelectable || !verification.PageUnchanged {
		t.Fatalf("verification failed: %+v", verification)
	}
}

func TestCMapUsesActiveTfFontResourceForTJArrayHexTextShow(t *testing.T) {
	content := []byte("BT\n/F1 12 Tf\n[<01> 20 <02>] TJ\nET\n")
	input := testPDF(
		"<< /Type /Catalog >>",
		"<< /Type /Page /Contents 4 0 R /Resources << /Font << /F1 3 0 R >> >> >>",
		"<< /Type /Font /Subtype /Type0 /ToUnicode 5 0 R >>",
		fmt.Sprintf("<< /Length %d >>\nstream\n%sendstream", len(content), content),
		testTwoCharToUnicodeCMapStream("0041", "0042"),
	)
	adapter := NewAdapter()
	tree, err := adapter.Parse(input, core.ParseOptions{Strict: true})
	if err != nil {
		t.Fatal(err)
	}
	matches := tree.Query(core.Match{Kind: KindTextShow, Text: "AB"})
	if len(matches) != 1 {
		t.Fatalf("AB matches = %d, want 1", len(matches))
	}
	if matches[0].Meta["encoding"] != "tj-array-cmap" {
		t.Fatalf("encoding meta = %v, want tj-array-cmap", matches[0].Meta["encoding"])
	}

	plan, err := adapter.PlanEdit(tree, core.Match{Kind: KindTextShow, Text: "AB"}, core.Mutation{Replace: "BA"})
	if err != nil {
		t.Fatal(err)
	}
	output, _, err := adapter.Apply(input, plan)
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Contains(output, []byte("[<0201>] TJ")) {
		t.Fatalf("expected CMap TJ array replacement to preserve hex/CMap encoding:\n%s", output)
	}
	verification, err := adapter.Verify(output, plan)
	if err != nil {
		t.Fatal(err)
	}
	if !verification.ReparseOK || !verification.OldTextRemoved || !verification.NewSelectable || !verification.PageUnchanged {
		t.Fatalf("verification failed: %+v", verification)
	}
}

func TestCMapEncodeHexUsesLongestDecodedTextMapping(t *testing.T) {
	cmap := pdfToUnicodeMap{
		"01": "fi",
		"02": "f",
		"03": "i",
		"04": "x",
	}
	encoded, ok := cmap.EncodeHex("fix")
	if !ok {
		t.Fatal("expected ligature text to encode")
	}
	if encoded != "0104" {
		t.Fatalf("encoded = %q, want longest mapping 0104", encoded)
	}
}

func TestCMapEncodeHexReportsMaxCodeBytes(t *testing.T) {
	cmap := pdfToUnicodeMap{
		"0001": "A",
		"02":   "B",
	}
	encoded, maxCodeBytes, ok := cmap.EncodeHexWithMaxCodeBytes("AB")
	if !ok {
		t.Fatal("expected mapped text to encode")
	}
	if encoded != "000102" {
		t.Fatalf("encoded = %q, want 000102", encoded)
	}
	if maxCodeBytes != 2 {
		t.Fatalf("maxCodeBytes = %d, want 2", maxCodeBytes)
	}
}

func TestCMapMultiByteReverseEncodingWithoutLayoutProofFailsClosed(t *testing.T) {
	content := []byte("BT\n/F1 12 Tf\n<0001> Tj\nET\n")
	input := testPDF(
		"<< /Type /Catalog >>",
		"<< /Type /Page /Contents 4 0 R /Resources << /Font << /F1 3 0 R >> >> >>",
		"<< /Type /Font /Subtype /Type0 /Encoding /Identity-H /ToUnicode 5 0 R >>",
		fmt.Sprintf("<< /Length %d >>\nstream\n%sendstream", len(content), content),
		testTwoCIDToUnicodeCMapStream(),
	)

	adapter := NewAdapter()
	tree, err := adapter.Parse(input, core.ParseOptions{Strict: true})
	if err != nil {
		t.Fatal(err)
	}
	if matches := tree.Query(core.Match{Kind: KindTextShow, Text: "A"}); len(matches) != 1 {
		t.Fatalf("A matches = %d, want 1", len(matches))
	}
	_, err = adapter.PlanEdit(tree, core.Match{Kind: KindTextShow, Text: "A"}, core.Mutation{Replace: "B"})
	if err == nil {
		t.Fatal("expected unsupported multi-byte CMap reverse encoding to fail closed")
	}
	for _, want := range []string{"multi-byte CMap reverse encoding", "layout_proof=unknown", "max_cmap_code_bytes=2"} {
		if !strings.Contains(err.Error(), want) {
			t.Fatalf("error = %q, want %q", err, want)
		}
	}
}

func TestCMapCanonicalMultiByteReverseEncodingWithoutLayoutProofFailsClosed(t *testing.T) {
	content := []byte("BT\n/F1 12 Tf\n<0001> Tj\nET\n")
	input := testPDF(
		"<< /Type /Catalog >>",
		"<< /Type /Page /Contents 4 0 R /Resources << /Font << /F1 3 0 R >> >> >>",
		"<< /Type /Font /Subtype /Type0 /Encoding /Identity-H /ToUnicode 5 0 R >>",
		fmt.Sprintf("<< /Length %d >>\nstream\n%sendstream", len(content), content),
		testTwoCIDToUnicodeCMapStream(),
	)

	_, _, _, err := ApplyCanonicalEdit(input, core.Match{Kind: KindTextShow, Text: "A"}, core.Mutation{Replace: "B"}, nil)
	if err == nil {
		t.Fatal("expected canonical multi-byte CMap reverse encoding to fail closed")
	}
	for _, want := range []string{"multi-byte CMap reverse encoding", "layout_proof=unknown", "max_cmap_code_bytes=2"} {
		if !strings.Contains(err.Error(), want) {
			t.Fatalf("error = %q, want %q", err, want)
		}
	}
}

func TestCMapInheritsPageResourcesFromParentPages(t *testing.T) {
	content := []byte("BT\n/F1 12 Tf\n<0102030405> Tj\nET\n")
	input := testPDF(
		"<< /Type /Catalog /Pages 2 0 R >>",
		"<< /Type /Pages /Kids [3 0 R] /Count 1 /Resources << /Font << /F1 4 0 R >> >> >>",
		"<< /Type /Page /Parent 2 0 R /Contents 5 0 R >>",
		"<< /Type /Font /Subtype /Type0 /ToUnicode 6 0 R >>",
		fmt.Sprintf("<< /Length %d >>\nstream\n%sendstream", len(content), content),
		testToUnicodeCMapStream(),
	)
	adapter := NewAdapter()
	tree, err := adapter.Parse(input, core.ParseOptions{Strict: true})
	if err != nil {
		t.Fatal(err)
	}
	matches := tree.Query(core.Match{Kind: KindTextShow, Text: "HELLO"})
	if len(matches) != 1 {
		t.Fatalf("HELLO matches = %d, want 1", len(matches))
	}
	if matches[0].Meta["encoding"] != "hex-cmap" {
		t.Fatalf("encoding meta = %v, want hex-cmap", matches[0].Meta["encoding"])
	}

	plan, err := adapter.PlanEdit(tree, core.Match{Kind: KindTextShow, Text: "HELLO"}, core.Mutation{Replace: "OLE"})
	if err != nil {
		t.Fatal(err)
	}
	output, _, err := adapter.Apply(input, plan)
	if err != nil {
		t.Fatal(err)
	}
	if bytes.Contains(output, []byte("<0102030405> Tj")) {
		t.Fatal("old inherited mapped text operand remains")
	}
	if !bytes.Contains(output, []byte("<050302> Tj")) {
		t.Fatalf("new inherited mapped operand missing:\n%s", output)
	}
	verification, err := adapter.Verify(output, plan)
	if err != nil {
		t.Fatal(err)
	}
	if !verification.ReparseOK || !verification.OldTextRemoved || !verification.NewSelectable || !verification.PageUnchanged {
		t.Fatalf("verification failed: %+v", verification)
	}
}

func testToUnicodeCMapStream() string {
	cmap := []byte(`/CIDInit /ProcSet findresource begin
12 dict begin
begincmap
2 beginbfchar
<01> <0048>
<02> <0045>
endbfchar
1 beginbfrange
<03> <05> [<004C> <004C> <004F>]
endbfrange
endcmap
CMapName currentdict /CMap defineresource pop
end
end`)
	return fmt.Sprintf("<< /Length %d >>\nstream\n%s\nendstream", len(cmap)+1, cmap)
}

func testTwoCharToUnicodeCMapStream(firstDstHex, secondDstHex string) string {
	cmap := []byte(fmt.Sprintf(`/CIDInit /ProcSet findresource begin
12 dict begin
begincmap
2 beginbfchar
<01> <%s>
<02> <%s>
endbfchar
endcmap
CMapName currentdict /CMap defineresource pop
end
end`, firstDstHex, secondDstHex))
	return fmt.Sprintf("<< /Length %d >>\nstream\n%s\nendstream", len(cmap)+1, cmap)
}
