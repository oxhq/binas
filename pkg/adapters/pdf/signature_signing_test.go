package pdf

import (
	"bytes"
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

func TestApplyIncrementalReSigningAppendsSignatureAndValidatesByteRangeDigest(t *testing.T) {
	input := signedPDFWithCMSMessageDigest(t, nil)
	called := false
	output, plan, verification, err := ApplyIncrementalReSigning(context.Background(), input, SignatureSigningPlanOptions{
		Callback: func(_ context.Context, req SignatureSigningRequest) (SignatureSigningResponse, error) {
			called = true
			if req.DigestAlgorithm != "sha256" || len(req.Digest) != 32 {
				t.Fatalf("signing request digest = %q/%d bytes, want sha256/32", req.DigestAlgorithm, len(req.Digest))
			}
			if len(req.ByteRanges) != 2 || req.ByteRanges[0].Offset != 0 || req.ByteRanges[0].Length <= 0 || req.ByteRanges[1].Offset <= req.ByteRanges[0].Length {
				t.Fatalf("signing request byte ranges = %+v, want two non-overlapping ranges", req.ByteRanges)
			}
			return SignatureSigningResponse{Signature: minimalDetachedCMSWithMessageDigest(req.Digest)}, nil
		},
		CallbackMetadata: SignatureSigningCallbackMetadata{
			Name:               "test external signer",
			ExternalKeyID:      "kms://tenant/pdf-signing-key",
			DigestAlgorithm:    "sha256",
			SignatureContainer: "pkcs7",
			SubFilter:          "adbe.pkcs7.detached",
		},
		ReservedBytes: 512,
	})
	if err != nil {
		t.Fatalf("ApplyIncrementalReSigning() error = %v", err)
	}
	if !called {
		t.Fatal("signing callback was not called")
	}
	if !bytes.HasPrefix(output, input) {
		t.Fatal("incremental re-signing did not preserve original bytes as prefix")
	}
	if !plan.Supported || len(plan.ByteRanges) != 2 {
		t.Fatalf("plan = %+v, want supported two-range plan", plan)
	}
	if !verification.IncrementalUpdate || !verification.ReparseOK || !verification.ByteRangeDigestValidation || verification.ByteRangeDigestValidationStatus != signatureByteRangeDigestValidationValid {
		t.Fatalf("verification = %+v, want valid byte-range digest proof", verification)
	}
	info := inspectSignatureInfo(output)
	if !info.ByteRangeDigestValidation || info.ByteRangeDigestValidationStatus != signatureByteRangeDigestValidationValid {
		t.Fatalf("output signature digest validation = %t/%q, want valid", info.ByteRangeDigestValidation, info.ByteRangeDigestValidationStatus)
	}
	if info.CryptographicValidationStatus != signatureCryptographicValidationByteRangeDigestValid {
		t.Fatalf("output cryptographic status = %q, want byte-range digest valid", info.CryptographicValidationStatus)
	}
}

func TestApplyIncrementalReSigningRejectsOversizedCallbackSignature(t *testing.T) {
	_, _, _, err := ApplyIncrementalReSigning(context.Background(), signedPDFWithCMSMessageDigest(t, nil), SignatureSigningPlanOptions{
		Callback: func(context.Context, SignatureSigningRequest) (SignatureSigningResponse, error) {
			return SignatureSigningResponse{Signature: []byte("too long")}, nil
		},
		CallbackMetadata: SignatureSigningCallbackMetadata{
			Name:               "test external signer",
			ExternalKeyID:      "kms://tenant/pdf-signing-key",
			DigestAlgorithm:    "sha256",
			SignatureContainer: "pkcs7",
			SubFilter:          "adbe.pkcs7.detached",
		},
		ReservedBytes: 2,
	})
	if err == nil {
		t.Fatal("expected oversized signature refusal")
	}
	if !strings.Contains(err.Error(), "exceeding reserved /Contents size") {
		t.Fatalf("error = %q, want reserved size guidance", err)
	}
}
