package pdf

import (
	"bytes"
	"testing"
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

func TestFilterFlateDecodePredictor1DecodeParmsRejectsNonDefaultGeometry(t *testing.T) {
	input, err := encodeFlateDecode([]byte("stream"))
	if err != nil {
		t.Fatal(err)
	}
	cases := []struct {
		name        string
		decodeParms string
	}{
		{
			name:        "non default columns",
			decodeParms: "<< /Predictor 1 /Columns 2 >>",
		},
		{
			name:        "non default colors",
			decodeParms: "<< /Predictor 1 /Colors 2 >>",
		},
		{
			name:        "non default bits per component",
			decodeParms: "<< /Predictor 1 /BitsPerComponent 1 >>",
		},
		{
			name:        "unsupported predictor",
			decodeParms: "<< /Predictor 9 /Columns 1 >>",
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
