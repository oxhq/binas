package pdf

import (
	"bytes"
	"fmt"
	"testing"
)

func TestGraphAPIParseGraphCatalogPageTreeNameTreeResolveAndStream(t *testing.T) {
	content := []byte("BT\n(Hello) Tj\nET\n")
	input := graphAPITestPDF(
		"<< /Type /Catalog /Pages 2 0 R /Names << /Dests 5 0 R >> /Lang (en-US) /Version /1.7 >>",
		"<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>",
		"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 6 0 R >>",
		"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>",
		"<< /Names [(Intro) 7 0 R <486578> 8 0 R] >>",
		fmt.Sprintf("<< /Length %d >>\nstream\n%sendstream", len(content), content),
		"<< /Title (Chapter One) >>",
		"<< /Title <48656c6c6f> >>",
	)

	graph, err := ParseGraph(input, GraphOptions{})
	if err != nil {
		t.Fatalf("ParseGraph: %v", err)
	}

	catalogRef, catalog, ok := graph.Catalog()
	if !ok {
		t.Fatal("missing catalog")
	}
	catalogObject, ok := graph.Object(catalogRef)
	if !ok || catalogObject.Number != 1 || catalogObject.Generation != 0 || catalogObject.Offset <= 0 {
		t.Fatalf("catalog provenance = ref %+v object %+v, want object 1 0 with source offset", catalogRef, catalogObject)
	}
	if got := catalog["Version"]; got != Name("1.7") {
		t.Fatalf("catalog Version = %#v, want pdf.Name(1.7)", got)
	}
	if got := catalog["Lang"]; got != String("en-US") {
		t.Fatalf("catalog Lang = %#v, want pdf.String(en-US)", got)
	}

	pages, err := graph.PageTree()
	if err != nil {
		t.Fatalf("PageTree: %v", err)
	}
	if len(pages) != 2 {
		t.Fatalf("page tree pages = %d, want 2", len(pages))
	}
	if pages[0].Ref.Number != 3 || pages[0].Index != 0 || pages[1].Index != 1 {
		t.Fatalf("page tree nodes = %+v, want ordered page refs/indexes", pages)
	}
	firstPage := pages[0].Dict
	mediaBox, ok := firstPage["MediaBox"].(Array)
	if !ok || len(mediaBox) != 4 {
		t.Fatalf("MediaBox = %#v, want four-number Array", firstPage["MediaBox"])
	}
	if mediaBox[2] != (Number{Value: 612, Integer: true}) {
		t.Fatalf("MediaBox[2] = %#v, want integer number 612", mediaBox[2])
	}

	pagesRef, ok := catalog["Pages"].(Ref)
	if !ok {
		t.Fatalf("catalog /Pages = %#v, want Ref", catalog["Pages"])
	}
	pagesValue, ok := graph.Resolve(pagesRef)
	if !ok {
		t.Fatal("catalog /Pages did not resolve")
	}
	pagesDict, ok := pagesValue.(Dict)
	if !ok || pagesDict["Type"] != Name("Pages") {
		t.Fatalf("resolved /Pages = %#v, want Pages dict", pagesValue)
	}

	nameTree, err := graph.NameTree(NameTreeDests)
	if err != nil {
		t.Fatalf("NameTree(Dests): %v", err)
	}
	if len(nameTree) != 2 {
		t.Fatalf("name tree entries = %d, want 2", len(nameTree))
	}
	if nameTree[0].Name != "Intro" || nameTree[1].Name != "Hex" {
		t.Fatalf("name tree entry names = %+v, want Intro and Hex", nameTree)
	}
	titleRef, ok := nameTree[1].Value.(Ref)
	if !ok {
		t.Fatalf("name tree hex entry value = %#v, want Ref", nameTree[1].Value)
	}
	titleValue, ok := graph.Resolve(titleRef)
	if !ok {
		t.Fatal("name tree hex entry did not resolve")
	}
	titleDict, ok := titleValue.(Dict)
	if !ok || titleDict["Title"] != HexString("48656c6c6f") {
		t.Fatalf("resolved hex name-tree value = %#v, want dict with HexString title", titleValue)
	}

	stream, ok := graph.Stream(Ref{Number: 6, Generation: 0})
	if !ok {
		t.Fatal("missing content stream")
	}
	if !bytes.Equal(stream.Data, content) {
		t.Fatalf("stream data = %q, want %q", stream.Data, content)
	}
	if !bytes.Equal(stream.Decoded, content) || stream.DecodeError != "" {
		t.Fatalf("stream decoded=%q decode_error=%q, want decoded content without error", stream.Decoded, stream.DecodeError)
	}
	if stream.Dict["Length"] != (Number{Value: float64(len(content)), Integer: true}) {
		t.Fatalf("stream Length = %#v, want integer length", stream.Dict["Length"])
	}
	streamObject, ok := graph.Object(Ref{Number: 6, Generation: 0})
	if !ok || streamObject.SourceKind != ObjectSourceNormal {
		t.Fatalf("stream object = %+v ok=%t, want normal source kind", streamObject, ok)
	}
}

func TestGraphAPIParseGraphExposesObjectStreamProvenance(t *testing.T) {
	objectStreamData := []byte("9 0 << /Type /Page /Parent 2 0 R >>")
	input := graphAPITestPDF(
		"<< /Type /Catalog /Pages 2 0 R >>",
		"<< /Type /Pages /Kids [9 0 R] /Count 1 >>",
		fmt.Sprintf("<< /Type /ObjStm /N 1 /First 4 /Length %d >>\nstream\n%sendstream", len(objectStreamData), objectStreamData),
	)

	graph, err := ParseGraph(input, GraphOptions{})
	if err != nil {
		t.Fatalf("ParseGraph: %v", err)
	}

	page, ok := graph.Object(Ref{Number: 9, Generation: 0})
	if !ok {
		t.Fatal("missing object-stream page object 9 0")
	}
	if !page.InObjectStream || page.SourceKind != ObjectSourceObjectStream {
		t.Fatalf("object provenance = %+v, want object-stream source kind", page)
	}
	container, ok := graph.Object(Ref{Number: 3, Generation: 0})
	if !ok || container.SourceKind != ObjectSourceObjectStreamContainer {
		t.Fatalf("object stream container = %+v ok=%t, want object-stream-container source kind", container, ok)
	}
	pageDict, ok := page.Value.(Dict)
	if !ok || pageDict["Type"] != Name("Page") {
		t.Fatalf("object-stream page value = %#v, want Page dict", page.Value)
	}
	pages, err := graph.PageTree()
	if err != nil {
		t.Fatalf("PageTree: %v", err)
	}
	if len(pages) != 1 || pages[0].Ref.Number != 9 || !pages[0].Object.InObjectStream {
		t.Fatalf("page tree pages = %+v, want object-stream page 9", pages)
	}
}

func graphAPITestPDF(objects ...string) []byte {
	var out bytes.Buffer
	out.WriteString("%PDF-1.3\n")
	offsets := make([]int, 0, len(objects))
	for i, object := range objects {
		offsets = append(offsets, out.Len())
		fmt.Fprintf(&out, "%d 0 obj\n%s\nendobj\n", i+1, object)
	}
	xrefOffset := out.Len()
	fmt.Fprintf(&out, "xref\n0 %d\n", len(objects)+1)
	out.WriteString("0000000000 65535 f \n")
	for _, offset := range offsets {
		fmt.Fprintf(&out, "%010d 00000 n \n", offset)
	}
	fmt.Fprintf(&out, "trailer\n<< /Size %d /Root 1 0 R >>\nstartxref\n%d\n%%%%EOF\n", len(objects)+1, xrefOffset)
	return out.Bytes()
}
