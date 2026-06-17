package pdfapi

import (
	"bytes"
	"compress/zlib"
	"fmt"
	"io"
	"strings"
	"testing"
)

func TestProfileReportsSimpleEditablePDF(t *testing.T) {
	input := testPDFAPIFile("<< /Type /Page >>", streamObject("alpha"))

	profile, err := Profile(input, ProfileOptions{})
	if err != nil {
		t.Fatal(err)
	}

	if profile.Format != "pdf" || !profile.Valid {
		t.Fatalf("profile validity = format %q valid %v", profile.Format, profile.Valid)
	}
	if !profile.Editable || profile.Text.NodeCount != 1 || !profile.Text.CanEdit {
		t.Fatalf("text/edit profile = %+v editable=%v", profile.Text, profile.Editable)
	}
	if profile.Fillable || profile.Forms.FieldCount != 0 {
		t.Fatalf("form profile = %+v fillable=%v, want no form capabilities", profile.Forms, profile.Fillable)
	}
	if profile.Streams.TotalCount != 1 || profile.Streams.UnsupportedCount != 0 {
		t.Fatalf("stream profile = %+v, want one supported raw stream", profile.Streams)
	}
	if profile.RewriteRecommendation != RewriteModeCanonical {
		t.Fatalf("rewrite recommendation = %q, want canonical", profile.RewriteRecommendation)
	}
	if len(profile.UnsupportedReasons) != 0 {
		t.Fatalf("unsupported reasons = %v, want none", profile.UnsupportedReasons)
	}
}

func TestProfileReportsUnsupportedFilterBlockersWithoutInvalidatingWholeFile(t *testing.T) {
	input := testPDFAPIFile(
		"<< /Type /Page >>",
		streamObject("editable text"),
		fmt.Sprintf("<< /Length %d /Filter /FooDecode >>\nstream\n%s\nendstream", len("not-foo"), "not-foo"),
	)

	profile, err := Profile(input, ProfileOptions{})
	if err != nil {
		t.Fatal(err)
	}

	if !profile.Valid {
		t.Fatalf("profile valid = false, unsupported reasons = %v", profile.UnsupportedReasons)
	}
	if !profile.Editable || profile.Text.NodeCount != 1 {
		t.Fatalf("text/edit profile = %+v editable=%v, want text edit still available", profile.Text, profile.Editable)
	}
	if profile.Streams.TotalCount != 2 || profile.Streams.UnsupportedCount != 1 || profile.Streams.PassThroughCount != 1 {
		t.Fatalf("stream profile = %+v, want raw pass-through stream plus unsupported filtered target", profile.Streams)
	}
	if profile.Streams.FilterCounts["FooDecode"] != 1 {
		t.Fatalf("filter counts = %+v, want FooDecode=1", profile.Streams.FilterCounts)
	}
	if !hasUnsupportedReason(profile.UnsupportedReasons, "unsupported_stream_filters:1") {
		t.Fatalf("unsupported reasons = %v, want unsupported stream blocker", profile.UnsupportedReasons)
	}
	if profile.RewriteRecommendation != RewriteModePreserveStructure {
		t.Fatalf("rewrite recommendation = %q, want preserve-structure", profile.RewriteRecommendation)
	}
}

func TestProfileReportsPassThroughImageLikeFilterWithoutBlockingTextEditing(t *testing.T) {
	input := testPDFAPIFile(
		"<< /Type /Page >>",
		streamObject("editable text"),
		fmt.Sprintf("<< /Length %d /Filter /DCTDecode >>\nstream\n%s\nendstream", len("not-jpeg"), "not-jpeg"),
	)

	profile, err := Profile(input, ProfileOptions{})
	if err != nil {
		t.Fatal(err)
	}

	if !profile.Valid || !profile.Editable {
		t.Fatalf("profile valid/editable = %v/%v unsupported=%v", profile.Valid, profile.Editable, profile.UnsupportedReasons)
	}
	if profile.Streams.TotalCount != 2 || profile.Streams.UnsupportedCount != 0 || profile.Streams.PassThroughCount != 2 {
		t.Fatalf("stream profile = %+v, want raw stream plus pass-through filtered stream with no unsupported target", profile.Streams)
	}
	if len(profile.UnsupportedReasons) != 0 {
		t.Fatalf("unsupported reasons = %v, want pass-through filter to stay non-blocking", profile.UnsupportedReasons)
	}
	if profile.RewriteRecommendation != RewriteModeCanonical {
		t.Fatalf("rewrite recommendation = %q, want canonical", profile.RewriteRecommendation)
	}
}

func TestProfileReportsFormCapabilities(t *testing.T) {
	input := testPDFAPIFile(
		"<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [4 0 R 5 0 R] >> >>",
		"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
		"<< /Type /Page /Parent 2 0 R >>",
		"<< /T (payer.name) /FT /Tx /V (Old Name) /Rect [0 0 100 20] >>",
		"<< /T (payer.locked) /FT /Tx /Ff 1 /V (Locked) /Rect [0 0 100 20] >>",
	)

	profile, err := Profile(input, ProfileOptions{})
	if err != nil {
		t.Fatal(err)
	}

	if !profile.Valid || !profile.Markers.AcroForm || !profile.Forms.HasAcroForm {
		t.Fatalf("form markers/profile = markers %+v forms %+v valid=%v", profile.Markers, profile.Forms, profile.Valid)
	}
	if !profile.Fillable || profile.Forms.FieldCount != 2 || profile.Forms.FillableCount != 1 || profile.Forms.BlockerCount != 1 {
		t.Fatalf("form capabilities = %+v fillable=%v, want 2 fields, 1 fillable, 1 blocker", profile.Forms, profile.Fillable)
	}
	if profile.RewriteRecommendation != RewriteModeCanonical {
		t.Fatalf("rewrite recommendation = %q, want canonical", profile.RewriteRecommendation)
	}
}

func TestProfileUsesOptions(t *testing.T) {
	input := testPDFAPIFile("<< /Type /Page >>", flateStreamObject(t, "compressed text"))

	profile, err := Profile(input, ProfileOptions{Options: Options{Rewrite: RewriteModePreserveStructure}})
	if err != nil {
		t.Fatal(err)
	}
	if !profile.Valid || profile.Text.NodeCount != 1 {
		t.Fatalf("profile = %+v, want valid compressed text profile", profile)
	}
	if profile.Streams.EditableCount != 1 || profile.Streams.FilterCounts["FlateDecode"] != 1 {
		t.Fatalf("stream profile = %+v, want editable FlateDecode stream", profile.Streams)
	}
}

func flateStreamObject(t *testing.T, text string) string {
	t.Helper()
	content := []byte(fmt.Sprintf("BT\n(%s) Tj\nET\n", text))
	var encoded bytes.Buffer
	writer := zlib.NewWriter(&encoded)
	if _, err := writer.Write(content); err != nil {
		t.Fatal(err)
	}
	if err := writer.Close(); err != nil {
		t.Fatal(err)
	}
	if _, err := zlib.NewReader(bytes.NewReader(encoded.Bytes())); err != nil && err != io.EOF {
		t.Fatal(err)
	}
	return fmt.Sprintf("<< /Length %d /Filter /FlateDecode >>\nstream\n%sendstream", encoded.Len(), encoded.String())
}

func hasUnsupportedReason(reasons []string, prefix string) bool {
	for _, reason := range reasons {
		if strings.HasPrefix(reason, prefix) {
			return true
		}
	}
	return false
}
