package pdf

import (
	"bytes"
	"fmt"
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
