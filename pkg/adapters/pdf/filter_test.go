package pdf

import (
	"bytes"
	"testing"
)

func TestFilterFlateDecodeRoundTrip(t *testing.T) {
	input := []byte("BT\n(05\\05504\\0552026) Tj\nET\n")

	encoded, err := encodeFlateDecode(input)
	if err != nil {
		t.Fatal(err)
	}
	if bytes.Equal(encoded, input) {
		t.Fatal("encoded stream unexpectedly matches input")
	}

	decoded, err := decodeFlateDecode(encoded)
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Equal(decoded, input) {
		t.Fatalf("decoded stream = %q, want %q", decoded, input)
	}
}

func TestFilterDispatchSupportsFlateDecodeNameToken(t *testing.T) {
	input := []byte("q 1 0 0 1 0 0 cm Q")

	encoded, err := encodeStreamFilter("/FlateDecode", input)
	if err != nil {
		t.Fatal(err)
	}
	decoded, err := decodeStreamFilter(" FlateDecode ", encoded)
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Equal(decoded, input) {
		t.Fatalf("decoded stream = %q, want %q", decoded, input)
	}
}

func TestFilterASCIIHexDecodeRoundTrip(t *testing.T) {
	input := []byte("BT\n(05-04-2026) Tj\nET\n")

	encoded, err := encodeStreamFilter("/ASCIIHexDecode", input)
	if err != nil {
		t.Fatal(err)
	}
	if bytes.Equal(encoded, input) {
		t.Fatal("encoded stream unexpectedly matches input")
	}
	if encoded[len(encoded)-1] != '>' {
		t.Fatalf("encoded stream = %q, want PDF ASCIIHex terminator", encoded)
	}

	decoded, err := decodeStreamFilter(" ASCIIHexDecode ", encoded)
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Equal(decoded, input) {
		t.Fatalf("decoded stream = %q, want %q", decoded, input)
	}
}

func TestFilterASCII85DecodeRoundTrip(t *testing.T) {
	input := []byte("BT\n(05-04-2026) Tj\nET\n")

	encoded, err := encodeStreamFilter("/ASCII85Decode", input)
	if err != nil {
		t.Fatal(err)
	}
	if bytes.Equal(encoded, input) {
		t.Fatal("encoded stream unexpectedly matches input")
	}

	decoded, err := decodeStreamFilter(" ASCII85Decode ", encoded)
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Equal(decoded, input) {
		t.Fatalf("decoded stream = %q, want %q", decoded, input)
	}
}

func TestFilterRunLengthDecodeRoundTrip(t *testing.T) {
	input := []byte("BT\n(aaaaabxyz) Tj\nET\n")

	encoded, err := encodeStreamFilter("/RunLengthDecode", input)
	if err != nil {
		t.Fatal(err)
	}
	if bytes.Equal(encoded, input) {
		t.Fatal("encoded stream unexpectedly matches input")
	}
	if encoded[len(encoded)-1] != 128 {
		t.Fatalf("encoded stream = %v, want RunLengthDecode EOD marker", encoded)
	}

	decoded, err := decodeStreamFilter(" RunLengthDecode ", encoded)
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Equal(decoded, input) {
		t.Fatalf("decoded stream = %q, want %q", decoded, input)
	}
}

func TestFilterRunLengthDecodeRejectsMalformedData(t *testing.T) {
	for _, input := range [][]byte{
		[]byte{0, 'B'},
		[]byte{129},
		[]byte{3, 'B', 'T'},
		[]byte("BT"),
	} {
		if _, err := decodeStreamFilter("/RunLengthDecode", input); err == nil {
			t.Fatalf("expected RunLengthDecode error for %v", input)
		}
	}
}

func TestFilterASCIIHexDecodeIgnoresWhitespaceAndPadsOddFinalNibble(t *testing.T) {
	decoded, err := decodeStreamFilter("/ASCIIHexDecode", []byte("  42\t5\n4 2>ignored"))
	if err != nil {
		t.Fatal(err)
	}
	want := []byte{'B', 'T', 0x20}
	if !bytes.Equal(decoded, want) {
		t.Fatalf("decoded stream = %x, want %x", decoded, want)
	}
}

func TestFilterASCIIHexDecodeRejectsInvalidHex(t *testing.T) {
	for _, input := range [][]byte{
		[]byte("42G4>"),
		[]byte("4244"),
	} {
		if _, err := decodeStreamFilter("/ASCIIHexDecode", input); err == nil {
			t.Fatalf("expected ASCIIHexDecode error for %q", input)
		}
	}
}

func TestFilterASCIIHexFlateDecodeArrayRoundTrip(t *testing.T) {
	input := []byte("BT\n(05-05-2026) Tj\nET\n")

	encoded, err := encodeStreamFilter("[/ASCIIHexDecode /FlateDecode]", input)
	if err != nil {
		t.Fatal(err)
	}
	if bytes.Equal(encoded, input) {
		t.Fatal("encoded stream unexpectedly matches input")
	}
	if encoded[len(encoded)-1] != '>' {
		t.Fatalf("encoded stream = %q, want PDF ASCIIHex terminator", encoded)
	}

	decodedASCIIHex, err := decodeASCIIHexDecode(encoded)
	if err != nil {
		t.Fatal(err)
	}
	if bytes.Contains(decodedASCIIHex, input) {
		t.Fatal("ASCIIHex decoded stream was not still Flate-compressed")
	}

	decoded, err := decodeStreamFilter("[/ASCIIHexDecode /FlateDecode]", encoded)
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Equal(decoded, input) {
		t.Fatalf("decoded stream = %q, want %q", decoded, input)
	}
}

func TestFilterASCIIHexFlateDecodeArrayDecodeParmsRoundTrip(t *testing.T) {
	input := []byte("BT\n(05-05-2026) Tj\nET\n")
	decodeParms := "[null << /Predictor 12 /Columns 1 /Colors 1 /BitsPerComponent 8 >>]"

	encoded, err := encodeStreamFilterWithDecodeParms("[/ASCIIHexDecode /FlateDecode]", decodeParms, input)
	if err != nil {
		t.Fatal(err)
	}
	deflatedPredicted, err := decodeASCIIHexDecode(encoded)
	if err != nil {
		t.Fatal(err)
	}
	predicted, err := decodeFlateDecode(deflatedPredicted)
	if err != nil {
		t.Fatal(err)
	}
	if !hasOnlyPNGFilterNoneRowsForTest(predicted) {
		t.Fatalf("encoded predictor rows were not PNG filter 0 one-byte rows: %v", predicted)
	}

	decoded, err := decodeStreamFilterWithDecodeParms("[/ASCIIHexDecode /FlateDecode]", decodeParms, encoded)
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Equal(decoded, input) {
		t.Fatalf("decoded stream = %q, want %q", decoded, input)
	}
}

func TestFilterRunLengthASCIIArmoredFlateDecodeArrayRoundTrip(t *testing.T) {
	input := []byte("BT\n(05-05-2026) Tj\nET\n")
	cases := []struct {
		name   string
		filter string
	}{
		{
			name:   "runlength ascii85 flate",
			filter: "[/RunLengthDecode /ASCII85Decode /FlateDecode]",
		},
		{
			name:   "runlength asciihex flate",
			filter: "[/RunLengthDecode /ASCIIHexDecode /FlateDecode]",
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

func TestFilterRunLengthASCIIArmoredFlateDecodeArrayDecodeParmsRoundTrip(t *testing.T) {
	input := []byte("BT\n(05-05-2026) Tj\nET\n")
	decodeParms := "[null null << /Predictor 12 /Columns 1 /Colors 1 /BitsPerComponent 8 >>]"
	cases := []struct {
		name   string
		filter string
	}{
		{
			name:   "runlength ascii85 flate",
			filter: "[/RunLengthDecode /ASCII85Decode /FlateDecode]",
		},
		{
			name:   "runlength asciihex flate",
			filter: "[/RunLengthDecode /ASCIIHexDecode /FlateDecode]",
		},
	}

	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			encoded, err := encodeStreamFilterWithDecodeParms(tc.filter, decodeParms, input)
			if err != nil {
				t.Fatal(err)
			}
			decoded, err := decodeStreamFilterWithDecodeParms(tc.filter, decodeParms, encoded)
			if err != nil {
				t.Fatal(err)
			}
			if !bytes.Equal(decoded, input) {
				t.Fatalf("decoded stream = %q, want %q", decoded, input)
			}
		})
	}
}

func TestFilterDecodeParmsArraysFailClosedForUnsupportedShapes(t *testing.T) {
	cases := []struct {
		name        string
		filter      string
		decodeParms string
		want        string
	}{
		{
			name:        "asciihex params must be null",
			filter:      "[/ASCIIHexDecode /FlateDecode]",
			decodeParms: "[<< /Predictor 1 >> null]",
			want:        "unsupported stream: /DecodeParms for /ASCIIHexDecode must be null",
		},
		{
			name:        "array length must match filter chain",
			filter:      "[/ASCIIHexDecode /FlateDecode]",
			decodeParms: "[null null << /Predictor 1 >>]",
			want:        "unsupported stream: /DecodeParms array length must match /Filter array length",
		},
		{
			name:        "array entries must be null or dictionaries",
			filter:      "[/ASCIIHexDecode /FlateDecode]",
			decodeParms: "[null /Predictor]",
			want:        "unsupported stream: /DecodeParms array entries must be null or direct dictionaries",
		},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			_, err := decodeStreamFilterWithDecodeParms(tc.filter, tc.decodeParms, []byte("ignored"))
			if err == nil {
				t.Fatal("expected DecodeParms array error")
			}
			if err.Error() != tc.want {
				t.Fatalf("error = %v, want %q", err, tc.want)
			}
		})
	}
}

func TestFilterPassthroughReturnsCloneForNoOpFilters(t *testing.T) {
	input := []byte("raw stream bytes")

	for _, filter := range []string{"", " ", "Identity", "/Identity"} {
		t.Run(filter, func(t *testing.T) {
			if !isPassthroughPDFStreamFilter(filter) {
				t.Fatalf("filter %q was not detected as passthrough", filter)
			}

			decoded, err := decodeStreamFilter(filter, input)
			if err != nil {
				t.Fatal(err)
			}
			if !bytes.Equal(decoded, input) {
				t.Fatalf("decoded stream = %q, want %q", decoded, input)
			}
			if len(decoded) > 0 && &decoded[0] == &input[0] {
				t.Fatal("passthrough decode returned aliased input")
			}

			encoded, err := encodeStreamFilter(filter, input)
			if err != nil {
				t.Fatal(err)
			}
			if !bytes.Equal(encoded, input) {
				t.Fatalf("encoded stream = %q, want %q", encoded, input)
			}
			if len(encoded) > 0 && &encoded[0] == &input[0] {
				t.Fatal("passthrough encode returned aliased input")
			}
		})
	}
}

func TestFilterDispatchRejectsUnsupportedFilter(t *testing.T) {
	if _, err := decodeStreamFilter("/LZWDecode", []byte("stream")); err == nil {
		t.Fatal("expected unsupported arbitrary decode filter error")
	}
	if _, err := encodeStreamFilter("/LZWDecode", []byte("stream")); err == nil {
		t.Fatal("expected unsupported arbitrary encode filter error")
	}
}
