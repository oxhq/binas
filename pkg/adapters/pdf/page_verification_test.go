package pdf

import "testing"

func TestPageOperationVerificationChecksGraphTextResourcesAndReferences(t *testing.T) {
	content := "BT\n(Hello page) Tj\nET\n"
	input := testPDF(
		"<< /Type /Catalog /Pages 2 0 R >>",
		"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
		"<< /Type /Page /Parent 2 0 R /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>",
		"<< /Length 23 >>\nstream\n"+content+"endstream",
		"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
	)

	verification, err := VerifyPageOperationOutput(input, PageOperationVerificationOptions{
		ExpectedPageCount:  1,
		ExpectedText:       []string{"Hello page"},
		RequirePageContent: true,
		RequireResources:   true,
	})
	if err != nil {
		t.Fatal(err)
	}

	if !verification.ReparseOK || !verification.PageCountOK || !verification.PageContentAvailable || !verification.ResourcesAvailable || !verification.TextAvailable || !verification.NoDanglingRefs {
		t.Fatalf("verification = %+v, want all page-operation checks true", verification)
	}
	if verification.ActualPageCount != 1 || verification.ExpectedPageCount != 1 {
		t.Fatalf("page counts = actual %d expected %d, want 1/1", verification.ActualPageCount, verification.ExpectedPageCount)
	}
	if len(verification.MissingText) != 0 || len(verification.DanglingRefs) != 0 {
		t.Fatalf("unexpected missing text or dangling refs: %+v", verification)
	}
	coreVerification := verification.CoreVerification()
	if !coreVerification.ReparseOK || !coreVerification.NewSelectable || !coreVerification.PageUnchanged {
		t.Fatalf("core verification = %+v, want reparse/selectable/page unchanged", coreVerification)
	}
}

func TestPageOperationVerificationReportsDanglingIndirectReferences(t *testing.T) {
	input := testPDF(
		"<< /Type /Catalog /Pages 2 0 R >>",
		"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
		"<< /Type /Page /Parent 2 0 R /Resources 99 0 R /Contents 4 0 R >>",
		"<< /Length 23 >>\nstream\nBT\n(Hello page) Tj\nET\nendstream",
	)

	verification, err := VerifyPageOperationOutput(input, PageOperationVerificationOptions{
		ExpectedPageCount:  1,
		RequirePageContent: true,
		RequireResources:   true,
	})
	if err != nil {
		t.Fatal(err)
	}

	if verification.NoDanglingRefs {
		t.Fatalf("NoDanglingRefs = true, want false: %+v", verification)
	}
	if verification.ResourcesAvailable {
		t.Fatalf("ResourcesAvailable = true, want false: %+v", verification)
	}
	if len(verification.DanglingRefs) != 1 {
		t.Fatalf("dangling refs = %+v, want one missing resource ref", verification.DanglingRefs)
	}
	if verification.DanglingRefs[0].ObjectNumber != 99 || verification.DanglingRefs[0].Generation != 0 {
		t.Fatalf("dangling ref = %+v, want 99 0 R", verification.DanglingRefs[0])
	}
}

func TestPageOperationVerificationReportsPageCountMismatch(t *testing.T) {
	input := testPDF(
		"<< /Type /Catalog /Pages 2 0 R >>",
		"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
		"<< /Type /Page /Parent 2 0 R /Resources << >> >>",
	)

	verification, err := VerifyPageOperationOutput(input, PageOperationVerificationOptions{
		ExpectedPageCount: 2,
		RequireResources:  true,
	})
	if err != nil {
		t.Fatal(err)
	}

	if verification.PageCountOK {
		t.Fatalf("PageCountOK = true, want false: %+v", verification)
	}
	if verification.ActualPageCount != 1 || verification.ExpectedPageCount != 2 {
		t.Fatalf("page counts = actual %d expected %d, want 1/2", verification.ActualPageCount, verification.ExpectedPageCount)
	}
	if verification.CoreVerification().PageUnchanged {
		t.Fatalf("core PageUnchanged = true, want false")
	}
}
