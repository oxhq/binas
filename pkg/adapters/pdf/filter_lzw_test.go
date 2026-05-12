package pdf

import (
	"bytes"
	"fmt"
	"testing"

	"github.com/oxhq/binas/pkg/core"
)

func TestFilterLZWDecodeRoundTrip(t *testing.T) {
	input := []byte("BT\n(05-05-2026) Tj\nET\nBT\n(05-05-2026) Tj\nET\n")

	for _, decodeParms := range []string{"", "<< /EarlyChange 1 >>", "<< /EarlyChange 0 >>"} {
		t.Run(decodeParms, func(t *testing.T) {
			encoded, err := encodeStreamFilterWithDecodeParms("/LZWDecode", decodeParms, input)
			if err != nil {
				t.Fatal(err)
			}
			if bytes.Equal(encoded, input) {
				t.Fatal("encoded stream unexpectedly matches input")
			}
			decoded, err := decodeStreamFilterWithDecodeParms("/LZWDecode", decodeParms, encoded)
			if err != nil {
				t.Fatal(err)
			}
			if !bytes.Equal(decoded, input) {
				t.Fatalf("decoded stream = %q, want %q", decoded, input)
			}
		})
	}
}

func TestFilterLZWDecodeParmsRejectUnsupportedKeys(t *testing.T) {
	input := []byte("BT\n(stream) Tj\nET\n")
	for _, decodeParms := range []string{
		"<< /Columns 1 >>",
		"<< /EarlyChange 2 >>",
		"<< /EarlyChange /One >>",
	} {
		t.Run(decodeParms, func(t *testing.T) {
			if _, err := encodeStreamFilterWithDecodeParms("/LZWDecode", decodeParms, input); err == nil {
				t.Fatal("expected LZW DecodeParms error")
			}
		})
	}
}

func TestLZWDecodeRewriteReencodesLengthAndXref(t *testing.T) {
	decoded := []byte("BT\n(08\\05515\\0552024) Tj\nET\n")
	encoded, err := encodeStreamFilter("/LZWDecode", decoded)
	if err != nil {
		t.Fatal(err)
	}
	input := testPDF(
		"<< /Type /Page >>",
		fmt.Sprintf("<< /Length %d /Filter /LZWDecode >>\nstream\n%sendstream", len(encoded), encoded),
	)

	assertLZWRewrite(t, input, encoded, "/LZWDecode", "")
}

func TestASCIIHexLZWDecodeFilterArrayRewriteReencodesLengthAndXref(t *testing.T) {
	decoded := []byte("BT\n(08\\05515\\0552024) Tj\nET\n")
	filter := "[/ASCIIHexDecode /LZWDecode]"
	encoded, err := encodeStreamFilter(filter, decoded)
	if err != nil {
		t.Fatal(err)
	}
	input := testPDF(
		"<< /Type /Page >>",
		fmt.Sprintf("<< /Length %d /Filter %s >>\nstream\n%sendstream", len(encoded), filter, encoded),
	)

	assertLZWRewrite(t, input, encoded, filter, "")
}

func TestLZWDecodeEarlyChangeDecodeParmsRewriteReencodesLengthAndXref(t *testing.T) {
	decoded := []byte("BT\n(08\\05515\\0552024) Tj\nET\n")
	decodeParms := "<< /EarlyChange 0 >>"
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

func TestLZWDecodePNGPredictorDecodeParmsRewriteReencodesPredictorRowsAndXref(t *testing.T) {
	decoded := []byte("BT\n(08\\05515\\0552024) Tj\nET\n")
	decodeParms := "<< /Predictor 12 /Columns 1 /Colors 1 /BitsPerComponent 8 >>"
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

func TestASCIIHexLZWDecodeFilterArrayPNGPredictorDecodeParmsRewriteReencodesPredictorRowsAndXref(t *testing.T) {
	decoded := []byte("BT\n(08\\05515\\0552024) Tj\nET\n")
	filter := "[/ASCIIHexDecode /LZWDecode]"
	decodeParms := "[null << /EarlyChange 0 /Predictor 12 /Columns 1 /Colors 1 /BitsPerComponent 8 >>]"
	encoded, err := encodeStreamFilterWithDecodeParms(filter, decodeParms, decoded)
	if err != nil {
		t.Fatal(err)
	}
	input := testPDF(
		"<< /Type /Page >>",
		fmt.Sprintf("<< /Length %d /Filter %s /DecodeParms %s >>\nstream\n%sendstream", len(encoded), filter, decodeParms, encoded),
	)

	assertLZWRewrite(t, input, encoded, filter, decodeParms)
}

func TestFilterLZWDecodePredictorDecodeParmsRoundTrip(t *testing.T) {
	cases := []struct {
		name        string
		input       []byte
		filter      string
		decodeParms string
	}{
		{
			name:        "png columns",
			input:       []byte("ABCDEFGHIJKLMNOPQRSTUVWX"),
			filter:      "/LZWDecode",
			decodeParms: "<< /Predictor 12 /Columns 4 /Colors 1 /BitsPerComponent 8 >>",
		},
		{
			name:        "png rgb omitted columns",
			input:       []byte("BT\n(ABCDEF) Tj\nET\n"),
			filter:      "/LZWDecode",
			decodeParms: "<< /Predictor 12 /Colors 3 >>",
		},
		{
			name:        "png bpc16 filter array",
			input:       []byte("BT\n(ABCD) Tj\nET\n"),
			filter:      "[/LZWDecode]",
			decodeParms: "[<< /Predictor 12 /Columns 1 /Colors 2 /BitsPerComponent 16 >>]",
		},
		{
			name:        "tiff predictor with early change",
			input:       []byte("ABCDEFGHIJKLMNOPQRSTUVWX"),
			filter:      "/LZWDecode",
			decodeParms: "<< /EarlyChange 0 /Predictor 2 /Columns 4 >>",
		},
		{
			name:        "tiff predictor bit-packed row",
			input:       []byte{0b10110011, 0b01100110, 0b11110001, 0b00001111},
			filter:      "/LZWDecode",
			decodeParms: "<< /Predictor 2 /Columns 7 /BitsPerComponent 1 >>",
		},
	}

	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			encoded, err := encodeStreamFilterWithDecodeParms(tc.filter, tc.decodeParms, tc.input)
			if err != nil {
				t.Fatal(err)
			}
			decoded, err := decodeStreamFilterWithDecodeParms(tc.filter, tc.decodeParms, encoded)
			if err != nil {
				t.Fatal(err)
			}
			if !bytes.Equal(decoded, tc.input) {
				t.Fatalf("decoded stream = %q, want %q", decoded, tc.input)
			}
		})
	}
}

func TestFilterLZWDecodePredictorDecodeParmsFailClosed(t *testing.T) {
	input := []byte("BT\n(stream) Tj\nET\n")
	cases := []struct {
		name        string
		decodeParms string
		want        string
	}{
		{
			name:        "unsupported key",
			decodeParms: "<< /Predictor 12 /Columns 1 /BlackIs1 true >>",
			want:        "unsupported stream: /DecodeParms key /BlackIs1 is not supported",
		},
		{
			name:        "missing predictor for geometry",
			decodeParms: "<< /Columns 1 >>",
			want:        "unsupported stream: /DecodeParms /Predictor is missing",
		},
		{
			name:        "unsupported predictor",
			decodeParms: "<< /Predictor 9 /Columns 1 >>",
			want:        "unsupported stream: /DecodeParms is not implemented",
		},
	}

	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			_, err := encodeStreamFilterWithDecodeParms("/LZWDecode", tc.decodeParms, input)
			if err == nil {
				t.Fatal("expected LZW DecodeParms error")
			}
			if err.Error() != tc.want {
				t.Fatalf("error = %q, want %q", err.Error(), tc.want)
			}
		})
	}
}

func TestFlateDecodePNGPredictorBPC1UnalignedRowRewriteReencodesPredictorRowsAndXref(t *testing.T) {
	decoded := []byte("BT\n(A1) Tj\nET\n")
	const rowBytes = 1
	predicted, err := encodePNGPredictorRowsWithBPPForTest(decoded, rowBytes, rowBytes, bytes.Repeat([]byte{0}, len(decoded)))
	if err != nil {
		t.Fatal(err)
	}
	encoded, err := encodeFlateDecode(predicted)
	if err != nil {
		t.Fatal(err)
	}
	decodeParms := "<< /Predictor 12 /Columns 7 /Colors 1 /BitsPerComponent 1 >>"
	input := testPDF(
		"<< /Type /Page >>",
		fmt.Sprintf("<< /Length %d /Filter /FlateDecode /DecodeParms %s >>\nstream\n%sendstream", len(encoded), decodeParms, encoded),
	)
	adapter := NewAdapter()
	tree, err := adapter.Parse(input, core.ParseOptions{Strict: true})
	if err != nil {
		t.Fatal(err)
	}
	if matches := tree.Query(core.Match{Kind: KindTextShow, Text: "A1"}); len(matches) != 1 {
		t.Fatalf("old selectable matches = %d, want 1", len(matches))
	}
	plan, err := adapter.PlanEdit(tree, core.Match{Kind: KindTextShow, Text: "A1"}, core.Mutation{Replace: "B2"})
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
	updatedPredicted, err := decodeFlateDecode(output[stream.dataStart:stream.dataEnd])
	if err != nil {
		t.Fatal(err)
	}
	if !hasOnlyPNGFilterNoneRowsForColumnsForTest(updatedPredicted, rowBytes) {
		t.Fatalf("encoded predictor rows were not PNG filter 0 BPC1 rows: %v", updatedPredicted)
	}
	updatedDecoded, err := decodePNGPredictorRowsWithBPPForTest(updatedPredicted, rowBytes, rowBytes)
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Contains(updatedDecoded, []byte("(B2) Tj")) {
		t.Fatalf("decoded stream = %q", updatedDecoded)
	}
	if !bytes.Contains(output, []byte("xref\n0 3\n")) {
		t.Fatalf("xref table was not rebuilt:\n%s", output)
	}
}

func assertLZWRewrite(t *testing.T, input, originalEncoded []byte, filter, decodeParms string) {
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
	if stream.lengthIndirect {
		t.Fatal("stream length unexpectedly became indirect")
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
	if !bytes.Contains(output, []byte("xref\n0 3\n")) {
		t.Fatalf("xref table was not rebuilt:\n%s", output)
	}
}
