package pdf

const (
	layoutProofStatusUnknown        = "unknown"
	layoutProofStatusWidthProven    = "width_proven"
	layoutProofStatusWidthUnproven  = "width_unproven"
	layoutProofStatusReflowRequired = "reflow_required"
)

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
