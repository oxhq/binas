package pdf

import (
	"bytes"
	"fmt"
	"strings"
	"testing"

	"github.com/oxhq/binas/pkg/core"
)

func TestFilterFlateDecodePredictor1DecodeParmsDefaultGeometryRoundTrip(t *testing.T) {
	input := []byte("BT\n(Predictor one is no prediction) Tj\nET\n")
	cases := []struct {
		name        string
		filter      string
		decodeParms string
	}{
		{
			name:        "direct dictionary full default geometry",
			filter:      "/FlateDecode",
			decodeParms: "<< /Predictor 1 /Columns 1 /Colors 1 /BitsPerComponent 8 >>",
		},
		{
			name:        "direct dictionary subset default geometry",
			filter:      "/FlateDecode",
			decodeParms: "<< /Predictor 1 /Columns 1 >>",
		},
		{
			name:        "filter array dictionary full default geometry",
			filter:      "[/FlateDecode]",
			decodeParms: "[<< /Predictor 1 /Columns 1 /Colors 1 /BitsPerComponent 8 >>]",
		},
		{
			name:        "direct dictionary non default ignored geometry",
			filter:      "/FlateDecode",
			decodeParms: "<< /Predictor 1 /Columns 4 /Colors 3 /BitsPerComponent 16 >>",
		},
		{
			name:        "empty direct dictionary defaults predictor one",
			filter:      "/FlateDecode",
			decodeParms: "<< >>",
		},
		{
			name:        "omitted predictor with geometry defaults predictor one",
			filter:      "/FlateDecode",
			decodeParms: "<< /Columns 4 /Colors 3 /BitsPerComponent 16 >>",
		},
		{
			name:        "filter array omitted predictor with geometry",
			filter:      "[/FlateDecode]",
			decodeParms: "[<< /Columns 4 /Colors 3 /BitsPerComponent 16 >>]",
		},
		{
			name:        "signed positive predictor and geometry",
			filter:      "/FlateDecode",
			decodeParms: "<< /Predictor +1 /Columns +4 /Colors +3 /BitsPerComponent +16 >>",
		},
	}

	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			encoded, err := encodeStreamFilterWithDecodeParms(tc.filter, tc.decodeParms, input)
			if err != nil {
				t.Fatal(err)
			}
			decoded, err := decodeStreamFilterWithDecodeParms(tc.filter, tc.decodeParms, encoded)
			if err != nil {
				t.Fatal(err)
			}
			if !bytes.Equal(decoded, input) {
				t.Fatalf("decoded stream = %q, want %q", decoded, input)
			}
		})
	}
}

func TestFilterDecodeParmsSupportedPredictorMatrixRoundTrip(t *testing.T) {
	input := []byte("BT\n(08\\05515\\0552024) Tj\nET\nBT\n(08\\05515\\0552024) Tj\nET\n")
	cases := []struct {
		name        string
		filter      string
		decodeParms string
	}{
		{name: "flate no decode parms", filter: "/FlateDecode"},
		{name: "flate null decode parms", filter: "/FlateDecode", decodeParms: "null"},
		{name: "flate predictor 1 direct dictionary", filter: "/FlateDecode", decodeParms: "<< /Predictor 1 /Columns 1 /Colors 1 /BitsPerComponent 8 >>"},
		{name: "flate tiff predictor 2 direct dictionary", filter: "/FlateDecode", decodeParms: "<< /Predictor 2 /Columns 1 /Colors 1 /BitsPerComponent 8 >>"},
		{name: "flate png predictor 10 direct dictionary", filter: "/FlateDecode", decodeParms: "<< /Predictor 10 /Columns 1 /Colors 1 /BitsPerComponent 8 >>"},
		{name: "flate png predictor 11 direct dictionary", filter: "/FlateDecode", decodeParms: "<< /Predictor 11 /Columns 1 /Colors 1 /BitsPerComponent 8 >>"},
		{name: "flate png predictor 12 direct dictionary", filter: "/FlateDecode", decodeParms: "<< /Predictor 12 /Columns 1 /Colors 1 /BitsPerComponent 8 >>"},
		{name: "flate png predictor 13 direct dictionary", filter: "/FlateDecode", decodeParms: "<< /Predictor 13 /Columns 1 /Colors 1 /BitsPerComponent 8 >>"},
		{name: "flate png predictor 14 direct dictionary", filter: "/FlateDecode", decodeParms: "<< /Predictor 14 /Columns 1 /Colors 1 /BitsPerComponent 8 >>"},
		{name: "flate png predictor 15 direct dictionary", filter: "/FlateDecode", decodeParms: "<< /Predictor 15 /Columns 1 /Colors 1 /BitsPerComponent 8 >>"},
		{name: "lzw no decode parms", filter: "/LZWDecode"},
		{name: "lzw null decode parms", filter: "/LZWDecode", decodeParms: "null"},
		{name: "lzw early change 0 direct dictionary", filter: "/LZWDecode", decodeParms: "<< /EarlyChange 0 >>"},
		{name: "lzw early change 1 direct dictionary", filter: "/LZWDecode", decodeParms: "<< /EarlyChange 1 >>"},
		{name: "lzw predictor 1 direct dictionary", filter: "/LZWDecode", decodeParms: "<< /EarlyChange 1 /Predictor 1 /Columns 1 /Colors 1 /BitsPerComponent 8 >>"},
		{name: "lzw tiff predictor 2 direct dictionary", filter: "/LZWDecode", decodeParms: "<< /EarlyChange 1 /Predictor 2 /Columns 1 /Colors 1 /BitsPerComponent 8 >>"},
		{name: "lzw png predictor 10 direct dictionary", filter: "/LZWDecode", decodeParms: "<< /EarlyChange 1 /Predictor 10 /Columns 1 /Colors 1 /BitsPerComponent 8 >>"},
		{name: "lzw png predictor 11 direct dictionary", filter: "/LZWDecode", decodeParms: "<< /EarlyChange 1 /Predictor 11 /Columns 1 /Colors 1 /BitsPerComponent 8 >>"},
		{name: "lzw png predictor 12 direct dictionary", filter: "/LZWDecode", decodeParms: "<< /EarlyChange 1 /Predictor 12 /Columns 1 /Colors 1 /BitsPerComponent 8 >>"},
		{name: "lzw png predictor 13 direct dictionary", filter: "/LZWDecode", decodeParms: "<< /EarlyChange 1 /Predictor 13 /Columns 1 /Colors 1 /BitsPerComponent 8 >>"},
		{name: "lzw png predictor 14 direct dictionary", filter: "/LZWDecode", decodeParms: "<< /EarlyChange 1 /Predictor 14 /Columns 1 /Colors 1 /BitsPerComponent 8 >>"},
		{name: "lzw png predictor 15 direct dictionary", filter: "/LZWDecode", decodeParms: "<< /EarlyChange 1 /Predictor 15 /Columns 1 /Colors 1 /BitsPerComponent 8 >>"},
	}

	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			encoded, err := encodeStreamFilterWithDecodeParms(tc.filter, tc.decodeParms, input)
			if err != nil {
				t.Fatal(err)
			}
			decoded, err := decodeStreamFilterWithDecodeParms(tc.filter, tc.decodeParms, encoded)
			if err != nil {
				t.Fatal(err)
			}
			if !bytes.Equal(decoded, input) {
				t.Fatalf("decoded stream = %q, want %q", decoded, input)
			}
		})
	}
}

func TestFilterDecodeParmsSupportedPredictorMatrixRewriteReencodesLengthAndXref(t *testing.T) {
	decoded := []byte("BT\n(08\\05515\\0552024) Tj\nET\n")
	cases := []struct {
		name        string
		filter      string
		decodeParms string
	}{
		{name: "flate predictor 1 direct dictionary", filter: "/FlateDecode", decodeParms: "<< /Predictor 1 /Columns 1 /Colors 1 /BitsPerComponent 8 >>"},
		{name: "flate tiff predictor 2 direct dictionary", filter: "/FlateDecode", decodeParms: "<< /Predictor 2 /Columns 1 /Colors 1 /BitsPerComponent 8 >>"},
		{name: "flate png predictor 10 direct dictionary", filter: "/FlateDecode", decodeParms: "<< /Predictor 10 /Columns 1 /Colors 1 /BitsPerComponent 8 >>"},
		{name: "flate png predictor 11 direct dictionary", filter: "/FlateDecode", decodeParms: "<< /Predictor 11 /Columns 1 /Colors 1 /BitsPerComponent 8 >>"},
		{name: "flate png predictor 12 direct dictionary", filter: "/FlateDecode", decodeParms: "<< /Predictor 12 /Columns 1 /Colors 1 /BitsPerComponent 8 >>"},
		{name: "flate png predictor 13 direct dictionary", filter: "/FlateDecode", decodeParms: "<< /Predictor 13 /Columns 1 /Colors 1 /BitsPerComponent 8 >>"},
		{name: "flate png predictor 14 direct dictionary", filter: "/FlateDecode", decodeParms: "<< /Predictor 14 /Columns 1 /Colors 1 /BitsPerComponent 8 >>"},
		{name: "flate png predictor 15 direct dictionary", filter: "/FlateDecode", decodeParms: "<< /Predictor 15 /Columns 1 /Colors 1 /BitsPerComponent 8 >>"},
		{name: "lzw early change 1 direct dictionary", filter: "/LZWDecode", decodeParms: "<< /EarlyChange 1 >>"},
		{name: "lzw predictor 1 direct dictionary", filter: "/LZWDecode", decodeParms: "<< /EarlyChange 1 /Predictor 1 /Columns 1 /Colors 1 /BitsPerComponent 8 >>"},
		{name: "lzw tiff predictor 2 direct dictionary", filter: "/LZWDecode", decodeParms: "<< /EarlyChange 1 /Predictor 2 /Columns 1 /Colors 1 /BitsPerComponent 8 >>"},
		{name: "lzw png predictor 10 direct dictionary", filter: "/LZWDecode", decodeParms: "<< /EarlyChange 1 /Predictor 10 /Columns 1 /Colors 1 /BitsPerComponent 8 >>"},
		{name: "lzw png predictor 11 direct dictionary", filter: "/LZWDecode", decodeParms: "<< /EarlyChange 1 /Predictor 11 /Columns 1 /Colors 1 /BitsPerComponent 8 >>"},
		{name: "lzw png predictor 12 direct dictionary", filter: "/LZWDecode", decodeParms: "<< /EarlyChange 1 /Predictor 12 /Columns 1 /Colors 1 /BitsPerComponent 8 >>"},
		{name: "lzw png predictor 13 direct dictionary", filter: "/LZWDecode", decodeParms: "<< /EarlyChange 1 /Predictor 13 /Columns 1 /Colors 1 /BitsPerComponent 8 >>"},
		{name: "lzw png predictor 14 direct dictionary", filter: "/LZWDecode", decodeParms: "<< /EarlyChange 1 /Predictor 14 /Columns 1 /Colors 1 /BitsPerComponent 8 >>"},
		{name: "lzw png predictor 15 direct dictionary", filter: "/LZWDecode", decodeParms: "<< /EarlyChange 1 /Predictor 15 /Columns 1 /Colors 1 /BitsPerComponent 8 >>"},
	}

	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			encoded, err := encodeStreamFilterWithDecodeParms(tc.filter, tc.decodeParms, decoded)
			if err != nil {
				t.Fatal(err)
			}
			input := testPDF(
				"<< /Type /Page >>",
				fmt.Sprintf("<< /Length %d /Filter %s /DecodeParms %s >>\nstream\n%sendstream", len(encoded), tc.filter, tc.decodeParms, encoded),
			)

			assertLZWRewrite(t, input, encoded, tc.filter, tc.decodeParms)
		})
	}
}

func TestFilterFlateDecodePredictor1NonDefaultGeometryRewriteReencodesLengthAndXref(t *testing.T) {
	decoded := []byte("BT\n(08\\05515\\0552024) Tj\nET\n")
	decodeParms := "<< /Predictor 1 /Columns 4 /Colors 3 /BitsPerComponent 16 >>"
	encoded, err := encodeStreamFilterWithDecodeParms("/FlateDecode", decodeParms, decoded)
	if err != nil {
		t.Fatal(err)
	}
	input := testPDF(
		"<< /Type /Page >>",
		fmt.Sprintf("<< /Length %d /Filter /FlateDecode /DecodeParms %s >>\nstream\n%sendstream", len(encoded), decodeParms, encoded),
	)

	assertLZWRewrite(t, input, encoded, "/FlateDecode", decodeParms)
}

func TestFilterLZWDecodePredictor1NonDefaultGeometryRewriteReencodesLengthAndXref(t *testing.T) {
	decoded := []byte("BT\n(08\\05515\\0552024) Tj\nET\n")
	decodeParms := "<< /EarlyChange 0 /Predictor 1 /Columns 4 /Colors 3 /BitsPerComponent 16 >>"
	encoded, err := encodeStreamFilterWithDecodeParms("/LZWDecode", decodeParms, decoded)
	if err != nil {
		t.Fatal(err)
	}
	input := testPDF(
		"<< /Type /Page >>",
		fmt.Sprintf("<< /Length %d /Filter /LZWDecode /DecodeParms %s >>\nstream\n%sendstream", len(encoded), decodeParms, encoded),
	)

	assertLZWRewrite(t, input, encoded, "/LZWDecode", decodeParms)
}

func TestFilterFlateDecodeOmittedPredictorGeometryRewriteReencodesLengthAndXref(t *testing.T) {
	decoded := []byte("BT\n(08\\05515\\0552024) Tj\nET\n")
	decodeParms := "<< /Columns 4 /Colors 3 /BitsPerComponent 16 >>"
	encoded, err := encodeStreamFilterWithDecodeParms("/FlateDecode", decodeParms, decoded)
	if err != nil {
		t.Fatal(err)
	}
	input := testPDF(
		"<< /Type /Page >>",
		fmt.Sprintf("<< /Length %d /Filter /FlateDecode /DecodeParms %s >>\nstream\n%sendstream", len(encoded), decodeParms, encoded),
	)

	assertLZWRewrite(t, input, encoded, "/FlateDecode", decodeParms)
}

func TestFilterLZWDecodeOmittedPredictorGeometryRewriteReencodesLengthAndXref(t *testing.T) {
	decoded := []byte("BT\n(08\\05515\\0552024) Tj\nET\n")
	decodeParms := "<< /EarlyChange 0 /Columns 4 /Colors 3 /BitsPerComponent 16 >>"
	encoded, err := encodeStreamFilterWithDecodeParms("/LZWDecode", decodeParms, decoded)
	if err != nil {
		t.Fatal(err)
	}
	input := testPDF(
		"<< /Type /Page >>",
		fmt.Sprintf("<< /Length %d /Filter /LZWDecode /DecodeParms %s >>\nstream\n%sendstream", len(encoded), decodeParms, encoded),
	)

	assertLZWRewrite(t, input, encoded, "/LZWDecode", decodeParms)
}

func TestIndirectDecodeParmsDictionaryRewriteReencodesLengthAndXref(t *testing.T) {
	decoded := []byte("BT\n(08\\05515\\0552024) Tj\nET\n")
	decodeParms := "<< /Predictor 1 /Columns 4 /Colors 3 /BitsPerComponent 16 >>"
	encoded, err := encodeStreamFilterWithDecodeParms("/FlateDecode", decodeParms, decoded)
	if err != nil {
		t.Fatal(err)
	}
	input := testPDF(
		"<< /Type /Page >>",
		fmt.Sprintf("<< /Length %d /Filter /FlateDecode /DecodeParms 3 0 R >>\nstream\n%sendstream", len(encoded), encoded),
		decodeParms,
	)

	assertIndirectDecodeParmsRewrite(t, input, encoded, "/FlateDecode", decodeParms, "xref\n0 4\n")
}

func TestIndirectDecodeParmsArrayEntryRewriteReencodesLengthAndXref(t *testing.T) {
	decoded := []byte("BT\n(08\\05515\\0552024) Tj\nET\n")
	decodeParms := "[null << /Predictor 1 /Columns 4 /Colors 3 /BitsPerComponent 16 >>]"
	encoded, err := encodeStreamFilterWithDecodeParms("[/ASCIIHexDecode /FlateDecode]", decodeParms, decoded)
	if err != nil {
		t.Fatal(err)
	}
	input := testPDF(
		"<< /Type /Page >>",
		fmt.Sprintf("<< /Length %d /Filter [/ASCIIHexDecode /FlateDecode] /DecodeParms [null 3 0 R] >>\nstream\n%sendstream", len(encoded), encoded),
		"<< /Predictor 1 /Columns 4 /Colors 3 /BitsPerComponent 16 >>",
	)

	assertIndirectDecodeParmsRewrite(t, input, encoded, "[/ASCIIHexDecode /FlateDecode]", decodeParms, "xref\n0 4\n")
}

func TestIndirectDecodeParmsArrayRepeatedReferenceRewriteReencodesLengthAndXref(t *testing.T) {
	decoded := []byte("BT\n(08\\05515\\0552024) Tj\nET\n")
	decodeParms := "[<< /Predictor 1 /Columns 4 >> << /Predictor 1 /Columns 4 >>]"
	encoded, err := encodeStreamFilterWithDecodeParms("[/FlateDecode /FlateDecode]", decodeParms, decoded)
	if err != nil {
		t.Fatal(err)
	}
	input := testPDF(
		"<< /Type /Page >>",
		fmt.Sprintf("<< /Length %d /Filter [/FlateDecode /FlateDecode] /DecodeParms [3 0 R 3 0 R] >>\nstream\n%sendstream", len(encoded), encoded),
		"<< /Predictor 1 /Columns 4 >>",
	)

	assertIndirectDecodeParmsRewrite(t, input, encoded, "[/FlateDecode /FlateDecode]", decodeParms, "xref\n0 4\n")
}

func TestCanonicalIndirectDecodeParmsDictionaryRewrite(t *testing.T) {
	decoded := []byte("BT\n(08\\05515\\0552024) Tj\nET\n")
	decodeParms := "<< /Predictor 1 /Columns 4 /Colors 3 /BitsPerComponent 16 >>"
	encoded, err := encodeStreamFilterWithDecodeParms("/FlateDecode", decodeParms, decoded)
	if err != nil {
		t.Fatal(err)
	}
	input := testPDF(
		"<< /Type /Page >>",
		fmt.Sprintf("<< /Length %d /Filter /FlateDecode /DecodeParms 3 0 R >>\nstream\n%sendstream", len(encoded), encoded),
		decodeParms,
	)

	output, _, verification, err := EditCanonical(input, core.Match{Kind: KindTextShow, Text: "08-15-2024"}, core.Mutation{Replace: "May 5, 2026"}, nil)
	if err != nil {
		t.Fatal(err)
	}
	if !verification.ReparseOK || !verification.OldTextRemoved || !verification.NewSelectable || !verification.PageUnchanged {
		t.Fatalf("verification failed: %+v", verification)
	}
	if bytes.Contains(output, []byte("/DecodeParms")) {
		t.Fatalf("canonical edited stream should be written decoded without DecodeParms:\n%s", output)
	}
	if !bytes.Contains(output, []byte("(May 5, 2026) Tj")) {
		t.Fatalf("canonical output missing replacement:\n%s", output)
	}
}

func TestIndirectDecodeParmsReferencesFailClosed(t *testing.T) {
	encoded, err := encodeFlateDecode([]byte("BT\n(08\\05515\\0552024) Tj\nET\n"))
	if err != nil {
		t.Fatal(err)
	}
	cases := []struct {
		name    string
		objects []string
		want    string
	}{
		{
			name: "missing reference",
			objects: []string{
				"<< /Type /Page >>",
				fmt.Sprintf("<< /Length %d /Filter /FlateDecode /DecodeParms 9 0 R >>\nstream\n%sendstream", len(encoded), encoded),
			},
			want: "unsupported stream: /DecodeParms reference must resolve to a dictionary, array, or null object",
		},
		{
			name: "scalar reference",
			objects: []string{
				"<< /Type /Page >>",
				fmt.Sprintf("<< /Length %d /Filter /FlateDecode /DecodeParms 3 0 R >>\nstream\n%sendstream", len(encoded), encoded),
				"12",
			},
			want: "unsupported stream: /DecodeParms reference must resolve to a dictionary, array, or null object",
		},
		{
			name: "referenced dictionary has trailing data",
			objects: []string{
				"<< /Type /Page >>",
				fmt.Sprintf("<< /Length %d /Filter /FlateDecode /DecodeParms 3 0 R >>\nstream\n%sendstream", len(encoded), encoded),
				"<< /Predictor 1 >> true",
			},
			want: "unsupported stream: /DecodeParms reference object has trailing data",
		},
		{
			name: "array scalar entry reference",
			objects: []string{
				"<< /Type /Page >>",
				fmt.Sprintf("<< /Length %d /Filter [/ASCIIHexDecode /FlateDecode] /DecodeParms [null 3 0 R] >>\nstream\n%sendstream", len(encoded), encoded),
				"12",
			},
			want: "unsupported stream: /DecodeParms array entries must resolve to null or direct dictionaries",
		},
		{
			name: "reference cycle",
			objects: []string{
				"<< /Type /Page >>",
				fmt.Sprintf("<< /Length %d /Filter /FlateDecode /DecodeParms 3 0 R >>\nstream\n%sendstream", len(encoded), encoded),
				"4 0 R",
				"3 0 R",
			},
			want: "unsupported stream: /DecodeParms reference cycle",
		},
		{
			name: "array entry resolves to nested array",
			objects: []string{
				"<< /Type /Page >>",
				fmt.Sprintf("<< /Length %d /Filter [/FlateDecode /FlateDecode] /DecodeParms [3 0 R << /Predictor 1 >>] >>\nstream\n%sendstream", len(encoded), encoded),
				"[null]",
			},
			want: "unsupported stream: /DecodeParms array entries must resolve to null or direct dictionaries",
		},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			tree, err := NewAdapter().Parse(testPDF(tc.objects...), core.ParseOptions{Strict: true})
			if err != nil {
				t.Fatal(err)
			}
			if matches := tree.Query(core.Match{Kind: KindTextShow, Text: "08-15-2024"}); len(matches) != 0 {
				t.Fatalf("text matches = %d, want 0", len(matches))
			}
			streams := tree.Query(core.Match{Kind: KindStream})
			if len(streams) != 1 {
				t.Fatalf("stream nodes = %d, want 1", len(streams))
			}
			if got := streams[0].Meta["unsupported"]; got != tc.want {
				t.Fatalf("unsupported metadata = %q, want %q", got, tc.want)
			}
			if _, err := NewAdapter().PlanEdit(tree, core.Match{Kind: KindTextShow, Text: "08-15-2024"}, core.Mutation{Replace: "May 5, 2026"}); err == nil {
				t.Fatal("expected edit planning to fail closed")
			}
		})
	}
}

func TestDecodeParmsMalformedShapesFailClosed(t *testing.T) {
	input, err := encodeFlateDecode([]byte("stream"))
	if err != nil {
		t.Fatal(err)
	}
	cases := []struct {
		name        string
		filter      string
		decodeParms string
		want        string
	}{
		{
			name:        "nested predictor reference",
			filter:      "/FlateDecode",
			decodeParms: "<< /Predictor 3 0 R /Columns 1 >>",
			want:        "unsupported stream: /DecodeParms /Predictor must be a direct integer",
		},
		{
			name:        "nested columns reference",
			filter:      "/FlateDecode",
			decodeParms: "<< /Predictor 12 /Columns 3 0 R >>",
			want:        "unsupported stream: /DecodeParms /Columns must be a direct integer",
		},
		{
			name:        "oversized PNG geometry",
			filter:      "/FlateDecode",
			decodeParms: "<< /Predictor 12 /Columns 9223372036854775807 /Colors 2 /BitsPerComponent 8 >>",
			want:        "PNG predictor stream: row width overflows",
		},
		{
			name:        "out of range columns identifies key",
			filter:      "/FlateDecode",
			decodeParms: "<< /Predictor 12 /Columns 999999999999999999999999999999999999999 /Colors 1 /BitsPerComponent 8 >>",
			want:        "unsupported stream: /DecodeParms /Columns must be a direct integer",
		},
		{
			name:        "unsupported predictor identifies key",
			filter:      "/FlateDecode",
			decodeParms: "<< /Predictor 9 /Columns 1 >>",
			want:        "unsupported stream: /DecodeParms /Predictor 9 is not supported",
		},
		{
			name:        "png predictor negative colors identifies key",
			filter:      "/FlateDecode",
			decodeParms: "<< /Predictor 12 /Columns 1 /Colors -1 >>",
			want:        "unsupported stream: /DecodeParms PNG predictors require /Colors >= 1",
		},
		{
			name:        "png predictor negative bits per component identifies key",
			filter:      "/FlateDecode",
			decodeParms: "<< /Predictor 12 /Columns 1 /BitsPerComponent -1 >>",
			want:        "unsupported stream: /DecodeParms PNG predictors require /BitsPerComponent >= 1",
		},
		{
			name:        "png predictor zero columns identifies key",
			filter:      "/FlateDecode",
			decodeParms: "<< /Predictor 12 /Columns 0 /Colors 1 /BitsPerComponent 8 >>",
			want:        "unsupported stream: /DecodeParms PNG predictors require /Columns >= 1",
		},
		{
			name:        "nested dictionary under predictor identifies predictor not nested names",
			filter:      "/FlateDecode",
			decodeParms: "<< /Predictor << /Foo 1 >> /Columns 1 >>",
			want:        "unsupported stream: /DecodeParms /Predictor must be a direct integer",
		},
		{
			name:        "crypt name wrong value type identifies key",
			filter:      "/Crypt",
			decodeParms: "<< /Name 12 >>",
			want:        "unsupported stream: /DecodeParms /Crypt /Name must be a name",
		},
		{
			name:        "lzw scalar indirect reference identifies key and ref",
			filter:      "/LZWDecode",
			decodeParms: "<< /EarlyChange 3 0 R >>",
			want:        "unsupported stream: /DecodeParms /EarlyChange must be a direct integer (got reference 3 0 R)",
		},
		{
			name:        "array entry error identifies position",
			filter:      "[/ASCIIHexDecode /FlateDecode]",
			decodeParms: "[null << /Predictor 1 /EarlyChange 0 >>]",
			want:        "unsupported stream: /DecodeParms array entry 1 for /FlateDecode: /DecodeParms for /FlateDecode key /EarlyChange is not supported",
		},
		{
			name:        "lzw png predictor negative columns identifies key",
			filter:      "/LZWDecode",
			decodeParms: "<< /Predictor 12 /Columns -1 /Colors 1 /BitsPerComponent 8 >>",
			want:        "unsupported stream: /DecodeParms PNG predictors require /Columns >= 1",
		},
		{
			name:        "direct dictionary for filter array",
			filter:      "[/FlateDecode /FlateDecode]",
			decodeParms: "<< /Predictor 1 >>",
			want:        "unsupported stream: direct /DecodeParms dictionary requires a single /Filter",
		},
	}

	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			_, err := decodeStreamFilterWithDecodeParms(tc.filter, tc.decodeParms, input)
			if err == nil {
				t.Fatal("expected DecodeParms error")
			}
			if !strings.Contains(err.Error(), tc.want) {
				t.Fatalf("DecodeParms error = %q, want containing %q", err, tc.want)
			}
		})
	}
}

func TestDecodeParmsRejectsUnsupportedKeysByFilter(t *testing.T) {
	input, err := encodeFlateDecode([]byte("stream"))
	if err != nil {
		t.Fatal(err)
	}

	cases := []struct {
		name        string
		filter      string
		decodeParms string
		want        string
	}{
		{
			name:        "flate rejects lzw early change",
			filter:      "/FlateDecode",
			decodeParms: "<< /Predictor 1 /EarlyChange 0 >>",
			want:        "unsupported stream: /DecodeParms for /FlateDecode key /EarlyChange is not supported",
		},
		{
			name:        "lzw rejects unrelated strategy key",
			filter:      "/LZWDecode",
			decodeParms: "<< /Predictor 1 /Columns 1 /Strategy 0 >>",
			want:        "unsupported stream: /DecodeParms for /LZWDecode key /Strategy is not supported",
		},
		{
			name:        "crypt rejects predictor key",
			filter:      "/Crypt",
			decodeParms: "<< /Predictor 1 >>",
			want:        "unsupported stream: /DecodeParms for /Crypt key /Predictor is not supported",
		},
		{
			name:        "dct rejects color transform key",
			filter:      "/DCTDecode",
			decodeParms: "<< /ColorTransform 1 >>",
			want:        "unsupported stream: /DecodeParms for /DCTDecode key /ColorTransform is not supported",
		},
		{
			name:        "ccitt rejects k key in array",
			filter:      "[/CCITTFaxDecode]",
			decodeParms: "[<< /K -1 >>]",
			want:        "unsupported stream: /DecodeParms array entry 0 for /CCITTFaxDecode: /DecodeParms for /CCITTFaxDecode key /K is not supported",
		},
	}

	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			_, err := decodeStreamFilterWithDecodeParms(tc.filter, tc.decodeParms, input)
			if err == nil {
				t.Fatal("expected DecodeParms key error")
			}
			if err.Error() != tc.want {
				t.Fatalf("DecodeParms error = %q, want %q", err, tc.want)
			}
		})
	}
}

func TestDecodeParmsImagePassThroughFiltersRequireNullWhenPresent(t *testing.T) {
	cases := []struct {
		name        string
		filter      string
		decodeParms string
		want        string
	}{
		{
			name:        "dct direct dictionary rejected",
			filter:      "/DCTDecode",
			decodeParms: "<< /ColorTransform 1 >>",
			want:        "unsupported stream: /DecodeParms for /DCTDecode key /ColorTransform is not supported",
		},
		{
			name:        "jpx direct dictionary rejected",
			filter:      "/JPXDecode",
			decodeParms: "<< /AnyKey 1 >>",
			want:        "unsupported stream: /DecodeParms for /JPXDecode key /AnyKey is not supported",
		},
		{
			name:        "ccitt array dictionary rejected",
			filter:      "[/CCITTFaxDecode]",
			decodeParms: "[<< /K -1 >>]",
			want:        "unsupported stream: /DecodeParms array entry 0 for /CCITTFaxDecode: /DecodeParms for /CCITTFaxDecode key /K is not supported",
		},
		{
			name:        "jbig2 mixed array dictionary rejected",
			filter:      "[/JBIG2Decode /FlateDecode]",
			decodeParms: "[<< /JBIG2Globals 3 0 R >> null]",
			want:        "unsupported stream: /DecodeParms array entry 0 for /JBIG2Decode: /DecodeParms for /JBIG2Decode key /JBIG2Globals is not supported",
		},
	}

	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			_, err := decodeStreamFilterWithDecodeParms(tc.filter, tc.decodeParms, []byte("image bytes"))
			if err == nil {
				t.Fatal("expected DecodeParms null requirement error")
			}
			if err.Error() != tc.want {
				t.Fatalf("DecodeParms error = %q, want %q", err, tc.want)
			}
		})
	}
}

func TestFilterFlateDecodePredictor1DecodeParmsRejectsMalformedGeometry(t *testing.T) {
	input, err := encodeFlateDecode([]byte("stream"))
	if err != nil {
		t.Fatal(err)
	}
	cases := []struct {
		name        string
		decodeParms string
	}{
		{
			name:        "non integer columns",
			decodeParms: "<< /Predictor 1 /Columns /Two >>",
		},
		{
			name:        "non integer colors",
			decodeParms: "<< /Predictor 1 /Colors /Two >>",
		},
		{
			name:        "non integer bits per component",
			decodeParms: "<< /Predictor 1 /BitsPerComponent /Eight >>",
		},
		{
			name:        "unsupported predictor",
			decodeParms: "<< /Predictor 9 /Columns 1 >>",
		},
		{
			name:        "negative unsupported predictor",
			decodeParms: "<< /Predictor -1 /Columns 1 >>",
		},
		{
			name:        "png predictor negative columns",
			decodeParms: "<< /Predictor 12 /Columns -1 >>",
		},
		{
			name:        "tiff predictor oversized bits per component",
			decodeParms: "<< /Predictor 2 /Columns 1 /BitsPerComponent 33 >>",
		},
	}

	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			if _, err := decodeStreamFilterWithDecodeParms("/FlateDecode", tc.decodeParms, input); err == nil {
				t.Fatal("expected DecodeParms error")
			}
		})
	}
}

func assertIndirectDecodeParmsRewrite(t *testing.T, input, originalEncoded []byte, filter, decodeParms, wantXref string) {
	t.Helper()

	adapter := NewAdapter()
	tree, err := adapter.Parse(input, core.ParseOptions{Strict: true})
	if err != nil {
		t.Fatal(err)
	}
	if matches := tree.Query(core.Match{Kind: KindTextShow, Text: "08-15-2024"}); len(matches) != 1 {
		t.Fatalf("old selectable matches = %d, want 1", len(matches))
	}
	plan, err := adapter.PlanEdit(tree, core.Match{Kind: KindTextShow, Text: "08-15-2024"}, core.Mutation{Replace: "May 5, 2026"})
	if err != nil {
		t.Fatal(err)
	}
	output, _, err := adapter.Apply(input, plan)
	if err != nil {
		t.Fatal(err)
	}
	verification, err := adapter.Verify(output, plan)
	if err != nil {
		t.Fatal(err)
	}
	if !verification.ReparseOK || !verification.OldTextRemoved || !verification.NewSelectable || !verification.PageUnchanged {
		t.Fatalf("verification failed: %+v", verification)
	}
	stream, ok, err := findNextStream(output, 0)
	if err != nil {
		t.Fatal(err)
	}
	if !ok {
		t.Fatal("missing stream")
	}
	if stream.lengthValue != stream.dataEnd-stream.dataStart {
		t.Fatalf("stream length = %d, want %d", stream.lengthValue, stream.dataEnd-stream.dataStart)
	}
	if bytes.Equal(output[stream.dataStart:stream.dataEnd], originalEncoded) {
		t.Fatal("encoded stream was not updated")
	}
	updatedDecoded, err := decodeStreamFilterWithDecodeParms(filter, decodeParms, output[stream.dataStart:stream.dataEnd])
	if err != nil {
		t.Fatal(err)
	}
	if bytes.Contains(updatedDecoded, []byte("08-15-2024")) {
		t.Fatalf("decoded stream still contains old text: %q", updatedDecoded)
	}
	if !bytes.Contains(updatedDecoded, []byte("(May 5, 2026) Tj")) {
		t.Fatalf("decoded stream = %q", updatedDecoded)
	}
	if !bytes.Contains(output, []byte(wantXref)) {
		t.Fatalf("xref table was not rebuilt:\n%s", output)
	}
}
