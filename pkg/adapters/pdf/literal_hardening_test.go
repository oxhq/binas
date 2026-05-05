package pdf

import (
	"fmt"
	"testing"

	"github.com/oxhq/binas/pkg/core"
)

func TestLiteralStringDecodeEscapesAndContinuations(t *testing.T) {
	tests := []struct {
		name    string
		encoded string
		want    string
	}{
		{
			name:    "escaped parentheses",
			encoded: `invoice \(draft\)`,
			want:    "invoice (draft)",
		},
		{
			name:    "escaped backslash",
			encoded: `C:\\Users\\example`,
			want:    `C:\Users\example`,
		},
		{
			name:    "octal escapes",
			encoded: `08\05515\0552024 \050ok\051`,
			want:    "08-15-2024 (ok)",
		},
		{
			name:    "line continuation lf",
			encoded: "line\\\ncontinued",
			want:    "linecontinued",
		},
		{
			name:    "line continuation crlf",
			encoded: "line\\\r\ncontinued",
			want:    "linecontinued",
		},
		{
			name:    "line continuation cr",
			encoded: "line\\\rcontinued",
			want:    "linecontinued",
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			if got := decodeLiteralString(tt.encoded); got != tt.want {
				t.Fatalf("decodeLiteralString(%q) = %q, want %q", tt.encoded, got, tt.want)
			}
		})
	}
}

func TestLiteralParserHandlesNestedAndEscapedParentheses(t *testing.T) {
	encoded := `root (nested \(escaped\) value) tail \\ path \055 done`
	want := `root (nested (escaped) value) tail \ path - done`

	got := parseSingleLiteralText(t, encoded)
	if got != want {
		t.Fatalf("parsed literal text = %q, want %q", got, want)
	}
}

func TestLiteralStringEncodeEscapesStructuralBytes(t *testing.T) {
	got := encodeLiteralString(`C:\tmp\(draft)-05`)
	want := `C:\\tmp\\\(draft\)\05505`
	if got != want {
		t.Fatalf("encodeLiteralString() = %q, want %q", got, want)
	}
}

func parseSingleLiteralText(t *testing.T, encoded string) string {
	t.Helper()

	input := []byte(fmt.Sprintf("%%PDF-1.3\n1 0 obj\n<< /Length %d >>\nstream\nBT\n(%s) Tj\nET\nendstream\nendobj\n%%%%EOF\n", len(encoded)+11, encoded))
	tree, err := NewAdapter().Parse(input, core.ParseOptions{Strict: true})
	if err != nil {
		t.Fatal(err)
	}

	matches := tree.Query(core.Match{Kind: KindTextShow})
	if len(matches) != 1 {
		t.Fatalf("text show matches = %d, want 1", len(matches))
	}
	return matches[0].Value.(string)
}
