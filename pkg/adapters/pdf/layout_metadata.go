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

const (
	textEditabilityStatusReplaceableCandidate = "replaceable_candidate"
	textEditabilityStatusReplaceable          = "replaceable"
	textEditabilityStatusUnsupported          = "unsupported"

	textWidthProofStatusKnown          = "known"
	textWidthProofStatusEqual          = "equal"
	textWidthProofStatusNarrower       = "narrower"
	textWidthProofStatusReflowRequired = "reflow_required"
	textWidthProofStatusUnproven       = "unproven"
	textWidthProofStatusUnknown        = "unknown"

	textReplacementUnsupportedReflowRequired               = "reflow_required"
	textReplacementUnsupportedWidthUnproven                = "width_unproven"
	textReplacementUnsupportedCMapMultiByteNeedsWidthProof = "cmap_multibyte_requires_width_proof"
	textReplacementUnsupportedCMapUnrepresentable          = "cmap_unrepresentable"
	textReplacementUnsupportedFontEncodingUnrepresentable  = "font_encoding_unrepresentable"
	textReplacementUnsupportedEncoding                     = "unsupported_encoding"
	textReplacementUnsupportedSharedFormXObject            = "shared_form_xobject"
)

type textReplacementEncodingProof struct {
	CMapReverseEncoded bool
	MaxCMapCodeBytes   int
}

type TextReplacementUnsupportedError struct {
	Reason   string
	Metadata map[string]any
	message  string
}

func (e *TextReplacementUnsupportedError) Error() string {
	if e == nil {
		return ""
	}
	return e.message
}

func annotateTextShowLayoutProofMetadata(meta map[string]any, oldWidthUnits, newWidthUnits *int) {
	if meta == nil {
		return
	}
	if oldWidthUnits == nil || newWidthUnits == nil {
		meta["layout_proof"] = layoutProofStatusUnknown
		meta["width_proof"] = textWidthProofStatusUnknown
		delete(meta, "width_delta_units")
		return
	}

	delta := *newWidthUnits - *oldWidthUnits
	meta["width_delta_units"] = delta
	if delta == 0 {
		meta["layout_proof"] = layoutProofStatusWidthProven
		meta["width_proof"] = textWidthProofStatusEqual
		meta["reflow_required"] = false
		return
	}
	if delta < 0 {
		meta["layout_proof"] = layoutProofStatusWidthNarrower
		meta["width_proof"] = textWidthProofStatusNarrower
		meta["reflow_required"] = false
		return
	}
	meta["layout_proof"] = layoutProofStatusReflowRequired
	meta["width_proof"] = textWidthProofStatusReflowRequired
	meta["reflow_required"] = true
}

func textShowReplacementReportMetadata(nodeMeta map[string]any, layoutMeta map[string]any) map[string]any {
	out := copyLayoutReportMetadata(layoutMeta)
	if out == nil {
		out = map[string]any{
			"layout_proof": layoutProofStatusUnknown,
			"width_proof":  textWidthProofStatusUnknown,
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
	if source, ok := nodeMeta["font_metrics_source"].(string); ok && source != "" {
		out["font_metrics_source"] = source
	}
	if widthSource, ok := nodeMeta["width_source"].(string); ok && widthSource != "" {
		out["width_source"] = widthSource
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
		message := fmt.Sprintf(
			"unsupported PDF text replacement: replacement changes text width and requires layout/reflow support (%s)",
			textReplacementLayoutMetadataSummary(meta),
		)
		return unsupportedTextReplacementError(textReplacementUnsupportedReflowRequired, message, meta)
	case layoutProofStatusWidthUnproven:
		message := fmt.Sprintf("unsupported PDF text replacement: replacement width cannot be proven without layout/reflow support (%s)", textReplacementLayoutMetadataSummary(meta))
		return unsupportedTextReplacementError(textReplacementUnsupportedWidthUnproven, message, meta)
	}
	if proof.CMapReverseEncoded && proof.MaxCMapCodeBytes > 1 && layoutProof != layoutProofStatusWidthProven {
		if layoutProof == "" {
			layoutProof = layoutProofStatusUnknown
		}
		meta = copyLayoutReportMetadata(meta)
		if meta == nil {
			meta = map[string]any{}
		}
		meta["cmap_reverse_encoding"] = true
		meta["max_cmap_code_bytes"] = proof.MaxCMapCodeBytes
		message := fmt.Sprintf(
			"unsupported PDF text replacement: multi-byte CMap reverse encoding requires proven equal-width layout metadata (layout_proof=%s max_cmap_code_bytes=%d)",
			layoutProof,
			proof.MaxCMapCodeBytes,
		)
		return unsupportedTextReplacementError(textReplacementUnsupportedCMapMultiByteNeedsWidthProof, message, meta)
	}
	return nil
}

func markTextReplacementSupportedMetadata(meta map[string]any, proof textReplacementEncodingProof) {
	if meta == nil {
		return
	}
	meta["text_editability_status"] = textEditabilityStatusReplaceable
	if proof.CMapReverseEncoded {
		meta["cmap_reverse_encoding"] = true
		meta["max_cmap_code_bytes"] = proof.MaxCMapCodeBytes
	}
}

func unsupportedTextReplacementError(reason, message string, meta map[string]any) error {
	out := copyLayoutReportMetadata(meta)
	if out == nil {
		out = map[string]any{}
	}
	out["text_editability_status"] = textEditabilityStatusUnsupported
	out["unsupported_reason"] = reason
	return &TextReplacementUnsupportedError{
		Reason:   reason,
		Metadata: out,
		message:  message,
	}
}

func unsupportedTextReplacementEncodingError(err error, meta map[string]any) error {
	if err == nil {
		return nil
	}
	reason := textReplacementUnsupportedEncoding
	message := err.Error()
	switch {
	case strings.Contains(message, "ToUnicode"):
		reason = textReplacementUnsupportedCMapUnrepresentable
	case strings.Contains(message, "font encoding"):
		reason = textReplacementUnsupportedFontEncodingUnrepresentable
	}
	return unsupportedTextReplacementError(reason, message, meta)
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
		"font_metrics_source",
		"width_source",
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
