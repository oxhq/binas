package pdf

import (
	"bytes"
	"errors"
	"fmt"
	"sort"
	"strconv"
	"strings"
)

type pdfCompressedObjectPlacement struct {
	StreamNumber int
	StreamIndex  int
}

type pdfPackedWritePlan struct {
	normalObjects      []*pdfIndirectObject
	objectStreams      []*pdfIndirectObject
	compressedEntries  map[pdfObjectID]pdfCompressedObjectPlacement
	xrefStreamObjectID pdfObjectID
	maxObjectNumber    int
}

func writePreserveStructurePDFWithOptions(graph *pdfGraph, opts pdfCanonicalWriteOptions) ([]byte, error) {
	if graph == nil {
		return nil, errors.New("nil PDF graph")
	}
	if graph.Boundaries.HasEncryption {
		if !opts.AllowEncryption {
			return nil, ErrEncryptedPDFPasswordRequired
		}
		if graph.Encryption == nil || graph.Encryption.security == nil {
			return nil, unsupportedPDFEncryption("preserve-structure encrypted rewrite requires an authenticated Standard Security graph")
		}
	}
	if graph.Boundaries.HasSignature && !opts.AllowSignatureInvalidation {
		return nil, ErrSignedPDFRequiresInvalidation
	}
	plan := summarizePDFStructurePlan(graph)
	if !plan.requiresPackedWriter() {
		return writeCanonicalPDFWithOptions(graph, opts)
	}
	packed, err := buildPDFPackedWritePlan(graph)
	if err != nil {
		return nil, err
	}
	var out bytes.Buffer
	out.WriteString(preserveStructurePDFHeader(graph.Header))
	out.WriteString("\n")
	offsets := make(map[pdfObjectID]int, len(packed.normalObjects)+len(packed.objectStreams)+1)
	for _, object := range packed.normalObjects {
		if err := writePreserveStructureIndirectObject(&out, offsets, object, graph, opts); err != nil {
			return nil, err
		}
	}
	for _, object := range packed.objectStreams {
		if err := writePreserveStructureIndirectObject(&out, offsets, object, graph, opts); err != nil {
			return nil, err
		}
	}
	xrefStreamOffset := out.Len()
	xrefStream, err := buildPDFXrefStreamObject(graph, packed, offsets, xrefStreamOffset)
	if err != nil {
		return nil, err
	}
	xrefObject := &pdfIndirectObject{ID: packed.xrefStreamObjectID, Value: xrefStream}
	if err := writePreserveStructureIndirectObject(&out, offsets, xrefObject, graph, opts); err != nil {
		return nil, err
	}
	if graph.Xref.HasHybridStream {
		tableOffset := out.Len()
		if err := writePreserveStructureTableXref(&out, graph, packed.maxObjectNumber+1, offsets, xrefStreamOffset); err != nil {
			return nil, err
		}
		out.WriteString("startxref\n")
		out.WriteString(strconv.Itoa(tableOffset))
		out.WriteString("\n%%EOF\n")
		return out.Bytes(), nil
	}
	out.WriteString("startxref\n")
	out.WriteString(strconv.Itoa(xrefStreamOffset))
	out.WriteString("\n%%EOF\n")
	return out.Bytes(), nil
}

func writePreserveStructureIndirectObject(out *bytes.Buffer, offsets map[pdfObjectID]int, object *pdfIndirectObject, graph *pdfGraph, opts pdfCanonicalWriteOptions) error {
	offsets[object.ID] = out.Len()
	fmt.Fprintf(out, "%d %d obj\n", object.ID.Number, object.ID.Generation)
	value := object.Value
	if graph.Boundaries.HasEncryption && (graph.Encryption.encryptObject == nil || object.ID != *graph.Encryption.encryptObject) {
		encrypted, err := encryptPDFObjectValue(graph.Encryption.security, graph.Encryption.fileKey, object.ID, value)
		if err != nil {
			return fmt.Errorf("encrypt object %d %d: %w", object.ID.Number, object.ID.Generation, err)
		}
		value = encrypted
	}
	if err := writePDFValue(out, value); err != nil {
		return fmt.Errorf("write object %d %d: %w", object.ID.Number, object.ID.Generation, err)
	}
	out.WriteString("\nendobj\n")
	return nil
}

func buildPDFPackedWritePlan(graph *pdfGraph) (pdfPackedWritePlan, error) {
	compressed := compressedPDFObjectPlacements(graph)
	objectStreamIDs := pdfObjectIDSetFromOffsets(graph.Xref.ObjectStreamObjects)
	xrefStreamIDs := pdfObjectIDSetFromOffsets(graph.Xref.StreamObjects)
	for _, object := range graph.Objects {
		if stream, ok := object.Value.(pdfStreamObject); ok {
			if dictHasType(stream.Dict, "ObjStm") {
				objectStreamIDs[object.ID] = true
			}
			if dictHasType(stream.Dict, "XRef") {
				xrefStreamIDs[object.ID] = true
			}
		}
	}
	grouped := make(map[int][]*pdfIndirectObject)
	normal := make([]*pdfIndirectObject, 0, len(graph.Objects))
	maxObject := 0
	for _, object := range sortedPDFObjects(graph.Objects) {
		if object.ID.Number > maxObject {
			maxObject = object.ID.Number
		}
		if xrefStreamIDs[object.ID] {
			continue
		}
		if objectStreamIDs[object.ID] {
			continue
		}
		if placement, ok := compressed[object.ID]; ok || object.InObjectStream {
			streamNumber := placement.StreamNumber
			if streamNumber == 0 {
				streamNumber = firstPDFObjectStreamNumber(objectStreamIDs)
			}
			if streamNumber == 0 {
				return pdfPackedWritePlan{}, preserveStructureUnsupported(graph, fmt.Sprintf("object %d %d is marked in an object stream but has no object stream xref metadata", object.ID.Number, object.ID.Generation))
			}
			if _, isStream := object.Value.(pdfStreamObject); isStream {
				return pdfPackedWritePlan{}, preserveStructureUnsupported(graph, "object streams cannot contain stream objects")
			}
			grouped[streamNumber] = append(grouped[streamNumber], object)
			continue
		}
		normal = append(normal, object)
	}
	objectStreams := make([]*pdfIndirectObject, 0, len(grouped))
	compressedEntries := make(map[pdfObjectID]pdfCompressedObjectPlacement, len(compressed))
	for id, placement := range compressed {
		compressedEntries[id] = placement
	}
	rebuiltObjectStreams := make(map[int]bool, len(grouped))
	for streamNumber, members := range grouped {
		sort.Slice(members, func(i, j int) bool {
			left := compressed[members[i].ID]
			right := compressed[members[j].ID]
			if left.StreamIndex != right.StreamIndex {
				return left.StreamIndex < right.StreamIndex
			}
			return members[i].ID.Number < members[j].ID.Number
		})
		objectStream, entries, err := buildPDFObjectStream(graph, streamNumber, members)
		if err != nil {
			return pdfPackedWritePlan{}, err
		}
		objectStreams = append(objectStreams, objectStream)
		for id, placement := range entries {
			compressedEntries[id] = placement
		}
		if streamNumber > maxObject {
			maxObject = streamNumber
		}
		rebuiltObjectStreams[streamNumber] = true
	}
	for id := range objectStreamIDs {
		if rebuiltObjectStreams[id.Number] {
			continue
		}
		object := graph.Objects[id]
		if object == nil {
			return pdfPackedWritePlan{}, preserveStructureUnsupported(graph, fmt.Sprintf("object stream container %d %d is missing", id.Number, id.Generation))
		}
		if _, ok := object.Value.(pdfStreamObject); !ok {
			return pdfPackedWritePlan{}, preserveStructureUnsupported(graph, fmt.Sprintf("object stream container %d %d is not a stream", id.Number, id.Generation))
		}
		objectStreams = append(objectStreams, object)
		if id.Number > maxObject {
			maxObject = id.Number
		}
	}
	sort.Slice(objectStreams, func(i, j int) bool {
		return objectStreams[i].ID.Number < objectStreams[j].ID.Number
	})
	xrefID := choosePDFXrefStreamObjectID(graph, xrefStreamIDs, maxObject)
	if xrefID.Number > maxObject {
		maxObject = xrefID.Number
	}
	return pdfPackedWritePlan{
		normalObjects:      normal,
		objectStreams:      objectStreams,
		compressedEntries:  compressedEntries,
		xrefStreamObjectID: xrefID,
		maxObjectNumber:    maxObject,
	}, nil
}

func compressedPDFObjectPlacements(graph *pdfGraph) map[pdfObjectID]pdfCompressedObjectPlacement {
	out := make(map[pdfObjectID]pdfCompressedObjectPlacement)
	for _, entry := range graph.Xref.Objects {
		if !entry.Compressed {
			continue
		}
		out[pdfObjectID{Number: entry.Number, Generation: entry.Generation}] = pdfCompressedObjectPlacement{
			StreamNumber: entry.ObjectStreamNumber,
			StreamIndex:  entry.ObjectStreamIndex,
		}
	}
	return out
}

func buildPDFObjectStream(graph *pdfGraph, streamNumber int, members []*pdfIndirectObject) (*pdfIndirectObject, map[pdfObjectID]pdfCompressedObjectPlacement, error) {
	var body bytes.Buffer
	offsets := make([]int, 0, len(members))
	for _, member := range members {
		offsets = append(offsets, body.Len())
		if err := writePDFValue(&body, member.Value); err != nil {
			return nil, nil, fmt.Errorf("write object stream member %d %d: %w", member.ID.Number, member.ID.Generation, err)
		}
		body.WriteByte('\n')
	}
	var header bytes.Buffer
	for i, member := range members {
		if i > 0 {
			header.WriteByte(' ')
		}
		fmt.Fprintf(&header, "%d %d", member.ID.Number, offsets[i])
	}
	header.WriteByte(' ')
	data := append(header.Bytes(), body.Bytes()...)
	dict := pdfDict{
		"Type":  pdfName("ObjStm"),
		"N":     len(members),
		"First": header.Len(),
	}
	if original := graph.Objects[pdfObjectID{Number: streamNumber, Generation: 0}]; original != nil {
		if stream, ok := original.Value.(pdfStreamObject); ok {
			dict = clonePDFDict(stream.Dict)
			delete(dict, "Filter")
			delete(dict, "DecodeParms")
			dict["Type"] = pdfName("ObjStm")
			dict["N"] = len(members)
			dict["First"] = header.Len()
		}
	}
	entries := make(map[pdfObjectID]pdfCompressedObjectPlacement, len(members))
	for i, member := range members {
		entries[member.ID] = pdfCompressedObjectPlacement{StreamNumber: streamNumber, StreamIndex: i}
	}
	return &pdfIndirectObject{
		ID:    pdfObjectID{Number: streamNumber, Generation: 0},
		Value: pdfStreamObject{Dict: dict, Data: data},
	}, entries, nil
}

func buildPDFXrefStreamObject(graph *pdfGraph, plan pdfPackedWritePlan, offsets map[pdfObjectID]int, xrefOffset int) (pdfStreamObject, error) {
	size := plan.maxObjectNumber + 1
	entries := make([]pdfXrefEntry, size)
	for i := range entries {
		entries[i] = pdfXrefEntry{ObjectNumber: i, Type: 0, Generation: 65535}
	}
	offsets = clonePDFObjectOffsets(offsets)
	offsets[plan.xrefStreamObjectID] = xrefOffset
	for id, offset := range offsets {
		if id.Number < 0 || id.Number >= size {
			return pdfStreamObject{}, preserveStructureUnsupported(graph, "object offset outside xref stream size")
		}
		entries[id.Number] = pdfXrefEntry{ObjectNumber: id.Number, Type: 1, Offset: offset, Generation: id.Generation}
	}
	for id, placement := range plan.compressedEntries {
		if id.Generation != 0 {
			return pdfStreamObject{}, preserveStructureUnsupported(graph, "compressed objects with non-zero generation are unsupported")
		}
		if id.Number < 0 || id.Number >= size {
			return pdfStreamObject{}, preserveStructureUnsupported(graph, "compressed object outside xref stream size")
		}
		entries[id.Number] = pdfXrefEntry{
			ObjectNumber: id.Number,
			Type:         2,
			StreamNumber: placement.StreamNumber,
			StreamIndex:  placement.StreamIndex,
			Generation:   0,
		}
	}
	xrefData := encodePDFXrefStreamEntries(entries)
	dict := canonicalTrailer(graph, size, graph.Boundaries.HasEncryption)
	delete(dict, "Prev")
	delete(dict, "XRefStm")
	dict["Type"] = pdfName("XRef")
	dict["Size"] = size
	dict["W"] = pdfArray{1, 8, 4}
	dict["Index"] = pdfArray{0, size}
	return pdfStreamObject{Dict: dict, Data: xrefData}, nil
}

func writePreserveStructureTableXref(out *bytes.Buffer, graph *pdfGraph, size int, offsets map[pdfObjectID]int, xrefStreamOffset int) error {
	out.WriteString("xref\n")
	fmt.Fprintf(out, "0 %d\n", size)
	out.WriteString("0000000000 65535 f \n")
	byNumber := make(map[int]pdfXrefEntry, len(offsets))
	for id, offset := range offsets {
		if id.Generation != 0 {
			return preserveStructureUnsupported(graph, "hybrid table xref cannot preserve non-zero generation objects")
		}
		byNumber[id.Number] = pdfXrefEntry{ObjectNumber: id.Number, Type: 1, Offset: offset, Generation: id.Generation}
	}
	for number := 1; number < size; number++ {
		entry, ok := byNumber[number]
		if !ok {
			out.WriteString("0000000000 65535 f \n")
			continue
		}
		fmt.Fprintf(out, "%010d %05d n \n", entry.Offset, entry.Generation)
	}
	trailer := canonicalTrailer(graph, size, graph.Boundaries.HasEncryption)
	trailer["XRefStm"] = xrefStreamOffset
	out.WriteString("trailer\n")
	if err := writePDFValue(out, trailer); err != nil {
		return err
	}
	out.WriteByte('\n')
	return nil
}

func encodePDFXrefStreamEntries(entries []pdfXrefEntry) []byte {
	var out bytes.Buffer
	for _, entry := range entries {
		out.WriteByte(byte(entry.Type))
		switch entry.Type {
		case 1:
			writePDFXrefStreamField(&out, entry.Offset, 8)
			writePDFXrefStreamField(&out, entry.Generation, 4)
		case 2:
			writePDFXrefStreamField(&out, entry.StreamNumber, 8)
			writePDFXrefStreamField(&out, entry.StreamIndex, 4)
		default:
			writePDFXrefStreamField(&out, 0, 8)
			writePDFXrefStreamField(&out, entry.Generation, 4)
		}
	}
	return out.Bytes()
}

func writePDFXrefStreamField(out *bytes.Buffer, value int, width int) {
	for shift := (width - 1) * 8; shift >= 0; shift -= 8 {
		out.WriteByte(byte(value >> shift))
	}
}

func choosePDFXrefStreamObjectID(graph *pdfGraph, xrefStreamIDs map[pdfObjectID]bool, maxObject int) pdfObjectID {
	for _, object := range sortedPDFObjects(graph.Objects) {
		if xrefStreamIDs[object.ID] {
			return object.ID
		}
	}
	return pdfObjectID{Number: maxObject + 1, Generation: 0}
}

func pdfObjectIDSetFromOffsets(offsets []xrefObjectOffset) map[pdfObjectID]bool {
	out := make(map[pdfObjectID]bool, len(offsets))
	for _, offset := range offsets {
		out[pdfObjectID{Number: offset.Number, Generation: offset.Generation}] = true
	}
	return out
}

func clonePDFObjectOffsets(offsets map[pdfObjectID]int) map[pdfObjectID]int {
	out := make(map[pdfObjectID]int, len(offsets)+1)
	for id, offset := range offsets {
		out[id] = offset
	}
	return out
}

func firstPDFObjectStreamNumber(ids map[pdfObjectID]bool) int {
	first := 0
	for id := range ids {
		if id.Generation != 0 {
			continue
		}
		if first == 0 || id.Number < first {
			first = id.Number
		}
	}
	return first
}

func preserveStructurePDFHeader(header string) string {
	if header == "" {
		return "%PDF-1.5"
	}
	if strings.HasPrefix(header, "%PDF-1.") && len(header) >= len("%PDF-1.5") && header < "%PDF-1.5" {
		return "%PDF-1.5"
	}
	return header
}

func preserveStructureUnsupported(graph *pdfGraph, reason string) error {
	details := pdfStructurePlan{}.metadata()
	if graph != nil {
		details = summarizePDFStructurePlan(graph).metadata()
	}
	details["reason"] = reason
	return &PreserveStructureUnsupportedError{
		Plan:    summarizePDFStructurePlan(graph),
		Details: details,
	}
}
