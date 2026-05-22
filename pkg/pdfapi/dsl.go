package pdfapi

import (
	"fmt"

	"github.com/oxhq/binas/pkg/adapters/pdf"
	"github.com/oxhq/binas/pkg/core"
)

type Document struct {
	input       []byte
	opts        Options
	selector    TextSelector
	replacement TextReplacement
}

func New(input []byte) *Document {
	return &Document{input: input}
}

func (d *Document) WithOptions(opts Options) *Document {
	d.opts = opts
	return d
}

func (d *Document) Rewrite(mode RewriteMode) *Document {
	d.opts.Rewrite = mode
	return d
}

func (d *Document) Password(password string) *Document {
	d.opts.Password = password
	return d
}

func (d *Document) SignatureMode(mode string) *Document {
	d.opts.SignatureMode = mode
	return d
}

func (d *Document) FindText(text string) *Document {
	d.selector.Text = text
	if d.selector.Kind == "" {
		d.selector.Kind = pdf.KindTextShow
	}
	return d
}

func (d *Document) Kind(kind string) *Document {
	d.selector.Kind = kind
	return d
}

func (d *Document) Meta(key, value string) *Document {
	if d.selector.Meta == nil {
		d.selector.Meta = make(map[string]string)
	}
	d.selector.Meta[key] = value
	return d
}

func (d *Document) MatchIndex(index int) *Document {
	d.selector.MatchIndex = &index
	return d
}

func (d *Document) ReplaceWith(text string) *Document {
	d.replacement.Replace = text
	return d
}

func (d *Document) Verify(invariants ...string) *Document {
	d.opts.Verify = append([]string(nil), invariants...)
	return d
}

func (d *Document) Inspect() (*core.Tree, error) {
	return Inspect(d.input, d.opts)
}

func (d *Document) Query() ([]core.Node, error) {
	return QueryText(d.input, d.selector, d.opts)
}

func (d *Document) Bytes() ([]byte, core.Report, core.Verification, error) {
	return EditText(d.input, d.selector, d.replacement, d.opts)
}

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
