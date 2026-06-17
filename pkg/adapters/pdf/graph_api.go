package pdf

import (
	"bytes"
	"errors"
	"fmt"
)

type GraphOptions struct {
	Strict          bool
	Password        string
	AllowSignature  bool
	AllowSignatures bool
	AllowXFA        bool
}

type Graph struct {
	graph *pdfGraph
}

type Value any

type Name string
type String string
type HexString string
type Bool bool

type Null struct{}

type Number struct {
	Value   float64
	Integer bool
}

type Array []Value
type Dict map[string]Value

type Ref struct {
	Number     int
	Generation int
}

type ObjectRef = Ref

type Stream struct {
	Dict        Dict
	Data        []byte
	Decoded     []byte
	DecodeError string
	Filter      string
	DecodeParms string
	SourceStart int
	SourceEnd   int
}

type ObjectSourceKind string

const (
	ObjectSourceNormal                ObjectSourceKind = "normal"
	ObjectSourceObjectStream          ObjectSourceKind = "object_stream"
	ObjectSourceObjectStreamContainer ObjectSourceKind = "object_stream_container"
	ObjectSourceXRefStream            ObjectSourceKind = "xref_stream"
)

type Object struct {
	Number         int
	Generation     int
	Offset         int
	SourceKind     ObjectSourceKind
	InObjectStream bool
	Value          Value
}

type PageNode struct {
	Ref    ObjectRef
	Index  int
	Dict   Dict
	Object Object
}

type NameTreeKind string

const (
	NameTreeDests         NameTreeKind = "Dests"
	NameTreeEmbeddedFiles NameTreeKind = "EmbeddedFiles"
	NameTreeJavaScript    NameTreeKind = "JavaScript"
)

type NameTreeEntry struct {
	Name  string
	Value Value
}

func ParseGraph(input []byte, opts GraphOptions) (*Graph, error) {
	if opts.Strict && !bytes.Contains(input, []byte("%%EOF")) {
		return nil, errors.New("malformed PDF: missing EOF marker")
	}
	graph, err := parsePDFGraphWithOptions(input, pdfGraphParseOptions{
		AllowEncryption: opts.Password != "",
		AllowSignature:  opts.AllowSignature || opts.AllowSignatures,
		AllowXFA:        opts.AllowXFA,
		Password:        opts.Password,
	})
	if err != nil {
		return nil, err
	}
	return &Graph{graph: graph}, nil
}

func (g *Graph) Header() string {
	if g == nil || g.graph == nil {
		return ""
	}
	return g.graph.Header
}

func (g *Graph) Trailer() Dict {
	if g == nil || g.graph == nil {
		return nil
	}
	dict, _ := exportPDFValue(g.graph.Trailer).(Dict)
	return dict
}

func (g *Graph) Objects() []Object {
	if g == nil || g.graph == nil {
		return nil
	}
	objects := sortedPDFObjects(g.graph.Objects)
	out := make([]Object, 0, len(objects))
	for _, object := range objects {
		out = append(out, exportPDFObject(object))
	}
	return out
}

func (g *Graph) Object(ref ObjectRef) (Object, bool) {
	if g == nil || g.graph == nil {
		return Object{}, false
	}
	object := g.graph.Objects[pdfObjectID{Number: ref.Number, Generation: ref.Generation}]
	if object == nil {
		return Object{}, false
	}
	return exportPDFObject(object), true
}

func (g *Graph) Resolve(ref ObjectRef) (Value, bool) {
	object, ok := g.Object(ref)
	if !ok {
		return nil, false
	}
	return object.Value, true
}

func (g *Graph) ResolveValue(value Value) (Value, bool) {
	ref, ok := value.(Ref)
	if !ok {
		return value, true
	}
	return g.Resolve(ref)
}

func (g *Graph) Stream(ref ObjectRef) (Stream, bool) {
	object, ok := g.Object(ref)
	if !ok {
		return Stream{}, false
	}
	stream, ok := object.Value.(Stream)
	if ok {
		if decoded, err := g.DecodeStream(ref); err == nil {
			stream.Decoded = decoded
		} else {
			stream.DecodeError = err.Error()
		}
	}
	return stream, ok
}

func (g *Graph) DecodeStream(ref Ref) ([]byte, error) {
	if g == nil || g.graph == nil {
		return nil, errors.New("nil PDF graph")
	}
	object := g.graph.Objects[pdfObjectID{Number: ref.Number, Generation: ref.Generation}]
	if object == nil {
		return nil, fmt.Errorf("object %d %d not found", ref.Number, ref.Generation)
	}
	stream, ok := object.Value.(pdfStreamObject)
	if !ok {
		return nil, fmt.Errorf("object %d %d is not a stream", ref.Number, ref.Generation)
	}
	return g.graph.decodePDFGraphObjectStream(object.ID, stream)
}

func (g *Graph) Catalog() (ObjectRef, Dict, bool) {
	if g == nil || g.graph == nil || g.graph.Root == nil {
		return ObjectRef{}, nil, false
	}
	ref := ObjectRef{Number: g.graph.Root.Number, Generation: g.graph.Root.Generation}
	object, ok := g.Object(ref)
	if !ok {
		return ObjectRef{}, nil, false
	}
	dict, ok := object.Value.(Dict)
	if !ok {
		if stream, streamOK := object.Value.(Stream); streamOK {
			dict, ok = stream.Dict, true
		}
	}
	if !ok {
		return ObjectRef{}, nil, false
	}
	return ref, dict, true
}

func (g *Graph) PageTree() ([]PageNode, error) {
	_, catalog, ok := g.Catalog()
	if !ok {
		return nil, errors.New("PDF catalog not found")
	}
	rootValue, ok := catalog["Pages"]
	if !ok {
		return nil, errors.New("PDF catalog missing /Pages")
	}
	_, _, ok = g.resolveDictWithObject(rootValue)
	if !ok {
		return nil, errors.New("PDF catalog /Pages does not resolve to a dictionary")
	}
	pages, err := g.collectPageTreePages(rootValue, nil)
	if err != nil {
		return nil, err
	}
	return pages, nil
}

func (g *Graph) NameTree(kind NameTreeKind) ([]NameTreeEntry, error) {
	_, catalog, ok := g.Catalog()
	if !ok {
		return nil, errors.New("PDF catalog not found")
	}
	namesValue, ok := catalog["Names"]
	if !ok {
		return nil, fmt.Errorf("PDF catalog missing /Names")
	}
	namesDict, _, ok := g.resolveDictWithObject(namesValue)
	if !ok {
		return nil, errors.New("PDF catalog /Names does not resolve to a dictionary")
	}
	name := string(kind)
	rootValue, ok := namesDict[name]
	if !ok {
		return nil, fmt.Errorf("PDF name tree /%s not found", name)
	}
	entries, err := g.collectNameTreeEntries(rootValue, nil)
	if err != nil {
		return nil, err
	}
	return entries, nil
}

func (g *Graph) resolveDictWithObject(value Value) (Dict, *Object, bool) {
	switch v := value.(type) {
	case Dict:
		return v, nil, true
	case Stream:
		return v.Dict, nil, true
	case Ref:
		object, ok := g.Object(v)
		if !ok {
			return nil, nil, false
		}
		switch objectValue := object.Value.(type) {
		case Dict:
			return objectValue, &object, true
		case Stream:
			return objectValue.Dict, &object, true
		default:
			return nil, &object, false
		}
	default:
		return nil, nil, false
	}
}

func (g *Graph) collectPageTreePages(value Value, seen map[Ref]bool) ([]PageNode, error) {
	var object *Object
	if ref, ok := value.(Ref); ok {
		if seen == nil {
			seen = make(map[Ref]bool)
		}
		if seen[ref] {
			return nil, fmt.Errorf("page tree cycle at object %d %d", ref.Number, ref.Generation)
		}
		seen[ref] = true
		resolved, ok := g.Object(ref)
		if !ok {
			return nil, fmt.Errorf("page tree object %d %d not found", ref.Number, ref.Generation)
		}
		object = &resolved
		value = resolved.Value
	}
	dict, ok := value.(Dict)
	if !ok {
		if stream, streamOK := value.(Stream); streamOK {
			dict, ok = stream.Dict, true
		}
	}
	if !ok {
		return nil, errors.New("page tree node is not a dictionary")
	}
	if dict["Type"] == Name("Page") {
		if object != nil {
			return []PageNode{{
				Ref:    ObjectRef{Number: object.Number, Generation: object.Generation},
				Dict:   dict,
				Object: *object,
			}}, nil
		}
		return []PageNode{{Dict: dict, Object: Object{Value: dict}}}, nil
	}
	if dict["Type"] != Name("Pages") {
		return nil, errors.New("page tree node is neither /Pages nor /Page")
	}
	kids, ok := dict["Kids"].(Array)
	if !ok {
		return nil, errors.New("page tree /Pages node missing /Kids array")
	}
	out := make([]PageNode, 0, len(kids))
	for _, kid := range kids {
		pages, err := g.collectPageTreePages(kid, clonePublicRefSet(seen))
		if err != nil {
			return nil, err
		}
		out = append(out, pages...)
	}
	for i := range out {
		out[i].Index = i
	}
	return out, nil
}

func (g *Graph) collectNameTreeEntries(value Value, seen map[Ref]bool) ([]NameTreeEntry, error) {
	if ref, ok := value.(Ref); ok {
		if seen == nil {
			seen = make(map[Ref]bool)
		}
		if seen[ref] {
			return nil, fmt.Errorf("name tree cycle at object %d %d", ref.Number, ref.Generation)
		}
		seen[ref] = true
		resolved, ok := g.Resolve(ref)
		if !ok {
			return nil, fmt.Errorf("name tree object %d %d not found", ref.Number, ref.Generation)
		}
		value = resolved
	}
	dict, ok := value.(Dict)
	if !ok {
		return nil, errors.New("name tree node is not a dictionary")
	}
	out := make([]NameTreeEntry, 0)
	if names, ok := dict["Names"].(Array); ok {
		if len(names)%2 != 0 {
			return nil, errors.New("name tree /Names array has an odd length")
		}
		for i := 0; i < len(names); i += 2 {
			key, ok := nameTreeKey(names[i])
			if !ok {
				return nil, errors.New("name tree key is not a string-like value")
			}
			out = append(out, NameTreeEntry{Name: key, Value: names[i+1]})
		}
	}
	if kids, ok := dict["Kids"].(Array); ok {
		for _, kid := range kids {
			entries, err := g.collectNameTreeEntries(kid, clonePublicRefSet(seen))
			if err != nil {
				return nil, err
			}
			out = append(out, entries...)
		}
	}
	return out, nil
}

func exportPDFObject(object *pdfIndirectObject) Object {
	if object == nil {
		return Object{}
	}
	return Object{
		Number:         object.ID.Number,
		Generation:     object.ID.Generation,
		Offset:         object.Offset,
		SourceKind:     pdfObjectSourceKind(object),
		InObjectStream: object.InObjectStream,
		Value:          exportPDFValue(object.Value),
	}
}

func pdfObjectSourceKind(object *pdfIndirectObject) ObjectSourceKind {
	if object == nil {
		return ObjectSourceNormal
	}
	if object.InObjectStream {
		return ObjectSourceObjectStream
	}
	if stream, ok := object.Value.(pdfStreamObject); ok {
		if dictHasType(stream.Dict, "ObjStm") {
			return ObjectSourceObjectStreamContainer
		}
		if dictHasType(stream.Dict, "XRef") {
			return ObjectSourceXRefStream
		}
	}
	return ObjectSourceNormal
}

func exportPDFValue(value pdfValue) Value {
	switch v := value.(type) {
	case nil:
		return Null{}
	case bool:
		return Bool(v)
	case int:
		return Number{Value: float64(v), Integer: true}
	case int64:
		return Number{Value: float64(v), Integer: true}
	case float64:
		return Number{Value: v, Integer: false}
	case pdfName:
		return Name(v)
	case pdfLiteralString:
		return String(decodeLiteralString(string(v)))
	case pdfDecryptedString:
		return String(string(v))
	case pdfHexString:
		return HexString(v)
	case pdfRef:
		return Ref{Number: v.ID.Number, Generation: v.ID.Generation}
	case pdfArray:
		out := make(Array, 0, len(v))
		for _, item := range v {
			out = append(out, exportPDFValue(item))
		}
		return out
	case pdfDict:
		out := make(Dict, len(v))
		for key, item := range v {
			out[key] = exportPDFValue(item)
		}
		return out
	case pdfStreamObject:
		dict, _ := exportPDFValue(v.Dict).(Dict)
		return Stream{
			Dict:        dict,
			Data:        bytes.Clone(v.Data),
			Filter:      normalizePDFStreamFilter(pdfGraphStreamFilterString(v.Dict)),
			DecodeParms: pdfGraphDecodeParmsString(v.Dict),
			SourceStart: v.SourceStart,
			SourceEnd:   v.SourceEnd,
		}
	case pdfRawObject:
		return bytes.Clone(v)
	default:
		return v
	}
}

func numberAsInt(value Value) (int, bool) {
	number, ok := value.(Number)
	if !ok || !number.Integer {
		return 0, false
	}
	return int(number.Value), true
}

func nameTreeKey(value Value) (string, bool) {
	switch v := value.(type) {
	case String:
		return string(v), true
	case HexString:
		if decoded, ok := decodeHexTextString([]byte(v)); ok {
			return decoded, true
		}
		return string(v), true
	case Name:
		return string(v), true
	default:
		return "", false
	}
}

func clonePublicRefSet(in map[Ref]bool) map[Ref]bool {
	if in == nil {
		return nil
	}
	out := make(map[Ref]bool, len(in))
	for key, value := range in {
		out[key] = value
	}
	return out
}
