package pdf

import (
	"fmt"
	"strings"
)

const (
	layoutProofStatusUnknown        = "unknown"
	layoutProofStatusWidthProven    = "width_proven"
	layoutProofStatusWidthNarrower  = "width_narrower"
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
	if delta < 0 {
		meta["layout_proof"] = layoutProofStatusWidthNarrower
		return
	}
	meta["layout_proof"] = layoutProofStatusReflowRequired
}

func textShowReplacementReportMetadata(nodeMeta map[string]any, layoutMeta map[string]any) map[string]any {
	out := copyLayoutReportMetadata(layoutMeta)
	if out == nil {
		out = map[string]any{
			"layout_proof": layoutProofStatusUnknown,
		}
	}
	enrichTextDecodeReportMetadata(out, nodeMeta)
	return out
}

func copyLayoutReportMetadata(meta map[string]any) map[string]any {
	if len(meta) == 0 {
		return nil
	}
	out := make(map[string]any, len(meta))
	for key, value := range meta {
		out[key] = value
	}
	return out
}

func enrichTextDecodeReportMetadata(out map[string]any, nodeMeta map[string]any) {
	if out == nil || nodeMeta == nil {
		return
	}
	if encoding, ok := nodeMeta["encoding"].(string); ok && encoding != "" {
		out["encoding"] = encoding
		out["encoding_path"] = textShowEncodingPath(encoding)
		out["text_decode_source"] = textShowDecodeSource(encoding)
	}
	if font, ok := nodeMeta["font"].(string); ok && font != "" {
		out["font_id"] = font
	}
}

func textShowDecodeSource(encoding string) string {
	switch encoding {
	case "literal":
		return "pdf_literal_string"
	case "hex":
		return "pdf_hex_string"
	case "tj-array":
		return "pdf_tj_array"
	case "hex-cmap", "tj-array-cmap":
		return "to_unicode_cmap"
	case "hex-font-encoding", "literal-font-encoding", "tj-array-font-encoding":
		return "simple_font_encoding"
	default:
		return "unknown"
	}
}

func textShowEncodingPath(encoding string) string {
	switch encoding {
	case "literal":
		return "text_show/literal"
	case "hex":
		return "text_show/hex"
	case "tj-array":
		return "text_show/tj_array"
	case "hex-cmap":
		return "text_show/hex/to_unicode_cmap"
	case "tj-array-cmap":
		return "text_show/tj_array/to_unicode_cmap"
	case "hex-font-encoding":
		return "text_show/hex/simple_font_encoding"
	case "literal-font-encoding":
		return "text_show/literal/simple_font_encoding"
	case "tj-array-font-encoding":
		return "text_show/tj_array/simple_font_encoding"
	default:
		return "text_show/unknown"
	}
}

func rejectUnsupportedTextReplacementLayout(meta map[string]any, proof textReplacementEncodingProof) error {
	layoutProof, _ := meta["layout_proof"].(string)
	switch layoutProof {
	case layoutProofStatusReflowRequired:
		return fmt.Errorf(
			"unsupported PDF text replacement: replacement changes text width and requires layout/reflow support (%s)",
			textReplacementLayoutMetadataSummary(meta),
		)
	case layoutProofStatusWidthUnproven:
		return fmt.Errorf("unsupported PDF text replacement: replacement width cannot be proven without layout/reflow support (%s)", textReplacementLayoutMetadataSummary(meta))
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

func textReplacementLayoutMetadataSummary(meta map[string]any) string {
	if meta == nil {
		return "layout_proof=unknown"
	}
	keys := []string{
		"layout_proof",
		"old_width_units",
		"new_width_units",
		"width_delta_units",
		"text_decode_source",
		"font_id",
		"encoding_path",
	}
	parts := make([]string, 0, len(keys))
	for _, key := range keys {
		if value, ok := meta[key]; ok && value != nil && fmt.Sprint(value) != "" {
			parts = append(parts, fmt.Sprintf("%s=%v", key, value))
		}
	}
	if len(parts) == 0 {
		return "layout_proof=unknown"
	}
	return strings.Join(parts, " ")
}
