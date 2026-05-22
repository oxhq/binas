package pdf

import (
	"bytes"
	"encoding/xml"
	"errors"
	"fmt"
	"io"
	"strings"

	"github.com/oxhq/binas/pkg/core"
)

type XFADatasetFieldUpdateOptions struct {
	Selector XFASelector
}

type XFADatasetFieldListOptions struct {
	Selector XFASelector
}

type XFATemplateDatasetMapping struct {
	FieldName           string `json:"field_name"`
	DatasetPath         string `json:"dataset_path"`
	Value               string `json:"value"`
	TemplatePacketIndex int    `json:"template_packet_index"`
	DatasetPacketIndex  int    `json:"dataset_packet_index"`
	Label               string `json:"label,omitempty"`
}

func ListXFADatasetFields(input []byte) ([]XFADatasetField, error) {
	return ListXFADatasetFieldsWithOptions(input, XFADatasetFieldListOptions{})
}

func ListXFADatasetFieldsWithOptions(input []byte, options XFADatasetFieldListOptions) ([]XFADatasetField, error) {
	graph, err := parsePDFGraphWithOptions(input, pdfGraphParseOptions{AllowXFA: true})
	if err != nil {
		return nil, err
	}
	packets := xfaPackets(graph, "")
	fields := make([]XFADatasetField, 0)
	for i, packet := range packets {
		if !xfaPacketMayContainDatasets(packet) || !options.Selector.matches(packet.label, packet.kind) {
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

func ListXFATemplateDatasetMappings(input []byte) ([]XFATemplateDatasetMapping, error) {
	graph, err := parsePDFGraphWithOptions(input, pdfGraphParseOptions{AllowXFA: true})
	if err != nil {
		return nil, err
	}
	packets := xfaPackets(graph, "")
	datasetFields := make([]XFADatasetField, 0)
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
			datasetFields = append(datasetFields, field)
		}
	}
	datasetsByPath := make(map[string][]XFADatasetField)
	datasetsByLeaf := make(map[string][]XFADatasetField)
	for _, field := range datasetFields {
		datasetsByPath[field.Path] = append(datasetsByPath[field.Path], field)
		leaf := xfaLeafName(field.Path)
		if leaf != "" {
			datasetsByLeaf[leaf] = append(datasetsByLeaf[leaf], field)
		}
	}

	mappings := make([]XFATemplateDatasetMapping, 0)
	for i, packet := range packets {
		if !xfaPacketMayContainTemplate(packet) {
			continue
		}
		templateFields, err := xfaTemplateFieldsFromPacket(packet.text)
		if err != nil {
			return nil, fmt.Errorf("XFA template packet %d: %w", i, err)
		}
		for _, field := range templateFields {
			dataset, ok := xfaTemplateFieldDatasetMatch(field, datasetsByPath, datasetsByLeaf)
			if !ok {
				continue
			}
			mappings = append(mappings, XFATemplateDatasetMapping{
				FieldName:           field.name,
				DatasetPath:         dataset.Path,
				Value:               dataset.Value,
				TemplatePacketIndex: i,
				DatasetPacketIndex:  dataset.PacketIndex,
				Label:               packet.label,
			})
		}
	}
	return mappings, nil
}

func ApplyXFADatasetFieldUpdate(input []byte, path, value string) ([]byte, core.Report, core.Verification, error) {
	return ApplyXFADatasetFieldUpdateWithOptions(input, path, value, XFADatasetFieldUpdateOptions{})
}

func ApplyXFADatasetFieldUpdateWithOptions(input []byte, path, value string, options XFADatasetFieldUpdateOptions) ([]byte, core.Report, core.Verification, error) {
	if strings.TrimSpace(path) == "" {
		return nil, core.Report{}, core.Verification{}, errors.New("XFA dataset field update requires path")
	}
	graph, err := parsePDFGraphWithOptions(input, pdfGraphParseOptions{AllowXFA: true})
	if err != nil {
		return nil, core.Report{}, core.Verification{}, err
	}
	packets := xfaPackets(graph, "")
	matches := make([]xfaDatasetFieldUpdateMatch, 0, 1)
	containerMatches := 0
	selectedDatasets := 0
	for i, packet := range packets {
		if !xfaPacketMayContainDatasets(packet) || !options.Selector.matches(packet.label, packet.kind) {
			continue
		}
		selectedDatasets++
		packetMatches, packetContainers, err := xfaDatasetFieldUpdateMatchesFromPacket(packet.text, path, value)
		if err != nil {
			return nil, core.Report{}, core.Verification{}, fmt.Errorf("XFA datasets packet %d: %w", i, err)
		}
		containerMatches += packetContainers
		for _, match := range packetMatches {
			match.packet = packet
			matches = append(matches, match)
		}
	}
	if selectedDatasets == 0 {
		if !graphHasDirectXFA(graph) {
			return nil, core.Report{}, core.Verification{}, errors.New("unsupported PDF: XFA packet is not directly represented")
		}
		if options.Selector.empty() {
			return nil, core.Report{}, core.Verification{}, errors.New("no XFA datasets packet")
		}
		return nil, core.Report{}, core.Verification{}, fmt.Errorf("no XFA datasets packet matches selector %s", options.Selector.describe())
	}
	if len(matches) == 0 {
		if containerMatches > 0 {
			return nil, core.Report{}, core.Verification{}, fmt.Errorf("XFA dataset field path %q is not a leaf field", path)
		}
		return nil, core.Report{}, core.Verification{}, fmt.Errorf("no XFA dataset field path %q", path)
	}
	if len(matches) > 1 {
		return nil, core.Report{}, core.Verification{}, fmt.Errorf("XFA dataset field path %q is ambiguous: %d matches", path, len(matches))
	}
	match := matches[0]
	if err := match.packet.replace(match.packet.text, match.updatedPacket); err != nil {
		return nil, core.Report{}, core.Verification{}, err
	}
	output, err := writeCanonicalPDF(graph)
	if err != nil {
		return nil, core.Report{}, core.Verification{}, err
	}
	verification, err := verifySemanticPDF(output, pdfGraphParseOptions{AllowXFA: true})
	if err != nil {
		return nil, core.Report{}, core.Verification{}, err
	}
	verification.OldTextRemoved = match.oldValue == "" || !bytes.Contains(output, []byte(match.oldValue))
	verification.NewSelectable = xfaOutputHasDatasetValue(output, path, value)
	return output, WithNoFallbackPolicy(core.Report{
		Format:        "pdf",
		Edit:          "pdf.xfa_dataset_field_update",
		FallbackUsed:  false,
		NodesModified: 1,
		Invariants:    []core.Invariant{core.InvariantReparse, core.InvariantNoFallbackUsed},
	}), verification, nil
}

func xfaPacketMayContainDatasets(packet xfaPacket) bool {
	if packet.label == "datasets" || packet.kind == "datasets" {
		return true
	}
	xmlDiagnostics := xfaPacketXMLDiagnostics(packet.text)
	if xmlDiagnostics.rootLocalName == "datasets" {
		return true
	}
	return packet.kind == "xdp" || packet.kind == "xml"
}

func xfaPacketMayContainTemplate(packet xfaPacket) bool {
	if packet.label == "template" || packet.kind == "template" {
		return true
	}
	xmlDiagnostics := xfaPacketXMLDiagnostics(packet.text)
	if xmlDiagnostics.rootLocalName == "template" {
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

type xfaDatasetFieldUpdateMatch struct {
	packet        xfaPacket
	oldValue      string
	updatedPacket string
}

type xfaDatasetUpdateFrame struct {
	localName    string
	pathSegment  string
	contentStart int
	text         strings.Builder
	hasChild     bool
	inside       bool
	skipData     bool
}

type xfaTemplateFrame struct {
	pathSegment    string
	insideTemplate bool
}

type xfaTemplateField struct {
	name       string
	candidates []string
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

func xfaTemplateFieldsFromPacket(text string) ([]xfaTemplateField, error) {
	decoder := xml.NewDecoder(strings.NewReader(text))
	stack := make([]xfaTemplateFrame, 0)
	fields := make([]xfaTemplateField, 0)
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
			insideTemplate := localName == "template"
			if len(stack) > 0 {
				insideTemplate = insideTemplate || stack[len(stack)-1].insideTemplate
			}
			name := xfaElementName(t)
			if insideTemplate && localName == "field" && name != "" {
				fields = append(fields, xfaTemplateField{
					name:       name,
					candidates: xfaTemplateFieldCandidates(stack, name),
				})
			}
			pathSegment := ""
			if insideTemplate && localName == "subform" && name != "" {
				pathSegment = name
			}
			stack = append(stack, xfaTemplateFrame{
				pathSegment:    pathSegment,
				insideTemplate: insideTemplate,
			})
		case xml.EndElement:
			if len(stack) == 0 {
				return nil, errors.New("malformed XML: unexpected closing element")
			}
			stack = stack[:len(stack)-1]
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

func xfaDatasetFieldUpdateMatchesFromPacket(text, targetPath, newValue string) ([]xfaDatasetFieldUpdateMatch, int, error) {
	decoder := xml.NewDecoder(strings.NewReader(text))
	stack := make([]xfaDatasetUpdateFrame, 0)
	matches := make([]xfaDatasetFieldUpdateMatch, 0, 1)
	containerMatches := 0
	for {
		token, err := decoder.Token()
		if err == io.EOF {
			break
		}
		if err != nil {
			return nil, 0, err
		}
		switch t := token.(type) {
		case xml.StartElement:
			localName := t.Name.Local
			insideDatasets := localName == "datasets"
			if len(stack) > 0 {
				parent := &stack[len(stack)-1]
				parent.hasChild = true
				insideDatasets = insideDatasets || parent.inside
			}
			skipData := insideDatasets && localName == "data" && len(stack) > 0 && stack[len(stack)-1].localName == "datasets"
			pathSegment := ""
			if insideDatasets && localName != "datasets" && !skipData {
				pathSegment = localName
			}
			stack = append(stack, xfaDatasetUpdateFrame{
				localName:    localName,
				pathSegment:  pathSegment,
				contentStart: int(decoder.InputOffset()),
				inside:       insideDatasets,
				skipData:     skipData,
			})
		case xml.CharData:
			if len(stack) > 0 {
				stack[len(stack)-1].text.Write([]byte(t))
			}
		case xml.EndElement:
			if len(stack) == 0 {
				return nil, 0, errors.New("malformed XML: unexpected closing element")
			}
			endOffset := int(decoder.InputOffset())
			endStart := strings.LastIndex(text[:endOffset], "</")
			if endStart < 0 {
				return nil, 0, errors.New("malformed XML: closing element span not found")
			}
			frame := stack[len(stack)-1]
			stack = stack[:len(stack)-1]
			if !frame.inside || frame.localName == "datasets" || frame.skipData {
				continue
			}
			path := xfaDatasetUpdatePath(stack, frame.pathSegment)
			if path != targetPath {
				continue
			}
			if frame.hasChild {
				containerMatches++
				continue
			}
			updated, err := xfaReplaceElementText(text, frame.contentStart, endStart, newValue)
			if err != nil {
				return nil, 0, err
			}
			matches = append(matches, xfaDatasetFieldUpdateMatch{
				oldValue:      strings.TrimSpace(frame.text.String()),
				updatedPacket: updated,
			})
		case xml.Directive:
			directive := strings.TrimSpace(strings.ToUpper(string(t)))
			if strings.HasPrefix(directive, "DOCTYPE") || strings.HasPrefix(directive, "ENTITY") {
				return nil, 0, errors.New("unsafe XML declaration: DOCTYPE and ENTITY are not supported")
			}
		}
	}
	if len(stack) != 0 {
		return nil, 0, errors.New("malformed XML: unclosed element")
	}
	return matches, containerMatches, nil
}

func xfaReplaceElementText(input string, start, end int, value string) (string, error) {
	if start < 0 || end < start || end > len(input) {
		return "", errors.New("unsafe XFA dataset field span")
	}
	var escaped bytes.Buffer
	if err := xml.EscapeText(&escaped, []byte(value)); err != nil {
		return "", err
	}
	return input[:start] + escaped.String() + input[end:], nil
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

func xfaTemplateFieldCandidates(stack []xfaTemplateFrame, name string) []string {
	candidates := make([]string, 0, 2)
	candidates = append(candidates, name)
	if !strings.Contains(name, ".") {
		parts := make([]string, 0, len(stack)+1)
		for _, frame := range stack {
			if frame.pathSegment != "" {
				parts = append(parts, frame.pathSegment)
			}
		}
		if len(parts) > 0 {
			parts = append(parts, name)
			candidate := strings.Join(parts, ".")
			if candidate != name {
				candidates = append(candidates, candidate)
			}
		}
	}
	return candidates
}

func xfaTemplateFieldDatasetMatch(field xfaTemplateField, datasetsByPath, datasetsByLeaf map[string][]XFADatasetField) (XFADatasetField, bool) {
	for _, candidate := range field.candidates {
		matches := datasetsByPath[candidate]
		if len(matches) == 1 {
			return matches[0], true
		}
	}
	matches := datasetsByLeaf[field.name]
	if len(matches) == 1 {
		return matches[0], true
	}
	return XFADatasetField{}, false
}

func xfaElementName(element xml.StartElement) string {
	for _, attr := range element.Attr {
		if attr.Name.Local == "name" {
			return strings.TrimSpace(attr.Value)
		}
	}
	return ""
}

func xfaLeafName(path string) string {
	if path == "" {
		return ""
	}
	index := strings.LastIndex(path, ".")
	if index < 0 {
		return path
	}
	return path[index+1:]
}

func xfaDatasetUpdatePath(stack []xfaDatasetUpdateFrame, leaf string) string {
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

func xfaOutputHasDatasetValue(output []byte, path, value string) bool {
	fields, err := ListXFADatasetFields(output)
	if err != nil {
		return false
	}
	for _, field := range fields {
		if field.Path == path && field.Value == value {
			return true
		}
	}
	return false
}
