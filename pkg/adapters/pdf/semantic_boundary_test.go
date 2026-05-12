package pdf

import (
	"fmt"
	"testing"

	"github.com/oxhq/binas/pkg/core"
)

func TestSemanticBoundariesFailClosedBeforeEditPlanning(t *testing.T) {
	tests := []struct {
		name  string
		input []byte
		want  string
	}{
		{
			name:  "encrypt",
			input: testPDF("<< /Type /Catalog /Encrypt 2 0 R >>", "<<>>"),
			want:  "unsupported PDF: encrypted PDFs require an explicit password-capable path; Adapter.Parse does not decrypt",
		},
		{
			name:  "signature",
			input: testPDF("<< /Type /Catalog >>", "<< /FT /Sig >>"),
			want:  "unsupported PDF: digital signatures require explicit invalidation mode; Adapter.Parse refuses silent signature invalidation",
		},
		{
			name:  "xfa",
			input: testPDF("<< /Type /Catalog /AcroForm << /XFA 2 0 R >> >>", "<<>>"),
			want:  "unsupported PDF: XFA forms are not implemented",
		},
	}

	for _, tc := range tests {
		t.Run(tc.name, func(t *testing.T) {
			tree, err := NewAdapter().Parse(tc.input, core.ParseOptions{Strict: true})
			if err == nil {
				t.Fatal("expected semantic boundary parse error")
			}
			if err.Error() != tc.want {
				t.Fatalf("error = %q, want %q", err, tc.want)
			}
			if tree != nil {
				t.Fatal("fail-closed semantic boundary returned a tree")
			}
		})
	}
}

func TestSemanticDetectionOnlyBoundariesKeepSimpleContentStreamEditable(t *testing.T) {
	content := []byte("BT\n(08\\05515\\0552024) Tj\nET\n")
	cmap := []byte("begincmap\nendcmap\n")
	input := testPDF(
		"<< /Type /Catalog /AcroForm 2 0 R >>",
		fmt.Sprintf("<< /Type /Page /Annots [ ] /Resources << /Font << /F1 3 0 R >> >> /Length %d >>\nstream\n%sendstream", len(content), content),
		"<< /Type /Font /Subtype /Type0 /ToUnicode 4 0 R /DescendantFonts [5 0 R] >>",
		fmt.Sprintf("<< /Length %d >>\nstream\n%sendstream", len(cmap), cmap),
		"<< /Type /Font /Subtype /CIDFontType2 >>",
	)

	adapter := NewAdapter()
	tree, err := adapter.Parse(input, core.ParseOptions{Strict: true})
	if err != nil {
		t.Fatal(err)
	}
	assertBoundaryFlags(t, tree, map[string]bool{
		"has_acroform":         true,
		"has_annotations":      true,
		"has_font_markers":     true,
		"has_cmap_markers":     true,
		"has_tounicode_cmap":   true,
		"has_cid_font_markers": true,
		"has_encrypt":          false,
		"has_signature":        false,
		"has_xfa":              false,
	})

	plan, err := adapter.PlanEdit(tree, core.Match{Kind: KindTextShow, Text: "08-15-2024"}, core.Mutation{Replace: "May 5, 2026"})
	if err != nil {
		t.Fatal(err)
	}
	if plan.Operation != "pdf.content_stream_text_rewrite" {
		t.Fatalf("operation = %q", plan.Operation)
	}
}

func TestSemanticBoundaryNameMatchingDoesNotTreatLongerNamesAsUnsupported(t *testing.T) {
	content := []byte("BT\n(08\\05515\\0552024) Tj\nET\n")
	input := testPDF(
		"<< /Type /Catalog /SignaturePolicy 3 /EncryptedPayload 2 0 R >>",
		fmt.Sprintf("<< /Type /Page /Length %d >>\nstream\n%sendstream", len(content), content),
	)

	tree, err := NewAdapter().Parse(input, core.ParseOptions{Strict: true})
	if err != nil {
		t.Fatal(err)
	}
	assertBoundaryFlags(t, tree, map[string]bool{
		"has_encrypt":   false,
		"has_signature": false,
	})
}

func TestSemanticBoundarySignatureMarkersInsideLiteralStringsDoNotFailClosed(t *testing.T) {
	content := []byte("BT\n( /ByteRange ) Tj\n( /SigFlags ) Tj\nET\n")
	input := testPDF(
		"<< /Type /Catalog >>",
		fmt.Sprintf("<< /Type /Page /Length %d >>\nstream\n%sendstream", len(content), content),
	)

	tree, err := NewAdapter().Parse(input, core.ParseOptions{Strict: true})
	if err != nil {
		t.Fatal(err)
	}
	assertBoundaryFlags(t, tree, map[string]bool{
		"has_signature": false,
	})
}

func TestSemanticBoundarySignatureMarkersInsideCommentsDoNotFailClosed(t *testing.T) {
	content := []byte("BT\n(visible) Tj\nET\n")
	input := testPDF(
		"<< /Type /Catalog % /ByteRange [0 10 20 30]\n/Pages 2 0 R >>",
		fmt.Sprintf("<< /Type /Page /Length %d >>\nstream\n%sendstream", len(content), content),
	)

	tree, err := NewAdapter().Parse(input, core.ParseOptions{Strict: true})
	if err != nil {
		t.Fatal(err)
	}
	assertBoundaryFlags(t, tree, map[string]bool{
		"has_signature": false,
	})
}

func TestSemanticBoundarySignatureMarkersInsideHexStringsDoNotFailClosed(t *testing.T) {
	content := []byte("BT\n(visible) Tj\nET\n")
	input := testPDF(
		"<< /Type /Catalog /Contents < /SigFlags > >>",
		fmt.Sprintf("<< /Type /Page /Length %d >>\nstream\n%sendstream", len(content), content),
	)

	tree, err := NewAdapter().Parse(input, core.ParseOptions{Strict: true})
	if err != nil {
		t.Fatal(err)
	}
	assertBoundaryFlags(t, tree, map[string]bool{
		"has_signature": false,
	})
}

func TestSemanticBoundarySignatureMarkersAsNamesStillFailClosed(t *testing.T) {
	tests := []struct {
		name   string
		object string
	}{
		{name: "sig", object: "<< /Type /Catalog /FT /Sig >>"},
		{name: "byte_range", object: "<< /Type /Catalog /ByteRange [0 10 20 30] >>"},
		{name: "sig_flags", object: "<< /Type /Catalog /SigFlags 3 >>"},
	}

	for _, tc := range tests {
		t.Run(tc.name, func(t *testing.T) {
			input := testPDF(tc.object, "<<>>")
			tree, err := NewAdapter().Parse(input, core.ParseOptions{Strict: true})
			if err == nil {
				t.Fatal("expected signature boundary parse error")
			}
			if err != ErrSignedPDFRequiresInvalidation {
				t.Fatalf("error = %v, want %v", err, ErrSignedPDFRequiresInvalidation)
			}
			if tree != nil {
				t.Fatal("signature boundary returned a tree")
			}
		})
	}
}

func TestSemanticBoundaryNamesInsideLiteralStringsCommentsAndHexStringsAreIgnored(t *testing.T) {
	content := []byte("BT\n( /Encrypt /XFA /AcroForm /Annots /Font /ToUnicode /CMap /CIDFontType0 /CIDFontType2 /CIDToGIDMap ) Tj\nET\n")
	input := testPDF(
		"<< /Type /Catalog % /Encrypt /XFA /AcroForm\n/Pages 2 0 R >>",
		fmt.Sprintf("<< /Type /Page /Contents < /Annots /Font /ToUnicode /CMap /CIDFontType0 /CIDFontType2 /CIDToGIDMap > /Length %d >>\nstream\n%sendstream", len(content), content),
	)

	tree, err := NewAdapter().Parse(input, core.ParseOptions{Strict: true})
	if err != nil {
		t.Fatal(err)
	}
	assertBoundaryFlags(t, tree, map[string]bool{
		"has_encrypt":          false,
		"has_acroform":         false,
		"has_xfa":              false,
		"has_annotations":      false,
		"has_font_markers":     false,
		"has_cmap_markers":     false,
		"has_tounicode_cmap":   false,
		"has_cid_font_markers": false,
	})
}

func TestSemanticBoundaryRawCMapMarkersInsideLiteralStringsCommentsAndHexStringsAreIgnored(t *testing.T) {
	content := []byte("BT\n( begincmap beginbfchar beginbfrange ) Tj\n% begincmap beginbfchar beginbfrange\n<626567696e636d617020626567696e62666368617220626567696e626672616e6765> Tj\nET\n")
	input := testPDF(
		"<< /Type /Catalog >>",
		fmt.Sprintf("<< /Type /Page /Length %d >>\nstream\n%sendstream", len(content), content),
	)

	tree, err := NewAdapter().Parse(input, core.ParseOptions{Strict: true})
	if err != nil {
		t.Fatal(err)
	}
	assertBoundaryFlags(t, tree, map[string]bool{
		"has_cmap_markers":   false,
		"has_tounicode_cmap": false,
	})
}

func TestSemanticBoundaryRawCMapMarkersRequireBareTokenBoundaries(t *testing.T) {
	content := []byte("prebegincmap beginbfcharSuffix /beginbfrange\n")
	input := testPDF(
		"<< /Type /Catalog >>",
		fmt.Sprintf("<< /Type /Page /Length %d >>\nstream\n%sendstream", len(content), content),
	)

	tree, err := NewAdapter().Parse(input, core.ParseOptions{Strict: true})
	if err != nil {
		t.Fatal(err)
	}
	assertBoundaryFlags(t, tree, map[string]bool{
		"has_cmap_markers": false,
	})
}

func TestSemanticBoundaryRawCMapMarkersAsBareStreamTokensAreDetected(t *testing.T) {
	tests := []struct {
		name    string
		content []byte
	}{
		{name: "begincmap", content: []byte("begincmap\nendcmap\n")},
		{name: "beginbfchar", content: []byte("1 beginbfchar\n<01> <0041>\nendbfchar\n")},
		{name: "beginbfrange", content: []byte("1 beginbfrange\n<01> <02> <0041>\nendbfrange\n")},
	}

	for _, tc := range tests {
		t.Run(tc.name, func(t *testing.T) {
			input := testPDF(
				"<< /Type /Catalog >>",
				fmt.Sprintf("<< /Type /Page /Length %d >>\nstream\n%sendstream", len(tc.content), tc.content),
			)

			tree, err := NewAdapter().Parse(input, core.ParseOptions{Strict: true})
			if err != nil {
				t.Fatal(err)
			}
			assertBoundaryFlags(t, tree, map[string]bool{
				"has_cmap_markers": true,
			})
		})
	}
}

func TestSemanticBoundaryNamesAsDictionaryNamesAreStillDetected(t *testing.T) {
	content := []byte("BT\n(visible) Tj\nET\n")
	cmap := []byte("begincmap\nendcmap\n")
	input := testPDF(
		"<< /Type /Catalog /AcroForm 2 0 R >>",
		fmt.Sprintf("<< /Type /Page /Annots [ ] /Resources << /Font << /F1 3 0 R >> >> /Length %d >>\nstream\n%sendstream", len(content), content),
		"<< /Type /Font /Subtype /Type0 /ToUnicode 4 0 R /DescendantFonts [5 0 R] >>",
		fmt.Sprintf("<< /Length %d >>\nstream\n%sendstream", len(cmap), cmap),
		"<< /Type /Font /Subtype /CIDFontType2 /CIDToGIDMap /Identity >>",
	)

	tree, err := NewAdapter().Parse(input, core.ParseOptions{Strict: true})
	if err != nil {
		t.Fatal(err)
	}
	assertBoundaryFlags(t, tree, map[string]bool{
		"has_acroform":         true,
		"has_annotations":      true,
		"has_font_markers":     true,
		"has_cmap_markers":     true,
		"has_tounicode_cmap":   true,
		"has_cid_font_markers": true,
	})
}

func TestSemanticBoundaryEncryptAndXFAAsDictionaryNamesStillFailClosed(t *testing.T) {
	tests := []struct {
		name  string
		input []byte
		want  string
	}{
		{
			name:  "encrypt",
			input: testPDF("<< /Type /Catalog /Encrypt 2 0 R >>", "<<>>"),
			want:  "unsupported PDF: encrypted PDFs require an explicit password-capable path; Adapter.Parse does not decrypt",
		},
		{
			name:  "xfa",
			input: testPDF("<< /Type /Catalog /AcroForm << /XFA 2 0 R >> >>", "<<>>"),
			want:  "unsupported PDF: XFA forms are not implemented",
		},
	}

	for _, tc := range tests {
		t.Run(tc.name, func(t *testing.T) {
			tree, err := NewAdapter().Parse(tc.input, core.ParseOptions{Strict: true})
			if err == nil {
				t.Fatal("expected semantic boundary parse error")
			}
			if err.Error() != tc.want {
				t.Fatalf("error = %q, want %q", err, tc.want)
			}
			if tree != nil {
				t.Fatal("fail-closed semantic boundary returned a tree")
			}
		})
	}
}

func assertBoundaryFlags(t *testing.T, tree *core.Tree, want map[string]bool) {
	t.Helper()
	root, ok := tree.Node(tree.Root)
	if !ok {
		t.Fatal("missing root node")
	}
	value := root.Value.(map[string]any)
	boundaries := value["boundaries"].(map[string]any)
	for key, expected := range want {
		if boundaries[key] != expected {
			t.Fatalf("%s = %v, want %v", key, boundaries[key], expected)
		}
	}
}
