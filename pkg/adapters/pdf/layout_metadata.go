package pdf

import "fmt"

const (
	layoutProofStatusUnknown        = "unknown"
	layoutProofStatusWidthProven    = "width_proven"
	layoutProofStatusWidthUnproven  = "width_unproven"
	layoutProofStatusReflowRequired = "reflow_required"
)

type textReplacementEncodingProof struct {
	CMapReverseEncoded bool
	MaxCMapCodeBytes   int
}

func annotateTextShowLayoutProofMetadata(meta map[string]any, oldWidthUnits, newWidthUnits *int) {
	if meta == nil {
		return
	}
	if oldWidthUnits == nil || newWidthUnits == nil {
		meta["layout_proof"] = layoutProofStatusUnknown
		delete(meta, "width_delta_units")
		return
	}

	delta := *newWidthUnits - *oldWidthUnits
	meta["width_delta_units"] = delta
	if delta == 0 {
		meta["layout_proof"] = layoutProofStatusWidthProven
		return
	}
	meta["layout_proof"] = layoutProofStatusReflowRequired
}

func rejectUnsupportedTextReplacementLayout(meta map[string]any, proof textReplacementEncodingProof) error {
	layoutProof, _ := meta["layout_proof"].(string)
	switch layoutProof {
	case layoutProofStatusReflowRequired:
		return fmt.Errorf(
			"unsupported PDF text replacement: replacement changes text width and requires layout/reflow support (layout_proof=%s old_width_units=%v new_width_units=%v width_delta_units=%v)",
			layoutProof,
			meta["old_width_units"],
			meta["new_width_units"],
			meta["width_delta_units"],
		)
	case layoutProofStatusWidthUnproven:
		return fmt.Errorf("unsupported PDF text replacement: replacement width cannot be proven without layout/reflow support (layout_proof=%s)", layoutProof)
	}
	if proof.CMapReverseEncoded && proof.MaxCMapCodeBytes > 1 && layoutProof != layoutProofStatusWidthProven {
		if layoutProof == "" {
			layoutProof = layoutProofStatusUnknown
		}
		return fmt.Errorf(
			"unsupported PDF text replacement: multi-byte CMap reverse encoding requires proven equal-width layout metadata (layout_proof=%s max_cmap_code_bytes=%d)",
			layoutProof,
			proof.MaxCMapCodeBytes,
		)
	}
	return nil
}
