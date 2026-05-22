package pdf

import (
	"bytes"
	"errors"
	"fmt"

	"github.com/oxhq/binas/pkg/core"
)

const incrementalTextRewriteOperation = "pdf.incremental_content_stream_text_rewrite"

type SignaturePreservationVerification struct {
	IncrementalUpdate           bool   `json:"incremental_update"`
	OriginalBytesPreserved      bool   `json:"original_bytes_preserved"`
	ByteRangeProof              bool   `json:"byte_range_proof"`
	ByteRangesChecked           int    `json:"byte_ranges_checked"`
	SignedByteRangesUnchanged   bool   `json:"signed_byte_ranges_unchanged"`
	CryptographicValidation     bool   `json:"cryptographic_validation"`
	CryptographicValidationNote string `json:"cryptographic_validation_note"`
}

func ApplyIncrementalTextEditPreservingSignatures(input []byte, selector core.Match, mutation core.Mutation, invariants []core.Invariant) ([]byte, core.Report, core.Verification, SignaturePreservationVerification, error) {
	return applyIncrementalTextEdit(input, selector, mutation, invariants, true)
}

func appendIncrementalContentStreamTextReplacement(input []byte, oldText, newText string) ([]byte, error) {
	output, _, _, _, err := applyIncrementalTextEdit(input, core.Match{Kind: KindTextShow, Text: oldText}, core.Mutation{Replace: newText}, nil, false)
	return output, err
}

func applyIncrementalTextEdit(input []byte, selector core.Match, mutation core.Mutation, invariants []core.Invariant, requireSignatureProof bool) ([]byte, core.Report, core.Verification, SignaturePreservationVerification, error) {
	if selector.Kind == "" {
		selector.Kind = KindTextShow
	}
	if selector.Kind != KindTextShow {
		return nil, core.Report{}, core.Verification{}, SignaturePreservationVerification{}, fmt.Errorf("incremental PDF edit supports kind=%q only", KindTextShow)
	}
	if selector.Text == "" {
		return nil, core.Report{}, core.Verification{}, SignaturePreservationVerification{}, errors.New("incremental text replacement requires old text")
	}
	if err := validateIncrementalTextReplacementInput(input); err != nil {
		return nil, core.Report{}, core.Verification{}, SignaturePreservationVerification{}, err
	}
	var byteRanges []signatureByteRange
	boundaries := summarizeResidualBoundariesForInput(input)
	if requireSignatureProof {
		if err := CheckSecurity(input, SecurityOptions{SignatureInvalidation: SignatureInvalidationPreserveIncremental}); err != nil {
			return nil, core.Report{}, core.Verification{}, SignaturePreservationVerification{}, err
		}
		if boundaries.HasSignature {
			var err error
			byteRanges, err = signatureByteRanges(input)
			if err != nil {
				return nil, core.Report{}, core.Verification{}, SignaturePreservationVerification{}, err
			}
		}
	}
	graph, err := parsePDFGraphWithOptions(input, pdfGraphParseOptions{AllowSignature: true})
	if err != nil {
		return nil, core.Report{}, core.Verification{}, SignaturePreservationVerification{}, err
	}
	candidates, err := graph.textShowCandidatesWithCMapContext(selector.Text, graph.cmapContext())
	if err != nil {
		return nil, core.Report{}, core.Verification{}, SignaturePreservationVerification{}, err
	}
	if len(candidates) == 0 {
		return nil, core.Report{}, core.Verification{}, SignaturePreservationVerification{}, fmt.Errorf("no nodes match kind=%q text=%q", KindTextShow, selector.Text)
	}
	index := 0
	var matchIndex *int
	if selector.MatchIndex != nil {
		if *selector.MatchIndex < 0 || *selector.MatchIndex >= len(candidates) {
			return nil, core.Report{}, core.Verification{}, SignaturePreservationVerification{}, fmt.Errorf("match index %d out of range for %d matches (zero-based)", *selector.MatchIndex, len(candidates))
		}
		index = *selector.MatchIndex
		selected := index
		matchIndex = &selected
	} else if len(candidates) > 1 {
		return nil, core.Report{}, core.Verification{}, SignaturePreservationVerification{}, fmt.Errorf("incremental text replacement is ambiguous: %d matches for %q; pass --match-index N (zero-based, 0..%d) to choose one", len(candidates), selector.Text, len(candidates)-1)
	}

	candidate := candidates[index]
	replacement, replacementProof, err := encodeCanonicalTextReplacement(candidate.Show, mutation.Replace)
	if err != nil {
		return nil, core.Report{}, core.Verification{}, SignaturePreservationVerification{}, err
	}
	layoutProofMeta := textShowReplacementReportMetadata(candidate.Show.Meta, textShowReplacementLayoutProofMetadata(candidate.Show.Meta, replacement))
	if err := rejectUnsupportedTextReplacementLayout(layoutProofMeta, replacementProof); err != nil {
		return nil, core.Report{}, core.Verification{}, SignaturePreservationVerification{}, err
	}
	updated, err := replacementContentStream(candidate, replacement)
	if err != nil {
		return nil, core.Report{}, core.Verification{}, SignaturePreservationVerification{}, err
	}
	output, err := appendIncrementalStreamObjectUpdate(input, candidate.Object, updated)
	if err != nil {
		return nil, core.Report{}, core.Verification{}, SignaturePreservationVerification{}, err
	}
	if len(invariants) == 0 {
		invariants = []core.Invariant{
			core.InvariantReparse,
			core.InvariantOldGone,
			core.InvariantNewSelectable,
			core.InvariantPageUnchanged,
			core.InvariantNoFallbackUsed,
		}
	}
	plan := &core.EditPlan{
		Operation:  incrementalTextRewriteOperation,
		OldText:    candidate.Show.Text,
		NewText:    mutation.Replace,
		PageCount:  graph.pageCount(),
		Invariants: invariants,
		Meta:       layoutProofMeta,
	}
	verification, preservation, err := verifyIncrementalTextReplacementOutput(input, output, plan, byteRanges, requireSignatureProof && boundaries.HasSignature)
	if err != nil {
		return nil, core.Report{}, core.Verification{}, SignaturePreservationVerification{}, err
	}
	report := WithNoFallbackPolicy(core.Report{
		Format:        "pdf",
		Edit:          plan.Operation,
		FallbackUsed:  false,
		NodesModified: 1,
		MatchIndex:    matchIndex,
		Invariants:    invariants,
		Meta:          plan.Meta,
	})
	return output, report, verification, preservation, nil
}

func validateIncrementalTextReplacementInput(input []byte) error {
	summary := summarizeXref(input)
	if summary.HasStream || summary.HasHybridStream {
		return errors.New("unsupported incremental text replacement: xref streams are not supported")
	}
	if summary.HasObjectStream {
		return errors.New("unsupported incremental text replacement: object streams are not supported")
	}
	return nil
}

func replacementContentStream(candidate canonicalTextCandidate, replacement string) (pdfStreamObject, error) {
	if candidate.Object == nil {
		return pdfStreamObject{}, errors.New("incremental text replacement target is missing an indirect object")
	}
	if candidate.Object.InObjectStream {
		return pdfStreamObject{}, errors.New("unsupported incremental text replacement: object stream targets are not supported")
	}
	stream := candidate.Stream
	if dictHasType(stream.Dict, "ObjStm") {
		return pdfStreamObject{}, errors.New("unsupported incremental text replacement: object stream targets are not supported")
	}
	if dictHasType(stream.Dict, "XRef") {
		return pdfStreamObject{}, errors.New("unsupported incremental text replacement: xref stream targets are not supported")
	}
	if _, ok := stream.Dict["Filter"]; ok {
		return pdfStreamObject{}, errors.New("unsupported incremental text replacement stream filter")
	}
	if _, ok := stream.Dict["DecodeParms"]; ok {
		return pdfStreamObject{}, errors.New("unsupported incremental text replacement stream decode parameters")
	}
	if !bytes.Equal(candidate.Decoded, stream.Data) {
		return pdfStreamObject{}, errors.New("unsupported incremental text replacement: decoded stream differs from stored stream bytes")
	}
	if candidate.Show.Start < 0 || candidate.Show.End < candidate.Show.Start || candidate.Show.End > len(candidate.Decoded) {
		return pdfStreamObject{}, errors.New("unsafe incremental text replacement: decoded span is outside stream")
	}
	if !bytes.Equal(candidate.Decoded[candidate.Show.Start:candidate.Show.End], []byte(candidate.Show.Encoded)) {
		return pdfStreamObject{}, errors.New("unsafe incremental text replacement: decoded span does not match encoded operand")
	}

	stream.Data = replaceByteRange(candidate.Decoded, candidate.Show.Start, candidate.Show.End, []byte(replacement))
	stream.Dict = clonePDFDict(stream.Dict)
	stream.Dict["Length"] = len(stream.Data)
	return stream, nil
}

func appendIncrementalStreamObjectUpdate(input []byte, object *pdfIndirectObject, stream pdfStreamObject) ([]byte, error) {
	if object == nil {
		return nil, errors.New("incremental stream update requires an indirect object")
	}
	if object.InObjectStream {
		return nil, errors.New("unsupported incremental stream update: object stream targets are not supported")
	}
	if dictHasType(stream.Dict, "ObjStm") {
		return nil, errors.New("unsupported incremental stream update: object streams are not supported")
	}
	if dictHasType(stream.Dict, "XRef") {
		return nil, errors.New("unsupported incremental stream update: xref streams are not supported")
	}
	return appendIncrementalUpdate(input, []incrementalObjectUpdate{
		{ID: object.ID, Value: stream},
	}, nil)
}

func verifyIncrementalTextReplacementOutput(input, output []byte, plan *core.EditPlan, byteRanges []signatureByteRange, signed bool) (core.Verification, SignaturePreservationVerification, error) {
	preservation := SignaturePreservationVerification{
		IncrementalUpdate:           true,
		CryptographicValidation:     false,
		CryptographicValidationNote: "signed byte ranges were compared byte-for-byte; cryptographic signature validation and re-signing are not performed",
	}
	if !bytes.HasPrefix(output, input) {
		return core.Verification{}, preservation, errors.New("incremental text replacement did not preserve original bytes as prefix")
	}
	preservation.OriginalBytesPreserved = true
	preservation.SignedByteRangesUnchanged = true
	if signed {
		preservation.ByteRangeProof = len(byteRanges) > 0
		preservation.ByteRangesChecked = len(byteRanges)
		if len(byteRanges) == 0 {
			return core.Verification{}, preservation, ErrSignedPDFByteRangeProofRequired
		}
		for _, byteRange := range byteRanges {
			start := byteRange.Offset
			end := byteRange.Offset + byteRange.Length
			if end > len(input) || end > len(output) || !bytes.Equal(input[start:end], output[start:end]) {
				preservation.SignedByteRangesUnchanged = false
				return core.Verification{}, preservation, errors.New("incremental text replacement changed bytes covered by /ByteRange")
			}
		}
	}
	graph, err := parsePDFGraphWithOptions(output, pdfGraphParseOptions{AllowSignature: true})
	if err != nil {
		return core.Verification{}, preservation, err
	}
	cmapContext := graph.cmapContext()
	oldMatches, err := graph.textShowCandidatesWithCMapContext(plan.OldText, cmapContext)
	if err != nil {
		return core.Verification{}, preservation, err
	}
	newMatches, err := graph.textShowCandidatesWithCMapContext(plan.NewText, cmapContext)
	if err != nil {
		return core.Verification{}, preservation, err
	}
	if prev, ok := dictInt(graph.Trailer, "Prev"); !ok || prev <= 0 {
		return core.Verification{}, preservation, errors.New("incremental text replacement trailer is missing /Prev")
	}
	pageUnchanged := true
	if plan.PageCount > 0 {
		pageUnchanged = graph.pageCount() == plan.PageCount
	}
	return core.Verification{
		ReparseOK:      true,
		OldTextRemoved: len(oldMatches) == 0,
		NewSelectable:  len(newMatches) > 0,
		PageUnchanged:  pageUnchanged,
	}, preservation, nil
}
