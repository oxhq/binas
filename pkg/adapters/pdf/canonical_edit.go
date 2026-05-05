package pdf

import (
	"errors"

	"github.com/oxhq/binas/pkg/core"
)

var ErrCanonicalRewriteUnsupported = errors.New("canonical PDF rewrite is not available")

func ApplyCanonicalEdit(input []byte, selector core.Match, mutation core.Mutation, invariants []core.Invariant) ([]byte, core.Report, core.Verification, error) {
	return EditCanonical(input, selector, mutation, invariants)
}

func ApplyCanonicalEditInvalidatingSignatures(input []byte, selector core.Match, mutation core.Mutation, invariants []core.Invariant) ([]byte, core.Report, core.Verification, error) {
	return EditCanonicalInvalidatingSignatures(input, selector, mutation, invariants)
}
