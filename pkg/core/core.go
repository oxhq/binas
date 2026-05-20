package core

import (
	"errors"
	"fmt"
)

type Span struct {
	Start int64 `json:"start"`
	End   int64 `json:"end"`
}

func (s Span) Len() int64 {
	return s.End - s.Start
}

type NodeID int

type Node struct {
	ID       NodeID         `json:"id"`
	Kind     string         `json:"kind"`
	Name     string         `json:"name,omitempty"`
	Span     Span           `json:"span"`
	Value    any            `json:"value,omitempty"`
	Children []NodeID       `json:"children,omitempty"`
	Meta     map[string]any `json:"meta,omitempty"`
}

type Tree struct {
	Format string `json:"format"`
	Root   NodeID `json:"root"`
	Nodes  []Node `json:"nodes"`
}

func (t *Tree) AddNode(n Node) NodeID {
	n.ID = NodeID(len(t.Nodes))
	t.Nodes = append(t.Nodes, n)
	return n.ID
}

func (t Tree) Node(id NodeID) (Node, bool) {
	i := int(id)
	if i < 0 || i >= len(t.Nodes) {
		return Node{}, false
	}
	return t.Nodes[i], true
}

type Selector struct {
	Kind       string         `json:"kind,omitempty"`
	Name       string         `json:"name,omitempty"`
	Text       string         `json:"text,omitempty"`
	Meta       map[string]any `json:"meta,omitempty"`
	MatchIndex *int           `json:"match_index,omitempty"`
}

type Match = Selector

var (
	ErrNoMatches            = errors.New("core: no nodes match selector")
	ErrMultipleMatches      = errors.New("core: multiple nodes match selector")
	ErrMatchIndexOutOfRange = errors.New("core: match index out of range")
)

func (t Tree) Query(m Match) []Node {
	matches := t.QueryAll(m)
	if m.MatchIndex == nil {
		return matches
	}
	index := *m.MatchIndex
	if index < 0 || index >= len(matches) {
		return nil
	}
	return []Node{matches[index]}
}

func (t Tree) QueryAll(m Match) []Node {
	m.MatchIndex = nil
	matches := make([]Node, 0)
	for _, n := range t.Nodes {
		if m.Kind != "" && n.Kind != m.Kind {
			continue
		}
		if m.Name != "" && n.Name != m.Name {
			continue
		}
		if m.Text != "" && fmt.Sprint(n.Value) != m.Text {
			continue
		}
		if !metaMatches(n.Meta, m.Meta) {
			continue
		}
		matches = append(matches, n)
	}
	return matches
}

func (t Tree) FindOne(m Match) (Node, error) {
	matches := t.QueryAll(m)
	if m.MatchIndex != nil {
		index := *m.MatchIndex
		if index < 0 || index >= len(matches) {
			return Node{}, fmt.Errorf("%w: match_index %d out of range for %d matches", ErrMatchIndexOutOfRange, index, len(matches))
		}
		return matches[index], nil
	}
	switch len(matches) {
	case 1:
		return matches[0], nil
	case 0:
		return Node{}, fmt.Errorf("%w: %+v", ErrNoMatches, m)
	default:
		return Node{}, fmt.Errorf("%w: %d matches for %+v", ErrMultipleMatches, len(matches), m)
	}
}

func metaMatches(nodeMeta, want map[string]any) bool {
	for k, v := range want {
		got, ok := nodeMeta[k]
		if !ok {
			return false
		}
		if fmt.Sprint(got) != fmt.Sprint(v) {
			return false
		}
	}
	return true
}

type ParseOptions struct {
	Strict bool
}

type Mutation struct {
	Replace string
	Index   int
}

type Invariant string

const (
	InvariantReparse        Invariant = "reparse"
	InvariantOldGone        Invariant = "old-gone"
	InvariantNewSelectable  Invariant = "new-selectable"
	InvariantPageUnchanged  Invariant = "page-count-unchanged"
	InvariantNoFallbackUsed Invariant = "no-fallback"
)

type EditPlan struct {
	Target     NodeID         `json:"target"`
	Operation  string         `json:"operation"`
	OldText    string         `json:"old_text"`
	NewText    string         `json:"new_text"`
	Old        []byte         `json:"-"`
	New        []byte         `json:"-"`
	Span       Span           `json:"span"`
	PageCount  int            `json:"page_count,omitempty"`
	Invariants []Invariant    `json:"invariants"`
	Meta       map[string]any `json:"meta,omitempty"`
}

type Report struct {
	Format         string          `json:"format"`
	Edit           string          `json:"edit,omitempty"`
	FallbackUsed   bool            `json:"fallback_used"`
	FallbackPolicy *FallbackPolicy `json:"fallback_policy,omitempty"`
	NodesModified  int             `json:"nodes_modified"`
	MatchIndex     *int            `json:"match_index,omitempty"`
	OutputPath     string          `json:"output_path,omitempty"`
	Invariants     []Invariant     `json:"invariants,omitempty"`
	Verification   *Verification   `json:"verification,omitempty"`
	Meta           map[string]any  `json:"meta,omitempty"`
}

type FallbackPolicy struct {
	Fallback string `json:"fallback"`
	Mode     string `json:"mode"`
}

type Verification struct {
	ReparseOK      bool `json:"reparse_ok"`
	OldTextRemoved bool `json:"old_text_removed"`
	NewSelectable  bool `json:"new_text_selectable"`
	PageUnchanged  bool `json:"page_count_unchanged"`
}

type Confidence float64

type Adapter interface {
	Detect(input []byte) (Confidence, error)
	Parse(input []byte, opts ParseOptions) (*Tree, error)
	PlanEdit(tree *Tree, selector Match, mutation Mutation) (*EditPlan, error)
	Apply(input []byte, plan *EditPlan) ([]byte, Report, error)
	Verify(output []byte, plan *EditPlan) (Verification, error)
}
