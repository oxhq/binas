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

func TestFilterAbbreviationsRoundTrip(t *testing.T) {
	input := []byte("BT\n(05-05-2026) Tj\nET\n")
	cases := []struct {
		name   string
		filter string
	}{
		{
			name:   "flate",
			filter: "/Fl",
		},
		{
			name:   "asciihex",
			filter: "/AHx",
		},
		{
			name:   "ascii85",
			filter: "/A85",
		},
		{
			name:   "runlength",
			filter: "/RL",
		},
		{
			name:   "mixed asciihex flate chain",
			filter: "[/AHx /FlateDecode]",
		},
		{
			name:   "mixed runlength ascii85 flate chain",
			filter: "[/RL /ASCII85Decode /Fl]",
		},
	}

	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			encoded, err := encodeStreamFilter(tc.filter, input)
			if err != nil {
				t.Fatal(err)
			}
			if bytes.Equal(encoded, input) {
				t.Fatal("encoded stream unexpectedly matches input")
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

func TestFilterFlateASCIIHexDecodeArrayRoundTrip(t *testing.T) {
	input := []byte("BT\n(05-05-2026) Tj\nET\n")
	filter := "[/FlateDecode /ASCIIHexDecode]"

	encoded, err := encodeStreamFilter(filter, input)
	if err != nil {
		t.Fatal(err)
	}
	if bytes.Equal(encoded, input) {
		t.Fatal("encoded stream unexpectedly matches input")
	}

	deflatedASCIIHex, err := decodeFlateDecode(encoded)
	if err != nil {
		t.Fatal(err)
	}
	if deflatedASCIIHex[len(deflatedASCIIHex)-1] != '>' {
		t.Fatalf("Flate decoded stream = %q, want ASCIIHex terminator", deflatedASCIIHex)
	}

	decoded, err := decodeStreamFilter(filter, encoded)
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Equal(decoded, input) {
		t.Fatalf("decoded stream = %q, want %q", decoded, input)
	}
}

func TestFilterAllNullDecodeParmsArrayRoundTripForFlateASCIIHex(t *testing.T) {
	input := []byte("BT\n(05-05-2026) Tj\nET\n")
	filter := "[/FlateDecode /ASCIIHexDecode]"
	decodeParms := "[null null]"

	encoded, err := encodeStreamFilterWithDecodeParms(filter, decodeParms, input)
	if err != nil {
		t.Fatal(err)
	}
	decoded, err := decodeStreamFilterWithDecodeParms(filter, decodeParms, encoded)
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

func TestFilterTIFFPredictorDecodeParmsSamplePackedRoundTrip(t *testing.T) {
	cases := []struct {
		name        string
		input       []byte
		filter      string
		decodeParms string
	}{
		{
			name:        "bpc1 non-byte row preserves padding bit",
			input:       []byte{0b10110011, 0b01100110, 0b11110001, 0b00001111},
			filter:      "/FlateDecode",
			decodeParms: "<< /Predictor 2 /Columns 7 /Colors 1 /BitsPerComponent 1 >>",
		},
		{
			name:        "bpc4 multi-color packed pixels",
			input:       []byte{0x12, 0x34, 0x56, 0x9a, 0xbc, 0xde},
			filter:      "/FlateDecode",
			decodeParms: "<< /Predictor 2 /Columns 2 /Colors 3 /BitsPerComponent 4 >>",
		},
		{
			name:        "bpc16 sample delta with borrow",
			input:       []byte{0x01, 0x00, 0x00, 0xff, 0x00, 0x01, 0xff, 0xff},
			filter:      "[/ASCIIHexDecode /FlateDecode]",
			decodeParms: "[null << /Predictor 2 /Columns 2 /Colors 1 /BitsPerComponent 16 >>]",
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
				t.Fatalf("decoded stream = %08b, want %08b", decoded, tc.input)
			}
		})
	}
}

func TestFilterCryptIdentityDecodeParmsRoundTripWithoutEncryptionContext(t *testing.T) {
	input := []byte("BT\n(Identity crypt filter) Tj\nET\n")
	decoded, err := decodeStreamFilterWithDecodeParms("/Crypt", "<< /Name /Identity >>", input)
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Equal(decoded, input) {
		t.Fatalf("decoded stream = %q, want %q", decoded, input)
	}
	encoded, err := encodeStreamFilterWithDecodeParms("[/Crypt]", "[<< /Name /Identity >>]", decoded)
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Equal(encoded, input) {
		t.Fatalf("encoded stream = %q, want %q", encoded, input)
	}
}

func TestFilterCryptStdCFRequiresEncryptionContext(t *testing.T) {
	input := []byte("encrypted")
	_, err := decodeStreamFilterWithDecodeParms("/Crypt", "<< /Name /StdCF >>", input)
	if err == nil || err.Error() != "unsupported PDF stream crypt filter /StdCF requires encryption context" {
		t.Fatalf("decode error = %v, want context refusal", err)
	}
	_, err = encodeStreamFilterWithDecodeParms("/Crypt", "<< /Name /StdCF >>", input)
	if err == nil || err.Error() != "unsupported PDF stream crypt filter /StdCF requires encryption context" {
		t.Fatalf("encode error = %v, want context refusal", err)
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
		{
			name:        "non-flate params must be null in broader supported chain",
			filter:      "[/FlateDecode /ASCIIHexDecode]",
			decodeParms: "[null << /Predictor 1 >>]",
			want:        "unsupported stream: /DecodeParms for /ASCIIHexDecode must be null",
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
	if _, err := decodeStreamFilter("/DCTDecode", []byte("stream")); err == nil {
		t.Fatal("expected unsupported arbitrary decode filter error")
	}
	if _, err := encodeStreamFilter("/JPXDecode", []byte("stream")); err == nil {
		t.Fatal("expected unsupported arbitrary encode filter error")
	}
	if _, err := decodeStreamFilter("/DCT", []byte("stream")); err == nil {
		t.Fatal("expected unsupported abbreviation decode filter error")
	}
	if _, err := encodeStreamFilter("[/CCF /Fl]", []byte("stream")); err == nil {
		t.Fatal("expected unsupported abbreviation chain encode filter error")
	}
}
