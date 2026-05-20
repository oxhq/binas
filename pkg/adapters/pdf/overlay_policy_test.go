package pdf

import (
	"encoding/json"
	"errors"
	"strings"
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

func TestTrueEditReportCarriesExplicitNoFallbackPolicy(t *testing.T) {
	report := WithNoFallbackPolicy(core.Report{
		Format:        "pdf",
		Edit:          "pdf.canonical_content_stream_text_rewrite",
		FallbackUsed:  false,
		NodesModified: 1,
		Invariants:    []core.Invariant{core.InvariantNoFallbackUsed},
	})

	if err := ValidateTrueTextEditReportFallbackPolicy(report); err != nil {
		t.Fatalf("ValidateTrueTextEditReportFallbackPolicy() error = %v, want nil", err)
	}

	encoded, err := json.Marshal(report)
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(string(encoded), `"fallback_used":false`) || !strings.Contains(string(encoded), `"fallback_policy":{"fallback":"none","mode":"none"}`) {
		t.Fatalf("report JSON missing explicit fallback policy: %s", encoded)
	}
}

func TestTrueEditReportRejectsOverlayOrOCRFallbackPolicy(t *testing.T) {
	for _, policy := range []OverlayPolicy{
		{Fallback: FallbackOverlay, Mode: FallbackModeExplicit},
		{Fallback: FallbackOCRTextLayer, Mode: FallbackModeExplicit},
	} {
		report := WithFallbackPolicy(core.Report{
			Format:        "pdf",
			Edit:          "pdf.canonical_content_stream_text_rewrite",
			FallbackUsed:  true,
			NodesModified: 1,
			Invariants:    []core.Invariant{core.InvariantNoFallbackUsed},
		}, policy)

		err := ValidateTrueTextEditReportFallbackPolicy(report)
		if !errors.Is(err, ErrTrueTextEditRejectsFallbackPolicy) {
			t.Fatalf("ValidateTrueTextEditReportFallbackPolicy(%+v) error = %v, want ErrTrueTextEditRejectsFallbackPolicy", policy, err)
		}
	}
}
