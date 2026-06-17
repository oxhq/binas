package pdf

import (
	"errors"
	"fmt"
)

var ErrUnsupportedImageMutation = errors.New("unsupported PDF image mutation")

type StreamObjectRef struct {
	ObjectNumber int `json:"object_number"`
	Generation   int `json:"generation"`
}

type StreamName string

type StreamMutationOptions struct {
	ObjectNumber int            `json:"object_number"`
	Generation   int            `json:"generation"`
	Replacement  []byte         `json:"-"`
	DictUpdates  map[string]any `json:"dict_updates,omitempty"`
}

type StreamMutationReport struct {
	Format        string         `json:"format"`
	Edit          string         `json:"edit"`
	NodesModified int            `json:"nodes_modified"`
	ObjectNumber  int            `json:"object_number"`
	Generation    int            `json:"generation"`
	ImageXObject  bool           `json:"image_xobject,omitempty"`
	Meta          map[string]any `json:"meta,omitempty"`
}

type StreamMutationVerification struct {
	ReparseOK      bool                       `json:"reparse_ok"`
	NoDanglingRefs bool                       `json:"no_dangling_refs"`
	DanglingRefs   []PageOperationDanglingRef `json:"dangling_refs,omitempty"`
}

type ReplaceImageXObjectOptions struct {
	ObjectNumber int            `json:"object_number"`
	Generation   int            `json:"generation"`
	ImageData    []byte         `json:"-"`
	DictUpdates  map[string]any `json:"dict_updates,omitempty"`
}

type ReplaceInlineImageOptions struct {
	PageObjectNumber int    `json:"page_object_number,omitempty"`
	PageGeneration   int    `json:"page_generation,omitempty"`
	Name             string `json:"name,omitempty"`
	Index            int    `json:"index,omitempty"`
	ImageData        []byte `json:"-"`
}

func MutateStream(input []byte, opts StreamMutationOptions) ([]byte, StreamMutationReport, StreamMutationVerification, error) {
	if opts.ObjectNumber <= 0 {
		return nil, StreamMutationReport{}, StreamMutationVerification{}, errors.New("stream mutation requires a positive object number")
	}
	graph, err := parsePDFGraph(input)
	if err != nil {
		return nil, StreamMutationReport{}, StreamMutationVerification{}, err
	}
	id := pdfObjectID{Number: opts.ObjectNumber, Generation: opts.Generation}
	object, ok := graph.Objects[id]
	if !ok {
		return nil, StreamMutationReport{}, StreamMutationVerification{}, fmt.Errorf("stream object %d %d not found", id.Number, id.Generation)
	}
	if object.InObjectStream {
		return nil, StreamMutationReport{}, StreamMutationVerification{}, fmt.Errorf("stream object %d %d is not a direct indirect object", id.Number, id.Generation)
	}
	stream, ok := object.Value.(pdfStreamObject)
	if !ok {
		return nil, StreamMutationReport{}, StreamMutationVerification{}, fmt.Errorf("object %d %d is not a stream", id.Number, id.Generation)
	}
	mutated, err := mutatedStreamObject(stream, opts.Replacement, opts.DictUpdates)
	if err != nil {
		return nil, StreamMutationReport{}, StreamMutationVerification{}, err
	}
	object.Value = mutated
	output, verification, err := writeAndVerifyStreamMutation(graph)
	if err != nil {
		return nil, StreamMutationReport{}, verification, err
	}
	report := streamMutationReport("pdf.stream_mutation", id, isPDFImageXObjectStreamDict(mutated.Dict), map[string]any{
		"replacement_policy": "raw_encoded_bytes",
		"decode_policy":      "no_decode_no_reencode",
		"writer":             "canonical",
	})
	return output, report, verification, nil
}

func ReplaceImageXObject(input []byte, opts ReplaceImageXObjectOptions) ([]byte, StreamMutationReport, StreamMutationVerification, error) {
	if opts.ObjectNumber <= 0 {
		return nil, StreamMutationReport{}, StreamMutationVerification{}, errors.New("image XObject replacement requires a positive object number")
	}
	graph, err := parsePDFGraph(input)
	if err != nil {
		return nil, StreamMutationReport{}, StreamMutationVerification{}, err
	}
	id := pdfObjectID{Number: opts.ObjectNumber, Generation: opts.Generation}
	object, ok := graph.Objects[id]
	if !ok {
		return nil, StreamMutationReport{}, StreamMutationVerification{}, fmt.Errorf("image XObject object %d %d not found", id.Number, id.Generation)
	}
	if object.InObjectStream {
		return nil, StreamMutationReport{}, StreamMutationVerification{}, fmt.Errorf("%w: image XObject object %d %d is not a direct indirect object", ErrUnsupportedImageMutation, id.Number, id.Generation)
	}
	stream, ok := object.Value.(pdfStreamObject)
	if !ok || !isPDFImageXObjectStreamDict(stream.Dict) {
		return nil, StreamMutationReport{}, StreamMutationVerification{}, fmt.Errorf("%w: object %d %d is not an image XObject stream", ErrUnsupportedImageMutation, id.Number, id.Generation)
	}
	if filter := pdfGraphStreamFilterString(stream.Dict); filter != "" {
		return nil, StreamMutationReport{}, StreamMutationVerification{}, fmt.Errorf("%w: image XObject stream %d %d has filter %q; encoded image filter replacement is not implemented", ErrUnsupportedImageMutation, id.Number, id.Generation, normalizePDFStreamFilter(filter))
	}
	mutated, err := mutatedStreamObject(stream, opts.ImageData, opts.DictUpdates)
	if err != nil {
		return nil, StreamMutationReport{}, StreamMutationVerification{}, err
	}
	object.Value = mutated
	output, verification, err := writeAndVerifyStreamMutation(graph)
	if err != nil {
		return nil, StreamMutationReport{}, verification, err
	}
	report := streamMutationReport("pdf.image_xobject_replace", id, true, map[string]any{
		"replacement_policy": "raw_unfiltered_image_bytes",
		"decode_policy":      "no_decode_no_reencode",
		"writer":             "canonical",
	})
	return output, report, verification, nil
}

func ReplaceInlineImage(input []byte, opts ReplaceInlineImageOptions) ([]byte, StreamMutationReport, StreamMutationVerification, error) {
	return nil, StreamMutationReport{}, StreamMutationVerification{}, fmt.Errorf("%w: inline image replacement is not implemented; use image XObject stream replacement by object reference", ErrUnsupportedImageMutation)
}

func mutatedStreamObject(stream pdfStreamObject, replacement []byte, updates map[string]any) (pdfStreamObject, error) {
	mutated := stream
	mutated.Data = append([]byte(nil), replacement...)
	mutated.Dict = clonePDFDict(stream.Dict)
	if mutated.Dict == nil {
		mutated.Dict = make(pdfDict)
	}
	for key, value := range updates {
		if key == "" {
			return pdfStreamObject{}, errors.New("stream dictionary update key cannot be empty")
		}
		if value == nil {
			delete(mutated.Dict, key)
			continue
		}
		converted, err := streamMutationPDFValue(value)
		if err != nil {
			return pdfStreamObject{}, fmt.Errorf("stream dictionary /%s: %w", key, err)
		}
		mutated.Dict[key] = converted
	}
	mutated.Dict["Length"] = len(mutated.Data)
	return mutated, nil
}

func streamMutationPDFValue(value any) (pdfValue, error) {
	switch v := value.(type) {
	case nil:
		return nil, nil
	case pdfName, pdfLiteralString, pdfHexString, pdfRef, pdfArray, pdfDict:
		return v, nil
	case StreamName:
		return pdfName(v), nil
	case StreamObjectRef:
		if v.ObjectNumber <= 0 {
			return nil, errors.New("object reference requires a positive object number")
		}
		return pdfRef{ID: pdfObjectID{Number: v.ObjectNumber, Generation: v.Generation}}, nil
	case []any:
		out := make(pdfArray, 0, len(v))
		for _, item := range v {
			converted, err := streamMutationPDFValue(item)
			if err != nil {
				return nil, err
			}
			out = append(out, converted)
		}
		return out, nil
	case []int:
		out := make(pdfArray, 0, len(v))
		for _, item := range v {
			out = append(out, item)
		}
		return out, nil
	case map[string]any:
		out := make(pdfDict, len(v))
		for key, item := range v {
			converted, err := streamMutationPDFValue(item)
			if err != nil {
				return nil, err
			}
			out[key] = converted
		}
		return out, nil
	case string:
		return pdfLiteralString(v), nil
	case int, int64, float64, bool:
		return v, nil
	default:
		return nil, fmt.Errorf("unsupported value type %T", value)
	}
}

func writeAndVerifyStreamMutation(graph *pdfGraph) ([]byte, StreamMutationVerification, error) {
	output, err := writeCanonicalPDF(graph)
	if err != nil {
		return nil, StreamMutationVerification{}, err
	}
	reparsed, err := parsePDFGraph(output)
	if err != nil {
		return output, StreamMutationVerification{}, err
	}
	dangling := reparsed.danglingIndirectReferences()
	verification := StreamMutationVerification{
		ReparseOK:      true,
		NoDanglingRefs: len(dangling) == 0,
		DanglingRefs:   dangling,
	}
	if len(dangling) > 0 {
		return output, verification, fmt.Errorf("stream mutation output has %d dangling indirect references", len(dangling))
	}
	return output, verification, nil
}

func streamMutationReport(edit string, id pdfObjectID, image bool, meta map[string]any) StreamMutationReport {
	return StreamMutationReport{
		Format:        "pdf",
		Edit:          edit,
		NodesModified: 1,
		ObjectNumber:  id.Number,
		Generation:    id.Generation,
		ImageXObject:  image,
		Meta:          meta,
	}
}
