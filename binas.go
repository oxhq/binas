// Package binas exposes the format-neutral public API.
package binas

import (
	"errors"
	"fmt"
	"strings"

	"github.com/oxhq/binas/pkg/adapters/pdf"
	"github.com/oxhq/binas/pkg/core"
)

type Format string

const PDF Format = "pdf"

var ErrUnsupportedFormat = errors.New("binas: unsupported format")

type Option func(*Binary)

func WithFormat(format Format) Option {
	return func(d *Binary) {
		d.format = Format(strings.ToLower(string(format)))
	}
}

func WithAdapter(adapter core.Adapter) Option {
	return func(d *Binary) {
		d.adapter = adapter
	}
}

func WithParseOptions(opts core.ParseOptions) Option {
	return func(d *Binary) {
		d.parse = opts
	}
}

type Binary struct {
	input      []byte
	format     Format
	adapter    core.Adapter
	parse      core.ParseOptions
	tree       *core.Tree
	selector   core.Selector
	mutation   core.Mutation
	invariants []core.Invariant
}

func Open(input []byte, options ...Option) *Binary {
	doc := &Binary{input: input}
	for _, option := range options {
		option(doc)
	}
	return doc
}

func (d *Binary) Select(selector core.Selector) *Binary {
	d.selector = selector
	return d
}

func (d *Binary) Kind(kind string) *Binary {
	d.selector.Kind = kind
	return d
}

func (d *Binary) Name(name string) *Binary {
	d.selector.Name = name
	return d
}

func (d *Binary) Text(text string) *Binary {
	d.selector.Text = text
	return d
}

func (d *Binary) Meta(key string, value any) *Binary {
	if d.selector.Meta == nil {
		d.selector.Meta = map[string]any{}
	}
	d.selector.Meta[key] = value
	return d
}

func (d *Binary) MatchIndex(index int) *Binary {
	d.selector.MatchIndex = &index
	return d
}

func (d *Binary) Replace(value string) *Binary {
	d.mutation.Replace = value
	return d
}

func (d *Binary) Verify(invariants ...core.Invariant) *Binary {
	d.invariants = append([]core.Invariant(nil), invariants...)
	return d
}

func (d *Binary) Inspect() (*core.Tree, error) {
	if d.tree != nil {
		return d.tree, nil
	}
	adapter, err := d.resolveAdapter()
	if err != nil {
		return nil, err
	}
	tree, err := adapter.Parse(d.input, d.parse)
	if err != nil {
		return nil, err
	}
	d.tree = tree
	return tree, nil
}

func (d *Binary) Query(selector ...core.Selector) ([]core.Node, error) {
	tree, err := d.Inspect()
	if err != nil {
		return nil, err
	}
	return tree.Query(d.selected(selector...)), nil
}

func (d *Binary) FindOne(selector ...core.Selector) (core.Node, error) {
	tree, err := d.Inspect()
	if err != nil {
		return core.Node{}, err
	}
	return tree.FindOne(d.selected(selector...))
}

func (d *Binary) Edit(selector core.Selector, mutation core.Mutation) ([]byte, core.Report, core.Verification, error) {
	return d.edit(selector, mutation)
}

func (d *Binary) Bytes() ([]byte, core.Report, core.Verification, error) {
	return d.edit(d.selector, d.mutation)
}

func (d *Binary) edit(selector core.Selector, mutation core.Mutation) ([]byte, core.Report, core.Verification, error) {
	tree, err := d.Inspect()
	if err != nil {
		return nil, core.Report{}, core.Verification{}, err
	}
	adapter, err := d.resolveAdapter()
	if err != nil {
		return nil, core.Report{}, core.Verification{}, err
	}
	plan, err := adapter.PlanEdit(tree, selector, mutation)
	if err != nil {
		return nil, core.Report{}, core.Verification{}, err
	}
	if len(d.invariants) > 0 {
		plan.Invariants = append([]core.Invariant(nil), d.invariants...)
	}
	output, report, err := adapter.Apply(d.input, plan)
	if err != nil {
		return nil, core.Report{}, core.Verification{}, err
	}
	verification, err := adapter.Verify(output, plan)
	if err != nil {
		return nil, core.Report{}, core.Verification{}, err
	}
	if err := checkInvariants(report, verification, plan.Invariants); err != nil {
		return nil, core.Report{}, verification, err
	}
	report.Verification = &verification
	return output, report, verification, nil
}

func (d *Binary) selected(selector ...core.Selector) core.Selector {
	if len(selector) > 0 {
		return selector[0]
	}
	return d.selector
}

func (d *Binary) resolveAdapter() (core.Adapter, error) {
	if d.adapter != nil {
		return d.adapter, nil
	}
	if d.format != "" {
		adapter, ok := builtInAdapter(d.format)
		if !ok {
			return nil, fmt.Errorf("%w: %s", ErrUnsupportedFormat, d.format)
		}
		d.adapter = adapter
		return adapter, nil
	}
	format, adapter, err := detectAdapter(d.input)
	if err != nil {
		return nil, err
	}
	d.format = format
	d.adapter = adapter
	return adapter, nil
}

func builtInAdapter(format Format) (core.Adapter, bool) {
	switch format {
	case PDF:
		return pdf.NewAdapter(), true
	default:
		return nil, false
	}
}

func detectAdapter(input []byte) (Format, core.Adapter, error) {
	adapter := pdf.NewAdapter()
	confidence, err := adapter.Detect(input)
	if err != nil {
		return "", nil, err
	}
	if confidence <= 0 {
		return "", nil, ErrUnsupportedFormat
	}
	return PDF, adapter, nil
}

func checkInvariants(report core.Report, verification core.Verification, invariants []core.Invariant) error {
	for _, invariant := range invariants {
		switch invariant {
		case core.InvariantReparse:
			if !verification.ReparseOK {
				return fmt.Errorf("verification invariant failed: %s", invariant)
			}
		case core.InvariantOldGone:
			if !verification.OldTextRemoved {
				return fmt.Errorf("verification invariant failed: %s", invariant)
			}
		case core.InvariantNewSelectable:
			if !verification.NewSelectable {
				return fmt.Errorf("verification invariant failed: %s", invariant)
			}
		case core.InvariantPageUnchanged:
			if !verification.PageUnchanged {
				return fmt.Errorf("verification invariant failed: %s", invariant)
			}
		case core.InvariantNoFallbackUsed:
			if report.FallbackUsed {
				return fmt.Errorf("verification invariant failed: %s", invariant)
			}
		default:
			return fmt.Errorf("unsupported verification invariant %q", invariant)
		}
	}
	return nil
}
