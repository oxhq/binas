package pdfapi

import (
	"fmt"

	"github.com/oxhq/binas/pkg/adapters/pdf"
	"github.com/oxhq/binas/pkg/core"
)

func (s TextSelector) match() core.Match {
	kind := s.Kind
	if kind == "" {
		kind = pdf.KindTextShow
	}
	return core.Match{
		Kind:       kind,
		Text:       s.Text,
		Meta:       stringMetaToAny(s.Meta),
		MatchIndex: s.MatchIndex,
	}
}

func stringMetaToAny(meta map[string]string) map[string]any {
	if len(meta) == 0 {
		return nil
	}
	out := make(map[string]any, len(meta))
	for key, value := range meta {
		out[key] = value
	}
	return out
}

func normalizeRewriteMode(mode RewriteMode) (RewriteMode, error) {
	if mode == "" {
		return RewriteModeAuto, nil
	}
	if err := validateRewriteMode(mode); err != nil {
		return "", err
	}
	return mode, nil
}

func validateRewriteMode(mode RewriteMode) error {
	switch mode {
	case "", RewriteModeAuto, RewriteModeSurgical, RewriteModeCanonical, RewriteModePreserveStructure:
		return nil
	default:
		return fmt.Errorf("unsupported rewrite mode %q", mode)
	}
}

func normalizeSignatureMode(mode string) (string, error) {
	switch mode {
	case "":
		return "", nil
	case string(pdf.SignatureInvalidationRefuse),
		string(pdf.SignatureInvalidationInvalidate),
		string(pdf.SignatureInvalidationPreserveIncremental):
		return mode, nil
	default:
		return "", fmt.Errorf("unsupported signature mode %q", mode)
	}
}

func securityOptions(opts Options) (pdf.SecurityOptions, error) {
	mode, err := normalizeSignatureMode(opts.SignatureMode)
	if err != nil {
		return pdf.SecurityOptions{}, err
	}
	security := pdf.SecurityOptions{Password: opts.Password}
	switch mode {
	case string(pdf.SignatureInvalidationInvalidate):
		security.SignatureInvalidation = pdf.SignatureInvalidationInvalidate
	case string(pdf.SignatureInvalidationPreserveIncremental):
		security.SignatureInvalidation = pdf.SignatureInvalidationPreserveIncremental
	default:
		security.SignatureInvalidation = pdf.SignatureInvalidationRefuse
	}
	return security, nil
}

func invariants(raw []string) ([]core.Invariant, error) {
	if len(raw) == 0 {
		return nil, nil
	}
	out := make([]core.Invariant, 0, len(raw))
	for _, value := range raw {
		invariant := core.Invariant(value)
		switch invariant {
		case core.InvariantReparse,
			core.InvariantOldGone,
			core.InvariantNewSelectable,
			core.InvariantPageUnchanged,
			core.InvariantNoFallbackUsed:
			out = append(out, invariant)
		default:
			return nil, fmt.Errorf("unsupported verification invariant %q", value)
		}
	}
	return out, nil
}
