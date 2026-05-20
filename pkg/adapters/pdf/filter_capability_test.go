package pdf

import (
	"bytes"
	"fmt"
	"strings"
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
		{name: "image only chain", filter: "[/DCTDecode /JPXDecode]", want: []string{"DCTDecode", "JPXDecode"}},
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
		{name: "abbreviated mixed image chain", filter: "[/A85 /DCT]", want: []string{"ASCII85Decode", "DCTDecode"}},
		{name: "crypt", filter: "/Crypt", want: []string{"Crypt"}},
		{name: "editable plus crypt", filter: "[/FlateDecode /Crypt]", want: []string{"FlateDecode", "Crypt"}},
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
	for _, filter := range []string{"", " ", "Identity", "/Identity", "null"} {
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
	cases := []struct {
		name   string
		filter string
	}{
		{name: "dct", filter: "/DCTDecode"},
		{name: "jpx", filter: "/JPXDecode"},
		{name: "ccitt", filter: "/CCITTFaxDecode"},
		{name: "jbig2", filter: "/JBIG2Decode"},
	}

	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			encoded := []byte("image pass-through bytes")
			input := testPDF(
				"<< /Type /Page >>",
				fmt.Sprintf("<< /Length %d /Filter %s >>\nstream\n%sendstream", len(encoded), tc.filter, encoded),
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
		})
	}
}

func TestFilterCapabilityImagePassThroughDoesNotBlockFlateTextEdit(t *testing.T) {
	decoded := []byte("BT\n(FLATE-TEXT) Tj\nET\n")
	encoded, err := encodeFlateDecode(decoded)
	if err != nil {
		t.Fatal(err)
	}

	for _, imageFilter := range []string{"/DCTDecode", "/JPXDecode", "/CCITTFaxDecode", "/JBIG2Decode"} {
		t.Run(imageFilter, func(t *testing.T) {
			imageLike := []byte("BT\n(IMAGE-ONLY) Tj\nET\n")
			input := testPDF(
				"<< /Type /Page >>",
				fmt.Sprintf("<< /Length %d /Filter %s >>\nstream\n%sendstream", len(imageLike), imageFilter, imageLike),
				fmt.Sprintf("<< /Length %d /Filter /FlateDecode >>\nstream\n%sendstream", len(encoded), encoded),
			)

			adapter := NewAdapter()
			tree, err := adapter.Parse(input, core.ParseOptions{Strict: true})
			if err != nil {
				t.Fatal(err)
			}
			if matches := tree.Query(core.Match{Kind: KindTextShow, Text: "FLATE-TEXT"}); len(matches) != 1 {
				t.Fatalf("Flate text matches = %d, want 1", len(matches))
			}
			if matches := tree.Query(core.Match{Kind: KindTextShow, Text: "IMAGE-ONLY"}); len(matches) != 0 {
				t.Fatalf("image text matches = %d, want 0", len(matches))
			}

			streams := tree.Query(core.Match{Kind: KindStream})
			if len(streams) != 2 {
				t.Fatalf("stream nodes = %d, want 2", len(streams))
			}
			imageMeta := streams[0].Meta
			if imageMeta["filter_capability"] != string(pdfStreamFilterCapabilityPassThroughImage) {
				t.Fatalf("image filter_capability = %v, want %q", imageMeta["filter_capability"], pdfStreamFilterCapabilityPassThroughImage)
			}
			if imageMeta["filter_editable"] != false || imageMeta["filter_pass_through"] != true || imageMeta["filter_target"] != false {
				t.Fatalf("image filter metadata = editable:%v pass_through:%v target:%v, want false/true/false", imageMeta["filter_editable"], imageMeta["filter_pass_through"], imageMeta["filter_target"])
			}
			if _, ok := imageMeta["decoded_length"]; ok {
				t.Fatalf("decoded_length present for image pass-through stream: %+v", imageMeta)
			}
			flateMeta := streams[1].Meta
			if flateMeta["filter_capability"] != string(pdfStreamFilterCapabilityEditableReversible) {
				t.Fatalf("Flate filter_capability = %v, want %q", flateMeta["filter_capability"], pdfStreamFilterCapabilityEditableReversible)
			}
			if flateMeta["filter_editable"] != true || flateMeta["filter_pass_through"] != false || flateMeta["filter_target"] != true {
				t.Fatalf("Flate filter metadata = editable:%v pass_through:%v target:%v, want true/false/true", flateMeta["filter_editable"], flateMeta["filter_pass_through"], flateMeta["filter_target"])
			}
			if flateMeta["decoded_length"] != len(decoded) {
				t.Fatalf("Flate decoded_length = %v, want %d", flateMeta["decoded_length"], len(decoded))
			}

			plan, err := adapter.PlanEdit(tree, core.Match{Kind: KindTextShow, Text: "FLATE-TEXT"}, core.Mutation{Replace: "EDITED"})
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

			if _, err := adapter.PlanEdit(tree, core.Match{Kind: KindTextShow, Text: "IMAGE-ONLY"}, core.Mutation{Replace: "EDITED"}); err == nil {
				t.Fatal("expected edit planning for image-only text to fail closed")
			}
		})
	}
}

func TestImageXObjectMixedFilterArrayPassThroughDoesNotBlockFlateTextEdit(t *testing.T) {
	decoded := []byte("BT\n(FLATE-TEXT) Tj\nET\n")
	encoded, err := encodeFlateDecode(decoded)
	if err != nil {
		t.Fatal(err)
	}
	imageLike := []byte("BT\n(IMAGE-ONLY) Tj\nET\n")
	input := testPDF(
		"<< /Type /Page >>",
		fmt.Sprintf("<< /Type /XObject /Subtype /Image /Width 1 /Height 1 /ColorSpace /DeviceGray /BitsPerComponent 8 /Length %d /Filter [/ASCII85Decode /DCTDecode] >>\nstream\n%sendstream", len(imageLike), imageLike),
		fmt.Sprintf("<< /Length %d /Filter /FlateDecode >>\nstream\n%sendstream", len(encoded), encoded),
	)

	adapter := NewAdapter()
	tree, err := adapter.Parse(input, core.ParseOptions{Strict: true})
	if err != nil {
		t.Fatal(err)
	}
	if matches := tree.Query(core.Match{Kind: KindTextShow, Text: "IMAGE-ONLY"}); len(matches) != 0 {
		t.Fatalf("image text matches = %d, want 0", len(matches))
	}
	if matches := tree.Query(core.Match{Kind: KindTextShow, Text: "FLATE-TEXT"}); len(matches) != 1 {
		t.Fatalf("Flate text matches = %d, want 1", len(matches))
	}

	streams := tree.Query(core.Match{Kind: KindStream})
	if len(streams) != 2 {
		t.Fatalf("stream nodes = %d, want 2", len(streams))
	}
	imageMeta := streams[0].Meta
	if imageMeta["image_xobject"] != true {
		t.Fatalf("image stream metadata = %+v, want image_xobject=true", imageMeta)
	}
	if imageMeta["filter_capability"] != string(pdfStreamFilterCapabilityPassThroughImage) {
		t.Fatalf("image filter_capability = %v, want %q", imageMeta["filter_capability"], pdfStreamFilterCapabilityPassThroughImage)
	}
	if imageMeta["filter_editable"] != false || imageMeta["filter_pass_through"] != true || imageMeta["filter_target"] != false {
		t.Fatalf("image filter metadata = editable:%v pass_through:%v target:%v, want false/true/false", imageMeta["filter_editable"], imageMeta["filter_pass_through"], imageMeta["filter_target"])
	}
	if _, ok := imageMeta["unsupported"]; ok {
		t.Fatalf("unsupported metadata present for image pass-through stream: %+v", imageMeta)
	}
	if _, ok := imageMeta["decoded_length"]; ok {
		t.Fatalf("decoded_length present for image pass-through stream: %+v", imageMeta)
	}

	plan, err := adapter.PlanEdit(tree, core.Match{Kind: KindTextShow, Text: "FLATE-TEXT"}, core.Mutation{Replace: "EDITED"})
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
	if !bytes.Contains(output, imageLike) {
		t.Fatal("image stream bytes changed during page content edit")
	}
}

func TestCanonicalGraphImageXObjectMixedFilterArrayPassThroughDoesNotBlockFlateTextEdit(t *testing.T) {
	decoded := []byte("BT\n(FLATE-TEXT) Tj\nET\n")
	encoded, err := encodeFlateDecode(decoded)
	if err != nil {
		t.Fatal(err)
	}
	imageLike := []byte("BT\n(IMAGE-ONLY) Tj\nET\n")
	input := testPDF(
		"<< /Type /Page >>",
		fmt.Sprintf("<< /Type /XObject /Subtype /Image /Width 1 /Height 1 /ColorSpace /DeviceGray /BitsPerComponent 8 /Length %d /Filter [/ASCII85Decode /DCTDecode] >>\nstream\n%sendstream", len(imageLike), imageLike),
		fmt.Sprintf("<< /Length %d /Filter /FlateDecode >>\nstream\n%sendstream", len(encoded), encoded),
	)

	graph, err := parsePDFGraph(input)
	if err != nil {
		t.Fatal(err)
	}
	tree := graph.toTree(input)
	enrichPDFStreamNodeMetadata(tree)
	if matches := tree.Query(core.Match{Kind: KindTextShow, Text: "IMAGE-ONLY"}); len(matches) != 0 {
		t.Fatalf("image text matches = %d, want 0", len(matches))
	}
	if matches := tree.Query(core.Match{Kind: KindTextShow, Text: "FLATE-TEXT"}); len(matches) != 1 {
		t.Fatalf("Flate text matches = %d, want 1", len(matches))
	}
	streams := tree.Query(core.Match{Kind: KindStream})
	if len(streams) != 2 {
		t.Fatalf("stream nodes = %d, want 2", len(streams))
	}
	imageMeta := streams[0].Meta
	if imageMeta["filter_capability"] != string(pdfStreamFilterCapabilityPassThroughImage) {
		t.Fatalf("image filter_capability = %v, want %q", imageMeta["filter_capability"], pdfStreamFilterCapabilityPassThroughImage)
	}
	if imageMeta["filter_editable"] != false || imageMeta["filter_pass_through"] != true || imageMeta["filter_target"] != false {
		t.Fatalf("image filter metadata = editable:%v pass_through:%v target:%v, want false/true/false", imageMeta["filter_editable"], imageMeta["filter_pass_through"], imageMeta["filter_target"])
	}
	if _, ok := imageMeta["unsupported"]; ok {
		t.Fatalf("unsupported metadata present for image pass-through stream: %+v", imageMeta)
	}
	if _, ok := imageMeta["decoded_length"]; ok {
		t.Fatalf("decoded_length present for image pass-through stream: %+v", imageMeta)
	}

	output, _, verification, err := ApplyCanonicalEdit(input, core.Match{Kind: KindTextShow, Text: "FLATE-TEXT"}, core.Mutation{Replace: "EDITED"}, nil)
	if err != nil {
		t.Fatal(err)
	}
	if !verification.ReparseOK || !verification.OldTextRemoved || !verification.NewSelectable || !verification.PageUnchanged {
		t.Fatalf("verification failed: %+v", verification)
	}
	if !bytes.Contains(output, imageLike) {
		t.Fatal("image stream bytes changed during canonical content edit")
	}
	if bytes.Contains(output, []byte("IMAGE-ONLY")) && !bytes.Contains(output, imageLike) {
		t.Fatal("image stream text-like bytes changed unexpectedly")
	}
}

func TestImageXObjectRawStreamDoesNotProduceEditableTextShow(t *testing.T) {
	imageBytes := []byte("BT\n(IMAGE) Tj\nET\n")
	contentBytes := []byte("BT\n(PAGE) Tj\nET\n")
	input := testPDF(
		"<< /Type /Page >>",
		fmt.Sprintf("<< /Type /XObject /Subtype /Image /Width 1 /Height 1 /ColorSpace /DeviceGray /BitsPerComponent 8 /Length %d >>\nstream\n%sendstream", len(imageBytes), imageBytes),
		fmt.Sprintf("<< /Length %d >>\nstream\n%sendstream", len(contentBytes), contentBytes),
	)

	adapter := NewAdapter()
	tree, err := adapter.Parse(input, core.ParseOptions{Strict: true})
	if err != nil {
		t.Fatal(err)
	}

	streams := tree.Query(core.Match{Kind: KindStream})
	if len(streams) != 2 {
		t.Fatalf("stream nodes = %d, want 2", len(streams))
	}
	if streams[0].Meta["image_xobject"] != true {
		t.Fatalf("image stream metadata = %+v, want image_xobject=true", streams[0].Meta)
	}
	if got := streams[0].Span.Len(); got != int64(len(imageBytes)) {
		t.Fatalf("image stream span length = %d, want %d", got, len(imageBytes))
	}
	if matches := tree.Query(core.Match{Kind: KindTextShow, Text: "IMAGE"}); len(matches) != 0 {
		t.Fatalf("image text matches = %d, want 0", len(matches))
	}
	if _, err := adapter.PlanEdit(tree, core.Match{Kind: KindTextShow, Text: "IMAGE"}, core.Mutation{Replace: "EDITED"}); err == nil {
		t.Fatal("expected image XObject text edit planning to fail closed")
	}

	if matches := tree.Query(core.Match{Kind: KindTextShow, Text: "PAGE"}); len(matches) != 1 {
		t.Fatalf("page text matches = %d, want 1", len(matches))
	}
	plan, err := adapter.PlanEdit(tree, core.Match{Kind: KindTextShow, Text: "PAGE"}, core.Mutation{Replace: "EDITED"})
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
	if !bytes.Contains(output, imageBytes) {
		t.Fatal("image stream bytes changed during page content edit")
	}
	if !bytes.Contains(output, []byte("(EDITED) Tj")) {
		t.Fatal("page content stream was not rewritten")
	}
}

func TestCanonicalGraphImageXObjectRawContentReferenceDoesNotProduceEditableTextShow(t *testing.T) {
	imageBytes := []byte("BT\n(IMAGE) Tj\nET\n")
	contentBytes := []byte("BT\n(PAGE) Tj\nET\n")
	input := testPDF(
		"<< /Type /Page /Contents [2 0 R 3 0 R] >>",
		fmt.Sprintf("<< /Type /XObject /Subtype /Image /Width 1 /Height 1 /ColorSpace /DeviceGray /BitsPerComponent 8 /Length %d >>\nstream\n%sendstream", len(imageBytes), imageBytes),
		fmt.Sprintf("<< /Length %d >>\nstream\n%sendstream", len(contentBytes), contentBytes),
	)

	graph, err := parsePDFGraph(input)
	if err != nil {
		t.Fatal(err)
	}
	if candidates, err := graph.textShowCandidates("IMAGE"); err != nil {
		t.Fatal(err)
	} else if len(candidates) != 0 {
		t.Fatalf("image text candidates = %d, want 0", len(candidates))
	}
	if candidates, err := graph.textShowCandidates("PAGE"); err != nil {
		t.Fatal(err)
	} else if len(candidates) != 1 {
		t.Fatalf("page text candidates = %d, want 1", len(candidates))
	}

	if _, _, _, err := ApplyCanonicalEdit(input, core.Match{Kind: KindTextShow, Text: "IMAGE"}, core.Mutation{Replace: "EDITED"}, nil); err == nil {
		t.Fatal("expected canonical edit inside image XObject bytes to fail closed")
	}
	output, _, verification, err := ApplyCanonicalEdit(input, core.Match{Kind: KindTextShow, Text: "PAGE"}, core.Mutation{Replace: "EDITED"}, nil)
	if err != nil {
		t.Fatal(err)
	}
	if !verification.ReparseOK || !verification.OldTextRemoved || !verification.NewSelectable || !verification.PageUnchanged {
		t.Fatalf("verification failed: %+v", verification)
	}
	if !bytes.Contains(output, imageBytes) {
		t.Fatal("image stream bytes changed during canonical page content edit")
	}
}

func TestFilterCapabilityUnsupportedTargetDoesNotBlockFlateTextEdit(t *testing.T) {
	unsupportedBytes := []byte("BT\n(UNSUPPORTED-TEXT) Tj\nET\n")
	decoded := []byte("BT\n(FLATE-TEXT) Tj\nET\n")
	encoded, err := encodeFlateDecode(decoded)
	if err != nil {
		t.Fatal(err)
	}
	input := testPDF(
		"<< /Type /Page >>",
		fmt.Sprintf("<< /Length %d /Filter /FooDecode >>\nstream\n%sendstream", len(unsupportedBytes), unsupportedBytes),
		fmt.Sprintf("<< /Length %d /Filter /FlateDecode >>\nstream\n%sendstream", len(encoded), encoded),
	)

	adapter := NewAdapter()
	tree, err := adapter.Parse(input, core.ParseOptions{Strict: true})
	if err != nil {
		t.Fatal(err)
	}
	if matches := tree.Query(core.Match{Kind: KindTextShow, Text: "FLATE-TEXT"}); len(matches) != 1 {
		t.Fatalf("Flate text matches = %d, want 1", len(matches))
	}
	if matches := tree.Query(core.Match{Kind: KindTextShow, Text: "UNSUPPORTED-TEXT"}); len(matches) != 0 {
		t.Fatalf("unsupported stream text matches = %d, want 0", len(matches))
	}

	streams := tree.Query(core.Match{Kind: KindStream})
	if len(streams) != 2 {
		t.Fatalf("stream nodes = %d, want 2", len(streams))
	}
	unsupportedMeta := streams[0].Meta
	if unsupportedMeta["unsupported"] != `unsupported PDF stream filter "FooDecode"` {
		t.Fatalf("unsupported metadata = %v, want FooDecode filter error", unsupportedMeta["unsupported"])
	}
	if unsupportedMeta["filter_capability"] != string(pdfStreamFilterCapabilityUnsupportedTarget) {
		t.Fatalf("filter_capability = %v, want %q", unsupportedMeta["filter_capability"], pdfStreamFilterCapabilityUnsupportedTarget)
	}
	if unsupportedMeta["filter_editable"] != false || unsupportedMeta["filter_pass_through"] != false || unsupportedMeta["filter_target"] != true {
		t.Fatalf("unsupported filter metadata = editable:%v pass_through:%v target:%v, want false/false/true", unsupportedMeta["filter_editable"], unsupportedMeta["filter_pass_through"], unsupportedMeta["filter_target"])
	}
	if _, ok := unsupportedMeta["decoded_length"]; ok {
		t.Fatalf("decoded_length present for unsupported target stream: %+v", unsupportedMeta)
	}
	if _, err := adapter.PlanEdit(tree, core.Match{Kind: KindTextShow, Text: "UNSUPPORTED-TEXT"}, core.Mutation{Replace: "EDITED"}); err == nil {
		t.Fatal("expected edit planning for unsupported target text to fail closed")
	}

	plan, err := adapter.PlanEdit(tree, core.Match{Kind: KindTextShow, Text: "FLATE-TEXT"}, core.Mutation{Replace: "EDITED"})
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
}

func TestFilterCapabilityUnsupportedCryptTargetDoesNotProduceEditableText(t *testing.T) {
	cryptBytes := []byte("BT\n(CRYPT-TEXT) Tj\nET\n")
	decoded := []byte("BT\n(FLATE-TEXT) Tj\nET\n")
	encoded, err := encodeFlateDecode(decoded)
	if err != nil {
		t.Fatal(err)
	}
	input := testPDF(
		"<< /Type /Page >>",
		fmt.Sprintf("<< /Length %d /Filter /Crypt >>\nstream\n%sendstream", len(cryptBytes), cryptBytes),
		fmt.Sprintf("<< /Length %d /Filter /FlateDecode >>\nstream\n%sendstream", len(encoded), encoded),
	)

	adapter := NewAdapter()
	tree, err := adapter.Parse(input, core.ParseOptions{Strict: true})
	if err != nil {
		t.Fatal(err)
	}
	if matches := tree.Query(core.Match{Kind: KindTextShow, Text: "CRYPT-TEXT"}); len(matches) != 0 {
		t.Fatalf("crypt stream text matches = %d, want 0", len(matches))
	}
	if _, err := adapter.PlanEdit(tree, core.Match{Kind: KindTextShow, Text: "CRYPT-TEXT"}, core.Mutation{Replace: "EDITED"}); err == nil {
		t.Fatal("expected edit planning for unsupported crypt target text to fail closed")
	}
	if matches := tree.Query(core.Match{Kind: KindTextShow, Text: "FLATE-TEXT"}); len(matches) != 1 {
		t.Fatalf("Flate text matches = %d, want 1", len(matches))
	}

	streams := tree.Query(core.Match{Kind: KindStream})
	if len(streams) != 2 {
		t.Fatalf("stream nodes = %d, want 2", len(streams))
	}
	cryptMeta := streams[0].Meta
	if cryptMeta["filter_capability"] != string(pdfStreamFilterCapabilityUnsupportedTarget) {
		t.Fatalf("crypt filter_capability = %v, want %q", cryptMeta["filter_capability"], pdfStreamFilterCapabilityUnsupportedTarget)
	}
	if cryptMeta["filter_editable"] != false || cryptMeta["filter_pass_through"] != false || cryptMeta["filter_target"] != true {
		t.Fatalf("crypt filter metadata = editable:%v pass_through:%v target:%v, want false/false/true", cryptMeta["filter_editable"], cryptMeta["filter_pass_through"], cryptMeta["filter_target"])
	}
	if cryptMeta["decoded_length"] != len(cryptBytes) {
		t.Fatalf("crypt decoded_length = %v, want %d", cryptMeta["decoded_length"], len(cryptBytes))
	}
}

func TestFilterCapabilityExplicitCryptIdentityTargetRemainsEditable(t *testing.T) {
	content := []byte("BT\n(CRYPT-IDENTITY) Tj\nET\n")
	input := testPDF(
		"<< /Type /Page >>",
		fmt.Sprintf("<< /Length %d /Filter /Crypt /DecodeParms << /Name /Identity >> >>\nstream\n%sendstream", len(content), content),
	)

	adapter := NewAdapter()
	tree, err := adapter.Parse(input, core.ParseOptions{Strict: true})
	if err != nil {
		t.Fatal(err)
	}
	if matches := tree.Query(core.Match{Kind: KindTextShow, Text: "CRYPT-IDENTITY"}); len(matches) != 1 {
		t.Fatalf("crypt identity text matches = %d, want 1", len(matches))
	}
	plan, err := adapter.PlanEdit(tree, core.Match{Kind: KindTextShow, Text: "CRYPT-IDENTITY"}, core.Mutation{Replace: "CRYPT-EDITED"})
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
}

func TestCanonicalGraphUnsupportedTargetFailsClosedButDoesNotBlockFlateTextEdit(t *testing.T) {
	unsupportedBytes := []byte("BT\n(UNSUPPORTED-TEXT) Tj\nET\n")
	decoded := []byte("BT\n(FLATE-TEXT) Tj\nET\n")
	encoded, err := encodeFlateDecode(decoded)
	if err != nil {
		t.Fatal(err)
	}
	input := testPDF(
		"<< /Type /Page >>",
		fmt.Sprintf("<< /Length %d /Filter /FooDecode >>\nstream\n%sendstream", len(unsupportedBytes), unsupportedBytes),
		fmt.Sprintf("<< /Length %d /Filter /FlateDecode >>\nstream\n%sendstream", len(encoded), encoded),
	)

	graph, err := parsePDFGraph(input)
	if err != nil {
		t.Fatal(err)
	}
	tree := graph.toTree(input)
	enrichPDFStreamNodeMetadata(tree)
	streams := tree.Query(core.Match{Kind: KindStream})
	if len(streams) != 2 {
		t.Fatalf("stream nodes = %d, want 2", len(streams))
	}
	unsupportedMeta := streams[0].Meta
	if unsupportedMeta["filter_capability"] != string(pdfStreamFilterCapabilityUnsupportedTarget) {
		t.Fatalf("unsupported filter_capability = %v, want %q", unsupportedMeta["filter_capability"], pdfStreamFilterCapabilityUnsupportedTarget)
	}
	if unsupportedMeta["filter_editable"] != false || unsupportedMeta["filter_pass_through"] != false || unsupportedMeta["filter_target"] != true {
		t.Fatalf("unsupported filter metadata = editable:%v pass_through:%v target:%v, want false/false/true", unsupportedMeta["filter_editable"], unsupportedMeta["filter_pass_through"], unsupportedMeta["filter_target"])
	}
	if got := fmt.Sprint(unsupportedMeta["unsupported"]); !strings.Contains(got, "FooDecode") {
		t.Fatalf("unsupported metadata = %v, want FooDecode", unsupportedMeta["unsupported"])
	}
	if _, ok := unsupportedMeta["decoded_length"]; ok {
		t.Fatalf("decoded_length present for unsupported target stream: %+v", unsupportedMeta)
	}
	if matches := tree.Query(core.Match{Kind: KindTextShow, Text: "UNSUPPORTED-TEXT"}); len(matches) != 0 {
		t.Fatalf("unsupported stream text matches = %d, want 0", len(matches))
	}

	if _, _, _, err := ApplyCanonicalEdit(input, core.Match{Kind: KindTextShow, Text: "UNSUPPORTED-TEXT"}, core.Mutation{Replace: "EDITED"}, nil); err == nil {
		t.Fatal("expected canonical edit inside unsupported target stream to fail closed")
	}
	_, _, verification, err := ApplyCanonicalEdit(input, core.Match{Kind: KindTextShow, Text: "FLATE-TEXT"}, core.Mutation{Replace: "EDITED"}, nil)
	if err != nil {
		t.Fatal(err)
	}
	if !verification.ReparseOK || !verification.OldTextRemoved || !verification.NewSelectable || !verification.PageUnchanged {
		t.Fatalf("verification failed: %+v", verification)
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
