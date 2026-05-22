package pdf

import (
	"bytes"
	"fmt"
	"strings"
	"testing"

	"github.com/oxhq/binas/pkg/core"
)

func TestFontEncodingWinAnsiDecodesAndEditsHighByteHexText(t *testing.T) {
	content := []byte("BT\n/F1 12 Tf\n<8041> Tj\nET\n")
	input := testPDF(
		"<< /Type /Catalog >>",
		"<< /Type /Page /Contents 4 0 R /Resources << /Font << /F1 3 0 R >> >> >>",
		"<< /Type /Font /Subtype /Type1 /Encoding /WinAnsiEncoding >>",
		fmt.Sprintf("<< /Length %d >>\nstream\n%sendstream", len(content), content),
	)
	adapter := NewAdapter()
	tree, err := adapter.Parse(input, core.ParseOptions{Strict: true})
	if err != nil {
		t.Fatal(err)
	}
	matches := tree.Query(core.Match{Kind: KindTextShow, Text: "€A"})
	if len(matches) != 1 {
		t.Fatalf("matches = %d, want 1", len(matches))
	}
	if matches[0].Meta["encoding"] != "hex-font-encoding" {
		t.Fatalf("encoding meta = %v, want hex-font-encoding", matches[0].Meta["encoding"])
	}
	if matches[0].Meta["font_encoding_name"] != "WinAnsiEncoding" {
		t.Fatalf("font_encoding_name meta = %v, want WinAnsiEncoding", matches[0].Meta["font_encoding_name"])
	}

	plan, err := adapter.PlanEdit(tree, core.Match{Kind: KindTextShow, Text: "€A"}, core.Mutation{Replace: "A€"})
	if err != nil {
		t.Fatal(err)
	}
	output, _, err := adapter.Apply(input, plan)
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Contains(output, []byte("<4180> Tj")) {
		t.Fatalf("expected replacement to preserve WinAnsi bytes:\n%s", output)
	}
	verification, err := adapter.Verify(output, plan)
	if err != nil {
		t.Fatal(err)
	}
	if !verification.ReparseOK || !verification.OldTextRemoved || !verification.NewSelectable || !verification.PageUnchanged {
		t.Fatalf("verification failed: %+v", verification)
	}

	_, err = adapter.PlanEdit(tree, core.Match{Kind: KindTextShow, Text: "€A"}, core.Mutation{Replace: "A☃"})
	if err == nil {
		t.Fatal("expected unrepresentable WinAnsi replacement to fail closed")
	}
	if !strings.Contains(err.Error(), "not representable by the font encoding") {
		t.Fatalf("error = %q, want font encoding representability refusal", err)
	}
}

func TestFontEncodingWinAnsiDecodesLiteralText(t *testing.T) {
	content := []byte("BT\n/F1 12 Tf\n(\\200A) Tj\nET\n")
	input := testPDF(
		"<< /Type /Catalog >>",
		"<< /Type /Page /Contents 4 0 R /Resources << /Font << /F1 3 0 R >> >> >>",
		"<< /Type /Font /Subtype /Type1 /Encoding /WinAnsiEncoding >>",
		fmt.Sprintf("<< /Length %d >>\nstream\n%sendstream", len(content), content),
	)
	adapter := NewAdapter()
	tree, err := adapter.Parse(input, core.ParseOptions{Strict: true})
	if err != nil {
		t.Fatal(err)
	}
	matches := tree.Query(core.Match{Kind: KindTextShow, Text: "€A"})
	if len(matches) != 1 {
		t.Fatalf("matches = %d, want 1", len(matches))
	}
	if matches[0].Meta["encoding"] != "literal-font-encoding" {
		t.Fatalf("encoding meta = %v, want literal-font-encoding", matches[0].Meta["encoding"])
	}

	_, err = adapter.PlanEdit(tree, core.Match{Kind: KindTextShow, Text: fmt.Sprint(matches[0].Value)}, core.Mutation{Replace: "A☃"})
	if err == nil {
		t.Fatal("expected unrepresentable WinAnsi literal replacement to fail closed")
	}
	if !strings.Contains(err.Error(), "not representable by the font encoding") {
		t.Fatalf("error = %q, want font encoding representability refusal", err)
	}
}

func TestFontEncodingDifferencesOverrideDecodesAndEditsHexText(t *testing.T) {
	content := []byte("BT\n/F1 12 Tf\n<4142> Tj\nET\n")
	input := testPDF(
		"<< /Type /Catalog >>",
		"<< /Type /Page /Contents 4 0 R /Resources << /Font << /F1 3 0 R >> >> >>",
		"<< /Type /Font /Subtype /Type1 /Encoding << /BaseEncoding /WinAnsiEncoding /Differences [65 /Euro /Aacute] >> >>",
		fmt.Sprintf("<< /Length %d >>\nstream\n%sendstream", len(content), content),
	)
	adapter := NewAdapter()
	tree, err := adapter.Parse(input, core.ParseOptions{Strict: true})
	if err != nil {
		t.Fatal(err)
	}
	matches := tree.Query(core.Match{Kind: KindTextShow, Text: "€Á"})
	if len(matches) != 1 {
		t.Fatalf("matches = %d, want 1", len(matches))
	}
	if matches[0].Meta["encoding"] != "hex-font-encoding" {
		t.Fatalf("encoding meta = %v, want hex-font-encoding", matches[0].Meta["encoding"])
	}
	if matches[0].Meta["font_encoding_name"] != "WinAnsiEncoding+Differences" {
		t.Fatalf("font_encoding_name meta = %v, want WinAnsiEncoding+Differences", matches[0].Meta["font_encoding_name"])
	}

	plan, err := adapter.PlanEdit(tree, core.Match{Kind: KindTextShow, Text: "€Á"}, core.Mutation{Replace: "Á€"})
	if err != nil {
		t.Fatal(err)
	}
	output, _, err := adapter.Apply(input, plan)
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Contains(output, []byte("<4241> Tj")) {
		t.Fatalf("expected replacement to preserve Differences bytes:\n%s", output)
	}
	verification, err := adapter.Verify(output, plan)
	if err != nil {
		t.Fatal(err)
	}
	if !verification.ReparseOK || !verification.OldTextRemoved || !verification.NewSelectable || !verification.PageUnchanged {
		t.Fatalf("verification failed: %+v", verification)
	}
}

func TestFontEncodingIndirectDifferencesOverrideDecodesAndEditsHexText(t *testing.T) {
	content := []byte("BT\n/F1 12 Tf\n<4142> Tj\nET\n")
	input := testPDF(
		"<< /Type /Catalog >>",
		"<< /Type /Page /Contents 5 0 R /Resources << /Font << /F1 3 0 R >> >> >>",
		"<< /Type /Font /Subtype /Type1 /Encoding 4 0 R >>",
		"<< /BaseEncoding /WinAnsiEncoding /Differences [65 /Euro /Aacute] >>",
		fmt.Sprintf("<< /Length %d >>\nstream\n%sendstream", len(content), content),
	)
	adapter := NewAdapter()
	tree, err := adapter.Parse(input, core.ParseOptions{Strict: true})
	if err != nil {
		t.Fatal(err)
	}
	matches := tree.Query(core.Match{Kind: KindTextShow, Text: "€Á"})
	if len(matches) != 1 {
		t.Fatalf("matches = %d, want 1", len(matches))
	}
	if matches[0].Meta["font_encoding_name"] != "WinAnsiEncoding+Differences" {
		t.Fatalf("font_encoding_name meta = %v, want WinAnsiEncoding+Differences", matches[0].Meta["font_encoding_name"])
	}

	plan, err := adapter.PlanEdit(tree, core.Match{Kind: KindTextShow, Text: "€Á"}, core.Mutation{Replace: "Á€"})
	if err != nil {
		t.Fatal(err)
	}
	output, _, err := adapter.Apply(input, plan)
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Contains(output, []byte("<4241> Tj")) {
		t.Fatalf("expected replacement to preserve indirect Differences bytes:\n%s", output)
	}
}

func TestFontEncodingStandardDecodesAndEditsHighByteHexText(t *testing.T) {
	content := []byte("BT\n/F1 12 Tf\n<AEBB> Tj\nET\n")
	input := testPDF(
		"<< /Type /Catalog >>",
		"<< /Type /Page /Contents 4 0 R /Resources << /Font << /F1 3 0 R >> >> >>",
		"<< /Type /Font /Subtype /Type1 /Encoding /StandardEncoding >>",
		fmt.Sprintf("<< /Length %d >>\nstream\n%sendstream", len(content), content),
	)
	adapter := NewAdapter()
	tree, err := adapter.Parse(input, core.ParseOptions{Strict: true})
	if err != nil {
		t.Fatal(err)
	}
	matches := tree.Query(core.Match{Kind: KindTextShow, Text: "\ufb01\u00bb"})
	if len(matches) != 1 {
		t.Fatalf("matches = %d, want 1", len(matches))
	}
	if matches[0].Meta["font_encoding_name"] != "StandardEncoding" {
		t.Fatalf("font_encoding_name meta = %v, want StandardEncoding", matches[0].Meta["font_encoding_name"])
	}

	plan, err := adapter.PlanEdit(tree, core.Match{Kind: KindTextShow, Text: "\ufb01\u00bb"}, core.Mutation{Replace: "\u00bb\ufb01"})
	if err != nil {
		t.Fatal(err)
	}
	output, _, err := adapter.Apply(input, plan)
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Contains(output, []byte("<BBAE> Tj")) {
		t.Fatalf("expected replacement to preserve StandardEncoding bytes:\n%s", output)
	}

	_, err = adapter.PlanEdit(tree, core.Match{Kind: KindTextShow, Text: "\ufb01\u00bb"}, core.Mutation{Replace: "\u20ac"})
	if err == nil {
		t.Fatal("expected unrepresentable StandardEncoding replacement to fail closed")
	}
	if !strings.Contains(err.Error(), "not representable by the font encoding") {
		t.Fatalf("error = %q, want font encoding representability refusal", err)
	}
}

func TestFontEncodingDefaultStandardEncodingForType1BaseFontDecodesAndEditsHighByteHexText(t *testing.T) {
	content := []byte("BT\n/F1 12 Tf\n<AEBB> Tj\nET\n")
	input := testPDF(
		"<< /Type /Catalog >>",
		"<< /Type /Page /Contents 4 0 R /Resources << /Font << /F1 3 0 R >> >> >>",
		"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
		fmt.Sprintf("<< /Length %d >>\nstream\n%sendstream", len(content), content),
	)
	adapter := NewAdapter()
	tree, err := adapter.Parse(input, core.ParseOptions{Strict: true})
	if err != nil {
		t.Fatal(err)
	}
	matches := tree.Query(core.Match{Kind: KindTextShow, Text: "\ufb01\u00bb"})
	if len(matches) != 1 {
		t.Fatalf("matches = %d, want 1", len(matches))
	}
	if matches[0].Meta["encoding"] != "hex-font-encoding" {
		t.Fatalf("encoding meta = %v, want hex-font-encoding", matches[0].Meta["encoding"])
	}
	if matches[0].Meta["font_encoding_name"] != "StandardEncoding" {
		t.Fatalf("font_encoding_name meta = %v, want StandardEncoding", matches[0].Meta["font_encoding_name"])
	}

	plan, err := adapter.PlanEdit(tree, core.Match{Kind: KindTextShow, Text: "\ufb01\u00bb"}, core.Mutation{Replace: "\u00bb\ufb01"})
	if err != nil {
		t.Fatal(err)
	}
	output, _, err := adapter.Apply(input, plan)
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Contains(output, []byte("<BBAE> Tj")) {
		t.Fatalf("expected replacement to preserve inferred StandardEncoding bytes:\n%s", output)
	}
}

func TestFontEncodingDefaultAbsentEncodingFailsClosedForUnsupportedBaseFonts(t *testing.T) {
	cases := []pdfDict{
		{"Subtype": pdfName("Type1"), "BaseFont": pdfName("Symbol")},
		{"Subtype": pdfName("Type1"), "BaseFont": pdfName("ZapfDingbats")},
		{"Subtype": pdfName("Type1"), "BaseFont": pdfName("CustomFont")},
		{"Subtype": pdfName("TrueType"), "BaseFont": pdfName("Helvetica")},
		{"Subtype": pdfName("Type1")},
	}
	for _, tc := range cases {
		if encoding, ok := defaultSimpleFontEncodingForFont(tc); ok {
			t.Fatalf("defaultSimpleFontEncodingForFont(%v) = %v, true; want fail-closed", tc, encoding.name)
		}
	}
}

func TestFontEncodingMacRomanDecodesAndEditsHighByteHexText(t *testing.T) {
	content := []byte("BT\n/F1 12 Tf\n<80DB> Tj\nET\n")
	input := testPDF(
		"<< /Type /Catalog >>",
		"<< /Type /Page /Contents 4 0 R /Resources << /Font << /F1 3 0 R >> >> >>",
		"<< /Type /Font /Subtype /Type1 /Encoding /MacRomanEncoding >>",
		fmt.Sprintf("<< /Length %d >>\nstream\n%sendstream", len(content), content),
	)
	adapter := NewAdapter()
	tree, err := adapter.Parse(input, core.ParseOptions{Strict: true})
	if err != nil {
		t.Fatal(err)
	}
	matches := tree.Query(core.Match{Kind: KindTextShow, Text: "\u00c4\u20ac"})
	if len(matches) != 1 {
		t.Fatalf("matches = %d, want 1", len(matches))
	}
	if matches[0].Meta["font_encoding_name"] != "MacRomanEncoding" {
		t.Fatalf("font_encoding_name meta = %v, want MacRomanEncoding", matches[0].Meta["font_encoding_name"])
	}

	plan, err := adapter.PlanEdit(tree, core.Match{Kind: KindTextShow, Text: "\u00c4\u20ac"}, core.Mutation{Replace: "\u20ac\u00c4"})
	if err != nil {
		t.Fatal(err)
	}
	output, _, err := adapter.Apply(input, plan)
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Contains(output, []byte("<DB80> Tj")) {
		t.Fatalf("expected replacement to preserve MacRomanEncoding bytes:\n%s", output)
	}

	_, err = adapter.PlanEdit(tree, core.Match{Kind: KindTextShow, Text: "\u00c4\u20ac"}, core.Mutation{Replace: "\u2603"})
	if err == nil {
		t.Fatal("expected unrepresentable MacRomanEncoding replacement to fail closed")
	}
	if !strings.Contains(err.Error(), "not representable by the font encoding") {
		t.Fatalf("error = %q, want font encoding representability refusal", err)
	}
}

func TestFontEncodingUnknownDifferenceGlyphFailsClosed(t *testing.T) {
	encoding, ok := parseSimpleFontEncoding(pdfDict{
		"BaseEncoding": pdfName("WinAnsiEncoding"),
		"Differences":  pdfArray{65, pdfName("DefinitelyUnknownGlyph")},
	})
	if !ok {
		t.Fatal("parseSimpleFontEncoding returned !ok")
	}
	if got := encoding.DecodeBytes([]byte{65}); got != "\ufffd" {
		t.Fatalf("DecodeBytes = %q, want replacement rune", got)
	}
	if _, ok := encoding.EncodeBytes("A"); ok {
		t.Fatal("expected overridden unknown Difference glyph to be unrepresentable")
	}
}

func TestFontEncodingDifferencesUnicodeGlyphNamesDecodeAndEdit(t *testing.T) {
	content := []byte("BT\n/F1 12 Tf\n<4142> Tj\nET\n")
	input := testPDF(
		"<< /Type /Catalog >>",
		"<< /Type /Page /Contents 4 0 R /Resources << /Font << /F1 3 0 R >> >> >>",
		"<< /Type /Font /Subtype /Type1 /Encoding << /BaseEncoding /StandardEncoding /Differences [65 /uni20AC /u2603] >> >>",
		fmt.Sprintf("<< /Length %d >>\nstream\n%sendstream", len(content), content),
	)
	adapter := NewAdapter()
	tree, err := adapter.Parse(input, core.ParseOptions{Strict: true})
	if err != nil {
		t.Fatal(err)
	}
	matches := tree.Query(core.Match{Kind: KindTextShow, Text: "\u20ac\u2603"})
	if len(matches) != 1 {
		t.Fatalf("matches = %d, want 1", len(matches))
	}

	plan, err := adapter.PlanEdit(tree, core.Match{Kind: KindTextShow, Text: "\u20ac\u2603"}, core.Mutation{Replace: "\u2603\u20ac"})
	if err != nil {
		t.Fatal(err)
	}
	output, _, err := adapter.Apply(input, plan)
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Contains(output, []byte("<4241> Tj")) {
		t.Fatalf("expected replacement to preserve Unicode Differences bytes:\n%s", output)
	}
}
