package pdf

type pdfStructureObjectOrigin string

const (
	pdfStructureOriginNormal           pdfStructureObjectOrigin = "normal"
	pdfStructureOriginObjectStream     pdfStructureObjectOrigin = "object_stream"
	pdfStructureOriginXrefStreamObject pdfStructureObjectOrigin = "xref_stream_object"
	pdfStructureOriginUnknown          pdfStructureObjectOrigin = "unknown"
)

type pdfStructurePlan struct {
	TotalObjects        int
	NormalObjects       int
	ObjectStreamObjects int
	XrefStreamObjects   int
	UnknownObjects      int
	HasTableXref        bool
	HasXrefStream       bool
	HasHybridXref       bool
	Objects             []pdfStructureObjectPlan
}

type pdfStructureObjectPlan struct {
	ID     pdfObjectID
	Origin pdfStructureObjectOrigin
}

func (p pdfStructurePlan) metadata() map[string]any {
	return map[string]any{
		"total_objects":          p.TotalObjects,
		"normal_objects":         p.NormalObjects,
		"object_stream_objects":  p.ObjectStreamObjects,
		"xref_stream_objects":    p.XrefStreamObjects,
		"unknown_objects":        p.UnknownObjects,
		"has_table_xref":         p.HasTableXref,
		"has_xref_stream":        p.HasXrefStream,
		"has_hybrid_xref":        p.HasHybridXref,
		"requires_packed_writer": p.requiresPackedWriter(),
	}
}

func (p pdfStructurePlan) requiresPackedWriter() bool {
	return p.ObjectStreamObjects > 0 || p.XrefStreamObjects > 0 || p.HasHybridXref
}

func (p pdfStructurePlan) writerPath() string {
	if !p.requiresPackedWriter() {
		return "canonical"
	}
	if p.ObjectStreamObjects > 0 {
		return "preserve-packed"
	}
	if p.HasHybridXref {
		return "hybrid_xref_stream"
	}
	return "xref_stream"
}

func summarizePDFStructurePlan(graph *pdfGraph) pdfStructurePlan {
	if graph == nil {
		return pdfStructurePlan{}
	}

	xrefObjects := make(map[pdfObjectID]xrefObjectOffset, len(graph.Xref.Objects))
	for _, object := range graph.Xref.Objects {
		xrefObjects[pdfObjectID{Number: object.Number, Generation: object.Generation}] = object
	}

	xrefStreamObjects := make(map[pdfObjectID]bool, len(graph.Xref.StreamObjects))
	for _, object := range graph.Xref.StreamObjects {
		xrefStreamObjects[pdfObjectID{Number: object.Number, Generation: object.Generation}] = true
	}

	plan := pdfStructurePlan{
		HasTableXref:  graph.Xref.HasTable,
		HasXrefStream: graph.Xref.HasStream,
		HasHybridXref: graph.Xref.HasHybridStream,
		Objects:       make([]pdfStructureObjectPlan, 0, len(graph.Objects)),
	}
	for _, object := range sortedPDFObjects(graph.Objects) {
		origin := pdfStructureOriginForObject(object, xrefObjects, xrefStreamObjects)
		plan.Objects = append(plan.Objects, pdfStructureObjectPlan{
			ID:     object.ID,
			Origin: origin,
		})
		plan.TotalObjects++
		switch origin {
		case pdfStructureOriginNormal:
			plan.NormalObjects++
		case pdfStructureOriginObjectStream:
			plan.ObjectStreamObjects++
		case pdfStructureOriginXrefStreamObject:
			plan.XrefStreamObjects++
		default:
			plan.UnknownObjects++
		}
	}
	return plan
}

func pdfStructureOriginForObject(object *pdfIndirectObject, xrefObjects map[pdfObjectID]xrefObjectOffset, xrefStreamObjects map[pdfObjectID]bool) pdfStructureObjectOrigin {
	if object == nil {
		return pdfStructureOriginUnknown
	}
	if object.InObjectStream {
		return pdfStructureOriginObjectStream
	}
	if xref, ok := xrefObjects[object.ID]; ok && xref.Compressed {
		return pdfStructureOriginObjectStream
	}
	if stream, ok := object.Value.(pdfStreamObject); ok && dictHasType(stream.Dict, "XRef") {
		return pdfStructureOriginXrefStreamObject
	}
	if xrefStreamObjects[object.ID] {
		return pdfStructureOriginXrefStreamObject
	}
	if _, ok := xrefObjects[object.ID]; ok {
		return pdfStructureOriginNormal
	}
	if object.Offset > 0 {
		return pdfStructureOriginNormal
	}
	return pdfStructureOriginUnknown
}
