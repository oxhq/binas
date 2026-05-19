package pdf

import (
	"fmt"
	"testing"

	"github.com/oxhq/binas/pkg/core"
)

func TestFilterCapabilityClassifiesEditableReversibleFilters(t *testing.T) {
	cases := []struct {
		name   string
		filter string
		want   []string
	}{
		{name: "flate name", filter: "/FlateDecode", want: []string{"FlateDecode"}},
		{name: "abbreviated chain", filter: "[/AHx /A85 /RL /Fl /LZW]", want: []string{"ASCIIHexDecode", "ASCII85Decode", "RunLengthDecode", "FlateDecode", "LZWDecode"}},
	}

	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			got := classifyPDFStreamFilterCapability(tc.filter)
			if got.Class != pdfStreamFilterCapabilityEditableReversible {
				t.Fatalf("Class = %q, want %q", got.Class, pdfStreamFilterCapabilityEditableReversible)
			}
			if !got.Editable || got.PassThrough {
				t.Fatalf("Editable/PassThrough = %v/%v, want true/false", got.Editable, got.PassThrough)
			}
			assertFilterCapabilityChain(t, got.Chain, tc.want)
		})
	}
}

func TestFilterCapabilityClassifiesImageFiltersAsNonEditablePassThrough(t *testing.T) {
	cases := []struct {
		name   string
		filter string
		want   []string
	}{
		{name: "dct", filter: "/DCTDecode", want: []string{"DCTDecode"}},
		{name: "dct abbreviation", filter: "/DCT", want: []string{"DCTDecode"}},
		{name: "jpx", filter: "/JPXDecode", want: []string{"JPXDecode"}},
		{name: "ccitt", filter: "/CCITTFaxDecode", want: []string{"CCITTFaxDecode"}},
		{name: "ccitt abbreviation", filter: "/CCF", want: []string{"CCITTFaxDecode"}},
		{name: "jbig2", filter: "/JBIG2Decode", want: []string{"JBIG2Decode"}},
	}

	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			got := classifyPDFStreamFilterCapability(tc.filter)
			if got.Class != pdfStreamFilterCapabilityPassThroughImage {
				t.Fatalf("Class = %q, want %q", got.Class, pdfStreamFilterCapabilityPassThroughImage)
			}
			if got.Editable || !got.PassThrough || got.Target {
				t.Fatalf("Editable/PassThrough/Target = %v/%v/%v, want false/true/false", got.Editable, got.PassThrough, got.Target)
			}
			assertFilterCapabilityChain(t, got.Chain, tc.want)
		})
	}
}

func TestFilterCapabilityClassifiesMixedOrUnknownFiltersAsUnsupportedTarget(t *testing.T) {
	cases := []struct {
		name   string
		filter string
		want   []string
	}{
		{name: "unknown", filter: "/FooDecode", want: []string{"FooDecode"}},
		{name: "image plus editable", filter: "[/DCTDecode /FlateDecode]", want: []string{"DCTDecode", "FlateDecode"}},
		{name: "editable plus image", filter: "[/ASCII85Decode /JPXDecode]", want: []string{"ASCII85Decode", "JPXDecode"}},
	}

	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			got := classifyPDFStreamFilterCapability(tc.filter)
			if got.Class != pdfStreamFilterCapabilityUnsupportedTarget {
				t.Fatalf("Class = %q, want %q", got.Class, pdfStreamFilterCapabilityUnsupportedTarget)
			}
			if got.Editable || got.PassThrough || !got.Target {
				t.Fatalf("Editable/PassThrough/Target = %v/%v/%v, want false/false/true", got.Editable, got.PassThrough, got.Target)
			}
			assertFilterCapabilityChain(t, got.Chain, tc.want)
		})
	}
}

func TestFilterCapabilityClassifiesIdentityAsPassThrough(t *testing.T) {
	for _, filter := range []string{"", " ", "Identity", "/Identity"} {
		t.Run(filter, func(t *testing.T) {
			got := classifyPDFStreamFilterCapability(filter)
			if got.Class != pdfStreamFilterCapabilityIdentityPassThrough {
				t.Fatalf("Class = %q, want %q", got.Class, pdfStreamFilterCapabilityIdentityPassThrough)
			}
			if got.Editable || !got.PassThrough || got.Target {
				t.Fatalf("Editable/PassThrough/Target = %v/%v/%v, want false/true/false", got.Editable, got.PassThrough, got.Target)
			}
			if len(got.Chain) != 0 {
				t.Fatalf("Chain = %#v, want empty", got.Chain)
			}
		})
	}
}

func TestFilterCapabilityMetadataMarksImageStreamsNonEditablePassThrough(t *testing.T) {
	encoded := []byte("not-jpeg")
	input := testPDF(
		"<< /Type /Page >>",
		fmt.Sprintf("<< /Length %d /Filter /DCTDecode >>\nstream\n%sendstream", len(encoded), encoded),
	)
	tree, err := NewAdapter().Parse(input, core.ParseOptions{Strict: true})
	if err != nil {
		t.Fatal(err)
	}
	streams := tree.Query(core.Match{Kind: KindStream})
	if len(streams) != 1 {
		t.Fatalf("stream nodes = %d, want 1", len(streams))
	}
	meta := streams[0].Meta
	if meta["filter_capability"] != string(pdfStreamFilterCapabilityPassThroughImage) {
		t.Fatalf("filter_capability = %v, want %q", meta["filter_capability"], pdfStreamFilterCapabilityPassThroughImage)
	}
	if meta["filter_editable"] != false || meta["filter_pass_through"] != true || meta["filter_target"] != false {
		t.Fatalf("filter metadata = editable:%v pass_through:%v target:%v, want false/true/false", meta["filter_editable"], meta["filter_pass_through"], meta["filter_target"])
	}
	if _, ok := meta["decoded_length"]; ok {
		t.Fatalf("decoded_length present for non-editable image pass-through stream: %+v", meta)
	}
}

func assertFilterCapabilityChain(t *testing.T, got, want []string) {
	t.Helper()
	if len(got) != len(want) {
		t.Fatalf("Chain = %#v, want %#v", got, want)
	}
	for i := range want {
		if got[i] != want[i] {
			t.Fatalf("Chain = %#v, want %#v", got, want)
		}
	}
}
