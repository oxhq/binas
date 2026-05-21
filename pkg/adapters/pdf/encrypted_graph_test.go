package pdf

import (
	"bytes"
	"crypto/aes"
	"crypto/cipher"
	"crypto/sha256"
	"encoding/hex"
	"errors"
	"fmt"
	"testing"

	"github.com/oxhq/binas/pkg/core"
)

func TestEncryptedGraphWithExplicitPasswordDecryptsDirectStringAndStream(t *testing.T) {
	fileID := []byte("fixture-file-id1")
	security := mustPDFStandardSecurity(t, standardSecurityR2FixtureDict(), fileID)
	fileKey, ok, err := security.authenticateUserPassword([]byte("user"))
	if err != nil {
		t.Fatal(err)
	}
	if !ok {
		t.Fatal("fixture password did not authenticate")
	}

	plaintextStream := []byte("BT\n(Encrypted graph text) Tj\nET\n")
	encryptedStream, err := security.decryptRC4Object(fileKey, pdfObjectID{Number: 3, Generation: 0}, plaintextStream)
	if err != nil {
		t.Fatal(err)
	}
	encryptedTitle, err := security.decryptRC4Object(fileKey, pdfObjectID{Number: 4, Generation: 0}, []byte("Sensitive Title"))
	if err != nil {
		t.Fatal(err)
	}
	input := encryptedGraphFixturePDF(t, fileID, standardSecurityR2FixtureObject(), encryptedStream, encryptedTitle)

	graph, err := parsePDFGraphWithOptions(input, pdfGraphParseOptions{
		AllowEncryption: true,
		Password:        "user",
	})
	if err != nil {
		t.Fatal(err)
	}

	stream, ok := graph.Objects[pdfObjectID{Number: 3, Generation: 0}].Value.(pdfStreamObject)
	if !ok {
		t.Fatalf("object 3 is %T, want pdfStreamObject", graph.Objects[pdfObjectID{Number: 3, Generation: 0}].Value)
	}
	if string(stream.Data) != string(plaintextStream) {
		t.Fatalf("stream data = %q, want %q", stream.Data, plaintextStream)
	}

	titleDict, ok := graph.Objects[pdfObjectID{Number: 4, Generation: 0}].Value.(pdfDict)
	if !ok {
		t.Fatalf("object 4 is %T, want pdfDict", graph.Objects[pdfObjectID{Number: 4, Generation: 0}].Value)
	}
	title, ok, err := pdfStringBytes(titleDict["Title"])
	if err != nil {
		t.Fatal(err)
	}
	if !ok || string(title) != "Sensitive Title" {
		t.Fatalf("decrypted title = %q ok=%t, want Sensitive Title", title, ok)
	}

	tree := graph.toTree(input)
	matches := tree.Query(core.Match{Kind: KindTextShow, Text: "Encrypted graph text"})
	if len(matches) != 1 {
		t.Fatalf("text matches = %d, want 1", len(matches))
	}
}

func TestEncryptedPDFPublicParseWithPasswordFindsText(t *testing.T) {
	input := standardEncryptedTextFixture(t, "08-15-2024")

	tree, err := ParseWithPassword(input, core.ParseOptions{Strict: true}, "user")
	if err != nil {
		t.Fatal(err)
	}
	matches := tree.Query(core.Match{Kind: KindTextShow, Text: "08-15-2024"})
	if len(matches) != 1 {
		t.Fatalf("matches = %d, want 1", len(matches))
	}

	_, err = ParseWithPassword(input, core.ParseOptions{Strict: true}, "wrong")
	if !errors.Is(err, ErrEncryptedPDFPasswordRequired) {
		t.Fatalf("wrong-password parse error = %v, want ErrEncryptedPDFPasswordRequired", err)
	}
}

func TestEncryptedPDFCanonicalEditWithPasswordReencryptsAndVerifies(t *testing.T) {
	input := standardEncryptedTextFixture(t, "08-15-2024")

	output, report, verification, err := ApplyCanonicalEditWithPassword(
		input,
		"user",
		core.Match{Kind: KindTextShow, Text: "08-15-2024"},
		core.Mutation{Replace: "05-05-2026"},
		nil,
	)
	if err != nil {
		t.Fatal(err)
	}
	if report.Edit != "pdf.canonical_content_stream_text_rewrite" || report.NodesModified != 1 || report.FallbackUsed {
		t.Fatalf("report = %+v, want one canonical edit without fallback", report)
	}
	if !verification.ReparseOK || !verification.OldTextRemoved || !verification.NewSelectable || !verification.PageUnchanged {
		t.Fatalf("verification = %+v, want all text invariants true", verification)
	}
	if bytes.Contains(output, []byte("05-05-2026")) {
		t.Fatal("output contains plaintext replacement; want re-encrypted content stream")
	}
	if _, err := NewAdapter().Parse(output, core.ParseOptions{Strict: true}); !errors.Is(err, ErrEncryptedPDFPasswordRequired) {
		t.Fatalf("default Parse(output) error = %v, want encrypted-PDF refusal", err)
	}
	if err := CheckSecurity(output, SecurityOptions{Password: "user"}); err != nil {
		t.Fatalf("CheckSecurity(output, password) = %v, want supported encrypted PDF", err)
	}

	graph, err := parsePDFGraphWithOptions(output, pdfGraphParseOptions{AllowEncryption: true, Password: "user"})
	if err != nil {
		t.Fatal(err)
	}
	cmapContext := graph.cmapContext()
	oldMatches, err := graph.textShowCandidatesWithCMapContext("08-15-2024", cmapContext)
	if err != nil {
		t.Fatal(err)
	}
	newMatches, err := graph.textShowCandidatesWithCMapContext("05-05-2026", cmapContext)
	if err != nil {
		t.Fatal(err)
	}
	if len(oldMatches) != 0 || len(newMatches) != 1 {
		t.Fatalf("old/new matches = %d/%d, want 0/1", len(oldMatches), len(newMatches))
	}
	titleDict, ok := graph.Objects[pdfObjectID{Number: 5, Generation: 0}].Value.(pdfDict)
	if !ok {
		t.Fatalf("object 5 is %T, want pdfDict", graph.Objects[pdfObjectID{Number: 5, Generation: 0}].Value)
	}
	title, ok, err := pdfStringBytes(titleDict["Title"])
	if err != nil {
		t.Fatal(err)
	}
	if !ok || string(title) != "Sensitive Title" {
		t.Fatalf("title after encrypted rewrite = %q ok=%t, want Sensitive Title", title, ok)
	}
}

func TestEncryptedPDFAESV2CanonicalEditWithPasswordReencryptsAndVerifies(t *testing.T) {
	input := standardEncryptedAESV2TextFixture(t, "08-15-2024")

	output, report, verification, err := ApplyCanonicalEditWithPassword(
		input,
		"user",
		core.Match{Kind: KindTextShow, Text: "08-15-2024"},
		core.Mutation{Replace: "05-05-2026"},
		nil,
	)
	if err != nil {
		t.Fatal(err)
	}
	if report.Edit != "pdf.canonical_content_stream_text_rewrite" || report.NodesModified != 1 || report.FallbackUsed {
		t.Fatalf("report = %+v, want one canonical edit without fallback", report)
	}
	if !verification.ReparseOK || !verification.OldTextRemoved || !verification.NewSelectable || !verification.PageUnchanged {
		t.Fatalf("verification = %+v, want all text invariants true", verification)
	}
	if bytes.Contains(output, []byte("05-05-2026")) {
		t.Fatal("AESV2 output contains plaintext replacement; want re-encrypted content stream")
	}
	if err := CheckSecurity(output, SecurityOptions{Password: "user"}); err != nil {
		t.Fatalf("CheckSecurity(output, password) = %v, want supported AESV2 encrypted PDF", err)
	}
	tree, err := ParseWithPassword(output, core.ParseOptions{Strict: true}, "user")
	if err != nil {
		t.Fatal(err)
	}
	if len(tree.Query(core.Match{Kind: KindTextShow, Text: "08-15-2024"})) != 0 {
		t.Fatal("old AESV2 text still selectable after edit")
	}
	if len(tree.Query(core.Match{Kind: KindTextShow, Text: "05-05-2026"})) != 1 {
		t.Fatal("new AESV2 text not selectable after edit")
	}
}

func TestEncryptedPDFAESV3CanonicalEditWithPasswordReencryptsAndVerifies(t *testing.T) {
	input := standardEncryptedAESV3TextFixture(t, "08-15-2024")

	if _, err := ParseWithPassword(input, core.ParseOptions{Strict: true}, "wrong"); !errors.Is(err, ErrEncryptedPDFPasswordRequired) {
		t.Fatalf("wrong-password parse error = %v, want ErrEncryptedPDFPasswordRequired", err)
	} else if err != nil && bytes.Contains([]byte(err.Error()), []byte("wrong")) {
		t.Fatalf("wrong-password parse error leaked password: %q", err)
	}

	output, report, verification, err := ApplyCanonicalEditWithPassword(
		input,
		"user",
		core.Match{Kind: KindTextShow, Text: "08-15-2024"},
		core.Mutation{Replace: "05-05-2026"},
		nil,
	)
	if err != nil {
		t.Fatal(err)
	}
	if report.Edit != "pdf.canonical_content_stream_text_rewrite" || report.NodesModified != 1 || report.FallbackUsed {
		t.Fatalf("report = %+v, want one canonical AESV3 edit without fallback", report)
	}
	if !verification.ReparseOK || !verification.OldTextRemoved || !verification.NewSelectable || !verification.PageUnchanged {
		t.Fatalf("verification = %+v, want all text invariants true", verification)
	}
	if bytes.Contains(output, []byte("08-15-2024")) || bytes.Contains(output, []byte("05-05-2026")) {
		t.Fatal("AESV3 output contains plaintext old/new text; want encrypted content")
	}
	if _, err := NewAdapter().Parse(output, core.ParseOptions{Strict: true}); !errors.Is(err, ErrEncryptedPDFPasswordRequired) {
		t.Fatalf("default Parse(output) error = %v, want encrypted-PDF refusal", err)
	}
	tree, err := ParseWithPassword(output, core.ParseOptions{Strict: true}, "user")
	if err != nil {
		t.Fatal(err)
	}
	if len(tree.Query(core.Match{Kind: KindTextShow, Text: "08-15-2024"})) != 0 {
		t.Fatal("old AESV3 text still selectable after edit")
	}
	if len(tree.Query(core.Match{Kind: KindTextShow, Text: "05-05-2026"})) != 1 {
		t.Fatal("new AESV3 text not selectable after edit")
	}
	if _, err := ParseWithPassword(output, core.ParseOptions{Strict: true}, "wrong"); !errors.Is(err, ErrEncryptedPDFPasswordRequired) {
		t.Fatalf("wrong-password output parse error = %v, want ErrEncryptedPDFPasswordRequired", err)
	} else if err != nil && bytes.Contains([]byte(err.Error()), []byte("wrong")) {
		t.Fatalf("wrong-password output parse error leaked password: %q", err)
	}
}

func TestEncryptedPDFStreamLevelCryptStdCFFilterArrayEditsWithPassword(t *testing.T) {
	input := standardEncryptedAESV2CryptFilteredTextFixture(t, "08-15-2024", "[/Crypt /FlateDecode]", "[<< /Name /StdCF >> null]")

	tree, err := ParseWithPassword(input, core.ParseOptions{Strict: true}, "user")
	if err != nil {
		t.Fatal(err)
	}
	if len(tree.Query(core.Match{Kind: KindTextShow, Text: "08-15-2024"})) != 1 {
		t.Fatal("stream-level /Crypt StdCF text was not selectable before edit")
	}

	output, report, verification, err := ApplyCanonicalEditWithPassword(
		input,
		"user",
		core.Match{Kind: KindTextShow, Text: "08-15-2024"},
		core.Mutation{Replace: "05-05-2026"},
		nil,
	)
	if err != nil {
		t.Fatal(err)
	}
	if report.Edit != "pdf.canonical_content_stream_text_rewrite" || report.NodesModified != 1 || report.FallbackUsed {
		t.Fatalf("report = %+v, want one canonical edit without fallback", report)
	}
	if !verification.ReparseOK || !verification.OldTextRemoved || !verification.NewSelectable || !verification.PageUnchanged {
		t.Fatalf("verification = %+v, want all text invariants true", verification)
	}
	if bytes.Contains(output, []byte("05-05-2026")) {
		t.Fatal("stream-level /Crypt output contains plaintext replacement; want encrypted output")
	}
	tree, err = ParseWithPassword(output, core.ParseOptions{Strict: true}, "user")
	if err != nil {
		t.Fatal(err)
	}
	if len(tree.Query(core.Match{Kind: KindTextShow, Text: "05-05-2026"})) != 1 {
		t.Fatal("stream-level /Crypt replacement not selectable after edit")
	}
}

func TestEncryptedPDFStreamLevelCryptIdentityEditsWithPassword(t *testing.T) {
	input := standardEncryptedAESV2CryptIdentityTextFixture(t, "08-15-2024")

	tree, err := ParseWithPassword(input, core.ParseOptions{Strict: true}, "user")
	if err != nil {
		t.Fatal(err)
	}
	if len(tree.Query(core.Match{Kind: KindTextShow, Text: "08-15-2024"})) != 1 {
		t.Fatal("stream-level /Crypt Identity text was not selectable before edit")
	}

	output, report, verification, err := ApplyCanonicalEditWithPassword(
		input,
		"user",
		core.Match{Kind: KindTextShow, Text: "08-15-2024"},
		core.Mutation{Replace: "05-05-2026"},
		nil,
	)
	if err != nil {
		t.Fatal(err)
	}
	if report.Edit != "pdf.canonical_content_stream_text_rewrite" || report.NodesModified != 1 || report.FallbackUsed {
		t.Fatalf("report = %+v, want one canonical edit without fallback", report)
	}
	if !verification.ReparseOK || !verification.OldTextRemoved || !verification.NewSelectable || !verification.PageUnchanged {
		t.Fatalf("verification = %+v, want all text invariants true", verification)
	}
	if bytes.Contains(output, []byte("05-05-2026")) {
		t.Fatal("stream-level /Crypt Identity output contains plaintext replacement; want canonical encrypted output")
	}
}

func TestEncryptedGraphAESV3WithFileKeyDecryptsDefaultStringAndStreamAndReencrypts(t *testing.T) {
	security, fileKey := aesV3TestSecurity()
	encryptObject := pdfObjectID{Number: 5, Generation: 0}
	streamID := pdfObjectID{Number: 3, Generation: 0}
	infoID := pdfObjectID{Number: 4, Generation: 0}
	plaintextStream := []byte("BT\n(AESV3 graph text) Tj\nET\n")
	plaintextTitle := []byte("Sensitive AESV3 Title")
	encryptedStream := deterministicAESV3FixtureEncryptedObject(t, fileKey, plaintextStream)
	encryptedTitle := deterministicAESV3FixtureEncryptedObject(t, fileKey, plaintextTitle)
	graph := aesV3EncryptedTestGraph(fileKey, security, encryptObject, map[pdfObjectID]*pdfIndirectObject{
		streamID: {
			ID:    streamID,
			Value: pdfStreamObject{Dict: pdfDict{"Length": len(encryptedStream)}, Data: encryptedStream},
		},
		infoID: {
			ID: infoID,
			Value: pdfDict{
				"Title": pdfHexString(hex.EncodeToString(encryptedTitle)),
			},
		},
	})

	if err := graph.decryptStandardSecurityObjects(); err != nil {
		t.Fatal(err)
	}
	stream, ok := graph.Objects[streamID].Value.(pdfStreamObject)
	if !ok {
		t.Fatalf("object 3 is %T, want stream", graph.Objects[streamID].Value)
	}
	if !bytes.Equal(stream.Data, plaintextStream) {
		t.Fatalf("AESV3 stream = %q, want %q", stream.Data, plaintextStream)
	}
	info, ok := graph.Objects[infoID].Value.(pdfDict)
	if !ok {
		t.Fatalf("object 4 is %T, want dict", graph.Objects[infoID].Value)
	}
	title, ok, err := pdfStringBytes(info["Title"])
	if err != nil {
		t.Fatal(err)
	}
	if !ok || !bytes.Equal(title, plaintextTitle) {
		t.Fatalf("AESV3 title = %q ok=%t, want %q", title, ok, plaintextTitle)
	}

	output, err := writePDFGraphWithOptions(graph, pdfCanonicalWriteOptions{AllowEncryption: true})
	if err != nil {
		t.Fatal(err)
	}
	if bytes.Contains(output, plaintextStream) || bytes.Contains(output, plaintextTitle) {
		t.Fatal("AESV3 canonical output contains plaintext stream/title; want re-encrypted output")
	}

	rawStream, ok := mustRawPDFObjectValue(t, output, streamID).(pdfStreamObject)
	if !ok {
		t.Fatalf("written object 3 is %T, want stream", mustRawPDFObjectValue(t, output, streamID))
	}
	decryptedStream, err := security.decryptObject(fileKey, streamID, rawStream.Data)
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Equal(decryptedStream, plaintextStream) {
		t.Fatalf("written AESV3 stream decrypts to %q, want %q", decryptedStream, plaintextStream)
	}
	rawInfo, ok := mustRawPDFObjectValue(t, output, infoID).(pdfDict)
	if !ok {
		t.Fatalf("written object 4 is %T, want dict", mustRawPDFObjectValue(t, output, infoID))
	}
	rawTitle, ok, err := pdfStringBytes(rawInfo["Title"])
	if err != nil {
		t.Fatal(err)
	}
	if !ok {
		t.Fatal("written AESV3 title is not a string")
	}
	decryptedTitle, err := security.decryptObject(fileKey, infoID, rawTitle)
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Equal(decryptedTitle, plaintextTitle) {
		t.Fatalf("written AESV3 title decrypts to %q, want %q", decryptedTitle, plaintextTitle)
	}
}

func TestEncryptedGraphAESV3ExplicitCryptStdCFStreamRoundTripsWithFileKey(t *testing.T) {
	security, fileKey := aesV3TestSecurity()
	encryptObject := pdfObjectID{Number: 5, Generation: 0}
	streamID := pdfObjectID{Number: 3, Generation: 0}
	plaintext := []byte("BT\n(AESV3 Crypt filter text) Tj\nET\n")
	filtered, err := encodeStreamFilterWithDecodeParms("/FlateDecode", "", plaintext)
	if err != nil {
		t.Fatal(err)
	}
	encryptedFiltered := deterministicAESV3FixtureEncryptedObject(t, fileKey, filtered)
	stream := pdfStreamObject{
		Dict: pdfDict{
			"Length":      len(encryptedFiltered),
			"Filter":      pdfArray{pdfName("Crypt"), pdfName("FlateDecode")},
			"DecodeParms": pdfArray{pdfDict{"Name": pdfName("StdCF")}, nil},
		},
		Data: encryptedFiltered,
	}
	graph := aesV3EncryptedTestGraph(fileKey, security, encryptObject, map[pdfObjectID]*pdfIndirectObject{
		streamID: {ID: streamID, Value: stream},
	})

	if err := graph.decryptStandardSecurityObjects(); err != nil {
		t.Fatal(err)
	}
	stream = graph.Objects[streamID].Value.(pdfStreamObject)
	decoded, err := graph.decodePDFGraphObjectStream(streamID, stream)
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Equal(decoded, plaintext) {
		t.Fatalf("AESV3 /Crypt decoded = %q, want %q", decoded, plaintext)
	}

	updated := []byte("BT\n(AESV3 Crypt replacement) Tj\nET\n")
	reencoded, err := encodeStreamFilterWithDecodeParmsAndCrypt(
		pdfGraphStreamFilterString(stream.Dict),
		graph.pdfGraphDecodeParmsString(stream.Dict),
		updated,
		graph.streamCryptHandler(streamID),
	)
	if err != nil {
		t.Fatal(err)
	}
	if bytes.Contains(reencoded, updated) {
		t.Fatal("AESV3 /Crypt encoded stream contains plaintext replacement")
	}
	stream.Data = reencoded
	graph.Objects[streamID].Value = stream

	output, err := writePDFGraphWithOptions(graph, pdfCanonicalWriteOptions{AllowEncryption: true})
	if err != nil {
		t.Fatal(err)
	}
	rawStream, ok := mustRawPDFObjectValue(t, output, streamID).(pdfStreamObject)
	if !ok {
		t.Fatalf("written object 3 is %T, want stream", mustRawPDFObjectValue(t, output, streamID))
	}
	decodedWritten, err := decodeStreamFilterWithDecodeParmsAndCrypt(
		pdfGraphStreamFilterString(rawStream.Dict),
		graph.pdfGraphDecodeParmsString(rawStream.Dict),
		rawStream.Data,
		graph.streamCryptHandler(streamID),
	)
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Equal(decodedWritten, updated) {
		t.Fatalf("written AESV3 /Crypt stream decodes to %q, want %q", decodedWritten, updated)
	}
}

func TestEncryptedGraphRejectsUnsupportedCryptFilterWithExplicitPassword(t *testing.T) {
	fileID := []byte("revision4-fileid")
	input := encryptedGraphFixturePDF(t, fileID, `<<
/Filter /Standard
/V 4
/R 4
/Length 128
/O <00112233445566778899aabbccddeeff102132435465768798a9babbdcddfeff>
/U <cc2a78aa2a17a179ab7fc41a992e19cfb0b1b2b3b4b5b6b7b8b9babbbcbdbebf>
/P -1028
/StmF /StdCF
/StrF /StdCF
/CF << /StdCF << /CFM /AESV3 >> >>
>>`, nil, nil)

	_, err := parsePDFGraphWithOptions(input, pdfGraphParseOptions{
		AllowEncryption: true,
		Password:        "user",
	})
	if !errors.Is(err, ErrEncryptedPDFUnsupportedAlgorithm) {
		t.Fatalf("parsePDFGraphWithOptions() error = %v, want ErrEncryptedPDFUnsupportedAlgorithm", err)
	}
}

func TestEncryptedGraphObjectStreamCanonicalEditWithPasswordInflatesAndReencrypts(t *testing.T) {
	input := standardEncryptedObjectStreamTextFixture(t, "08-15-2024")

	graph, err := parsePDFGraphWithOptions(input, pdfGraphParseOptions{
		AllowEncryption: true,
		Password:        "user",
	})
	if err != nil {
		t.Fatal(err)
	}
	pageObject := graph.Objects[pdfObjectID{Number: 3, Generation: 0}]
	if pageObject == nil || !pageObject.InObjectStream {
		t.Fatalf("page object = %+v, want inflated object-stream object", pageObject)
	}
	tree := graph.toTree(input)
	if len(tree.Query(core.Match{Kind: KindTextShow, Text: "08-15-2024"})) != 1 {
		t.Fatal("object-stream encrypted graph text was not selectable before edit")
	}

	output, report, verification, err := ApplyCanonicalEditWithPassword(
		input,
		"user",
		core.Match{Kind: KindTextShow, Text: "08-15-2024"},
		core.Mutation{Replace: "05-05-2026"},
		nil,
	)
	if err != nil {
		t.Fatal(err)
	}
	if report.Edit != "pdf.canonical_content_stream_text_rewrite" || report.NodesModified != 1 || report.FallbackUsed {
		t.Fatalf("report = %+v, want one canonical edit without fallback", report)
	}
	if !verification.ReparseOK || !verification.OldTextRemoved || !verification.NewSelectable || !verification.PageUnchanged {
		t.Fatalf("verification = %+v, want all text invariants true", verification)
	}
	if bytes.Contains(output, []byte("/Type /ObjStm")) {
		t.Fatal("canonical encrypted output preserved object stream container")
	}
	if bytes.Contains(output, []byte("05-05-2026")) {
		t.Fatal("canonical encrypted object-stream output contains plaintext replacement")
	}
	tree, err = ParseWithPassword(output, core.ParseOptions{Strict: true}, "user")
	if err != nil {
		t.Fatal(err)
	}
	if len(tree.Query(core.Match{Kind: KindTextShow, Text: "08-15-2024"})) != 0 {
		t.Fatal("old object-stream text still selectable after edit")
	}
	if len(tree.Query(core.Match{Kind: KindTextShow, Text: "05-05-2026"})) != 1 {
		t.Fatal("new object-stream text not selectable after edit")
	}
}

func TestEncryptedGraphXrefStreamCanonicalEditWithPasswordInflatesAndReencrypts(t *testing.T) {
	input := standardEncryptedXrefStreamTextFixture(t, "08-15-2024")

	graph, err := parsePDFGraphWithOptions(input, pdfGraphParseOptions{
		AllowEncryption: true,
		Password:        "user",
	})
	if err != nil {
		t.Fatal(err)
	}
	if graph.Trailer == nil || graph.Root == nil {
		t.Fatalf("xref-stream encrypted graph trailer/root = %+v/%+v, want parsed trailer root", graph.Trailer, graph.Root)
	}
	if len(graph.XrefStream) != 9 {
		t.Fatalf("xref stream entries = %d, want 9", len(graph.XrefStream))
	}
	tree := graph.toTree(input)
	if len(tree.Query(core.Match{Kind: KindTextShow, Text: "08-15-2024"})) != 1 {
		t.Fatal("xref-stream encrypted graph text was not selectable before edit")
	}

	output, report, verification, err := ApplyCanonicalEditWithPassword(
		input,
		"user",
		core.Match{Kind: KindTextShow, Text: "08-15-2024"},
		core.Mutation{Replace: "05-05-2026"},
		nil,
	)
	if err != nil {
		t.Fatal(err)
	}
	if report.Edit != "pdf.canonical_content_stream_text_rewrite" || report.NodesModified != 1 || report.FallbackUsed {
		t.Fatalf("report = %+v, want one canonical edit without fallback", report)
	}
	if !verification.ReparseOK || !verification.OldTextRemoved || !verification.NewSelectable || !verification.PageUnchanged {
		t.Fatalf("verification = %+v, want all text invariants true", verification)
	}
	if bytes.Contains(output, []byte("/Type /XRef")) {
		t.Fatal("canonical encrypted output preserved xref stream container")
	}
	if bytes.Contains(output, []byte("05-05-2026")) {
		t.Fatal("canonical encrypted xref-stream output contains plaintext replacement")
	}
	tree, err = ParseWithPassword(output, core.ParseOptions{Strict: true}, "user")
	if err != nil {
		t.Fatal(err)
	}
	if len(tree.Query(core.Match{Kind: KindTextShow, Text: "08-15-2024"})) != 0 {
		t.Fatal("old xref-stream text still selectable after edit")
	}
	if len(tree.Query(core.Match{Kind: KindTextShow, Text: "05-05-2026"})) != 1 {
		t.Fatal("new xref-stream text not selectable after edit")
	}
}

func TestEncryptedGraphAESV2XrefStreamCanonicalEditWithPasswordInflatesAndReencrypts(t *testing.T) {
	input := standardEncryptedAESV2XrefStreamTextFixture(t, "08-15-2024")

	output, report, verification, err := ApplyCanonicalEditWithPassword(
		input,
		"user",
		core.Match{Kind: KindTextShow, Text: "08-15-2024"},
		core.Mutation{Replace: "05-05-2026"},
		nil,
	)
	if err != nil {
		t.Fatal(err)
	}
	if report.Edit != "pdf.canonical_content_stream_text_rewrite" || report.NodesModified != 1 || report.FallbackUsed {
		t.Fatalf("report = %+v, want one canonical AESV2 xref-stream edit without fallback", report)
	}
	if !verification.ReparseOK || !verification.OldTextRemoved || !verification.NewSelectable || !verification.PageUnchanged {
		t.Fatalf("verification = %+v, want all text invariants true", verification)
	}
	if bytes.Contains(output, []byte("/Type /XRef")) {
		t.Fatal("canonical AESV2 encrypted output preserved xref stream container")
	}
	if bytes.Contains(output, []byte("05-05-2026")) {
		t.Fatal("canonical AESV2 encrypted xref-stream output contains plaintext replacement")
	}
	tree, err := ParseWithPassword(output, core.ParseOptions{Strict: true}, "user")
	if err != nil {
		t.Fatal(err)
	}
	if len(tree.Query(core.Match{Kind: KindTextShow, Text: "08-15-2024"})) != 0 {
		t.Fatal("old AESV2 xref-stream text still selectable after edit")
	}
	if len(tree.Query(core.Match{Kind: KindTextShow, Text: "05-05-2026"})) != 1 {
		t.Fatal("new AESV2 xref-stream text not selectable after edit")
	}
}

func TestEncryptedGraphAESV3ObjectStreamCanonicalEditWithPasswordInflatesAndReencrypts(t *testing.T) {
	input := standardEncryptedAESV3ObjectStreamTextFixture(t, "08-15-2024")

	tree, err := ParseWithPassword(input, core.ParseOptions{Strict: true}, "user")
	if err != nil {
		t.Fatal(err)
	}
	if len(tree.Query(core.Match{Kind: KindTextShow, Text: "08-15-2024"})) != 1 {
		t.Fatal("AESV3 object-stream encrypted graph text was not selectable before edit")
	}

	output, report, verification, err := ApplyCanonicalEditWithPassword(
		input,
		"user",
		core.Match{Kind: KindTextShow, Text: "08-15-2024"},
		core.Mutation{Replace: "05-05-2026"},
		nil,
	)
	if err != nil {
		t.Fatal(err)
	}
	if report.Edit != "pdf.canonical_content_stream_text_rewrite" || report.NodesModified != 1 || report.FallbackUsed {
		t.Fatalf("report = %+v, want one canonical AESV3 object-stream edit without fallback", report)
	}
	if !verification.ReparseOK || !verification.OldTextRemoved || !verification.NewSelectable || !verification.PageUnchanged {
		t.Fatalf("verification = %+v, want all text invariants true", verification)
	}
	if bytes.Contains(output, []byte("/Type /ObjStm")) {
		t.Fatal("canonical AESV3 encrypted output preserved object stream container")
	}
	if bytes.Contains(output, []byte("08-15-2024")) || bytes.Contains(output, []byte("05-05-2026")) {
		t.Fatal("canonical AESV3 encrypted object-stream output contains plaintext old/new text")
	}
	tree, err = ParseWithPassword(output, core.ParseOptions{Strict: true}, "user")
	if err != nil {
		t.Fatal(err)
	}
	if len(tree.Query(core.Match{Kind: KindTextShow, Text: "08-15-2024"})) != 0 {
		t.Fatal("old AESV3 object-stream text still selectable after edit")
	}
	if len(tree.Query(core.Match{Kind: KindTextShow, Text: "05-05-2026"})) != 1 {
		t.Fatal("new AESV3 object-stream text not selectable after edit")
	}
	if _, err := ParseWithPassword(output, core.ParseOptions{Strict: true}, "wrong"); !errors.Is(err, ErrEncryptedPDFPasswordRequired) {
		t.Fatalf("wrong-password AESV3 object-stream output parse error = %v, want ErrEncryptedPDFPasswordRequired", err)
	} else if err != nil && bytes.Contains([]byte(err.Error()), []byte("wrong")) {
		t.Fatalf("wrong-password AESV3 object-stream output parse error leaked password: %q", err)
	}
}

func TestEncryptedGraphAESV3XrefStreamCanonicalEditWithPasswordInflatesAndReencrypts(t *testing.T) {
	input := standardEncryptedAESV3XrefStreamTextFixture(t, "08-15-2024")

	tree, err := ParseWithPassword(input, core.ParseOptions{Strict: true}, "user")
	if err != nil {
		t.Fatal(err)
	}
	if len(tree.Query(core.Match{Kind: KindTextShow, Text: "08-15-2024"})) != 1 {
		t.Fatal("AESV3 xref-stream encrypted graph text was not selectable before edit")
	}

	output, report, verification, err := ApplyCanonicalEditWithPassword(
		input,
		"user",
		core.Match{Kind: KindTextShow, Text: "08-15-2024"},
		core.Mutation{Replace: "05-05-2026"},
		nil,
	)
	if err != nil {
		t.Fatal(err)
	}
	if report.Edit != "pdf.canonical_content_stream_text_rewrite" || report.NodesModified != 1 || report.FallbackUsed {
		t.Fatalf("report = %+v, want one canonical AESV3 xref-stream edit without fallback", report)
	}
	if !verification.ReparseOK || !verification.OldTextRemoved || !verification.NewSelectable || !verification.PageUnchanged {
		t.Fatalf("verification = %+v, want all text invariants true", verification)
	}
	if bytes.Contains(output, []byte("/Type /XRef")) {
		t.Fatal("canonical AESV3 encrypted output preserved xref stream container")
	}
	if bytes.Contains(output, []byte("08-15-2024")) || bytes.Contains(output, []byte("05-05-2026")) {
		t.Fatal("canonical AESV3 encrypted xref-stream output contains plaintext old/new text")
	}
	tree, err = ParseWithPassword(output, core.ParseOptions{Strict: true}, "user")
	if err != nil {
		t.Fatal(err)
	}
	if len(tree.Query(core.Match{Kind: KindTextShow, Text: "08-15-2024"})) != 0 {
		t.Fatal("old AESV3 xref-stream text still selectable after edit")
	}
	if len(tree.Query(core.Match{Kind: KindTextShow, Text: "05-05-2026"})) != 1 {
		t.Fatal("new AESV3 xref-stream text not selectable after edit")
	}
	if _, err := ParseWithPassword(output, core.ParseOptions{Strict: true}, "wrong"); !errors.Is(err, ErrEncryptedPDFPasswordRequired) {
		t.Fatalf("wrong-password AESV3 xref-stream output parse error = %v, want ErrEncryptedPDFPasswordRequired", err)
	} else if err != nil && bytes.Contains([]byte(err.Error()), []byte("wrong")) {
		t.Fatalf("wrong-password AESV3 xref-stream output parse error leaked password: %q", err)
	}
}

func standardEncryptedAESV2TextFixture(t *testing.T, text string) []byte {
	t.Helper()
	fileID := []byte("revision4-aes-id")
	dict := standardSecurityAESV2FixtureDict(t, fileID)
	security := mustPDFStandardSecurity(t, dict, fileID)
	fileKey, ok, err := security.authenticateUserPassword([]byte("user"))
	if err != nil {
		t.Fatal(err)
	}
	if !ok {
		t.Fatal("fixture password did not authenticate")
	}
	plaintextStream := []byte(fmt.Sprintf("BT\n(%s) Tj\nET\n", encodeLiteralString(text)))
	encryptedStream := deterministicAESV2EncryptedObject(t, security, fileKey, pdfObjectID{Number: 4, Generation: 0}, plaintextStream)
	encryptedTitle := deterministicAESV2EncryptedObject(t, security, fileKey, pdfObjectID{Number: 5, Generation: 0}, []byte("Sensitive Title"))
	return encryptedGraphFixturePDFWithObjects(t, fileID, pdfValueString(t, dict), encryptedStream, encryptedTitle)
}

func standardEncryptedAESV3TextFixture(t *testing.T, text string) []byte {
	t.Helper()
	fileID := []byte("revision5-aesv3")
	dict, fileKey := standardSecurityAESV3GraphFixtureDict(t, fileID)
	plaintextStream := []byte(fmt.Sprintf("BT\n(%s) Tj\nET\n", encodeLiteralString(text)))
	encryptedStream := deterministicAESV3FixtureEncryptedObject(t, fileKey, plaintextStream)
	encryptedTitle := deterministicAESV3FixtureEncryptedObject(t, fileKey, []byte("Sensitive Title"))
	return encryptedGraphFixturePDFWithObjects(t, fileID, pdfValueString(t, dict), encryptedStream, encryptedTitle)
}

func standardEncryptedObjectStreamTextFixture(t *testing.T, text string) []byte {
	t.Helper()
	fileID := []byte("fixture-file-id1")
	security := mustPDFStandardSecurity(t, standardSecurityR2FixtureDict(), fileID)
	fileKey, ok, err := security.authenticateUserPassword([]byte("user"))
	if err != nil {
		t.Fatal(err)
	}
	if !ok {
		t.Fatal("fixture password did not authenticate")
	}
	plaintextStream := []byte(fmt.Sprintf("BT\n(%s) Tj\nET\n", encodeLiteralString(text)))
	encryptedContent, err := security.decryptRC4Object(fileKey, pdfObjectID{Number: 4, Generation: 0}, plaintextStream)
	if err != nil {
		t.Fatal(err)
	}
	encryptedTitle, err := security.decryptRC4Object(fileKey, pdfObjectID{Number: 5, Generation: 0}, []byte("Sensitive Title"))
	if err != nil {
		t.Fatal(err)
	}
	objectStreamData := makeObjectStreamData(t,
		objectStreamEntry{number: 1, value: "<< /Type /Catalog /Pages 2 0 R >>"},
		objectStreamEntry{number: 2, value: "<< /Type /Pages /Kids [3 0 R] /Count 1 >>"},
		objectStreamEntry{number: 3, value: "<< /Type /Page /Parent 2 0 R /Contents 4 0 R >>"},
	)
	first := bytes.IndexByte(objectStreamData, '\n') + 1
	encryptedObjectStream, err := security.decryptRC4Object(fileKey, pdfObjectID{Number: 7, Generation: 0}, objectStreamData)
	if err != nil {
		t.Fatal(err)
	}
	return encryptedGraphFixturePDFWithEncryptedObjectStream(t, fileID, standardSecurityR2FixtureObject(), encryptedContent, encryptedTitle, encryptedObjectStream, first)
}

func standardEncryptedAESV3ObjectStreamTextFixture(t *testing.T, text string) []byte {
	t.Helper()
	fileID := []byte("revision5-aesv3")
	dict, fileKey := standardSecurityAESV3GraphFixtureDict(t, fileID)
	plaintextStream := []byte(fmt.Sprintf("BT\n(%s) Tj\nET\n", encodeLiteralString(text)))
	encryptedContent := deterministicAESV3FixtureEncryptedObject(t, fileKey, plaintextStream)
	encryptedTitle := deterministicAESV3FixtureEncryptedObject(t, fileKey, []byte("Sensitive Title"))
	objectStreamData := makeObjectStreamData(t,
		objectStreamEntry{number: 1, value: "<< /Type /Catalog /Pages 2 0 R >>"},
		objectStreamEntry{number: 2, value: "<< /Type /Pages /Kids [3 0 R] /Count 1 >>"},
		objectStreamEntry{number: 3, value: "<< /Type /Page /Parent 2 0 R /Contents 4 0 R >>"},
	)
	first := bytes.IndexByte(objectStreamData, '\n') + 1
	encryptedObjectStream := deterministicAESV3FixtureEncryptedObject(t, fileKey, objectStreamData)
	return encryptedGraphFixturePDFWithEncryptedObjectStream(t, fileID, pdfValueString(t, dict), encryptedContent, encryptedTitle, encryptedObjectStream, first)
}

func standardEncryptedXrefStreamTextFixture(t *testing.T, text string) []byte {
	t.Helper()
	fileID := []byte("fixture-file-id1")
	security := mustPDFStandardSecurity(t, standardSecurityR2FixtureDict(), fileID)
	fileKey, ok, err := security.authenticateUserPassword([]byte("user"))
	if err != nil {
		t.Fatal(err)
	}
	if !ok {
		t.Fatal("fixture password did not authenticate")
	}
	plaintextStream := []byte(fmt.Sprintf("BT\n(%s) Tj\nET\n", encodeLiteralString(text)))
	encryptedContent, err := security.decryptRC4Object(fileKey, pdfObjectID{Number: 4, Generation: 0}, plaintextStream)
	if err != nil {
		t.Fatal(err)
	}
	encryptedTitle, err := security.decryptRC4Object(fileKey, pdfObjectID{Number: 5, Generation: 0}, []byte("Sensitive Title"))
	if err != nil {
		t.Fatal(err)
	}
	return encryptedGraphFixturePDFWithEncryptedXrefStream(t, fileID, security, fileKey, standardSecurityR2FixtureObject(), encryptedContent, encryptedTitle)
}

func standardEncryptedAESV3XrefStreamTextFixture(t *testing.T, text string) []byte {
	t.Helper()
	fileID := []byte("revision5-aesv3")
	dict, fileKey := standardSecurityAESV3GraphFixtureDict(t, fileID)
	plaintextStream := []byte(fmt.Sprintf("BT\n(%s) Tj\nET\n", encodeLiteralString(text)))
	encryptedContent := deterministicAESV3FixtureEncryptedObject(t, fileKey, plaintextStream)
	encryptedTitle := deterministicAESV3FixtureEncryptedObject(t, fileKey, []byte("Sensitive Title"))
	return encryptedGraphFixturePDFWithAESV3EncryptedXrefStream(t, fileID, fileKey, pdfValueString(t, dict), encryptedContent, encryptedTitle)
}

func standardEncryptedAESV2XrefStreamTextFixture(t *testing.T, text string) []byte {
	t.Helper()
	fileID := []byte("revision4-aes-id")
	dict := standardSecurityAESV2FixtureDict(t, fileID)
	security := mustPDFStandardSecurity(t, dict, fileID)
	fileKey, ok, err := security.authenticateUserPassword([]byte("user"))
	if err != nil {
		t.Fatal(err)
	}
	if !ok {
		t.Fatal("fixture password did not authenticate")
	}
	plaintextStream := []byte(fmt.Sprintf("BT\n(%s) Tj\nET\n", encodeLiteralString(text)))
	encryptedContent := deterministicAESV2EncryptedObject(t, security, fileKey, pdfObjectID{Number: 4, Generation: 0}, plaintextStream)
	encryptedTitle := deterministicAESV2EncryptedObject(t, security, fileKey, pdfObjectID{Number: 5, Generation: 0}, []byte("Sensitive Title"))
	return encryptedGraphFixturePDFWithEncryptedXrefStream(t, fileID, security, fileKey, pdfValueString(t, dict), encryptedContent, encryptedTitle)
}

func standardEncryptedAESV2CryptFilteredTextFixture(t *testing.T, text, filter, decodeParms string) []byte {
	t.Helper()
	fileID := []byte("revision4-cryptf")
	dict := standardSecurityAESV2FixtureDict(t, fileID)
	security := mustPDFStandardSecurity(t, dict, fileID)
	fileKey, ok, err := security.authenticateUserPassword([]byte("user"))
	if err != nil {
		t.Fatal(err)
	}
	if !ok {
		t.Fatal("fixture password did not authenticate")
	}
	plaintextStream := []byte(fmt.Sprintf("BT\n(%s) Tj\nET\n", encodeLiteralString(text)))
	filteredStream, err := encodeStreamFilterWithDecodeParms("/FlateDecode", "", plaintextStream)
	if err != nil {
		t.Fatal(err)
	}
	encryptedStream := deterministicAESV2EncryptedObject(t, security, fileKey, pdfObjectID{Number: 3, Generation: 0}, filteredStream)
	encryptedTitle := deterministicAESV2EncryptedObject(t, security, fileKey, pdfObjectID{Number: 4, Generation: 0}, []byte("Sensitive Title"))
	streamDict := fmt.Sprintf("<< /Length %%d /Filter %s /DecodeParms %s >>", filter, decodeParms)
	return encryptedGraphFixturePDFWithStreamDict(t, fileID, pdfValueString(t, dict), streamDict, encryptedStream, encryptedTitle)
}

func standardEncryptedAESV2CryptIdentityTextFixture(t *testing.T, text string) []byte {
	t.Helper()
	fileID := []byte("revision4-ident")
	dict := standardSecurityAESV2FixtureDict(t, fileID)
	security := mustPDFStandardSecurity(t, dict, fileID)
	fileKey, ok, err := security.authenticateUserPassword([]byte("user"))
	if err != nil {
		t.Fatal(err)
	}
	if !ok {
		t.Fatal("fixture password did not authenticate")
	}
	plaintextStream := []byte(fmt.Sprintf("BT\n(%s) Tj\nET\n", encodeLiteralString(text)))
	encryptedTitle := deterministicAESV2EncryptedObject(t, security, fileKey, pdfObjectID{Number: 4, Generation: 0}, []byte("Sensitive Title"))
	streamDict := "<< /Length %d /Filter /Crypt /DecodeParms << /Name /Identity >> >>"
	return encryptedGraphFixturePDFWithStreamDict(t, fileID, pdfValueString(t, dict), streamDict, plaintextStream, encryptedTitle)
}

func deterministicAESV2EncryptedObject(t *testing.T, security *pdfStandardSecurity, fileKey []byte, id pdfObjectID, plaintext []byte) []byte {
	t.Helper()
	objectKey, err := security.objectKeyWithMethod(fileKey, id, pdfStandardCryptAESV2)
	if err != nil {
		t.Fatal(err)
	}
	block, err := aes.NewCipher(objectKey)
	if err != nil {
		t.Fatal(err)
	}
	for i := 0; i < 256; i++ {
		iv := []byte("binas-test-iv-00")
		iv[len(iv)-1] = byte(i)
		data := pkcs7Pad(plaintext, aes.BlockSize)
		cipher.NewCBCEncrypter(block, iv).CryptBlocks(data, data)
		out := make([]byte, 0, len(iv)+len(data))
		out = append(out, iv...)
		out = append(out, data...)
		if !bytes.Contains(out, []byte("stream")) && !bytes.Contains(out, []byte("endobj")) {
			return out
		}
	}
	t.Fatal("could not build deterministic AESV2 fixture stream without PDF delimiter tokens")
	return nil
}

func aesV3TestSecurity() (*pdfStandardSecurity, []byte) {
	fileKey := []byte("0123456789abcdef0123456789abcdef")
	return &pdfStandardSecurity{
		version:         5,
		revision:        5,
		keyLengthBytes:  len(fileKey),
		encryptMetadata: true,
		cryptMethod:     pdfStandardCryptAESV3,
	}, bytes.Clone(fileKey)
}

func aesV3EncryptedTestGraph(fileKey []byte, security *pdfStandardSecurity, encryptObject pdfObjectID, objects map[pdfObjectID]*pdfIndirectObject) *pdfGraph {
	fileID := []byte("aesv3-file-id-01")
	objects[pdfObjectID{Number: 1, Generation: 0}] = &pdfIndirectObject{
		ID:    pdfObjectID{Number: 1, Generation: 0},
		Value: pdfDict{"Type": pdfName("Catalog")},
	}
	objects[encryptObject] = &pdfIndirectObject{
		ID: encryptObject,
		Value: pdfDict{
			"Filter": pdfName("Standard"),
			"V":      5,
			"R":      5,
			"Length": 256,
			"StmF":   pdfName("StdCF"),
			"StrF":   pdfName("StdCF"),
			"CF": pdfDict{
				"StdCF": pdfDict{
					"CFM":    pdfName("AESV3"),
					"Length": 256,
				},
			},
		},
	}
	return &pdfGraph{
		Header:     "%PDF-1.7",
		Objects:    objects,
		Trailer:    pdfDict{"Size": 6, "Root": pdfRef{ID: pdfObjectID{Number: 1, Generation: 0}}, "Encrypt": pdfRef{ID: encryptObject}, "ID": pdfArray{pdfHexString(hex.EncodeToString(fileID)), pdfHexString(hex.EncodeToString(fileID))}},
		Root:       &pdfObjectID{Number: 1, Generation: 0},
		Boundaries: residualBoundarySummary{HasEncryption: true},
		Encryption: &pdfGraphEncryption{security: security, fileKey: bytes.Clone(fileKey), encryptObject: &encryptObject},
	}
}

func mustRawPDFObjectValue(t *testing.T, input []byte, id pdfObjectID) pdfValue {
	t.Helper()
	objects := findXrefObjectOffsets(input)
	for _, object := range objects {
		if object.Number != id.Number || object.Generation != id.Generation || object.Offset < 0 {
			continue
		}
		value, err := parsePDFObjectValueAt(input, object, objects)
		if err != nil {
			t.Fatal(err)
		}
		return value
	}
	t.Fatalf("object %d %d not found", id.Number, id.Generation)
	return nil
}

func standardSecurityAESV3GraphFixtureDict(t *testing.T, fileID []byte) (pdfDict, []byte) {
	t.Helper()
	fileKey := []byte("0123456789abcdef0123456789abcdef")
	userValidationSalt := []byte("uvalsalt")
	userKeySalt := []byte("ukeysalt")
	ownerValidationSalt := []byte("ovalsalt")
	ownerKeySalt := []byte("okeysalt")
	userPassword := []byte("user")
	ownerPassword := []byte("owner")

	userHash := sha256.Sum256(append(bytes.Clone(userPassword), userValidationSalt...))
	userEntry := make([]byte, 0, 48)
	userEntry = append(userEntry, userHash[:]...)
	userEntry = append(userEntry, userValidationSalt...)
	userEntry = append(userEntry, userKeySalt...)

	ownerHashInput := append(bytes.Clone(ownerPassword), ownerValidationSalt...)
	ownerHashInput = append(ownerHashInput, userEntry...)
	ownerHash := sha256.Sum256(ownerHashInput)
	ownerEntry := make([]byte, 0, 48)
	ownerEntry = append(ownerEntry, ownerHash[:]...)
	ownerEntry = append(ownerEntry, ownerValidationSalt...)
	ownerEntry = append(ownerEntry, ownerKeySalt...)

	userFileKeyHash := sha256.Sum256(append(bytes.Clone(userPassword), userKeySalt...))
	ownerFileKeyInput := append(bytes.Clone(ownerPassword), ownerKeySalt...)
	ownerFileKeyInput = append(ownerFileKeyInput, userEntry...)
	ownerFileKeyHash := sha256.Sum256(ownerFileKeyInput)
	userEncryptedFileKey := aes256CBCNoPaddingEncrypt(t, userFileKeyHash[:], fileKey)
	ownerEncryptedFileKey := aes256CBCNoPaddingEncrypt(t, ownerFileKeyHash[:], fileKey)

	return pdfDict{
		"Filter": pdfName("Standard"),
		"V":      5,
		"R":      5,
		"Length": 256,
		"O":      pdfHexString(hex.EncodeToString(ownerEntry)),
		"U":      pdfHexString(hex.EncodeToString(userEntry)),
		"OE":     pdfHexString(hex.EncodeToString(ownerEncryptedFileKey)),
		"UE":     pdfHexString(hex.EncodeToString(userEncryptedFileKey)),
		"P":      -1028,
		"Perms":  pdfHexString(hex.EncodeToString(deterministicAESV3Perms(t, fileKey, -1028, true))),
		"StmF":   pdfName("StdCF"),
		"StrF":   pdfName("StdCF"),
		"CF": pdfDict{
			"StdCF": pdfDict{
				"CFM":    pdfName("AESV3"),
				"Length": 256,
			},
		},
	}, bytes.Clone(fileKey)
}

func aes256CBCNoPaddingEncrypt(t *testing.T, key, plaintext []byte) []byte {
	t.Helper()
	if len(plaintext)%aes.BlockSize != 0 {
		t.Fatalf("AES-256-CBC fixture plaintext length = %d, want block aligned", len(plaintext))
	}
	block, err := aes.NewCipher(key)
	if err != nil {
		t.Fatal(err)
	}
	out := bytes.Clone(plaintext)
	iv := make([]byte, aes.BlockSize)
	cipher.NewCBCEncrypter(block, iv).CryptBlocks(out, out)
	return out
}

func pdfValueString(t *testing.T, value pdfValue) string {
	t.Helper()
	var out bytes.Buffer
	if err := writePDFValue(&out, value); err != nil {
		t.Fatal(err)
	}
	return out.String()
}

func standardEncryptedTextFixture(t *testing.T, text string) []byte {
	t.Helper()
	fileID := []byte("fixture-file-id1")
	security := mustPDFStandardSecurity(t, standardSecurityR2FixtureDict(), fileID)
	fileKey, ok, err := security.authenticateUserPassword([]byte("user"))
	if err != nil {
		t.Fatal(err)
	}
	if !ok {
		t.Fatal("fixture password did not authenticate")
	}
	plaintextStream := []byte(fmt.Sprintf("BT\n(%s) Tj\nET\n", encodeLiteralString(text)))
	encryptedStream, err := security.decryptRC4Object(fileKey, pdfObjectID{Number: 4, Generation: 0}, plaintextStream)
	if err != nil {
		t.Fatal(err)
	}
	encryptedTitle, err := security.decryptRC4Object(fileKey, pdfObjectID{Number: 5, Generation: 0}, []byte("Sensitive Title"))
	if err != nil {
		t.Fatal(err)
	}
	return encryptedGraphFixturePDFWithObjects(t, fileID, standardSecurityR2FixtureObject(), encryptedStream, encryptedTitle)
}

func encryptedGraphFixturePDFWithObjects(t *testing.T, fileID []byte, encryptObject string, streamData, titleData []byte) []byte {
	t.Helper()

	var input bytes.Buffer
	input.WriteString("%PDF-1.3\n")
	offsets := make([]int, 0, 6)
	writeObject := func(number int, body []byte) {
		t.Helper()
		offsets = append(offsets, input.Len())
		fmt.Fprintf(&input, "%d 0 obj\n", number)
		input.Write(body)
		input.WriteString("\nendobj\n")
	}

	writeObject(1, []byte("<< /Type /Catalog /Pages 2 0 R >>"))
	writeObject(2, []byte("<< /Type /Pages /Kids [3 0 R] /Count 1 >>"))
	writeObject(3, []byte("<< /Type /Page /Parent 2 0 R /Contents 4 0 R >>"))
	offsets = append(offsets, input.Len())
	input.WriteString("4 0 obj\n")
	fmt.Fprintf(&input, "<< /Length %d >>\nstream\n", len(streamData))
	input.Write(streamData)
	input.WriteString("\nendstream\nendobj\n")
	writeObject(5, []byte(fmt.Sprintf("<< /Title <%s> >>", hex.EncodeToString(titleData))))
	writeObject(6, []byte(encryptObject))

	xrefOffset := input.Len()
	input.WriteString("xref\n0 7\n")
	input.WriteString("0000000000 65535 f \n")
	for _, offset := range offsets {
		fmt.Fprintf(&input, "%010d 00000 n \n", offset)
	}
	fmt.Fprintf(&input, "trailer\n<< /Size 7 /Root 1 0 R /Info 5 0 R /Encrypt 6 0 R /ID [<%s> <%s>] >>\nstartxref\n%d\n%%%%EOF\n",
		hex.EncodeToString(fileID),
		hex.EncodeToString(fileID),
		xrefOffset,
	)
	return input.Bytes()
}

func encryptedGraphFixturePDFWithEncryptedXrefStream(t *testing.T, fileID []byte, security *pdfStandardSecurity, fileKey []byte, encryptObject string, contentStream, titleData []byte) []byte {
	t.Helper()

	var input bytes.Buffer
	input.WriteString("%PDF-1.5\n")
	offsets := map[int]int{}
	writeObject := func(number int, body []byte) {
		t.Helper()
		offsets[number] = input.Len()
		fmt.Fprintf(&input, "%d 0 obj\n", number)
		input.Write(body)
		input.WriteString("\nendobj\n")
	}

	writeObject(1, []byte("<< /Type /Catalog /Pages 2 0 R >>"))
	writeObject(2, []byte("<< /Type /Pages /Kids [3 0 R] /Count 1 >>"))
	writeObject(3, []byte("<< /Type /Page /Parent 2 0 R /Contents 4 0 R >>"))
	offsets[4] = input.Len()
	input.WriteString("4 0 obj\n")
	fmt.Fprintf(&input, "<< /Length %d >>\nstream\n", len(contentStream))
	input.Write(contentStream)
	input.WriteString("\nendstream\nendobj\n")
	writeObject(5, []byte(fmt.Sprintf("<< /Title <%s> >>", hex.EncodeToString(titleData))))
	writeObject(6, []byte(encryptObject))

	xrefOffset := input.Len()
	offsets[8] = xrefOffset
	var xrefData bytes.Buffer
	for number := 0; number <= 8; number++ {
		offset, ok := offsets[number]
		if !ok {
			writeXrefStreamEntry(&xrefData, 0, 0, 0)
			continue
		}
		writeXrefStreamEntry(&xrefData, 1, offset, 0)
	}
	// XRef streams must stay parseable before object decryption. The fixture still
	// encrypts regular stream/string objects, but keeps the cross-reference stream
	// bytes clear so the parser can locate the encryption dictionary deterministically.
	xrefStream := xrefData.Bytes()
	input.WriteString("8 0 obj\n")
	fmt.Fprintf(&input, "<< /Type /XRef /Size 9 /Root 1 0 R /Info 5 0 R /Encrypt 6 0 R /ID [<%s> <%s>] /W [1 4 1] /Length %d >>\nstream\n",
		hex.EncodeToString(fileID),
		hex.EncodeToString(fileID),
		len(xrefStream),
	)
	input.Write(xrefStream)
	input.WriteString("\nendstream\nendobj\n")
	fmt.Fprintf(&input, "startxref\n%d\n%%%%EOF\n", xrefOffset)
	return input.Bytes()
}

func encryptedGraphFixturePDFWithAESV3EncryptedXrefStream(t *testing.T, fileID, fileKey []byte, encryptObject string, contentStream, titleData []byte) []byte {
	t.Helper()

	var input bytes.Buffer
	input.WriteString("%PDF-1.7\n")
	offsets := map[int]int{}
	writeObject := func(number int, body []byte) {
		t.Helper()
		offsets[number] = input.Len()
		fmt.Fprintf(&input, "%d 0 obj\n", number)
		input.Write(body)
		input.WriteString("\nendobj\n")
	}

	writeObject(1, []byte("<< /Type /Catalog /Pages 2 0 R >>"))
	writeObject(2, []byte("<< /Type /Pages /Kids [3 0 R] /Count 1 >>"))
	writeObject(3, []byte("<< /Type /Page /Parent 2 0 R /Contents 4 0 R >>"))
	offsets[4] = input.Len()
	input.WriteString("4 0 obj\n")
	fmt.Fprintf(&input, "<< /Length %d >>\nstream\n", len(contentStream))
	input.Write(contentStream)
	input.WriteString("\nendstream\nendobj\n")
	writeObject(5, []byte(fmt.Sprintf("<< /Title <%s> >>", hex.EncodeToString(titleData))))
	writeObject(6, []byte(encryptObject))

	xrefOffset := input.Len()
	offsets[8] = xrefOffset
	var xrefData bytes.Buffer
	for number := 0; number <= 8; number++ {
		offset, ok := offsets[number]
		if !ok {
			writeXrefStreamEntry(&xrefData, 0, 0, 0)
			continue
		}
		writeXrefStreamEntry(&xrefData, 1, offset, 0)
	}
	encryptedXrefStream := deterministicAESV3FixtureEncryptedObject(t, fileKey, xrefData.Bytes())
	input.WriteString("8 0 obj\n")
	fmt.Fprintf(&input, "<< /Type /XRef /Size 9 /Root 1 0 R /Info 5 0 R /Encrypt 6 0 R /ID [<%s> <%s>] /W [1 4 1] /Length %d >>\nstream\n",
		hex.EncodeToString(fileID),
		hex.EncodeToString(fileID),
		len(encryptedXrefStream),
	)
	input.Write(encryptedXrefStream)
	input.WriteString("\nendstream\nendobj\n")
	fmt.Fprintf(&input, "startxref\n%d\n%%%%EOF\n", xrefOffset)
	return input.Bytes()
}

type objectStreamEntry struct {
	number int
	value  string
}

func makeObjectStreamData(t *testing.T, entries ...objectStreamEntry) []byte {
	t.Helper()
	var body bytes.Buffer
	offsets := make([]int, 0, len(entries))
	for _, entry := range entries {
		offsets = append(offsets, body.Len())
		body.WriteString(entry.value)
		body.WriteByte('\n')
	}
	var header bytes.Buffer
	for i, entry := range entries {
		fmt.Fprintf(&header, "%d %d ", entry.number, offsets[i])
	}
	header.WriteByte('\n')
	out := append(bytes.Clone(header.Bytes()), body.Bytes()...)
	return out
}

func encryptedGraphFixturePDFWithEncryptedObjectStream(t *testing.T, fileID []byte, encryptObject string, contentStream, titleData, objectStreamData []byte, first int) []byte {
	t.Helper()

	var input bytes.Buffer
	input.WriteString("%PDF-1.5\n")
	offsets := make([]int, 0, 4)
	writeObject := func(number int, body []byte) {
		t.Helper()
		offsets = append(offsets, input.Len())
		fmt.Fprintf(&input, "%d 0 obj\n", number)
		input.Write(body)
		input.WriteString("\nendobj\n")
	}

	offsets = append(offsets, input.Len())
	input.WriteString("4 0 obj\n")
	fmt.Fprintf(&input, "<< /Length %d >>\nstream\n", len(contentStream))
	input.Write(contentStream)
	input.WriteString("\nendstream\nendobj\n")
	writeObject(5, []byte(fmt.Sprintf("<< /Title <%s> >>", hex.EncodeToString(titleData))))
	writeObject(6, []byte(encryptObject))
	offsets = append(offsets, input.Len())
	input.WriteString("7 0 obj\n")
	fmt.Fprintf(&input, "<< /Type /ObjStm /N 3 /First %d /Length %d >>\nstream\n", first, len(objectStreamData))
	input.Write(objectStreamData)
	input.WriteString("\nendstream\nendobj\n")

	xrefOffset := input.Len()
	input.WriteString("xref\n0 8\n")
	input.WriteString("0000000000 65535 f \n")
	offsetByObject := map[int]int{
		4: offsets[0],
		5: offsets[1],
		6: offsets[2],
		7: offsets[3],
	}
	for number := 1; number <= 7; number++ {
		if offset, ok := offsetByObject[number]; ok {
			fmt.Fprintf(&input, "%010d 00000 n \n", offset)
			continue
		}
		input.WriteString("0000000000 65535 f \n")
	}
	fmt.Fprintf(&input, "trailer\n<< /Size 8 /Root 1 0 R /Info 5 0 R /Encrypt 6 0 R /ID [<%s> <%s>] >>\nstartxref\n%d\n%%%%EOF\n",
		hex.EncodeToString(fileID),
		hex.EncodeToString(fileID),
		xrefOffset,
	)
	return input.Bytes()
}

func standardSecurityR2FixtureDict() pdfDict {
	return pdfDict{
		"Filter": pdfName("Standard"),
		"V":      1,
		"R":      2,
		"O":      pdfHexString("000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f"),
		"U":      pdfHexString("f2e39758794025127846c07006961602075fb11d4b5c227e0b27c9a2abb73e61"),
		"P":      -44,
	}
}

func standardSecurityR2FixtureObject() string {
	return `<<
/Filter /Standard
/V 1
/R 2
/O <000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f>
/U <f2e39758794025127846c07006961602075fb11d4b5c227e0b27c9a2abb73e61>
/P -44
>>`
}

func encryptedGraphFixturePDF(t *testing.T, fileID []byte, encryptObject string, streamData, titleData []byte) []byte {
	t.Helper()
	return encryptedGraphFixturePDFWithStreamDict(t, fileID, encryptObject, "<< /Length %d >>", streamData, titleData)
}

func encryptedGraphFixturePDFWithStreamDict(t *testing.T, fileID []byte, encryptObject, streamDictFormat string, streamData, titleData []byte) []byte {
	t.Helper()

	var input bytes.Buffer
	input.WriteString("%PDF-1.3\n")
	offsets := make([]int, 0, 5)

	writeObject := func(number int, body []byte) {
		t.Helper()
		offsets = append(offsets, input.Len())
		fmt.Fprintf(&input, "%d 0 obj\n", number)
		input.Write(body)
		input.WriteString("\nendobj\n")
	}

	writeObject(1, []byte("<< /Type /Catalog /Pages 2 0 R >>"))
	writeObject(2, []byte("<< /Type /Page /Contents 3 0 R >>"))
	offsets = append(offsets, input.Len())
	input.WriteString("3 0 obj\n")
	fmt.Fprintf(&input, streamDictFormat+"\nstream\n", len(streamData))
	input.Write(streamData)
	input.WriteString("\nendstream\nendobj\n")
	writeObject(4, []byte(fmt.Sprintf("<< /Title <%s> >>", hex.EncodeToString(titleData))))
	writeObject(5, []byte(encryptObject))

	xrefOffset := input.Len()
	input.WriteString("xref\n0 6\n")
	input.WriteString("0000000000 65535 f \n")
	for _, offset := range offsets {
		fmt.Fprintf(&input, "%010d 00000 n \n", offset)
	}
	fmt.Fprintf(&input, "trailer\n<< /Size 6 /Root 1 0 R /Encrypt 5 0 R /ID [<%s> <%s>] >>\nstartxref\n%d\n%%%%EOF\n",
		hex.EncodeToString(fileID),
		hex.EncodeToString(fileID),
		xrefOffset,
	)
	return input.Bytes()
}
