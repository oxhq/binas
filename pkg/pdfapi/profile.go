package pdfapi

import (
	"bytes"
	"fmt"

	"github.com/oxhq/binas/pkg/adapters/pdf"
	"github.com/oxhq/binas/pkg/core"
)

type ProfileOptions struct {
	Options
}

type ProfileReport struct {
	Format                string                    `json:"format"`
	Valid                 bool                      `json:"valid"`
	Editable              bool                      `json:"editable"`
	Fillable              bool                      `json:"fillable"`
	RewriteRecommendation RewriteMode               `json:"rewrite_recommendation"`
	UnsupportedReasons    []string                  `json:"unsupported_reasons,omitempty"`
	Markers               ProfileMarkers            `json:"markers"`
	Text                  ProfileTextCapabilities   `json:"text"`
	Streams               ProfileStreamCapabilities `json:"streams"`
	Forms                 ProfileFormCapabilities   `json:"forms"`
	Annotations           ProfileAnnotationSummary  `json:"annotations"`
}

type ProfileMarkers struct {
	Encrypted   bool `json:"encrypted"`
	Signed      bool `json:"signed"`
	XFA         bool `json:"xfa"`
	AcroForm    bool `json:"acroform"`
	Annotations bool `json:"annotations"`
	Fonts       bool `json:"fonts"`
	CMaps       bool `json:"cmaps"`
	ToUnicode   bool `json:"to_unicode"`
	CIDFonts    bool `json:"cid_fonts"`
}

type ProfileTextCapabilities struct {
	NodeCount     int  `json:"node_count"`
	EditableCount int  `json:"editable_count"`
	CanEdit       bool `json:"can_edit"`
}

type ProfileStreamCapabilities struct {
	TotalCount       int            `json:"total_count"`
	EditableCount    int            `json:"editable_count"`
	PassThroughCount int            `json:"pass_through_count"`
	UnsupportedCount int            `json:"unsupported_count"`
	FilterCounts     map[string]int `json:"filter_counts,omitempty"`
	CapabilityCounts map[string]int `json:"capability_counts,omitempty"`
}

type ProfileFormCapabilities struct {
	HasAcroForm   bool `json:"has_acroform"`
	FieldCount    int  `json:"field_count"`
	FillableCount int  `json:"fillable_count"`
	BlockerCount  int  `json:"blocker_count"`
}

type ProfileAnnotationSummary struct {
	Present       bool `json:"present"`
	Count         int  `json:"count"`
	EditableCount int  `json:"editable_count"`
	BlockerCount  int  `json:"blocker_count"`
}

func Profile(input []byte, opts ProfileOptions) (ProfileReport, error) {
	report := ProfileReport{
		Format:                "pdf",
		RewriteRecommendation: RewriteModePreserveStructure,
		Streams: ProfileStreamCapabilities{
			FilterCounts:     map[string]int{},
			CapabilityCounts: map[string]int{},
		},
	}
	report.Markers = scanProfileMarkers(input)
	if err := validateRewriteMode(opts.Rewrite); err != nil {
		report.Valid = false
		report.UnsupportedReasons = append(report.UnsupportedReasons, err.Error())
		return report, nil
	}

	tree, err := Inspect(input, opts.Options)
	if err != nil {
		report.Valid = false
		report.UnsupportedReasons = append(report.UnsupportedReasons, err.Error())
		return report, nil
	}
	report.Valid = true
	report.Markers = mergeProfileTreeMarkers(report.Markers, tree)
	report.Text = profileTextCapabilities(tree)
	report.Streams = profileStreamCapabilities(tree)

	if report.Markers.AcroForm {
		report.Forms.HasAcroForm = true
		if fields, err := pdf.ListFormFields(input); err == nil {
			report.Forms.FieldCount = len(fields)
			for _, field := range fields {
				if profileFormFieldFillable(field) {
					report.Forms.FillableCount++
				} else {
					report.Forms.BlockerCount++
				}
			}
		} else {
			report.Forms.BlockerCount++
			report.UnsupportedReasons = append(report.UnsupportedReasons, "form_profile_unavailable: "+err.Error())
		}
	}

	if report.Markers.Annotations {
		report.Annotations.Present = true
		if annotations, err := pdf.ListAnnotationCandidates(input); err == nil {
			report.Annotations.Count = len(annotations)
			for _, annotation := range annotations {
				if annotation.AppearanceGenerationStatus == "approximate_supported" {
					report.Annotations.EditableCount++
				} else {
					report.Annotations.BlockerCount++
				}
			}
		} else {
			report.Annotations.BlockerCount++
			report.UnsupportedReasons = append(report.UnsupportedReasons, "annotation_profile_unavailable: "+err.Error())
		}
	}

	report.UnsupportedReasons = append(report.UnsupportedReasons, profileMarkerUnsupportedReasons(report.Markers)...)
	report.UnsupportedReasons = append(report.UnsupportedReasons, profileStreamUnsupportedReasons(report.Streams)...)
	report.Editable = report.Valid && report.Text.CanEdit && !report.Markers.Encrypted && !report.Markers.Signed && !report.Markers.XFA
	report.Fillable = report.Valid && report.Forms.FillableCount > 0 && !report.Markers.Encrypted && !report.Markers.Signed && !report.Markers.XFA
	report.RewriteRecommendation = profileRewriteRecommendation(report)
	return report, nil
}

func scanProfileMarkers(input []byte) ProfileMarkers {
	return ProfileMarkers{
		Encrypted:   bytes.Contains(input, []byte("/Encrypt")),
		Signed:      bytes.Contains(input, []byte("/Sig")) || bytes.Contains(input, []byte("/ByteRange")),
		XFA:         bytes.Contains(input, []byte("/XFA")),
		AcroForm:    bytes.Contains(input, []byte("/AcroForm")),
		Annotations: bytes.Contains(input, []byte("/Annots")),
		Fonts:       bytes.Contains(input, []byte("/Font")),
		CMaps:       bytes.Contains(input, []byte("/CMap")),
		ToUnicode:   bytes.Contains(input, []byte("/ToUnicode")),
		CIDFonts:    bytes.Contains(input, []byte("/CIDFontType0")) || bytes.Contains(input, []byte("/CIDFontType2")) || bytes.Contains(input, []byte("/CIDToGIDMap")),
	}
}

func mergeProfileTreeMarkers(markers ProfileMarkers, tree *core.Tree) ProfileMarkers {
	if tree == nil {
		return markers
	}
	root, ok := tree.Node(tree.Root)
	if !ok {
		return markers
	}
	value, ok := root.Value.(map[string]any)
	if !ok {
		return markers
	}
	boundaries, ok := value["boundaries"].(map[string]any)
	if !ok {
		return markers
	}
	markers.Encrypted = markers.Encrypted || boolMapValue(boundaries, "has_encrypt")
	markers.Signed = markers.Signed || boolMapValue(boundaries, "has_signature")
	markers.XFA = markers.XFA || boolMapValue(boundaries, "has_xfa")
	markers.AcroForm = markers.AcroForm || boolMapValue(boundaries, "has_acroform")
	markers.Annotations = markers.Annotations || boolMapValue(boundaries, "has_annotations")
	markers.Fonts = markers.Fonts || boolMapValue(boundaries, "has_font_markers")
	markers.CMaps = markers.CMaps || boolMapValue(boundaries, "has_cmap_markers")
	markers.ToUnicode = markers.ToUnicode || boolMapValue(boundaries, "has_tounicode_cmap")
	markers.CIDFonts = markers.CIDFonts || boolMapValue(boundaries, "has_cid_font_markers")
	return markers
}

func profileTextCapabilities(tree *core.Tree) ProfileTextCapabilities {
	if tree == nil {
		return ProfileTextCapabilities{}
	}
	nodes := tree.Query(core.Match{Kind: pdf.KindTextShow})
	return ProfileTextCapabilities{
		NodeCount:     len(nodes),
		EditableCount: len(nodes),
		CanEdit:       len(nodes) > 0,
	}
}

func profileStreamCapabilities(tree *core.Tree) ProfileStreamCapabilities {
	out := ProfileStreamCapabilities{
		FilterCounts:     map[string]int{},
		CapabilityCounts: map[string]int{},
	}
	if tree == nil {
		return out
	}
	for _, stream := range tree.Query(core.Match{Kind: pdf.KindStream}) {
		out.TotalCount++
		filter := stringMetaValue(stream.Meta, "filter")
		if filter != "" {
			out.FilterCounts[filter]++
		}
		capability := stringMetaValue(stream.Meta, "filter_capability")
		if capability != "" {
			out.CapabilityCounts[capability]++
		}
		if boolMetaValue(stream.Meta, "filter_editable") {
			out.EditableCount++
		}
		if boolMetaValue(stream.Meta, "filter_pass_through") {
			out.PassThroughCount++
		}
		if capability == "unsupported_target" || (stream.Meta["unsupported"] != nil && boolMetaValue(stream.Meta, "filter_target")) {
			out.UnsupportedCount++
		}
	}
	return out
}

func profileFormFieldFillable(field pdf.FormFieldMetadata) bool {
	if field.ReadOnly {
		return false
	}
	if field.FillStatus != "" {
		return field.FillStatus == "supported"
	}
	switch field.FieldType {
	case "Tx", "Ch", "Btn":
		return true
	default:
		return false
	}
}

func profileMarkerUnsupportedReasons(markers ProfileMarkers) []string {
	var reasons []string
	if markers.Encrypted {
		reasons = append(reasons, "encrypted_pdf_requires_password_and_limited_support")
	}
	if markers.Signed {
		reasons = append(reasons, "signed_pdf_requires_explicit_signature_mode")
	}
	if markers.XFA {
		reasons = append(reasons, "xfa_forms_are_not_general_edit_targets")
	}
	if markers.CIDFonts {
		reasons = append(reasons, "cid_font_text_rewrite_is_conservative")
	}
	return reasons
}

func profileStreamUnsupportedReasons(streams ProfileStreamCapabilities) []string {
	if streams.UnsupportedCount == 0 {
		return nil
	}
	return []string{fmt.Sprintf("unsupported_stream_filters:%d", streams.UnsupportedCount)}
}

func profileRewriteRecommendation(report ProfileReport) RewriteMode {
	if !report.Valid {
		return RewriteModePreserveStructure
	}
	if report.Markers.Signed {
		return RewriteModeSurgical
	}
	if report.Streams.UnsupportedCount > 0 || report.Markers.Encrypted || report.Markers.XFA {
		return RewriteModePreserveStructure
	}
	return RewriteModeCanonical
}

func boolMapValue(values map[string]any, key string) bool {
	value, _ := values[key].(bool)
	return value
}

func stringMetaValue(meta map[string]any, key string) string {
	value, _ := meta[key].(string)
	return value
}

func boolMetaValue(meta map[string]any, key string) bool {
	value, _ := meta[key].(bool)
	return value
}
