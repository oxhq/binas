package main

import (
	"encoding/json"
	"errors"
	"fmt"

	"github.com/oxhq/binas/pkg/adapters/pdf"
	"github.com/oxhq/binas/pkg/core"
)

type pdfEditResult struct {
	OK           bool               `json:"ok"`
	Error        string             `json:"error,omitempty"`
	Bytes        []byte             `json:"-"`
	Report       core.Report        `json:"report,omitempty"`
	Verification *core.Verification `json:"verification,omitempty"`
}

func inspectPDFJSON(input []byte) string {
	adapter := pdf.NewAdapter()
	tree, parseErr := adapter.Parse(input, core.ParseOptions{Strict: true})
	if parseErr != nil && tree == nil {
		return jsonString(map[string]any{"ok": false, "error": parseErr.Error()})
	}
	if tree == nil {
		return jsonString(map[string]any{"ok": false, "error": "inspect produced no parse tree"})
	}
	root, _ := tree.Node(tree.Root)
	result := map[string]any{
		"ok":     true,
		"format": tree.Format,
		"nodes":  len(tree.Nodes),
		"root":   root.Value,
	}
	if parseErr != nil {
		result["parse_error"] = parseErr.Error()
	}
	return jsonString(result)
}

func queryPDFJSON(input []byte, text string) string {
	adapter := pdf.NewAdapter()
	tree, err := adapter.Parse(input, core.ParseOptions{Strict: true})
	if err != nil {
		return jsonString(map[string]any{"ok": false, "error": err.Error()})
	}
	matches := tree.QueryAll(core.Match{Kind: pdf.KindTextShow, Text: text})
	return jsonString(map[string]any{
		"ok":      true,
		"matches": matches,
		"count":   len(matches),
	})
}

func editPDFText(input []byte, oldText, newText string) pdfEditResult {
	if oldText == "" || newText == "" {
		return pdfEditError(errors.New("binasEditPDFText requires oldText and newText"))
	}
	selector := core.Match{Kind: pdf.KindTextShow, Text: oldText}
	mutation := core.Mutation{Replace: newText}
	invariants := []core.Invariant{
		core.InvariantReparse,
		core.InvariantOldGone,
		core.InvariantNewSelectable,
	}

	adapter := pdf.NewAdapter()
	tree, err := adapter.Parse(input, core.ParseOptions{Strict: true})
	if err != nil {
		return applyCanonicalPDFEdit(input, selector, mutation, invariants)
	}
	if pdf.NeedsCanonicalRewrite(tree) {
		return applyCanonicalPDFEdit(input, selector, mutation, invariants)
	}
	plan, err := adapter.PlanEdit(tree, selector, mutation)
	if err != nil {
		return pdfEditError(err)
	}
	plan.Invariants = invariants
	output, report, err := adapter.Apply(input, plan)
	if err != nil {
		return pdfEditError(err)
	}
	verification, err := adapter.Verify(output, plan)
	if err != nil {
		return pdfEditError(err)
	}
	if err := verificationError(verification, invariants); err != nil {
		return pdfEditError(err)
	}
	report.Verification = &verification
	return pdfEditResult{
		OK:           true,
		Bytes:        output,
		Report:       report,
		Verification: &verification,
	}
}

func applyCanonicalPDFEdit(input []byte, selector core.Match, mutation core.Mutation, invariants []core.Invariant) pdfEditResult {
	output, report, verification, err := pdf.ApplyCanonicalEdit(input, selector, mutation, invariants)
	if err != nil {
		return pdfEditError(err)
	}
	if err := verificationError(verification, invariants); err != nil {
		return pdfEditError(err)
	}
	report.Verification = &verification
	return pdfEditResult{
		OK:           true,
		Bytes:        output,
		Report:       report,
		Verification: &verification,
	}
}

func verificationError(verification core.Verification, invariants []core.Invariant) error {
	failed := make([]string, 0)
	for _, invariant := range invariants {
		switch invariant {
		case core.InvariantReparse:
			if !verification.ReparseOK {
				failed = append(failed, string(invariant))
			}
		case core.InvariantOldGone:
			if !verification.OldTextRemoved {
				failed = append(failed, string(invariant))
			}
		case core.InvariantNewSelectable:
			if !verification.NewSelectable {
				failed = append(failed, string(invariant))
			}
		case core.InvariantPageUnchanged:
			if !verification.PageUnchanged {
				failed = append(failed, string(invariant))
			}
		}
	}
	if len(failed) > 0 {
		return fmt.Errorf("verification failed for %v: %+v", failed, verification)
	}
	return nil
}

func pdfEditError(err error) pdfEditResult {
	if err == nil {
		err = errors.New("unknown PDF edit error")
	}
	return pdfEditResult{OK: false, Error: err.Error()}
}

func jsonString(value any) string {
	raw, err := json.Marshal(value)
	if err != nil {
		raw, _ = json.Marshal(map[string]any{"ok": false, "error": err.Error()})
	}
	return string(raw)
}
