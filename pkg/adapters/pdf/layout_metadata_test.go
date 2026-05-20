package pdf

import "testing"

func TestLayoutMetadataAnnotatesKnownEqualWidthAsProven(t *testing.T) {
	meta := map[string]any{}
	oldWidth := 1200
	newWidth := 1200

	annotateTextShowLayoutProofMetadata(meta, &oldWidth, &newWidth)

	if meta["layout_proof"] != layoutProofStatusWidthProven {
		t.Fatalf("layout_proof = %v, want %q", meta["layout_proof"], layoutProofStatusWidthProven)
	}
	if meta["width_delta_units"] != 0 {
		t.Fatalf("width_delta_units = %v, want 0", meta["width_delta_units"])
	}
}

func TestLayoutMetadataAnnotatesKnownChangedWidthAsReflowRequired(t *testing.T) {
	meta := map[string]any{}
	oldWidth := 1200
	newWidth := 1333

	annotateTextShowLayoutProofMetadata(meta, &oldWidth, &newWidth)

	if meta["layout_proof"] != layoutProofStatusReflowRequired {
		t.Fatalf("layout_proof = %v, want %q", meta["layout_proof"], layoutProofStatusReflowRequired)
	}
	if meta["width_delta_units"] != 133 {
		t.Fatalf("width_delta_units = %v, want 133", meta["width_delta_units"])
	}
}

func TestLayoutMetadataAnnotatesUnknownWidthWithoutDelta(t *testing.T) {
	meta := map[string]any{}
	oldWidth := 1200

	annotateTextShowLayoutProofMetadata(meta, &oldWidth, nil)

	if meta["layout_proof"] != layoutProofStatusUnknown {
		t.Fatalf("layout_proof = %v, want %q", meta["layout_proof"], layoutProofStatusUnknown)
	}
	if _, ok := meta["width_delta_units"]; ok {
		t.Fatalf("width_delta_units present for unknown width metadata: %+v", meta)
	}
}
