package pdf

import (
	"errors"
	"fmt"

	"github.com/oxhq/binas/pkg/core"
)

type PDFWriterMode string

const (
	PDFWriterModeCanonical         PDFWriterMode = "canonical"
	PDFWriterModePreserveStructure PDFWriterMode = "preserve-structure"
)

var ErrPreserveStructureRepackUnsupported = errors.New("preserve-structure PDF writer cannot repack this object/xref stream layout")

type PreserveStructureUnsupportedError struct {
	Plan    pdfStructurePlan
	Details map[string]any
}

func (e *PreserveStructureUnsupportedError) Error() string {
	details := e.structureDetails()
	message := fmt.Sprintf(
		"%v: preserve-structure requested but structure_plan requires unsupported packed writer handling (has_table_xref=%t, has_xref_stream=%t, has_hybrid_xref=%t, object_stream_objects=%d, xref_stream_objects=%d)",
		ErrPreserveStructureRepackUnsupported,
		details["has_table_xref"],
		details["has_xref_stream"],
		details["has_hybrid_xref"],
		details["object_stream_objects"],
		details["xref_stream_objects"],
	)
	if reason, ok := details["reason"].(string); ok && reason != "" {
		message += ": " + reason
	}
	return message
}

func (e *PreserveStructureUnsupportedError) Unwrap() error {
	return ErrPreserveStructureRepackUnsupported
}

func (e *PreserveStructureUnsupportedError) StructureDetails() map[string]any {
	return cloneStringAnyMap(e.structureDetails())
}

func (e *PreserveStructureUnsupportedError) structureDetails() map[string]any {
	if e == nil {
		return pdfStructurePlan{}.metadata()
	}
	if e.Details != nil {
		return e.Details
	}
	return e.Plan.metadata()
}

func NormalizePDFWriterMode(raw string) (PDFWriterMode, error) {
	switch raw {
	case "", string(PDFWriterModeCanonical):
		return PDFWriterModeCanonical, nil
	case string(PDFWriterModePreserveStructure):
		return PDFWriterModePreserveStructure, nil
	default:
		return "", fmt.Errorf("unsupported PDF writer mode %q (expected canonical or preserve-structure)", raw)
	}
}

func ApplyCanonicalEditWithWriterMode(input []byte, mode PDFWriterMode, selector core.Match, mutation core.Mutation, invariants []core.Invariant) ([]byte, core.Report, core.Verification, error) {
	proof, err := ensurePDFWriterModeSupported(input, mode, pdfGraphParseOptions{})
	if err != nil {
		return nil, core.Report{}, core.Verification{}, err
	}
	output, report, verification, err := editCanonicalWithOptions(
		input,
		selector,
		mutation,
		invariants,
		pdfGraphParseOptions{},
		pdfCanonicalWriteOptions{WriterMode: mode},
	)
	if err != nil {
		return nil, core.Report{}, core.Verification{}, err
	}
	report.Meta = mergeReportMeta(report.Meta, proof.reportMeta())
	return output, report, verification, nil
}

type pdfWriterModeProof struct {
	Mode                    PDFWriterMode
	StructurePlan           pdfStructurePlan
	UsedCanonicalWriterPath bool
}

func (p pdfWriterModeProof) reportMeta() map[string]any {
	writerPath := "canonical"
	if p.Mode == PDFWriterModePreserveStructure && !p.UsedCanonicalWriterPath {
		writerPath = p.StructurePlan.writerPath()
	}
	return map[string]any{
		"writer_mode":                string(p.Mode),
		"writer_path":                writerPath,
		"used_canonical_writer_path": p.UsedCanonicalWriterPath,
		"structure_plan":             p.StructurePlan.metadata(),
	}
}

func ensurePDFWriterModeSupported(input []byte, mode PDFWriterMode, parseOpts pdfGraphParseOptions) (pdfWriterModeProof, error) {
	if mode == "" {
		mode = PDFWriterModeCanonical
	}
	if mode != PDFWriterModeCanonical && mode != PDFWriterModePreserveStructure {
		return pdfWriterModeProof{}, fmt.Errorf("unsupported PDF writer mode %q (expected canonical or preserve-structure)", mode)
	}
	if mode != PDFWriterModePreserveStructure {
		return pdfWriterModeProof{Mode: mode, UsedCanonicalWriterPath: true}, nil
	}
	xref := summarizeXref(input)
	graph, err := parsePDFGraphWithOptions(input, parseOpts)
	if err != nil {
		if xrefSummaryRequiresPackedWriter(xref) {
			return pdfWriterModeProof{}, newPreserveStructureUnsupportedXrefSummaryError(xref, err)
		}
		return pdfWriterModeProof{}, err
	}
	return ensurePDFWriterModeSupportedForGraph(graph, mode)
}

func ensurePDFWriterModeSupportedForGraph(graph *pdfGraph, mode PDFWriterMode) (pdfWriterModeProof, error) {
	if mode == "" {
		mode = PDFWriterModeCanonical
	}
	if mode != PDFWriterModePreserveStructure {
		return pdfWriterModeProof{Mode: mode, UsedCanonicalWriterPath: true}, nil
	}
	plan := summarizePDFStructurePlan(graph)
	proof := pdfWriterModeProof{
		Mode:                    mode,
		StructurePlan:           plan,
		UsedCanonicalWriterPath: !plan.requiresPackedWriter(),
	}
	if plan.UnknownObjects > 0 {
		return pdfWriterModeProof{}, &PreserveStructureUnsupportedError{
			Plan:    plan,
			Details: plan.metadata(),
		}
	}
	return proof, nil
}

func xrefSummaryRequiresPackedWriter(xref xrefSummary) bool {
	return xref.HasObjectStream || xref.HasStream || xref.HasHybridStream
}

func newPreserveStructureUnsupportedXrefSummaryError(xref xrefSummary, parseErr error) *PreserveStructureUnsupportedError {
	details := map[string]any{
		"total_objects":             len(xref.Objects),
		"normal_objects":            len(xref.Objects),
		"object_stream_objects":     len(xref.ObjectStreamObjects),
		"xref_stream_objects":       len(xref.StreamObjects),
		"unknown_objects":           0,
		"has_table_xref":            xref.HasTable,
		"has_xref_stream":           xref.HasStream,
		"has_hybrid_xref":           xref.HasHybridStream,
		"hybrid_stream_offset":      xref.HybridStreamOffset,
		"unsupported_xref_stream":   xref.UnsupportedXrefStream,
		"unsupported_object_stream": xref.UnsupportedObjectStream,
		"requires_packed_writer":    xrefSummaryRequiresPackedWriter(xref),
	}
	if parseErr != nil {
		details["parse_error"] = parseErr.Error()
	}
	return &PreserveStructureUnsupportedError{Details: details}
}

func mergeReportMeta(base map[string]any, extra map[string]any) map[string]any {
	if len(extra) == 0 {
		return base
	}
	out := cloneStringAnyMap(base)
	for key, value := range extra {
		out[key] = value
	}
	return out
}

func cloneStringAnyMap(input map[string]any) map[string]any {
	out := make(map[string]any, len(input))
	for key, value := range input {
		out[key] = value
	}
	return out
}
