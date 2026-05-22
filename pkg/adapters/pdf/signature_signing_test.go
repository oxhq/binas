package pdf

import (
	"context"
	"errors"
	"strings"
	"testing"
)

func TestPlanIncrementalReSigningRequiresCallback(t *testing.T) {
	plan, err := PlanIncrementalReSigning(signedTextPDF("08-15-2024"), SignatureSigningPlanOptions{})

	if !errors.Is(err, ErrSignatureSigningCallbackRequired) {
		t.Fatalf("PlanIncrementalReSigning() error = %v, want ErrSignatureSigningCallbackRequired", err)
	}
	if plan.Supported {
		t.Fatalf("plan.Supported = true, want false")
	}
	if !strings.Contains(plan.UnsupportedReason, "callback") {
		t.Fatalf("UnsupportedReason = %q, want callback reason", plan.UnsupportedReason)
	}
}

func TestPlanIncrementalReSigningValidatesCallbackMetadataWithoutCallingSigner(t *testing.T) {
	called := false
	callback := func(context.Context, SignatureSigningRequest) (SignatureSigningResponse, error) {
		called = true
		return SignatureSigningResponse{Signature: []byte("not-used")}, nil
	}

	plan, err := PlanIncrementalReSigning(signedTextPDF("08-15-2024"), SignatureSigningPlanOptions{
		Callback: callback,
		CallbackMetadata: SignatureSigningCallbackMetadata{
			Name:               "test external signer",
			ExternalKeyID:      "kms://tenant/pdf-signing-key",
			DigestAlgorithm:    "sha256",
			SignatureContainer: "pkcs7",
			SubFilter:          "adbe.pkcs7.detached",
		},
	})

	if err != nil {
		t.Fatalf("PlanIncrementalReSigning() error = %v", err)
	}
	if !plan.Supported {
		t.Fatalf("plan.Supported = false, want true: %+v", plan)
	}
	if called {
		t.Fatal("PlanIncrementalReSigning called the signing callback; planning must not produce fake signatures")
	}
	if plan.CallbackMetadata.ExternalKeyID != "kms://tenant/pdf-signing-key" {
		t.Fatalf("ExternalKeyID = %q, want external key handle", plan.CallbackMetadata.ExternalKeyID)
	}
	if plan.CallbackMetadata.DigestAlgorithm != "sha256" || plan.CallbackMetadata.SignatureContainer != "pkcs7" {
		t.Fatalf("callback crypto metadata = %+v", plan.CallbackMetadata)
	}
	if plan.Signature.ByteRangeStatus != signatureByteRangeStatusValid || len(plan.ByteRanges) != 2 {
		t.Fatalf("signature proof = status %q ranges %+v, want valid two-range proof", plan.Signature.ByteRangeStatus, plan.ByteRanges)
	}
}

func TestPlanIncrementalReSigningRejectsPrivateKeyMaterialInCallbackMetadata(t *testing.T) {
	_, err := PlanIncrementalReSigning(signedTextPDF("08-15-2024"), SignatureSigningPlanOptions{
		Callback: func(context.Context, SignatureSigningRequest) (SignatureSigningResponse, error) {
			return SignatureSigningResponse{}, nil
		},
		CallbackMetadata: SignatureSigningCallbackMetadata{
			Name:               "unsafe signer",
			ExternalKeyID:      "-----BEGIN PRIVATE KEY-----",
			DigestAlgorithm:    "sha256",
			SignatureContainer: "pkcs7",
			SubFilter:          "adbe.pkcs7.detached",
		},
	})

	if !errors.Is(err, ErrSignatureSigningCallbackMetadataInvalid) {
		t.Fatalf("PlanIncrementalReSigning() error = %v, want ErrSignatureSigningCallbackMetadataInvalid", err)
	}
	if !strings.Contains(err.Error(), "private key") {
		t.Fatalf("error = %q, want private key guidance", err)
	}
}
