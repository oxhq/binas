package pdf

import (
	"errors"
	"fmt"

	"github.com/oxhq/binas/pkg/core"
)

type FallbackKind string

const (
	FallbackNone         FallbackKind = "none"
	FallbackOverlay      FallbackKind = "overlay"
	FallbackOCRTextLayer FallbackKind = "ocr_text_layer"
)

type FallbackMode string

const (
	FallbackModeNone     FallbackMode = "none"
	FallbackModeExplicit FallbackMode = "explicit"
)

var (
	ErrFallbackRequiresExplicitMode      = errors.New("pdf fallback policy requires explicit mode")
	ErrFallbackModeWithoutFallback       = errors.New("pdf fallback policy mode requires a fallback kind")
	ErrUnknownFallbackKind               = errors.New("pdf fallback policy has unknown fallback kind")
	ErrUnknownFallbackMode               = errors.New("pdf fallback policy has unknown mode")
	ErrTrueTextEditRejectsFallbackPolicy = errors.New("pdf true text edit rejects fallback policy")
)

type OverlayPolicy struct {
	Fallback FallbackKind `json:"fallback"`
	Mode     FallbackMode `json:"mode"`
}

func DefaultOverlayPolicy() OverlayPolicy {
	return OverlayPolicy{Fallback: FallbackNone, Mode: FallbackModeNone}
}

func (p OverlayPolicy) UsesFallback() bool {
	return p.normalized().Fallback != FallbackNone
}

func (p OverlayPolicy) Validate() error {
	p = p.normalized()

	switch p.Mode {
	case FallbackModeNone, FallbackModeExplicit:
	default:
		return fmt.Errorf("%w: %q", ErrUnknownFallbackMode, p.Mode)
	}

	switch p.Fallback {
	case FallbackNone:
		if p.Mode != FallbackModeNone {
			return fmt.Errorf("%w: %q", ErrFallbackModeWithoutFallback, p.Mode)
		}
	case FallbackOverlay, FallbackOCRTextLayer:
		if p.Mode != FallbackModeExplicit {
			return fmt.Errorf("%w: %s requires %s", ErrFallbackRequiresExplicitMode, p.Fallback, FallbackModeExplicit)
		}
	default:
		return fmt.Errorf("%w: %q", ErrUnknownFallbackKind, p.Fallback)
	}

	return nil
}

func ValidateTrueTextEditFallbackPolicy(policy OverlayPolicy, invariants []core.Invariant) error {
	if err := policy.Validate(); err != nil {
		return err
	}
	if !policy.UsesFallback() {
		return nil
	}

	policy = policy.normalized()
	if hasCoreInvariant(invariants, core.InvariantNoFallbackUsed) {
		return fmt.Errorf("%w: %s violates %s", ErrTrueTextEditRejectsFallbackPolicy, policy.Fallback, core.InvariantNoFallbackUsed)
	}
	return fmt.Errorf("%w: %s is not a true text edit", ErrTrueTextEditRejectsFallbackPolicy, policy.Fallback)
}

func WithNoFallbackPolicy(report core.Report) core.Report {
	return WithFallbackPolicy(report, DefaultOverlayPolicy())
}

func WithFallbackPolicy(report core.Report, policy OverlayPolicy) core.Report {
	policy = policy.normalized()
	report.FallbackUsed = policy.UsesFallback()
	report.FallbackPolicy = &core.FallbackPolicy{
		Fallback: string(policy.Fallback),
		Mode:     string(policy.Mode),
	}
	return report
}

func ValidateTrueTextEditReportFallbackPolicy(report core.Report) error {
	policy := DefaultOverlayPolicy()
	if report.FallbackPolicy != nil {
		policy = OverlayPolicy{
			Fallback: FallbackKind(report.FallbackPolicy.Fallback),
			Mode:     FallbackMode(report.FallbackPolicy.Mode),
		}
	}
	if err := ValidateTrueTextEditFallbackPolicy(policy, report.Invariants); err != nil {
		return err
	}
	if report.FallbackUsed {
		return fmt.Errorf("%w: report fallback_used=true violates %s", ErrTrueTextEditRejectsFallbackPolicy, core.InvariantNoFallbackUsed)
	}
	return nil
}

func (p OverlayPolicy) normalized() OverlayPolicy {
	if p.Fallback == "" {
		p.Fallback = FallbackNone
	}
	if p.Mode == "" {
		p.Mode = FallbackModeNone
	}
	return p
}

func hasCoreInvariant(invariants []core.Invariant, want core.Invariant) bool {
	for _, invariant := range invariants {
		if invariant == want {
			return true
		}
	}
	return false
}
