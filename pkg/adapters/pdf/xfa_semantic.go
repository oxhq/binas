package pdf

import (
	"encoding/xml"
	"errors"
	"fmt"
	"io"
	"strings"
)

func ListXFADatasetFields(input []byte) ([]XFADatasetField, error) {
	graph, err := parsePDFGraphWithOptions(input, pdfGraphParseOptions{AllowXFA: true})
	if err != nil {
		return nil, err
	}
	packets := xfaPackets(graph, "")
	fields := make([]XFADatasetField, 0)
	for i, packet := range packets {
		if !xfaPacketMayContainDatasets(packet) {
			continue
		}
		packetFields, err := xfaDatasetFieldsFromPacket(packet.text)
		if err != nil {
			return nil, fmt.Errorf("XFA datasets packet %d: %w", i, err)
		}
		for _, field := range packetFields {
			field.PacketIndex = i
			field.Label = packet.label
			fields = append(fields, field)
		}
	}
	return fields, nil
}

func xfaPacketMayContainDatasets(packet xfaPacket) bool {
	if packet.label == "datasets" || packet.kind == "datasets" {
		return true
	}
	return packet.kind == "xdp" || packet.kind == "xml"
}

type xfaDatasetFrame struct {
	localName      string
	pathSegment    string
	text           strings.Builder
	hasChild       bool
	insideDatasets bool
	skipData       bool
}

func xfaDatasetFieldsFromPacket(text string) ([]XFADatasetField, error) {
	decoder := xml.NewDecoder(strings.NewReader(text))
	stack := make([]xfaDatasetFrame, 0)
	fields := make([]XFADatasetField, 0)
	for {
		token, err := decoder.Token()
		if err == io.EOF {
			break
		}
		if err != nil {
			return nil, err
		}
		switch t := token.(type) {
		case xml.StartElement:
			localName := t.Name.Local
			insideDatasets := localName == "datasets"
			if len(stack) > 0 {
				parent := &stack[len(stack)-1]
				parent.hasChild = true
				insideDatasets = insideDatasets || parent.insideDatasets
			}
			skipData := insideDatasets && localName == "data" && len(stack) > 0 && stack[len(stack)-1].localName == "datasets"
			pathSegment := ""
			if insideDatasets && localName != "datasets" && !skipData {
				pathSegment = localName
			}
			stack = append(stack, xfaDatasetFrame{
				localName:      localName,
				pathSegment:    pathSegment,
				insideDatasets: insideDatasets,
				skipData:       skipData,
			})
		case xml.CharData:
			if len(stack) > 0 {
				stack[len(stack)-1].text.Write([]byte(t))
			}
		case xml.EndElement:
			if len(stack) == 0 {
				return nil, errors.New("malformed XML: unexpected closing element")
			}
			frame := stack[len(stack)-1]
			stack = stack[:len(stack)-1]
			if !frame.insideDatasets || frame.localName == "datasets" || frame.skipData || frame.hasChild {
				continue
			}
			value := strings.TrimSpace(frame.text.String())
			if value == "" {
				continue
			}
			path := xfaDatasetPath(stack, frame.pathSegment)
			if path == "" {
				continue
			}
			fields = append(fields, XFADatasetField{Path: path, Value: value})
		case xml.Directive:
			directive := strings.TrimSpace(strings.ToUpper(string(t)))
			if strings.HasPrefix(directive, "DOCTYPE") || strings.HasPrefix(directive, "ENTITY") {
				return nil, errors.New("unsafe XML declaration: DOCTYPE and ENTITY are not supported")
			}
		}
	}
	if len(stack) != 0 {
		return nil, errors.New("malformed XML: unclosed element")
	}
	return fields, nil
}

func xfaDatasetPath(stack []xfaDatasetFrame, leaf string) string {
	parts := make([]string, 0, len(stack)+1)
	for _, frame := range stack {
		if frame.pathSegment != "" {
			parts = append(parts, frame.pathSegment)
		}
	}
	if leaf != "" {
		parts = append(parts, leaf)
	}
	return strings.Join(parts, ".")
}
