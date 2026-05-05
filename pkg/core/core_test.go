package core

import (
	"encoding/json"
	"errors"
	"testing"
)

func TestTreeQueryMatchesKindTextAndMeta(t *testing.T) {
	tree := &Tree{Format: "test"}
	tree.AddNode(Node{Kind: "root"})
	tree.AddNode(Node{Kind: "text", Value: "hello", Meta: map[string]any{"op": "Tj"}})
	tree.AddNode(Node{Kind: "text", Value: "hello", Meta: map[string]any{"op": "TJ"}})

	matches := tree.Query(Match{Kind: "text", Text: "hello", Meta: map[string]any{"op": "Tj"}})
	if len(matches) != 1 {
		t.Fatalf("matches = %d, want 1", len(matches))
	}
	if matches[0].Meta["op"] != "Tj" {
		t.Fatalf("wrong match: %+v", matches[0])
	}
}

func TestTreeQueryMatchesStructuredSelectorFields(t *testing.T) {
	tree := &Tree{Format: "test"}
	tree.AddNode(Node{Kind: "text", Name: "body", Value: "hello"})
	tree.AddNode(Node{Kind: "text", Name: "title", Value: "hello"})

	matches := tree.Query(Selector{Kind: "text", Name: "title", Text: "hello"})
	if len(matches) != 1 {
		t.Fatalf("matches = %d, want 1", len(matches))
	}
	if matches[0].Name != "title" {
		t.Fatalf("wrong match: %+v", matches[0])
	}
}

func TestTreeQueryRequiresRequestedMetaKeys(t *testing.T) {
	tree := &Tree{Format: "test"}
	tree.AddNode(Node{Kind: "text", Value: "hello", Meta: map[string]any{"op": "Tj"}})

	matches := tree.Query(Match{Kind: "text", Meta: map[string]any{"missing": nil}})
	if len(matches) != 0 {
		t.Fatalf("matches = %d, want 0", len(matches))
	}
}

func TestTreeQueryKeepsMatchMetaStringCompatibility(t *testing.T) {
	tree := &Tree{Format: "test"}
	tree.AddNode(Node{Kind: "text", Value: "hello", Meta: map[string]any{"page": 12}})
	tree.AddNode(Node{Kind: "text", Value: "hello", Meta: map[string]any{"page": "12"}})

	matches := tree.Query(Match{Kind: "text", Meta: map[string]any{"page": 12}})
	if len(matches) != 2 {
		t.Fatalf("matches = %d, want 2", len(matches))
	}
}

func TestTreeQueryMatchIndexIsZeroBased(t *testing.T) {
	tree := &Tree{Format: "test"}
	tree.AddNode(Node{Kind: "text", Name: "first", Value: "hello"})
	tree.AddNode(Node{Kind: "text", Name: "second", Value: "hello"})
	tree.AddNode(Node{Kind: "text", Name: "third", Value: "hello"})

	index := 1
	matches := tree.Query(Match{Kind: "text", Text: "hello", MatchIndex: &index})
	if len(matches) != 1 {
		t.Fatalf("matches = %d, want 1", len(matches))
	}
	if matches[0].Name != "second" {
		t.Fatalf("match index 1 returned %+v, want second node", matches[0])
	}

	all := tree.QueryAll(Match{Kind: "text", Text: "hello", MatchIndex: &index})
	if len(all) != 3 {
		t.Fatalf("QueryAll matches = %d, want 3", len(all))
	}
}

func TestTreeQueryMatchIndexOutOfRangeReturnsNoMatches(t *testing.T) {
	tree := &Tree{Format: "test"}
	tree.AddNode(Node{Kind: "text", Value: "hello"})

	index := 1
	matches := tree.Query(Match{Kind: "text", Text: "hello", MatchIndex: &index})
	if len(matches) != 0 {
		t.Fatalf("matches = %d, want 0", len(matches))
	}
}

func TestTreeFindOneReturnsSingleMatch(t *testing.T) {
	tree := &Tree{Format: "test"}
	tree.AddNode(Node{Kind: "text", Name: "body", Value: "hello"})
	tree.AddNode(Node{Kind: "text", Name: "title", Value: "hello"})

	node, err := tree.FindOne(Match{Kind: "text", Name: "title"})
	if err != nil {
		t.Fatalf("FindOne() error = %v", err)
	}
	if node.Name != "title" {
		t.Fatalf("FindOne() = %+v, want title node", node)
	}
}

func TestTreeFindOneErrorsWhenNoNodesMatch(t *testing.T) {
	tree := &Tree{Format: "test"}
	tree.AddNode(Node{Kind: "text", Name: "body", Value: "hello"})

	node, err := tree.FindOne(Match{Kind: "text", Name: "missing"})
	if !errors.Is(err, ErrNoMatches) {
		t.Fatalf("FindOne() error = %v, want ErrNoMatches", err)
	}
	if node.Kind != "" || node.Name != "" || node.Value != nil {
		t.Fatalf("FindOne() node = %+v, want zero Node", node)
	}
}

func TestTreeFindOneErrorsWhenMultipleNodesMatch(t *testing.T) {
	tree := &Tree{Format: "test"}
	tree.AddNode(Node{Kind: "text", Value: "hello"})
	tree.AddNode(Node{Kind: "text", Value: "hello"})

	node, err := tree.FindOne(Match{Kind: "text", Text: "hello"})
	if !errors.Is(err, ErrMultipleMatches) {
		t.Fatalf("FindOne() error = %v, want ErrMultipleMatches", err)
	}
	if node.Kind != "" || node.Name != "" || node.Value != nil {
		t.Fatalf("FindOne() node = %+v, want zero Node", node)
	}
}

func TestTreeFindOneUsesMatchIndexForMultipleMatches(t *testing.T) {
	tree := &Tree{Format: "test"}
	tree.AddNode(Node{Kind: "text", Name: "first", Value: "hello"})
	tree.AddNode(Node{Kind: "text", Name: "second", Value: "hello"})

	index := 0
	node, err := tree.FindOne(Match{Kind: "text", Text: "hello", MatchIndex: &index})
	if err != nil {
		t.Fatalf("FindOne() error = %v", err)
	}
	if node.Name != "first" {
		t.Fatalf("FindOne() = %+v, want first node", node)
	}
}

func TestTreeFindOneErrorsWhenMatchIndexOutOfRange(t *testing.T) {
	tree := &Tree{Format: "test"}
	tree.AddNode(Node{Kind: "text", Value: "hello"})

	index := 1
	node, err := tree.FindOne(Match{Kind: "text", Text: "hello", MatchIndex: &index})
	if !errors.Is(err, ErrMatchIndexOutOfRange) {
		t.Fatalf("FindOne() error = %v, want ErrMatchIndexOutOfRange", err)
	}
	if node.Kind != "" || node.Name != "" || node.Value != nil {
		t.Fatalf("FindOne() node = %+v, want zero Node", node)
	}
}

func TestReportCarriesInvariantAndVerificationContracts(t *testing.T) {
	matchIndex := 1
	report := Report{
		Format:        "pdf",
		Edit:          "replace-text",
		FallbackUsed:  false,
		NodesModified: 1,
		MatchIndex:    &matchIndex,
		Invariants: []Invariant{
			InvariantReparse,
			InvariantOldGone,
			InvariantNewSelectable,
		},
		Verification: &Verification{
			ReparseOK:      true,
			OldTextRemoved: true,
			NewSelectable:  true,
			PageUnchanged:  true,
		},
	}

	got, err := json.Marshal(report)
	if err != nil {
		t.Fatalf("Marshal() error = %v", err)
	}

	var decoded struct {
		MatchIndex   int          `json:"match_index"`
		Invariants   []Invariant  `json:"invariants"`
		Verification Verification `json:"verification"`
	}
	if err := json.Unmarshal(got, &decoded); err != nil {
		t.Fatalf("Unmarshal() error = %v", err)
	}

	if len(decoded.Invariants) != 3 || decoded.Invariants[0] != InvariantReparse {
		t.Fatalf("decoded invariants = %+v", decoded.Invariants)
	}
	if decoded.MatchIndex != 1 {
		t.Fatalf("decoded match_index = %d, want 1", decoded.MatchIndex)
	}
	if !decoded.Verification.ReparseOK || !decoded.Verification.OldTextRemoved || !decoded.Verification.NewSelectable || !decoded.Verification.PageUnchanged {
		t.Fatalf("decoded verification = %+v", decoded.Verification)
	}
}
