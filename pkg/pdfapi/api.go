// Package pdfapi exposes a thin Go API over the existing PDF adapter.
//
// This v0 surface defaults RewriteModeAuto to the canonical PDF rewrite path.
// That keeps the package shell-free while preserving the adapter's existing
// parse, edit, report, and verification behavior.
package pdfapi

import (
	"errors"
	"fmt"

	"github.com/oxhq/binas/pkg/adapters/pdf"
	"github.com/oxhq/binas/pkg/core"
)

type RewriteMode string

const (
	RewriteModeAuto              RewriteMode = "auto"
	RewriteModeSurgical          RewriteMode = "surgical"
	RewriteModeCanonical         RewriteMode = "canonical"
	RewriteModePreserveStructure RewriteMode = "preserve-structure"
)

type Options struct {
	Password      string
	Rewrite       RewriteMode
	SignatureMode string
	Verify        []string
}

type TextSelector struct {
	Text       string
	Kind       string
	Meta       map[string]string
	MatchIndex *int
}

type TextReplacement struct {
	Replace string
}

func Inspect(input []byte, opts Options) (*core.Tree, error) {
	if err := validateRewriteMode(opts.Rewrite); err != nil {
		return nil, err
	}
	security, err := securityOptions(opts)
	if err != nil {
		return nil, err
	}
	return pdf.ParseWithSecurityOptions(input, core.ParseOptions{}, security)
}

func Validate(input []byte, opts Options) (core.Verification, error) {
	if _, err := Inspect(input, opts); err != nil {
		return core.Verification{}, err
	}
	return core.Verification{ReparseOK: true}, nil
}

func QueryText(input []byte, selector TextSelector, opts Options) ([]core.Node, error) {
	tree, err := Inspect(input, opts)
	if err != nil {
		return nil, err
	}
	return tree.Query(selector.match()), nil
}

func EditText(input []byte, selector TextSelector, replacement TextReplacement, opts Options) ([]byte, core.Report, core.Verification, error) {
	mode, err := normalizeRewriteMode(opts.Rewrite)
	if err != nil {
		return nil, core.Report{}, core.Verification{}, err
	}
	signatureMode, err := normalizeSignatureMode(opts.SignatureMode)
	if err != nil {
		return nil, core.Report{}, core.Verification{}, err
	}
	invariants, err := invariants(opts.Verify)
	if err != nil {
		return nil, core.Report{}, core.Verification{}, err
	}

	match := selector.match()
	mutation := core.Mutation{Replace: replacement.Replace}

	if signatureMode == string(pdf.SignatureInvalidationPreserveIncremental) {
		if mode != RewriteModeAuto && mode != RewriteModeCanonical {
			return nil, core.Report{}, core.Verification{}, fmt.Errorf("signature mode %q requires canonical/auto rewrite", signatureMode)
		}
		output, report, verification, _, err := pdf.ApplyIncrementalTextEditPreservingSignatures(input, match, mutation, invariants)
		return output, report, verification, err
	}

	switch mode {
	case RewriteModeAuto, RewriteModeCanonical:
		if opts.Password != "" && signatureMode == string(pdf.SignatureInvalidationInvalidate) {
			return pdf.ApplyCanonicalEditWithPasswordInvalidatingSignatures(input, opts.Password, match, mutation, invariants)
		}
		if opts.Password != "" {
			return pdf.ApplyCanonicalEditWithPassword(input, opts.Password, match, mutation, invariants)
		}
		if signatureMode == string(pdf.SignatureInvalidationInvalidate) {
			return pdf.ApplyCanonicalEditInvalidatingSignatures(input, match, mutation, invariants)
		}
		return pdf.ApplyCanonicalEdit(input, match, mutation, invariants)
	case RewriteModePreserveStructure:
		if opts.Password != "" || signatureMode == string(pdf.SignatureInvalidationInvalidate) {
			return nil, core.Report{}, core.Verification{}, errors.New("preserve-structure rewrite does not support password or signature invalidation options")
		}
		return pdf.ApplyCanonicalEditWithWriterMode(input, pdf.PDFWriterModePreserveStructure, match, mutation, invariants)
	case RewriteModeSurgical:
		if opts.Password != "" || signatureMode != "" {
			return nil, core.Report{}, core.Verification{}, errors.New("surgical rewrite does not support password or signature options")
		}
		return editSurgical(input, match, mutation)
	default:
		return nil, core.Report{}, core.Verification{}, fmt.Errorf("unsupported rewrite mode %q", opts.Rewrite)
	}
}

func editSurgical(input []byte, selector core.Match, mutation core.Mutation) ([]byte, core.Report, core.Verification, error) {
	adapter := pdf.NewAdapter()
	tree, err := adapter.Parse(input, core.ParseOptions{})
	if err != nil {
		return nil, core.Report{}, core.Verification{}, err
	}
	plan, err := adapter.PlanEdit(tree, selector, mutation)
	if err != nil {
		return nil, core.Report{}, core.Verification{}, err
	}
	output, report, err := adapter.Apply(input, plan)
	if err != nil {
		return nil, core.Report{}, core.Verification{}, err
	}
	verification, err := adapter.Verify(output, plan)
	if err != nil {
		return nil, core.Report{}, core.Verification{}, err
	}
	report.Verification = &verification
	return output, report, verification, nil
}
