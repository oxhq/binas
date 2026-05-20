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

var ErrPreserveStructureRepackUnsupported = errors.New("preserve-structure PDF writer cannot repack object streams or xref streams")

type PreserveStructureUnsupportedError struct {
	Plan    pdfStructurePlan
	Details map[string]any
}

func (e *PreserveStructureUnsupportedError) Error() string {
	details := e.structureDetails()
	return fmt.Sprintf(
		"%v: preserve-structure requested but structure_plan requires packed writer support (has_table_xref=%t, has_xref_stream=%t, has_hybrid_xref=%t, object_stream_objects=%d, xref_stream_objects=%d)",
		ErrPreserveStructureRepackUnsupported,
		details["has_table_xref"],
		details["has_xref_stream"],
		details["has_hybrid_xref"],
		details["object_stream_objects"],
		details["xref_stream_objects"],
	)
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
	output, report, verification, err := ApplyCanonicalEdit(input, selector, mutation, invariants)
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
	return map[string]any{
		"writer_mode":                string(p.Mode),
		"writer_path":                "canonical",
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
	graph, err := parsePDFGraphWithOptions(input, parseOpts)
	if err != nil {
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
	if !plan.requiresPackedWriter() {
		return proof, nil
	}
	return pdfWriterModeProof{}, &PreserveStructureUnsupportedError{
		Plan:    plan,
		Details: plan.metadata(),
	}
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
