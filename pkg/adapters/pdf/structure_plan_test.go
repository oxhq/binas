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
	input := testPDF(
		"<< /Type /Catalog /Pages 3 0 R >>",
		fmt.Sprintf("<< /Type /ObjStm /N 1 /First 4 /Length %d >>\nstream\n%sendstream", len(objectStreamData), objectStreamData),
	)
	graph, err := parsePDFGraph(input)
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

func TestStructurePlanMetadataCountsObjectStreamMembers(t *testing.T) {
	objectValue := "<< /Type /Page >>"
	objectStreamData := "3 0 " + objectValue
	graph, err := parsePDFGraph(testPDF(
		"<< /Type /Catalog /Pages 3 0 R >>",
		fmt.Sprintf("<< /Type /ObjStm /N 1 /First 4 /Length %d >>\nstream\n%sendstream", len(objectStreamData), objectStreamData),
	))
	if err != nil {
		t.Fatalf("parse graph: %v", err)
	}

	tree := graph.toTree(nil)
	plan := rootStructurePlanMetadata(t, tree.Nodes[tree.Root].Value.(map[string]any))

	if plan["total_objects"] != 3 || plan["normal_objects"] != 2 || plan["object_stream_objects"] != 1 || plan["xref_stream_objects"] != 0 || plan["unknown_objects"] != 0 {
		t.Fatalf("plan counts = %+v, want two normal objects and one object stream member", plan)
	}
	if plan["has_table_xref"] != true || plan["has_xref_stream"] != false || plan["has_hybrid_xref"] != false {
		t.Fatalf("plan xref metadata = %+v, want table xref without stream or hybrid", plan)
	}
	if plan["requires_packed_writer"] != true {
		t.Fatalf("plan packed writer metadata = %+v, want packed writer required", plan)
	}
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

func TestStructurePlanMetadataCountsXrefStreamObject(t *testing.T) {
	graph, err := parsePDFGraph(testXrefStreamPDF(t))
	if err != nil {
		t.Fatalf("parse graph: %v", err)
	}

	tree := graph.toTree(nil)
	plan := rootStructurePlanMetadata(t, tree.Nodes[tree.Root].Value.(map[string]any))

	if plan["total_objects"] != 4 || plan["normal_objects"] != 3 || plan["object_stream_objects"] != 0 || plan["xref_stream_objects"] != 1 || plan["unknown_objects"] != 0 {
		t.Fatalf("plan counts = %+v, want three normal objects and one xref stream object", plan)
	}
	if plan["has_table_xref"] != false || plan["has_xref_stream"] != true || plan["has_hybrid_xref"] != false {
		t.Fatalf("plan xref metadata = %+v, want xref stream without table or hybrid", plan)
	}
	if plan["requires_packed_writer"] != true {
		t.Fatalf("plan packed writer metadata = %+v, want packed writer required", plan)
	}
}

func TestStructurePlanMetadataCountsHybridXref(t *testing.T) {
	graph, err := parsePDFGraph(buildHybridXrefPDFFixture(t, validHybridXrefStreamData).input)
	if err != nil {
		t.Fatalf("parse graph: %v", err)
	}

	tree := graph.toTree(nil)
	plan := rootStructurePlanMetadata(t, tree.Nodes[tree.Root].Value.(map[string]any))

	if plan["total_objects"] != 3 || plan["normal_objects"] != 2 || plan["object_stream_objects"] != 0 || plan["xref_stream_objects"] != 1 || plan["unknown_objects"] != 0 {
		t.Fatalf("plan counts = %+v, want two normal objects and one hybrid xref stream object", plan)
	}
	if plan["has_table_xref"] != true || plan["has_xref_stream"] != true || plan["has_hybrid_xref"] != true {
		t.Fatalf("plan xref metadata = %+v, want explicit hybrid xref details", plan)
	}
	if plan["requires_packed_writer"] != true {
		t.Fatalf("plan packed writer metadata = %+v, want packed writer required", plan)
	}
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

func rootStructurePlanMetadata(t *testing.T, root map[string]any) map[string]any {
	t.Helper()
	objectGraph, ok := root["object_graph"].(map[string]any)
	if !ok {
		t.Fatalf("object_graph metadata = %#v, want map", root["object_graph"])
	}
	plan, ok := objectGraph["structure_plan"].(map[string]any)
	if !ok {
		t.Fatalf("structure_plan metadata = %#v, want map", objectGraph["structure_plan"])
	}
	return plan
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
