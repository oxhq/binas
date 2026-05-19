package pdf

import (
	"errors"
	"testing"

	"github.com/oxhq/binas/pkg/core"
)

func TestOverlayPolicyDefaultIsNoFallback(t *testing.T) {
	policy := DefaultOverlayPolicy()

	if policy.Fallback != FallbackNone {
		t.Fatalf("fallback = %q, want %q", policy.Fallback, FallbackNone)
	}
	if policy.Mode != FallbackModeNone {
		t.Fatalf("mode = %q, want %q", policy.Mode, FallbackModeNone)
	}
	if policy.UsesFallback() {
		t.Fatal("default policy must not use fallback")
	}
	if err := policy.Validate(); err != nil {
		t.Fatalf("Validate() error = %v, want nil", err)
	}
}

func TestOverlayPolicyFallbackRequiresExplicitMode(t *testing.T) {
	err := (OverlayPolicy{Fallback: FallbackOverlay}).Validate()
	if !errors.Is(err, ErrFallbackRequiresExplicitMode) {
		t.Fatalf("Validate() error = %v, want ErrFallbackRequiresExplicitMode", err)
	}

	policy := OverlayPolicy{Fallback: FallbackOverlay, Mode: FallbackModeExplicit}
	if err := policy.Validate(); err != nil {
		t.Fatalf("Validate() explicit overlay error = %v, want nil", err)
	}
	if !policy.UsesFallback() {
		t.Fatal("explicit overlay policy must use fallback")
	}
}

func TestTrueEditFallbackPolicyRejectsFallbackInvariant(t *testing.T) {
	policy := OverlayPolicy{Fallback: FallbackOverlay, Mode: FallbackModeExplicit}
	err := ValidateTrueTextEditFallbackPolicy(policy, []core.Invariant{core.InvariantNoFallbackUsed})
	if !errors.Is(err, ErrTrueTextEditRejectsFallbackPolicy) {
		t.Fatalf("ValidateTrueTextEditFallbackPolicy() error = %v, want ErrTrueTextEditRejectsFallbackPolicy", err)
	}

	if err := ValidateTrueTextEditFallbackPolicy(DefaultOverlayPolicy(), []core.Invariant{core.InvariantNoFallbackUsed}); err != nil {
		t.Fatalf("ValidateTrueTextEditFallbackPolicy() no-fallback error = %v, want nil", err)
	}
}
