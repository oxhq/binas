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
	Objects             []pdfStructureObjectPlan
}

type pdfStructureObjectPlan struct {
	ID     pdfObjectID
	Origin pdfStructureObjectOrigin
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
		Objects: make([]pdfStructureObjectPlan, 0, len(graph.Objects)),
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
