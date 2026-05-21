package pdf

import (
	"bytes"
	"encoding/ascii85"
	"fmt"
	"strings"
	"testing"

	"github.com/oxhq/binas/pkg/core"
)

func TestASCII85FlateDecodeFilterArrayRewriteReencodesLengthAndXref(t *testing.T) {
	decoded := []byte("BT\n(08\\05515\\0552024) Tj\nET\n")
	encoded, err := encodeASCII85FlateDecodeForTest(decoded)
	if err != nil {
		t.Fatal(err)
	}
	input := testPDF(
		"<< /Type /Page >>",
		fmt.Sprintf("<< /Length %d /Filter [/ASCII85Decode /FlateDecode] >>\nstream\n%sendstream", len(encoded), encoded),
	)
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
	if stream.lengthValue == len(encoded) {
		t.Fatal("encoded stream length was not updated")
	}
	updatedDecoded, err := decodeASCII85FlateDecodeForTest(output[stream.dataStart:stream.dataEnd])
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Contains(updatedDecoded, []byte("(May 5, 2026) Tj")) {
		t.Fatalf("decoded stream = %q", updatedDecoded)
	}
	if !bytes.Contains(output, []byte("xref\n0 3\n")) {
		t.Fatalf("xref table was not rebuilt:\n%s", output)
	}
}

func TestASCII85FlateDecodeFilterArrayIndirectLengthRewriteReencodesReferencedObjectAndXref(t *testing.T) {
	decoded := []byte("BT\n(08\\05515\\0552024) Tj\nET\n")
	encoded, err := encodeASCII85FlateDecodeForTest(decoded)
	if err != nil {
		t.Fatal(err)
	}
	input := testPDF(
		"<< /Type /Page >>",
		fmt.Sprintf("<< /Length 3 0 R /Filter [/ASCII85Decode /FlateDecode] >>\nstream\n%sendstream", encoded),
		fmt.Sprintf("%d", len(encoded)),
	)
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
	stream, ok, err := findNextStream(output, 0)
	if err != nil {
		t.Fatal(err)
	}
	if !ok {
		t.Fatal("missing stream")
	}
	if !stream.lengthIndirect {
		t.Fatal("stream length is no longer indirect")
	}
	if stream.lengthValue == len(encoded) {
		t.Fatal("referenced encoded stream length was not updated")
	}
	updatedDecoded, err := decodeASCII85FlateDecodeForTest(output[stream.dataStart:stream.dataEnd])
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Contains(updatedDecoded, []byte("(May 5, 2026) Tj")) {
		t.Fatalf("decoded stream = %q", updatedDecoded)
	}
	if !bytes.Contains(output, []byte("/Length 3 0 R")) {
		t.Fatalf("stream no longer references indirect length object:\n%s", output)
	}
	wantLengthObject := []byte(fmt.Sprintf("3 0 obj\n%d\nendobj", stream.lengthValue))
	if !bytes.Contains(output, wantLengthObject) {
		t.Fatalf("referenced length object was not updated to %d:\n%s", stream.lengthValue, output)
	}
	if !bytes.Contains(output, []byte("xref\n0 4\n")) {
		t.Fatalf("xref table was not rebuilt:\n%s", output)
	}
	verification, err := adapter.Verify(output, plan)
	if err != nil {
		t.Fatal(err)
	}
	if !verification.ReparseOK || !verification.OldTextRemoved || !verification.NewSelectable || !verification.PageUnchanged {
		t.Fatalf("verification failed: %+v", verification)
	}
}

func TestASCII85DecodeStandaloneRewriteReencodesLengthAndXref(t *testing.T) {
	decoded := []byte("BT\n(08\\05515\\0552024) Tj\nET\n")
	encoded, err := encodeASCII85Decode(decoded)
	if err != nil {
		t.Fatal(err)
	}
	input := testPDF(
		"<< /Type /Page >>",
		fmt.Sprintf("<< /Length %d /Filter /ASCII85Decode >>\nstream\n%sendstream", len(encoded), encoded),
	)
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
	if stream.lengthValue == len(encoded) {
		t.Fatal("encoded stream length was not updated")
	}
	updatedDecoded, err := decodeASCII85Decode(output[stream.dataStart:stream.dataEnd])
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Contains(updatedDecoded, []byte("(May 5, 2026) Tj")) {
		t.Fatalf("decoded stream = %q", updatedDecoded)
	}
	if !bytes.Contains(output, []byte("xref\n0 3\n")) {
		t.Fatalf("xref table was not rebuilt:\n%s", output)
	}
}

func TestASCIIHexFlateDecodeFilterArrayRewriteReencodesLengthAndXref(t *testing.T) {
	decoded := []byte("BT\n(08\\05515\\0552024) Tj\nET\n")
	encoded, err := encodeASCIIHexFlateDecodeForTest(decoded)
	if err != nil {
		t.Fatal(err)
	}
	input := testPDF(
		"<< /Type /Page >>",
		fmt.Sprintf("<< /Length %d /Filter [/ASCIIHexDecode /FlateDecode] >>\nstream\n%sendstream", len(encoded), encoded),
	)
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
	if stream.lengthValue == len(encoded) {
		t.Fatal("encoded stream length was not updated")
	}
	updatedDecoded, err := decodeASCIIHexFlateDecodeForTest(output[stream.dataStart:stream.dataEnd])
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Contains(updatedDecoded, []byte("(May 5, 2026) Tj")) {
		t.Fatalf("decoded stream = %q", updatedDecoded)
	}
	if !bytes.Contains(output, []byte("xref\n0 3\n")) {
		t.Fatalf("xref table was not rebuilt:\n%s", output)
	}
}

func TestASCIIHexFlateDecodeFilterArrayDecodeParmsRewriteReencodesPredictorRowsAndXref(t *testing.T) {
	decoded := []byte("BT\n(08\\05515\\0552024) Tj\nET\n")
	predicted := encodePNGOneByteRowsForTest(decoded)
	encoded, err := encodeASCIIHexFlateDecodeForTest(predicted)
	if err != nil {
		t.Fatal(err)
	}
	decodeParms := "[null << /Predictor 12 /Columns 1 /Colors 1 /BitsPerComponent 8 >>]"
	input := testPDF(
		"<< /Type /Page >>",
		fmt.Sprintf("<< /Length %d /Filter [/ASCIIHexDecode /FlateDecode] /DecodeParms %s >>\nstream\n%sendstream", len(encoded), decodeParms, encoded),
	)
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
	if stream.lengthValue == len(encoded) {
		t.Fatal("predicted encoded stream length was not updated")
	}
	updatedPredicted, err := decodeASCIIHexFlateDecodeForTest(output[stream.dataStart:stream.dataEnd])
	if err != nil {
		t.Fatal(err)
	}
	updatedDecoded, err := decodePNGOneByteRowsForTest(updatedPredicted)
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Contains(updatedDecoded, []byte("(May 5, 2026) Tj")) {
		t.Fatalf("decoded stream = %q", updatedDecoded)
	}
	if !hasOnlyPNGFilterNoneRowsForTest(updatedPredicted) {
		t.Fatalf("encoded predictor rows were not PNG filter 0 one-byte rows: %v", updatedPredicted)
	}
	if !bytes.Contains(output, []byte("xref\n0 3\n")) {
		t.Fatalf("xref table was not rebuilt:\n%s", output)
	}
}

func TestSingleFlateDecodeFilterArrayRewriteReencodesLengthAndXref(t *testing.T) {
	decoded := []byte("BT\n(08\\05515\\0552024) Tj\nET\n")
	encoded, err := encodeFlateDecode(decoded)
	if err != nil {
		t.Fatal(err)
	}
	input := testPDF(
		"<< /Type /Page >>",
		fmt.Sprintf("<< /Length %d /Filter [/FlateDecode] >>\nstream\n%sendstream", len(encoded), encoded),
	)
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
	if stream.lengthValue == len(encoded) {
		t.Fatal("compressed stream length was not updated")
	}
	updatedDecoded, err := decodeFlateDecode(output[stream.dataStart:stream.dataEnd])
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Contains(updatedDecoded, []byte("(May 5, 2026) Tj")) {
		t.Fatalf("decoded stream = %q", updatedDecoded)
	}
	if !bytes.Contains(output, []byte("xref\n0 3\n")) {
		t.Fatalf("xref table was not rebuilt:\n%s", output)
	}
}

func TestRunLengthFlateDecodeFilterArrayRoundTrip(t *testing.T) {
	input := []byte("BT\n(08\\05515\\0552024) Tj\nET\n")
	encoded, err := encodeStreamFilter("[/RunLengthDecode /FlateDecode]", input)
	if err != nil {
		t.Fatal(err)
	}
	runLengthDecoded, err := decodeRunLengthDecode(encoded)
	if err != nil {
		t.Fatal(err)
	}
	if bytes.Contains(runLengthDecoded, input) {
		t.Fatal("RunLength decoded stream was not still Flate-compressed")
	}
	decoded, err := decodeStreamFilter("[/RunLengthDecode /FlateDecode]", encoded)
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Equal(decoded, input) {
		t.Fatalf("decoded stream = %q, want %q", decoded, input)
	}
}

func TestRunLengthFlateDecodeFilterArrayRewriteReencodesLengthAndXref(t *testing.T) {
	decoded := []byte("BT\n(08\\05515\\0552024) Tj\nET\n")
	encoded, err := encodeRunLengthFlateDecodeForTest(decoded)
	if err != nil {
		t.Fatal(err)
	}
	input := testPDF(
		"<< /Type /Page >>",
		fmt.Sprintf("<< /Length %d /Filter [/RunLengthDecode /FlateDecode] >>\nstream\n%sendstream", len(encoded), encoded),
	)
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
	if stream.lengthValue == len(encoded) {
		t.Fatal("encoded stream length was not updated")
	}
	updatedDecoded, err := decodeRunLengthFlateDecodeForTest(output[stream.dataStart:stream.dataEnd])
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Contains(updatedDecoded, []byte("(May 5, 2026) Tj")) {
		t.Fatalf("decoded stream = %q", updatedDecoded)
	}
	if !bytes.Contains(output, []byte("xref\n0 3\n")) {
		t.Fatalf("xref table was not rebuilt:\n%s", output)
	}
}

func TestFlateASCIIHexDecodeFilterArrayRewriteReencodesLengthAndXref(t *testing.T) {
	decoded := []byte("BT\n(08\\05515\\0552024) Tj\nET\n")
	encoded, err := encodeFlateASCIIHexDecodeForTest(decoded)
	if err != nil {
		t.Fatal(err)
	}
	input := testPDF(
		"<< /Type /Page >>",
		fmt.Sprintf("<< /Length %d /Filter [/FlateDecode /ASCIIHexDecode] >>\nstream\n%sendstream", len(encoded), encoded),
	)
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
	if stream.lengthValue == len(encoded) {
		t.Fatal("encoded stream length was not updated")
	}
	if stream.lengthValue != stream.dataEnd-stream.dataStart {
		t.Fatalf("stream length = %d, want %d", stream.lengthValue, stream.dataEnd-stream.dataStart)
	}
	updatedDecoded, err := decodeFlateASCIIHexDecodeForTest(output[stream.dataStart:stream.dataEnd])
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

func TestASCIIArmoredRunLengthFlateDecodeFilterArrayRoundTrip(t *testing.T) {
	input := []byte("BT\n(08\\05515\\0552024) Tj\nET\n")
	cases := []struct {
		name   string
		filter string
	}{
		{
			name:   "ascii85 runlength flate",
			filter: "[/ASCII85Decode /RunLengthDecode /FlateDecode]",
		},
		{
			name:   "asciihex runlength flate",
			filter: "[/ASCIIHexDecode /RunLengthDecode /FlateDecode]",
		},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			encoded, err := encodeStreamFilter(tc.filter, input)
			if err != nil {
				t.Fatal(err)
			}
			decoded, err := decodeStreamFilter(tc.filter, encoded)
			if err != nil {
				t.Fatal(err)
			}
			if !bytes.Equal(decoded, input) {
				t.Fatalf("decoded stream = %q, want %q", decoded, input)
			}
		})
	}
}

func TestASCII85RunLengthFlateDecodeFilterArrayDecodeParmsRewriteReencodesPredictorRowsAndXref(t *testing.T) {
	decoded := []byte("BT\n(08\\05515\\0552024) Tj\nET\n")
	predicted := encodePNGOneByteRowsForTest(decoded)
	filter := "[/ASCII85Decode /RunLengthDecode /FlateDecode]"
	decodeParms := "[null null << /Predictor 12 /Columns 1 /Colors 1 /BitsPerComponent 8 >>]"
	encoded, err := encodeStreamFilterWithDecodeParms(filter, decodeParms, decoded)
	if err != nil {
		t.Fatal(err)
	}
	input := testPDF(
		"<< /Type /Page >>",
		fmt.Sprintf("<< /Length %d /Filter %s /DecodeParms %s >>\nstream\n%sendstream", len(encoded), filter, decodeParms, encoded),
	)
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
	if stream.lengthValue == len(encoded) {
		t.Fatal("predicted encoded stream length was not updated")
	}
	updatedPredicted, err := decodeASCII85RunLengthFlateDecodeForTest(output[stream.dataStart:stream.dataEnd])
	if err != nil {
		t.Fatal(err)
	}
	updatedDecoded, err := decodePNGOneByteRowsForTest(updatedPredicted)
	if err != nil {
		t.Fatal(err)
	}
	if bytes.Equal(updatedPredicted, predicted) {
		t.Fatal("predicted stream was not rewritten")
	}
	if !bytes.Contains(updatedDecoded, []byte("(May 5, 2026) Tj")) {
		t.Fatalf("decoded stream = %q", updatedDecoded)
	}
	if !hasOnlyPNGFilterNoneRowsForTest(updatedPredicted) {
		t.Fatalf("encoded predictor rows were not PNG filter 0 one-byte rows: %v", updatedPredicted)
	}
	if !bytes.Contains(output, []byte("xref\n0 3\n")) {
		t.Fatalf("xref table was not rebuilt:\n%s", output)
	}
}

func TestASCIIHexRunLengthFlateDecodeFilterArrayDecodeParmsRewriteReencodesPredictorRowsAndXref(t *testing.T) {
	decoded := []byte("BT\n(08\\05515\\0552024) Tj\nET\n")
	predicted := encodePNGOneByteRowsForTest(decoded)
	filter := "[/ASCIIHexDecode /RunLengthDecode /FlateDecode]"
	decodeParms := "[null null << /Predictor 12 /Columns 1 /Colors 1 /BitsPerComponent 8 >>]"
	encoded, err := encodeStreamFilterWithDecodeParms(filter, decodeParms, decoded)
	if err != nil {
		t.Fatal(err)
	}
	input := testPDF(
		"<< /Type /Page >>",
		fmt.Sprintf("<< /Length %d /Filter %s /DecodeParms %s >>\nstream\n%sendstream", len(encoded), filter, decodeParms, encoded),
	)
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
	if stream.lengthValue == len(encoded) {
		t.Fatal("predicted encoded stream length was not updated")
	}
	updatedPredicted, err := decodeASCIIHexRunLengthFlateDecodeForTest(output[stream.dataStart:stream.dataEnd])
	if err != nil {
		t.Fatal(err)
	}
	updatedDecoded, err := decodePNGOneByteRowsForTest(updatedPredicted)
	if err != nil {
		t.Fatal(err)
	}
	if bytes.Equal(updatedPredicted, predicted) {
		t.Fatal("predicted stream was not rewritten")
	}
	if !bytes.Contains(updatedDecoded, []byte("(May 5, 2026) Tj")) {
		t.Fatalf("decoded stream = %q", updatedDecoded)
	}
	if !hasOnlyPNGFilterNoneRowsForTest(updatedPredicted) {
		t.Fatalf("encoded predictor rows were not PNG filter 0 one-byte rows: %v", updatedPredicted)
	}
	if !bytes.Contains(output, []byte("xref\n0 3\n")) {
		t.Fatalf("xref table was not rebuilt:\n%s", output)
	}
}

func TestRunLengthFlateDecodeFilterArrayDecodeParmsRewriteReencodesPredictorRowsAndXref(t *testing.T) {
	decoded := []byte("BT\n(08\\05515\\0552024) Tj\nET\n")
	predicted := encodePNGOneByteRowsForTest(decoded)
	encoded, err := encodeRunLengthFlateDecodeForTest(predicted)
	if err != nil {
		t.Fatal(err)
	}
	decodeParms := "[null << /Predictor 12 /Columns 1 /Colors 1 /BitsPerComponent 8 >>]"
	input := testPDF(
		"<< /Type /Page >>",
		fmt.Sprintf("<< /Length %d /Filter [/RunLengthDecode /FlateDecode] /DecodeParms %s >>\nstream\n%sendstream", len(encoded), decodeParms, encoded),
	)
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
	if stream.lengthValue == len(encoded) {
		t.Fatal("predicted encoded stream length was not updated")
	}
	updatedPredicted, err := decodeRunLengthFlateDecodeForTest(output[stream.dataStart:stream.dataEnd])
	if err != nil {
		t.Fatal(err)
	}
	updatedDecoded, err := decodePNGOneByteRowsForTest(updatedPredicted)
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Contains(updatedDecoded, []byte("(May 5, 2026) Tj")) {
		t.Fatalf("decoded stream = %q", updatedDecoded)
	}
	if !hasOnlyPNGFilterNoneRowsForTest(updatedPredicted) {
		t.Fatalf("encoded predictor rows were not PNG filter 0 one-byte rows: %v", updatedPredicted)
	}
	if !bytes.Contains(output, []byte("xref\n0 3\n")) {
		t.Fatalf("xref table was not rebuilt:\n%s", output)
	}
}

func TestSingleFlateDecodeFilterArrayIndirectLengthRewriteReencodesReferencedObjectAndXref(t *testing.T) {
	decoded := []byte("BT\n(08\\05515\\0552024) Tj\nET\n")
	encoded, err := encodeFlateDecode(decoded)
	if err != nil {
		t.Fatal(err)
	}
	input := testPDF(
		"<< /Type /Page >>",
		fmt.Sprintf("<< /Length 3 0 R /Filter [/FlateDecode] >>\nstream\n%sendstream", encoded),
		fmt.Sprintf("%d", len(encoded)),
	)
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
	stream, ok, err := findNextStream(output, 0)
	if err != nil {
		t.Fatal(err)
	}
	if !ok {
		t.Fatal("missing stream")
	}
	if !stream.lengthIndirect {
		t.Fatal("stream length is no longer indirect")
	}
	if stream.lengthValue == len(encoded) {
		t.Fatal("referenced compressed stream length was not updated")
	}
	updatedDecoded, err := decodeFlateDecode(output[stream.dataStart:stream.dataEnd])
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Contains(updatedDecoded, []byte("(May 5, 2026) Tj")) {
		t.Fatalf("decoded stream = %q", updatedDecoded)
	}
	if !bytes.Contains(output, []byte("/Length 3 0 R")) {
		t.Fatalf("stream no longer references indirect length object:\n%s", output)
	}
	wantLengthObject := []byte(fmt.Sprintf("3 0 obj\n%d\nendobj", stream.lengthValue))
	if !bytes.Contains(output, wantLengthObject) {
		t.Fatalf("referenced length object was not updated to %d:\n%s", stream.lengthValue, output)
	}
	if !bytes.Contains(output, []byte("xref\n0 4\n")) {
		t.Fatalf("xref table was not rebuilt:\n%s", output)
	}
	verification, err := adapter.Verify(output, plan)
	if err != nil {
		t.Fatal(err)
	}
	if !verification.ReparseOK || !verification.OldTextRemoved || !verification.NewSelectable || !verification.PageUnchanged {
		t.Fatalf("verification failed: %+v", verification)
	}
}

func TestFlateDecodePNGDecodeParmsDirectLengthRewriteReencodesPredictorRowsAndXref(t *testing.T) {
	decoded := []byte("BT\n(08\\05515\\0552024) Tj\nET\n")
	predicted := encodePNGOneByteRowsForTest(decoded)
	encoded, err := encodeFlateDecode(predicted)
	if err != nil {
		t.Fatal(err)
	}
	decodeParms := "<< /Predictor 12 /Columns 1 /Colors 1 /BitsPerComponent 8 >>"
	input := testPDF(
		"<< /Type /Page >>",
		fmt.Sprintf("<< /Length %d /Filter /FlateDecode /DecodeParms %s >>\nstream\n%sendstream", len(encoded), decodeParms, encoded),
	)
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
	if stream.lengthValue == len(encoded) {
		t.Fatal("predicted compressed stream length was not updated")
	}
	updatedPredicted, err := decodeFlateDecode(output[stream.dataStart:stream.dataEnd])
	if err != nil {
		t.Fatal(err)
	}
	updatedDecoded, err := decodePNGOneByteRowsForTest(updatedPredicted)
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Contains(updatedDecoded, []byte("(May 5, 2026) Tj")) {
		t.Fatalf("decoded stream = %q", updatedDecoded)
	}
	if !hasOnlyPNGFilterNoneRowsForTest(updatedPredicted) {
		t.Fatalf("encoded predictor rows were not PNG filter 0 one-byte rows: %v", updatedPredicted)
	}
	if !bytes.Contains(output, []byte("xref\n0 3\n")) {
		t.Fatalf("xref table was not rebuilt:\n%s", output)
	}
}

func TestFlateDecodePNGDecodeParmsOmittedColumnsRewriteReencodesPredictorRowsAndXref(t *testing.T) {
	decoded := []byte("BT\n(08\\05515\\0552024) Tj\nET\n")
	predicted := encodePNGOneByteRowsForTest(decoded)
	encoded, err := encodeFlateDecode(predicted)
	if err != nil {
		t.Fatal(err)
	}
	decodeParms := "<< /Predictor 12 /Colors 1 /BitsPerComponent 8 >>"
	input := testPDF(
		"<< /Type /Page >>",
		fmt.Sprintf("<< /Length %d /Filter /FlateDecode /DecodeParms %s >>\nstream\n%sendstream", len(encoded), decodeParms, encoded),
	)
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
	if stream.lengthValue == len(encoded) {
		t.Fatal("predicted compressed stream length was not updated")
	}
	updatedPredicted, err := decodeFlateDecode(output[stream.dataStart:stream.dataEnd])
	if err != nil {
		t.Fatal(err)
	}
	updatedDecoded, err := decodePNGOneByteRowsForTest(updatedPredicted)
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Contains(updatedDecoded, []byte("(May 5, 2026) Tj")) {
		t.Fatalf("decoded stream = %q", updatedDecoded)
	}
	if !hasOnlyPNGFilterNoneRowsForTest(updatedPredicted) {
		t.Fatalf("encoded predictor rows were not PNG filter 0 one-byte rows: %v", updatedPredicted)
	}
	if !bytes.Contains(output, []byte("xref\n0 3\n")) {
		t.Fatalf("xref table was not rebuilt:\n%s", output)
	}
}

func TestFlateDecodePNGDecodeParmsIndirectLengthRewriteReencodesReferencedObjectAndXref(t *testing.T) {
	decoded := []byte("BT\n(08\\05515\\0552024) Tj\nET\n")
	predicted := encodePNGOneByteRowsForTest(decoded)
	encoded, err := encodeFlateDecode(predicted)
	if err != nil {
		t.Fatal(err)
	}
	decodeParms := "<< /Predictor 12 /Columns 1 /Colors 1 /BitsPerComponent 8 >>"
	input := testPDF(
		"<< /Type /Page >>",
		fmt.Sprintf("<< /Length 3 0 R /Filter /FlateDecode /DecodeParms %s >>\nstream\n%sendstream", decodeParms, encoded),
		fmt.Sprintf("%d", len(encoded)),
	)
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
	stream, ok, err := findNextStream(output, 0)
	if err != nil {
		t.Fatal(err)
	}
	if !ok {
		t.Fatal("missing stream")
	}
	if !stream.lengthIndirect {
		t.Fatal("stream length is no longer indirect")
	}
	if stream.lengthValue == len(encoded) {
		t.Fatal("referenced predicted compressed stream length was not updated")
	}
	updatedPredicted, err := decodeFlateDecode(output[stream.dataStart:stream.dataEnd])
	if err != nil {
		t.Fatal(err)
	}
	updatedDecoded, err := decodePNGOneByteRowsForTest(updatedPredicted)
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Contains(updatedDecoded, []byte("(May 5, 2026) Tj")) {
		t.Fatalf("decoded stream = %q", updatedDecoded)
	}
	if !hasOnlyPNGFilterNoneRowsForTest(updatedPredicted) {
		t.Fatalf("encoded predictor rows were not PNG filter 0 one-byte rows: %v", updatedPredicted)
	}
	if !bytes.Contains(output, []byte("/Length 3 0 R")) {
		t.Fatalf("stream no longer references indirect length object:\n%s", output)
	}
	wantLengthObject := []byte(fmt.Sprintf("3 0 obj\n%d\nendobj", stream.lengthValue))
	if !bytes.Contains(output, wantLengthObject) {
		t.Fatalf("referenced length object was not updated to %d:\n%s", stream.lengthValue, output)
	}
	if !bytes.Contains(output, []byte("xref\n0 4\n")) {
		t.Fatalf("xref table was not rebuilt:\n%s", output)
	}
	verification, err := adapter.Verify(output, plan)
	if err != nil {
		t.Fatal(err)
	}
	if !verification.ReparseOK || !verification.OldTextRemoved || !verification.NewSelectable || !verification.PageUnchanged {
		t.Fatalf("verification failed: %+v", verification)
	}
}

func TestFlateDecodePNGDecodeParmsColumns4DirectLengthRewriteReencodesPredictorRowsAndXref(t *testing.T) {
	decoded := []byte("BT\n(08\\05515\\0552024) Tj\nET\n")
	const columns = 4
	predicted, err := encodePNGPredictorRowsForTest(decoded, columns, []byte{0, 1, 2, 3, 4, 1, 0})
	if err != nil {
		t.Fatal(err)
	}
	encoded, err := encodeFlateDecode(predicted)
	if err != nil {
		t.Fatal(err)
	}
	decodeParms := "<< /Predictor 12 /Columns 4 /Colors 1 /BitsPerComponent 8 >>"
	input := testPDF(
		"<< /Type /Page >>",
		fmt.Sprintf("<< /Length %d /Filter /FlateDecode /DecodeParms %s >>\nstream\n%sendstream", len(encoded), decodeParms, encoded),
	)
	adapter := NewAdapter()
	tree, err := adapter.Parse(input, core.ParseOptions{Strict: true})
	if err != nil {
		t.Fatal(err)
	}
	if matches := tree.Query(core.Match{Kind: KindTextShow, Text: "08-15-2024"}); len(matches) != 1 {
		t.Fatalf("old selectable matches = %d, want 1", len(matches))
	}
	plan, err := adapter.PlanEdit(tree, core.Match{Kind: KindTextShow, Text: "08-15-2024"}, core.Mutation{Replace: "May 05, 2026"})
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
	if stream.lengthValue == len(encoded) {
		t.Fatal("predicted compressed stream length was not updated")
	}
	updatedPredicted, err := decodeFlateDecode(output[stream.dataStart:stream.dataEnd])
	if err != nil {
		t.Fatal(err)
	}
	updatedDecoded, err := decodePNGPredictorRowsForTest(updatedPredicted, columns)
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Contains(updatedDecoded, []byte("(May 05, 2026) Tj")) {
		t.Fatalf("decoded stream = %q", updatedDecoded)
	}
	if !hasOnlyPNGFilterNoneRowsForColumnsForTest(updatedPredicted, columns) {
		t.Fatalf("encoded predictor rows were not PNG filter 0 columns=%d rows: %v", columns, updatedPredicted)
	}
	if !bytes.Contains(output, []byte("xref\n0 3\n")) {
		t.Fatalf("xref table was not rebuilt:\n%s", output)
	}
}

func TestFlateDecodePNGDecodeParmsColumns4IndirectLengthRewriteReencodesReferencedObjectAndXref(t *testing.T) {
	decoded := []byte("BT\n(08\\05515\\0552024) Tj\nET\n")
	const columns = 4
	predicted, err := encodePNGPredictorRowsForTest(decoded, columns, []byte{0, 1, 2, 3, 4, 1, 0})
	if err != nil {
		t.Fatal(err)
	}
	encoded, err := encodeFlateDecode(predicted)
	if err != nil {
		t.Fatal(err)
	}
	decodeParms := "<< /Predictor 12 /Columns 4 /Colors 1 /BitsPerComponent 8 >>"
	input := testPDF(
		"<< /Type /Page >>",
		fmt.Sprintf("<< /Length 3 0 R /Filter /FlateDecode /DecodeParms %s >>\nstream\n%sendstream", decodeParms, encoded),
		fmt.Sprintf("%d", len(encoded)),
	)
	adapter := NewAdapter()
	tree, err := adapter.Parse(input, core.ParseOptions{Strict: true})
	if err != nil {
		t.Fatal(err)
	}
	if matches := tree.Query(core.Match{Kind: KindTextShow, Text: "08-15-2024"}); len(matches) != 1 {
		t.Fatalf("old selectable matches = %d, want 1", len(matches))
	}
	plan, err := adapter.PlanEdit(tree, core.Match{Kind: KindTextShow, Text: "08-15-2024"}, core.Mutation{Replace: "May 05, 2026"})
	if err != nil {
		t.Fatal(err)
	}
	output, _, err := adapter.Apply(input, plan)
	if err != nil {
		t.Fatal(err)
	}
	stream, ok, err := findNextStream(output, 0)
	if err != nil {
		t.Fatal(err)
	}
	if !ok {
		t.Fatal("missing stream")
	}
	if !stream.lengthIndirect {
		t.Fatal("stream length is no longer indirect")
	}
	if stream.lengthValue == len(encoded) {
		t.Fatal("referenced predicted compressed stream length was not updated")
	}
	updatedPredicted, err := decodeFlateDecode(output[stream.dataStart:stream.dataEnd])
	if err != nil {
		t.Fatal(err)
	}
	updatedDecoded, err := decodePNGPredictorRowsForTest(updatedPredicted, columns)
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Contains(updatedDecoded, []byte("(May 05, 2026) Tj")) {
		t.Fatalf("decoded stream = %q", updatedDecoded)
	}
	if !hasOnlyPNGFilterNoneRowsForColumnsForTest(updatedPredicted, columns) {
		t.Fatalf("encoded predictor rows were not PNG filter 0 columns=%d rows: %v", columns, updatedPredicted)
	}
	if !bytes.Contains(output, []byte("/Length 3 0 R")) {
		t.Fatalf("stream no longer references indirect length object:\n%s", output)
	}
	wantLengthObject := []byte(fmt.Sprintf("3 0 obj\n%d\nendobj", stream.lengthValue))
	if !bytes.Contains(output, wantLengthObject) {
		t.Fatalf("referenced length object was not updated to %d:\n%s", stream.lengthValue, output)
	}
	if !bytes.Contains(output, []byte("xref\n0 4\n")) {
		t.Fatalf("xref table was not rebuilt:\n%s", output)
	}
	verification, err := adapter.Verify(output, plan)
	if err != nil {
		t.Fatal(err)
	}
	if !verification.ReparseOK || !verification.OldTextRemoved || !verification.NewSelectable || !verification.PageUnchanged {
		t.Fatalf("verification failed: %+v", verification)
	}
}

func TestFlateDecodePNGDecodeParmsColumns4RewriteFailsWhenDecodedLengthIsPartialRow(t *testing.T) {
	decoded := []byte("BT\n(08\\05515\\0552024) Tj\nET\n")
	const columns = 4
	predicted, err := encodePNGPredictorRowsForTest(decoded, columns, []byte{0, 1, 2, 3, 4, 1, 0})
	if err != nil {
		t.Fatal(err)
	}
	encoded, err := encodeFlateDecode(predicted)
	if err != nil {
		t.Fatal(err)
	}
	decodeParms := "<< /Predictor 12 /Columns 4 /Colors 1 /BitsPerComponent 8 >>"
	input := testPDF(
		"<< /Type /Page >>",
		fmt.Sprintf("<< /Length %d /Filter /FlateDecode /DecodeParms %s >>\nstream\n%sendstream", len(encoded), decodeParms, encoded),
	)
	tree, err := NewAdapter().Parse(input, core.ParseOptions{Strict: true})
	if err != nil {
		t.Fatal(err)
	}
	_, err = NewAdapter().PlanEdit(tree, core.Match{Kind: KindTextShow, Text: "08-15-2024"}, core.Mutation{Replace: "May 5, 2026"})
	if err == nil {
		t.Fatal("expected edit planning to fail for partial predictor row")
	}
	if !strings.Contains(err.Error(), "partial row") {
		t.Fatalf("edit planning error = %v, want partial row", err)
	}
}

func TestFlateDecodePNGDecodeParmsRGBRewriteReencodesPredictorRowsAndXref(t *testing.T) {
	decoded := []byte("BT\n(ABCDEF) Tj\nET\n")
	const (
		columns = 1
		colors  = 3
	)
	predicted, err := encodePNGPredictorRowsWithBPPForTest(decoded, columns*colors, colors, []byte{0, 1, 2, 3, 4, 1})
	if err != nil {
		t.Fatal(err)
	}
	encoded, err := encodeFlateDecode(predicted)
	if err != nil {
		t.Fatal(err)
	}
	decodeParms := "<< /Predictor 12 /Columns 1 /Colors 3 /BitsPerComponent 8 >>"
	input := testPDF(
		"<< /Type /Page >>",
		fmt.Sprintf("<< /Length %d /Filter /FlateDecode /DecodeParms %s >>\nstream\n%sendstream", len(encoded), decodeParms, encoded),
	)
	adapter := NewAdapter()
	tree, err := adapter.Parse(input, core.ParseOptions{Strict: true})
	if err != nil {
		t.Fatal(err)
	}
	if matches := tree.Query(core.Match{Kind: KindTextShow, Text: "ABCDEF"}); len(matches) != 1 {
		t.Fatalf("old selectable matches = %d, want 1", len(matches))
	}
	plan, err := adapter.PlanEdit(tree, core.Match{Kind: KindTextShow, Text: "ABCDEF"}, core.Mutation{Replace: "UVWXYZ123"})
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
	if stream.lengthValue == len(encoded) {
		t.Fatal("predicted compressed stream length was not updated")
	}
	updatedPredicted, err := decodeFlateDecode(output[stream.dataStart:stream.dataEnd])
	if err != nil {
		t.Fatal(err)
	}
	updatedDecoded, err := decodePNGPredictorRowsWithBPPForTest(updatedPredicted, columns*colors, colors)
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Contains(updatedDecoded, []byte("(UVWXYZ123) Tj")) {
		t.Fatalf("decoded stream = %q", updatedDecoded)
	}
	if !hasOnlyPNGFilterNoneRowsForColumnsForTest(updatedPredicted, columns*colors) {
		t.Fatalf("encoded predictor rows were not PNG filter 0 RGB rows: %v", updatedPredicted)
	}
	if !bytes.Contains(output, []byte("xref\n0 3\n")) {
		t.Fatalf("xref table was not rebuilt:\n%s", output)
	}
}

func TestFlateDecodePNGDecodeParmsOmittedColumnsRGBRewriteReencodesPredictorRowsAndXref(t *testing.T) {
	decoded := []byte("BT\n(ABCDEF) Tj\nET\n")
	const (
		columns  = 1
		colors   = 3
		rowBytes = columns * colors
	)
	cases := []struct {
		name        string
		filter      string
		decodeParms string
	}{
		{
			name:        "direct dictionary",
			filter:      "/FlateDecode",
			decodeParms: "<< /Predictor 12 /Colors 3 >>",
		},
		{
			name:        "single-item filter array",
			filter:      "[/FlateDecode]",
			decodeParms: "[<< /Predictor 12 /Colors 3 >>]",
		},
	}

	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			predicted, err := encodePNGPredictorRowsWithBPPForTest(decoded, rowBytes, colors, []byte{0, 1, 2, 3, 4, 1})
			if err != nil {
				t.Fatal(err)
			}
			encoded, err := encodeFlateDecode(predicted)
			if err != nil {
				t.Fatal(err)
			}
			input := testPDF(
				"<< /Type /Page >>",
				fmt.Sprintf("<< /Length %d /Filter %s /DecodeParms %s >>\nstream\n%sendstream", len(encoded), tc.filter, tc.decodeParms, encoded),
			)
			adapter := NewAdapter()
			tree, err := adapter.Parse(input, core.ParseOptions{Strict: true})
			if err != nil {
				t.Fatal(err)
			}
			if matches := tree.Query(core.Match{Kind: KindTextShow, Text: "ABCDEF"}); len(matches) != 1 {
				t.Fatalf("old selectable matches = %d, want 1", len(matches))
			}
			plan, err := adapter.PlanEdit(tree, core.Match{Kind: KindTextShow, Text: "ABCDEF"}, core.Mutation{Replace: "UVWXYZ123"})
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
			if stream.lengthValue == len(encoded) {
				t.Fatal("predicted compressed stream length was not updated")
			}
			if stream.lengthValue != stream.dataEnd-stream.dataStart {
				t.Fatalf("stream length = %d, want %d", stream.lengthValue, stream.dataEnd-stream.dataStart)
			}
			updatedPredicted, err := decodeFlateDecode(output[stream.dataStart:stream.dataEnd])
			if err != nil {
				t.Fatal(err)
			}
			if bytes.Equal(updatedPredicted, predicted) {
				t.Fatal("predicted rows were not rewritten")
			}
			updatedDecoded, err := decodePNGPredictorRowsWithBPPForTest(updatedPredicted, rowBytes, colors)
			if err != nil {
				t.Fatal(err)
			}
			if bytes.Contains(updatedDecoded, []byte("ABCDEF")) {
				t.Fatalf("decoded stream still contains old text: %q", updatedDecoded)
			}
			if !bytes.Contains(updatedDecoded, []byte("(UVWXYZ123) Tj")) {
				t.Fatalf("decoded stream = %q", updatedDecoded)
			}
			if !hasOnlyPNGFilterNoneRowsForColumnsForTest(updatedPredicted, rowBytes) {
				t.Fatalf("encoded predictor rows were not PNG filter 0 omitted-Columns RGB rows: %v", updatedPredicted)
			}
			if !bytes.Contains(output, []byte("xref\n0 3\n")) {
				t.Fatalf("xref table was not rebuilt:\n%s", output)
			}
		})
	}
}

func TestFilterArrayDecodeParmsSupportedShapesSurgicalAndCanonicalRewrite(t *testing.T) {
	cases := []struct {
		name          string
		filter        string
		decodeParms   string
		oldText       string
		newText       string
		decoded       []byte
		assertEncoded func(t *testing.T, encoded []byte, newText string)
	}{
		{
			name:        "direct flate decode parms predictor one",
			filter:      "/FlateDecode",
			decodeParms: "<< /Predictor 1 /Columns 4 /Colors 3 /BitsPerComponent 16 >>",
			oldText:     "08-15-2024",
			newText:     "May 5, 2026",
			decoded:     []byte("BT\n(08\\05515\\0552024) Tj\nET\n"),
			assertEncoded: func(t *testing.T, encoded []byte, newText string) {
				t.Helper()
				decoded, err := decodeStreamFilterWithDecodeParms("/FlateDecode", "<< /Predictor 1 /Columns 4 /Colors 3 /BitsPerComponent 16 >>", encoded)
				if err != nil {
					t.Fatal(err)
				}
				if !bytes.Contains(decoded, []byte("("+newText+") Tj")) {
					t.Fatalf("decoded stream = %q", decoded)
				}
			},
		},
		{
			name:        "filter array decode parms null alignment",
			filter:      "[/ASCIIHexDecode /FlateDecode]",
			decodeParms: "[null << /Predictor 1 /Columns 4 /Colors 3 /BitsPerComponent 16 >>]",
			oldText:     "08-15-2024",
			newText:     "May 5, 2026",
			decoded:     []byte("BT\n(08\\05515\\0552024) Tj\nET\n"),
			assertEncoded: func(t *testing.T, encoded []byte, newText string) {
				t.Helper()
				decoded, err := decodeStreamFilterWithDecodeParms("[/ASCIIHexDecode /FlateDecode]", "[null << /Predictor 1 /Columns 4 /Colors 3 /BitsPerComponent 16 >>]", encoded)
				if err != nil {
					t.Fatal(err)
				}
				if !bytes.Contains(decoded, []byte("("+newText+") Tj")) {
					t.Fatalf("decoded stream = %q", decoded)
				}
			},
		},
		{
			name:        "lzw decode parms predictor one",
			filter:      "/LZWDecode",
			decodeParms: "<< /EarlyChange 0 /Predictor 1 /Columns 4 /Colors 3 /BitsPerComponent 16 >>",
			oldText:     "08-15-2024",
			newText:     "May 5, 2026",
			decoded:     []byte("BT\n(08\\05515\\0552024) Tj\nET\n"),
			assertEncoded: func(t *testing.T, encoded []byte, newText string) {
				t.Helper()
				decoded, err := decodeStreamFilterWithDecodeParms("/LZWDecode", "<< /EarlyChange 0 /Predictor 1 /Columns 4 /Colors 3 /BitsPerComponent 16 >>", encoded)
				if err != nil {
					t.Fatal(err)
				}
				if !bytes.Contains(decoded, []byte("("+newText+") Tj")) {
					t.Fatalf("decoded stream = %q", decoded)
				}
			},
		},
		{
			name:        "single flate array with tiff predictor defaults",
			filter:      "[/FlateDecode]",
			decodeParms: "[<< /Predictor 2 >>]",
			oldText:     "08-15-2024",
			newText:     "May 5, 2026",
			decoded:     []byte("BT\n(08\\05515\\0552024) Tj\nET\n"),
			assertEncoded: func(t *testing.T, encoded []byte, newText string) {
				t.Helper()
				predicted, err := decodeFlateDecode(encoded)
				if err != nil {
					t.Fatal(err)
				}
				decoded, err := decodeTIFFPredictorRowsForTest(predicted, 1, 1)
				if err != nil {
					t.Fatal(err)
				}
				if !bytes.Contains(decoded, []byte("("+newText+") Tj")) {
					t.Fatalf("decoded stream = %q", decoded)
				}
			},
		},
		{
			name:        "ascii85 runlength flate array with png predictor",
			filter:      "[/ASCII85Decode /RunLengthDecode /FlateDecode]",
			decodeParms: "[null null << /Predictor 12 /Columns 1 /Colors 1 /BitsPerComponent 8 >>]",
			oldText:     "08-15-2024",
			newText:     "May 5, 2026",
			decoded:     []byte("BT\n(08\\05515\\0552024) Tj\nET\n"),
			assertEncoded: func(t *testing.T, encoded []byte, newText string) {
				t.Helper()
				ascii85Decoded, err := decodeASCII85Decode(encoded)
				if err != nil {
					t.Fatal(err)
				}
				runLengthDecoded, err := decodeRunLengthDecode(ascii85Decoded)
				if err != nil {
					t.Fatal(err)
				}
				predicted, err := decodeFlateDecode(runLengthDecoded)
				if err != nil {
					t.Fatal(err)
				}
				decoded, err := decodePNGOneByteRowsForTest(predicted)
				if err != nil {
					t.Fatal(err)
				}
				if !bytes.Contains(decoded, []byte("("+newText+") Tj")) {
					t.Fatalf("decoded stream = %q", decoded)
				}
				if !hasOnlyPNGFilterNoneRowsForTest(predicted) {
					t.Fatalf("encoded predictor rows were not PNG filter 0 one-byte rows: %v", predicted)
				}
			},
		},
		{
			name:        "filter array all null decode parms",
			filter:      "[/ASCIIHexDecode /FlateDecode]",
			decodeParms: "[null null]",
			oldText:     "08-15-2024",
			newText:     "May 5, 2026",
			decoded:     []byte("BT\n(08\\05515\\0552024) Tj\nET\n"),
			assertEncoded: func(t *testing.T, encoded []byte, newText string) {
				t.Helper()
				decoded, err := decodeStreamFilterWithDecodeParms("[/ASCIIHexDecode /FlateDecode]", "[null null]", encoded)
				if err != nil {
					t.Fatal(err)
				}
				if !bytes.Contains(decoded, []byte("("+newText+") Tj")) {
					t.Fatalf("decoded stream = %q", decoded)
				}
			},
		},
	}

	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			encoded, err := encodeStreamFilterWithDecodeParms(tc.filter, tc.decodeParms, tc.decoded)
			if err != nil {
				t.Fatal(err)
			}
			input := testPDF(
				"<< /Type /Page >>",
				fmt.Sprintf("<< /Length %d /Filter %s /DecodeParms %s >>\nstream\n%sendstream", len(encoded), tc.filter, tc.decodeParms, encoded),
			)

			adapter := NewAdapter()
			tree, err := adapter.Parse(input, core.ParseOptions{Strict: true})
			if err != nil {
				t.Fatal(err)
			}
			if matches := tree.Query(core.Match{Kind: KindTextShow, Text: tc.oldText}); len(matches) != 1 {
				t.Fatalf("old selectable matches = %d, want 1", len(matches))
			}
			streams := tree.Query(core.Match{Kind: KindStream})
			if len(streams) != 1 {
				t.Fatalf("stream nodes = %d, want 1", len(streams))
			}
			if streams[0].Meta["decode_parms"] != tc.decodeParms {
				t.Fatalf("surgical decode_parms = %q, want %q", streams[0].Meta["decode_parms"], tc.decodeParms)
			}

			plan, err := adapter.PlanEdit(tree, core.Match{Kind: KindTextShow, Text: tc.oldText}, core.Mutation{Replace: tc.newText})
			if err != nil {
				t.Fatal(err)
			}
			output, report, err := adapter.Apply(input, plan)
			if err != nil {
				t.Fatal(err)
			}
			if report.FallbackUsed {
				t.Fatal("unexpected fallback")
			}
			verification, err := adapter.Verify(output, plan)
			if err != nil {
				t.Fatal(err)
			}
			if !verification.ReparseOK || !verification.OldTextRemoved || !verification.NewSelectable || !verification.PageUnchanged {
				t.Fatalf("verification failed: %+v", verification)
			}
			assertDecodeParmsEditReparseLengthAndXref(t, adapter, output, tc.oldText, tc.newText)
			stream, ok, err := findNextStream(output, 0)
			if err != nil {
				t.Fatal(err)
			}
			if !ok {
				t.Fatal("missing stream")
			}
			if stream.lengthValue == len(encoded) {
				t.Fatal("surgical encoded stream length was not updated")
			}
			tc.assertEncoded(t, output[stream.dataStart:stream.dataEnd], tc.newText)

			canonical, _, canonicalVerification, err := ApplyCanonicalEdit(input, core.Match{Kind: KindTextShow, Text: tc.oldText}, core.Mutation{Replace: tc.newText}, nil)
			if err != nil {
				t.Fatal(err)
			}
			if !canonicalVerification.ReparseOK || !canonicalVerification.OldTextRemoved || !canonicalVerification.NewSelectable || !canonicalVerification.PageUnchanged {
				t.Fatalf("canonical verification failed: %+v", canonicalVerification)
			}
			assertDecodeParmsEditReparseLengthAndXref(t, adapter, canonical, tc.oldText, tc.newText)
			if bytes.Contains(canonical, []byte("/Filter")) || bytes.Contains(canonical, []byte("/DecodeParms")) {
				t.Fatalf("canonical edited stream should be written decoded without filters:\n%s", canonical)
			}
			if !bytes.Contains(canonical, []byte("("+tc.newText+") Tj")) {
				t.Fatalf("canonical output missing replacement:\n%s", canonical)
			}
		})
	}
}

func assertDecodeParmsEditReparseLengthAndXref(t *testing.T, adapter Adapter, output []byte, oldText, newText string) {
	t.Helper()

	reparsed, err := adapter.Parse(output, core.ParseOptions{Strict: true})
	if err != nil {
		t.Fatal(err)
	}
	if oldMatches := reparsed.Query(core.Match{Kind: KindTextShow, Text: oldText}); len(oldMatches) != 0 {
		t.Fatalf("old selectable matches after reparse = %d, want 0", len(oldMatches))
	}
	if newMatches := reparsed.Query(core.Match{Kind: KindTextShow, Text: newText}); len(newMatches) != 1 {
		t.Fatalf("new selectable matches after reparse = %d, want 1", len(newMatches))
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
	if !bytes.Contains(output, []byte("xref\n0 3\n")) {
		t.Fatalf("xref table was not rebuilt:\n%s", output)
	}
}

func TestCanonicalFilterArrayDecodeParmsMalformedFailsClosed(t *testing.T) {
	encoded, err := encodeFlateDecode([]byte("BT\n(08\\05515\\0552024) Tj\nET\n"))
	if err != nil {
		t.Fatal(err)
	}
	input := testPDF(
		"<< /Type /Page >>",
		fmt.Sprintf("<< /Length %d /Filter [/ASCII85Decode /FlateDecode] /DecodeParms [<< /Predictor 12 >> null] >>\nstream\n%sendstream", len(encoded), encoded),
	)

	tree, err := NewAdapter().Parse(input, core.ParseOptions{Strict: true})
	if err != nil {
		t.Fatal(err)
	}
	streams := tree.Query(core.Match{Kind: KindStream, Meta: map[string]any{
		"unsupported": "unsupported stream: /DecodeParms for /ASCII85Decode must be null",
	}})
	if len(streams) != 1 {
		t.Fatalf("unsupported stream matches = %d, want 1", len(streams))
	}

	_, _, _, err = EditCanonical(input, core.Match{Kind: KindTextShow, Text: "08-15-2024"}, core.Mutation{Replace: "May 5, 2026"}, nil)
	if err == nil {
		t.Fatal("expected canonical edit to fail closed")
	}
	if !strings.Contains(err.Error(), "unsupported stream: /DecodeParms for /ASCII85Decode must be null") &&
		!strings.Contains(err.Error(), `no nodes match kind="pdf.content.text_show" text="08-15-2024"`) {
		t.Fatalf("canonical edit error = %v, want DecodeParms refusal or no editable node", err)
	}
}

func TestFlateDecodePNGDecodeParmsRGBRewriteFailsWhenDecodedLengthIsPartialRow(t *testing.T) {
	decoded := []byte("BT\n(ABCDEF) Tj\nET\n")
	const (
		columns = 1
		colors  = 3
	)
	predicted, err := encodePNGPredictorRowsWithBPPForTest(decoded, columns*colors, colors, []byte{0, 1, 2, 3, 4, 1})
	if err != nil {
		t.Fatal(err)
	}
	encoded, err := encodeFlateDecode(predicted)
	if err != nil {
		t.Fatal(err)
	}
	decodeParms := "<< /Predictor 12 /Columns 1 /Colors 3 /BitsPerComponent 8 >>"
	input := testPDF(
		"<< /Type /Page >>",
		fmt.Sprintf("<< /Length %d /Filter /FlateDecode /DecodeParms %s >>\nstream\n%sendstream", len(encoded), decodeParms, encoded),
	)
	tree, err := NewAdapter().Parse(input, core.ParseOptions{Strict: true})
	if err != nil {
		t.Fatal(err)
	}
	_, err = NewAdapter().PlanEdit(tree, core.Match{Kind: KindTextShow, Text: "ABCDEF"}, core.Mutation{Replace: "ABCDE"})
	if err == nil {
		t.Fatal("expected edit planning to fail for partial predictor row")
	}
	if !strings.Contains(err.Error(), "partial row") {
		t.Fatalf("edit planning error = %v, want partial row", err)
	}
}

func TestFlateDecodePNGDecodeParmsBitsPerComponent16RewriteReencodesPredictorRowsAndXref(t *testing.T) {
	decoded := []byte("BT\n(ABCD) Tj\nET\n")
	const (
		columns          = 1
		colors           = 2
		bitsPerComponent = 16
		rowBytes         = columns * colors * bitsPerComponent / 8
		bytesPerPixel    = (colors*bitsPerComponent + 7) / 8
	)
	predicted, err := encodePNGPredictorRowsWithBPPForTest(decoded, rowBytes, bytesPerPixel, []byte{0, 1, 2, 3})
	if err != nil {
		t.Fatal(err)
	}
	encoded, err := encodeFlateDecode(predicted)
	if err != nil {
		t.Fatal(err)
	}
	decodeParms := "<< /Predictor 12 /Columns 1 /Colors 2 /BitsPerComponent 16 >>"
	input := testPDF(
		"<< /Type /Page >>",
		fmt.Sprintf("<< /Length %d /Filter /FlateDecode /DecodeParms %s >>\nstream\n%sendstream", len(encoded), decodeParms, encoded),
	)
	adapter := NewAdapter()
	tree, err := adapter.Parse(input, core.ParseOptions{Strict: true})
	if err != nil {
		t.Fatal(err)
	}
	if matches := tree.Query(core.Match{Kind: KindTextShow, Text: "ABCD"}); len(matches) != 1 {
		t.Fatalf("old selectable matches = %d, want 1", len(matches))
	}
	plan, err := adapter.PlanEdit(tree, core.Match{Kind: KindTextShow, Text: "ABCD"}, core.Mutation{Replace: "WXYZ"})
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
	updatedDecoded, err := decodePNGPredictorRowsWithBPPForTest(updatedPredicted, rowBytes, bytesPerPixel)
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Contains(updatedDecoded, []byte("(WXYZ) Tj")) {
		t.Fatalf("decoded stream = %q", updatedDecoded)
	}
	if !hasOnlyPNGFilterNoneRowsForColumnsForTest(updatedPredicted, rowBytes) {
		t.Fatalf("encoded predictor rows were not PNG filter 0 BPC16 rows: %v", updatedPredicted)
	}
	if !bytes.Contains(output, []byte("xref\n0 3\n")) {
		t.Fatalf("xref table was not rebuilt:\n%s", output)
	}
}

func TestFlateDecodeTIFFPredictor2DecodeParmsFilterArrayRewriteReencodesPredictorRowsAndXref(t *testing.T) {
	decoded := []byte("BT\n(08\\05515\\0552024) Tj\nET\n")
	const (
		rowBytes      = 4
		bytesPerPixel = 1
	)
	predicted, err := encodeTIFFPredictorRowsForTest(decoded, rowBytes, bytesPerPixel)
	if err != nil {
		t.Fatal(err)
	}
	encoded, err := encodeFlateDecode(predicted)
	if err != nil {
		t.Fatal(err)
	}
	decodeParms := "[<< /Predictor 2 /Columns 4 >>]"
	input := testPDF(
		"<< /Type /Page >>",
		fmt.Sprintf("<< /Length %d /Filter [/FlateDecode] /DecodeParms %s >>\nstream\n%sendstream", len(encoded), decodeParms, encoded),
	)
	adapter := NewAdapter()
	tree, err := adapter.Parse(input, core.ParseOptions{Strict: true})
	if err != nil {
		t.Fatal(err)
	}
	if matches := tree.Query(core.Match{Kind: KindTextShow, Text: "08-15-2024"}); len(matches) != 1 {
		t.Fatalf("old selectable matches = %d, want 1", len(matches))
	}
	plan, err := adapter.PlanEdit(tree, core.Match{Kind: KindTextShow, Text: "08-15-2024"}, core.Mutation{Replace: "May 05, 2026"})
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
	if stream.lengthValue == len(encoded) {
		t.Fatal("predicted compressed stream length was not updated")
	}
	updatedPredicted, err := decodeFlateDecode(output[stream.dataStart:stream.dataEnd])
	if err != nil {
		t.Fatal(err)
	}
	updatedDecoded, err := decodeTIFFPredictorRowsForTest(updatedPredicted, rowBytes, bytesPerPixel)
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Contains(updatedDecoded, []byte("(May 05, 2026) Tj")) {
		t.Fatalf("decoded stream = %q", updatedDecoded)
	}
	wantPredicted, err := encodeTIFFPredictorRowsForTest(updatedDecoded, rowBytes, bytesPerPixel)
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Equal(updatedPredicted, wantPredicted) {
		t.Fatalf("encoded predictor rows were not TIFF predictor 2 rows: got %v want %v", updatedPredicted, wantPredicted)
	}
	if !bytes.Contains(output, []byte("xref\n0 3\n")) {
		t.Fatalf("xref table was not rebuilt:\n%s", output)
	}
}

func TestFlateDecodeTIFFPredictor2DecodeParmsDefaultsRewriteReencodesLengthAndXref(t *testing.T) {
	decoded := []byte("BT\n(08\\05515\\0552024) Tj\nET\n")
	predicted, err := encodeTIFFPredictorRowsForTest(decoded, 1, 1)
	if err != nil {
		t.Fatal(err)
	}
	encoded, err := encodeFlateDecode(predicted)
	if err != nil {
		t.Fatal(err)
	}
	decodeParms := "<< /Predictor 2 >>"
	input := testPDF(
		"<< /Type /Page >>",
		fmt.Sprintf("<< /Length %d /Filter /FlateDecode /DecodeParms %s >>\nstream\n%sendstream", len(encoded), decodeParms, encoded),
	)
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
	if stream.lengthValue == len(encoded) {
		t.Fatal("predicted compressed stream length was not updated")
	}
	updatedPredicted, err := decodeFlateDecode(output[stream.dataStart:stream.dataEnd])
	if err != nil {
		t.Fatal(err)
	}
	updatedDecoded, err := decodeTIFFPredictorRowsForTest(updatedPredicted, 1, 1)
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Contains(updatedDecoded, []byte("(May 5, 2026) Tj")) {
		t.Fatalf("decoded stream = %q", updatedDecoded)
	}
	if !bytes.Contains(output, []byte("xref\n0 3\n")) {
		t.Fatalf("xref table was not rebuilt:\n%s", output)
	}
}

func TestFilterArrayStreamsFailClosed(t *testing.T) {
	encoded, err := encodeFlateDecode([]byte("BT\n(08\\05515\\0552024) Tj\nET\n"))
	if err != nil {
		t.Fatal(err)
	}

	cases := []struct {
		name   string
		filter string
	}{
		{
			name:   "unsupported second filter",
			filter: "[/FlateDecode /DCTDecode]",
		},
		{
			name:   "unsupported runlength broader chain",
			filter: "[/RunLengthDecode /JPXDecode /ASCIIHexDecode]",
		},
		{
			name:   "unsupported three filter order",
			filter: "[/ASCII85Decode /CCITTFaxDecode /RunLengthDecode]",
		},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			dict := fmt.Sprintf("<< /Length %d /Filter %s >>", len(encoded), tc.filter)
			assertPDFStreamFailsClosed(t, dict, encoded, fmt.Sprintf("unsupported PDF stream filter %q", strings.Join(parsePDFStreamFilterChain(tc.filter), " ")))
		})
	}
}

func encodeASCII85FlateDecodeForTest(input []byte) ([]byte, error) {
	deflated, err := encodeFlateDecode(input)
	if err != nil {
		return nil, err
	}
	out := make([]byte, ascii85.MaxEncodedLen(len(deflated)))
	n := ascii85.Encode(out, deflated)
	return out[:n], nil
}

func decodeASCII85FlateDecodeForTest(input []byte) ([]byte, error) {
	decodedASCII85 := make([]byte, len(input))
	n, _, err := ascii85.Decode(decodedASCII85, input, true)
	if err != nil {
		return nil, err
	}
	return decodeFlateDecode(decodedASCII85[:n])
}

func encodeASCIIHexFlateDecodeForTest(input []byte) ([]byte, error) {
	deflated, err := encodeFlateDecode(input)
	if err != nil {
		return nil, err
	}
	return encodeASCIIHexDecode(deflated)
}

func decodeASCIIHexFlateDecodeForTest(input []byte) ([]byte, error) {
	decodedASCIIHex, err := decodeASCIIHexDecode(input)
	if err != nil {
		return nil, err
	}
	return decodeFlateDecode(decodedASCIIHex)
}

func encodeRunLengthFlateDecodeForTest(input []byte) ([]byte, error) {
	deflated, err := encodeFlateDecode(input)
	if err != nil {
		return nil, err
	}
	return encodeRunLengthDecode(deflated)
}

func decodeRunLengthFlateDecodeForTest(input []byte) ([]byte, error) {
	decodedRunLength, err := decodeRunLengthDecode(input)
	if err != nil {
		return nil, err
	}
	return decodeFlateDecode(decodedRunLength)
}

func encodeFlateASCIIHexDecodeForTest(input []byte) ([]byte, error) {
	asciiHex, err := encodeASCIIHexDecode(input)
	if err != nil {
		return nil, err
	}
	return encodeFlateDecode(asciiHex)
}

func decodeFlateASCIIHexDecodeForTest(input []byte) ([]byte, error) {
	decodedFlate, err := decodeFlateDecode(input)
	if err != nil {
		return nil, err
	}
	return decodeASCIIHexDecode(decodedFlate)
}

func decodeASCII85RunLengthFlateDecodeForTest(input []byte) ([]byte, error) {
	decodedASCII85, err := decodeASCII85Decode(input)
	if err != nil {
		return nil, err
	}
	return decodeRunLengthFlateDecodeForTest(decodedASCII85)
}

func decodeASCIIHexRunLengthFlateDecodeForTest(input []byte) ([]byte, error) {
	decodedASCIIHex, err := decodeASCIIHexDecode(input)
	if err != nil {
		return nil, err
	}
	return decodeRunLengthFlateDecodeForTest(decodedASCIIHex)
}

func encodePNGOneByteRowsForTest(input []byte) []byte {
	output := make([]byte, 0, len(input)*2)
	for _, b := range input {
		output = append(output, 0, b)
	}
	return output
}

func decodePNGOneByteRowsForTest(input []byte) ([]byte, error) {
	if len(input)%2 != 0 {
		return nil, fmt.Errorf("predicted stream has partial row")
	}
	output := make([]byte, 0, len(input)/2)
	for i := 0; i < len(input); i += 2 {
		if input[i] != 0 {
			return nil, fmt.Errorf("unsupported PNG row filter %d", input[i])
		}
		output = append(output, input[i+1])
	}
	return output, nil
}

func hasOnlyPNGFilterNoneRowsForTest(input []byte) bool {
	if len(input)%2 != 0 {
		return false
	}
	for i := 0; i < len(input); i += 2 {
		if input[i] != 0 {
			return false
		}
	}
	return true
}

func encodePNGPredictorRowsForTest(input []byte, columns int, filters []byte) ([]byte, error) {
	return encodePNGPredictorRowsWithBPPForTest(input, columns, 1, filters)
}

func encodePNGPredictorRowsWithBPPForTest(input []byte, rowBytes, bytesPerPixel int, filters []byte) ([]byte, error) {
	if rowBytes <= 0 {
		return nil, fmt.Errorf("row bytes must be positive")
	}
	if bytesPerPixel <= 0 {
		return nil, fmt.Errorf("bytes per pixel must be positive")
	}
	if len(input)%rowBytes != 0 {
		return nil, fmt.Errorf("decoded stream has partial row")
	}
	rowCount := len(input) / rowBytes
	if len(filters) != rowCount {
		return nil, fmt.Errorf("filters = %d, want %d", len(filters), rowCount)
	}
	output := make([]byte, 0, rowCount*(rowBytes+1))
	previousRow := make([]byte, rowBytes)
	for row := 0; row < rowCount; row++ {
		filter := filters[row]
		rowData := input[row*rowBytes : (row+1)*rowBytes]
		output = append(output, filter)
		for col, decoded := range rowData {
			left := byte(0)
			if col >= bytesPerPixel {
				left = rowData[col-bytesPerPixel]
			}
			up := previousRow[col]
			upLeft := byte(0)
			if col >= bytesPerPixel {
				upLeft = previousRow[col-bytesPerPixel]
			}
			var predicted byte
			switch filter {
			case 0:
				predicted = decoded
			case 1:
				predicted = decoded - left
			case 2:
				predicted = decoded - up
			case 3:
				predicted = decoded - byte((int(left)+int(up))/2)
			case 4:
				predicted = decoded - paethPredictorForTest(left, up, upLeft)
			default:
				return nil, fmt.Errorf("unsupported PNG row filter %d", filter)
			}
			output = append(output, predicted)
		}
		copy(previousRow, rowData)
	}
	return output, nil
}

func decodePNGPredictorRowsForTest(input []byte, columns int) ([]byte, error) {
	return decodePNGPredictorRowsWithBPPForTest(input, columns, 1)
}

func decodePNGPredictorRowsWithBPPForTest(input []byte, rowBytes, bytesPerPixel int) ([]byte, error) {
	if rowBytes <= 0 {
		return nil, fmt.Errorf("row bytes must be positive")
	}
	if bytesPerPixel <= 0 {
		return nil, fmt.Errorf("bytes per pixel must be positive")
	}
	rowLength := rowBytes + 1
	if len(input)%rowLength != 0 {
		return nil, fmt.Errorf("predicted stream has partial row")
	}
	output := make([]byte, 0, len(input)/rowLength*rowBytes)
	previousRow := make([]byte, rowBytes)
	for rowStart := 0; rowStart < len(input); rowStart += rowLength {
		filter := input[rowStart]
		row := make([]byte, rowBytes)
		for col := 0; col < rowBytes; col++ {
			x := input[rowStart+1+col]
			left := byte(0)
			if col >= bytesPerPixel {
				left = row[col-bytesPerPixel]
			}
			up := previousRow[col]
			upLeft := byte(0)
			if col >= bytesPerPixel {
				upLeft = previousRow[col-bytesPerPixel]
			}
			switch filter {
			case 0:
				row[col] = x
			case 1:
				row[col] = x + left
			case 2:
				row[col] = x + up
			case 3:
				row[col] = x + byte((int(left)+int(up))/2)
			case 4:
				row[col] = x + paethPredictorForTest(left, up, upLeft)
			default:
				return nil, fmt.Errorf("unsupported PNG row filter %d", filter)
			}
		}
		output = append(output, row...)
		copy(previousRow, row)
	}
	return output, nil
}

func hasOnlyPNGFilterNoneRowsForColumnsForTest(input []byte, columns int) bool {
	if columns <= 0 {
		return false
	}
	rowLength := columns + 1
	if len(input)%rowLength != 0 {
		return false
	}
	for i := 0; i < len(input); i += rowLength {
		if input[i] != 0 {
			return false
		}
	}
	return true
}

func encodeTIFFPredictorRowsForTest(input []byte, rowBytes, bytesPerPixel int) ([]byte, error) {
	if rowBytes <= 0 {
		return nil, fmt.Errorf("row bytes must be positive")
	}
	if bytesPerPixel <= 0 {
		return nil, fmt.Errorf("bytes per pixel must be positive")
	}
	if len(input)%rowBytes != 0 {
		return nil, fmt.Errorf("decoded stream has partial row")
	}
	output := make([]byte, 0, len(input))
	for rowStart := 0; rowStart < len(input); rowStart += rowBytes {
		row := input[rowStart : rowStart+rowBytes]
		for col, decoded := range row {
			if col < bytesPerPixel {
				output = append(output, decoded)
				continue
			}
			output = append(output, decoded-row[col-bytesPerPixel])
		}
	}
	return output, nil
}

func decodeTIFFPredictorRowsForTest(input []byte, rowBytes, bytesPerPixel int) ([]byte, error) {
	if rowBytes <= 0 {
		return nil, fmt.Errorf("row bytes must be positive")
	}
	if bytesPerPixel <= 0 {
		return nil, fmt.Errorf("bytes per pixel must be positive")
	}
	if len(input)%rowBytes != 0 {
		return nil, fmt.Errorf("predicted stream has partial row")
	}
	output := make([]byte, 0, len(input))
	for rowStart := 0; rowStart < len(input); rowStart += rowBytes {
		row := bytes.Clone(input[rowStart : rowStart+rowBytes])
		for col := bytesPerPixel; col < rowBytes; col++ {
			row[col] += row[col-bytesPerPixel]
		}
		output = append(output, row...)
	}
	return output, nil
}

func paethPredictorForTest(left, up, upLeft byte) byte {
	p := int(left) + int(up) - int(upLeft)
	pa := absForTest(p - int(left))
	pb := absForTest(p - int(up))
	pc := absForTest(p - int(upLeft))
	if pa <= pb && pa <= pc {
		return left
	}
	if pb <= pc {
		return up
	}
	return upLeft
}

func absForTest(v int) int {
	if v < 0 {
		return -v
	}
	return v
}

func TestDecodeParmsStreamsFailClosed(t *testing.T) {
	encoded, err := encodeFlateDecode([]byte("BT\n(08\\05515\\0552024) Tj\nET\n"))
	if err != nil {
		t.Fatal(err)
	}

	cases := []struct {
		name            string
		dict            string
		wantUnsupported string
	}{
		{
			name:            "asciihex filter decode parms",
			dict:            fmt.Sprintf("<< /Length %d /Filter [/ASCIIHexDecode /FlateDecode] /DecodeParms [<< /Predictor 1 >> null] >>", len(encoded)),
			wantUnsupported: "unsupported stream: /DecodeParms for /ASCIIHexDecode must be null",
		},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			wantUnsupported := "unsupported stream: /DecodeParms is not implemented"
			if tc.wantUnsupported != "" {
				wantUnsupported = tc.wantUnsupported
			}
			assertPDFStreamFailsClosed(t, tc.dict, encoded, wantUnsupported)
		})
	}
}

func assertPDFStreamFailsClosed(t *testing.T, dict string, encoded []byte, wantUnsupported string) {
	t.Helper()

	input := testPDF("<< /Type /Page >>", fmt.Sprintf("%s\nstream\n%sendstream", dict, encoded))
	tree, err := NewAdapter().Parse(input, core.ParseOptions{Strict: true})
	if err != nil {
		t.Fatal(err)
	}
	if matches := tree.Query(core.Match{Kind: KindTextShow, Text: "08-15-2024"}); len(matches) != 0 {
		t.Fatalf("text matches = %d, want 0", len(matches))
	}
	streams := tree.Query(core.Match{Kind: KindStream, Meta: map[string]any{"unsupported": wantUnsupported}})
	if len(streams) != 1 {
		t.Fatalf("unsupported stream matches = %d, want 1", len(streams))
	}
	_, err = NewAdapter().PlanEdit(tree, core.Match{Kind: KindTextShow, Text: "08-15-2024"}, core.Mutation{Replace: "05-04-2026"})
	if err == nil {
		t.Fatal("expected edit planning to fail closed")
	}
}
