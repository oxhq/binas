package pdf

import (
	"fmt"
	"testing"
)

func TestStructurePlanClassifiesNormalObjects(t *testing.T) {
	graph, err := parsePDFGraph(testPDF(
		"<< /Type /Catalog /Pages 2 0 R >>",
		"<< /Type /Page >>",
	))
	if err != nil {
		t.Fatalf("parse graph: %v", err)
	}

	plan := summarizePDFStructurePlan(graph)

	if plan.TotalObjects != 2 || plan.NormalObjects != 2 || plan.ObjectStreamObjects != 0 || plan.XrefStreamObjects != 0 || plan.UnknownObjects != 0 {
		t.Fatalf("plan counts = %+v, want two normal objects", plan)
	}
	assertStructurePlanOrigin(t, plan, pdfObjectID{Number: 1, Generation: 0}, pdfStructureOriginNormal)
	assertStructurePlanOrigin(t, plan, pdfObjectID{Number: 2, Generation: 0}, pdfStructureOriginNormal)
}

func TestStructurePlanClassifiesObjectStreamMembers(t *testing.T) {
	objectValue := "<< /Type /Page >>"
	objectStreamData := "3 0 " + objectValue
	graph, err := parsePDFGraph(testPDF(
		"<< /Type /Catalog /Pages 3 0 R >>",
		fmt.Sprintf("<< /Type /ObjStm /N 1 /First 4 /Length %d >>\nstream\n%sendstream", len(objectStreamData), objectStreamData),
	))
	if err != nil {
		t.Fatalf("parse graph: %v", err)
	}

	plan := summarizePDFStructurePlan(graph)

	if plan.TotalObjects != 3 || plan.NormalObjects != 2 || plan.ObjectStreamObjects != 1 || plan.XrefStreamObjects != 0 || plan.UnknownObjects != 0 {
		t.Fatalf("plan counts = %+v, want two normal objects and one object stream member", plan)
	}
	assertStructurePlanOrigin(t, plan, pdfObjectID{Number: 1, Generation: 0}, pdfStructureOriginNormal)
	assertStructurePlanOrigin(t, plan, pdfObjectID{Number: 2, Generation: 0}, pdfStructureOriginNormal)
	assertStructurePlanOrigin(t, plan, pdfObjectID{Number: 3, Generation: 0}, pdfStructureOriginObjectStream)
}

func TestStructurePlanClassifiesXrefStreamObject(t *testing.T) {
	graph, err := parsePDFGraph(testXrefStreamPDF(t))
	if err != nil {
		t.Fatalf("parse graph: %v", err)
	}

	plan := summarizePDFStructurePlan(graph)

	if plan.TotalObjects != 4 || plan.NormalObjects != 3 || plan.ObjectStreamObjects != 0 || plan.XrefStreamObjects != 1 || plan.UnknownObjects != 0 {
		t.Fatalf("plan counts = %+v, want three normal objects and one xref stream object", plan)
	}
	assertStructurePlanOrigin(t, plan, pdfObjectID{Number: 4, Generation: 0}, pdfStructureOriginXrefStreamObject)
}

func TestStructurePlanClassifiesUnknownWhenOriginSignalsAreMissing(t *testing.T) {
	id := pdfObjectID{Number: 9, Generation: 0}
	graph := &pdfGraph{
		Objects: map[pdfObjectID]*pdfIndirectObject{
			id: &pdfIndirectObject{ID: id, Value: pdfDict{"Synthetic": true}, Offset: -1},
		},
	}

	plan := summarizePDFStructurePlan(graph)

	if plan.TotalObjects != 1 || plan.UnknownObjects != 1 {
		t.Fatalf("plan counts = %+v, want one unknown object", plan)
	}
	assertStructurePlanOrigin(t, plan, id, pdfStructureOriginUnknown)
}

func assertStructurePlanOrigin(t *testing.T, plan pdfStructurePlan, id pdfObjectID, want pdfStructureObjectOrigin) {
	t.Helper()
	for _, object := range plan.Objects {
		if object.ID == id {
			if object.Origin != want {
				t.Fatalf("object %+v origin = %q, want %q", id, object.Origin, want)
			}
			return
		}
	}
	t.Fatalf("object %+v not found in structure plan %+v", id, plan)
}
