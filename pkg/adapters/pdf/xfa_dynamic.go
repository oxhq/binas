package pdf

import (
	"encoding/xml"
	"fmt"
	"io"
	"strings"
)

type XFADynamicReport struct {
	Present            bool               `json:"present"`
	Dynamic            bool               `json:"dynamic"`
	Static             bool               `json:"static"`
	Markers            []XFADynamicMarker `json:"markers,omitempty"`
	UnsupportedReasons []string           `json:"unsupported_reasons,omitempty"`
}

func InspectXFADynamic(input []byte) (XFADynamicReport, error) {
	graph, err := parsePDFGraphWithOptions(input, pdfGraphParseOptions{AllowXFA: true})
	if err != nil {
		return XFADynamicReport{}, err
	}
	metadata := xfaPacketMetadataFromGraph(graph)
	report := XFADynamicReport{
		Present: len(metadata) > 0,
	}
	if !report.Present {
		return report, nil
	}

	for i, packet := range xfaPackets(graph, "") {
		report.Markers = append(report.Markers, inspectXFADynamicMarkersFromPacket(i, packet)...)
	}
	if len(report.Markers) > 0 {
		report.Dynamic = true
		report.UnsupportedReasons = []string{
			"dynamic XFA requires renderer-grade layout/render semantics",
			"XFA edit/render support is not implemented for dynamic packets",
		}
		return report, nil
	}

	if xfaPacketsAreStaticTemplateDatasets(metadata) {
		report.Static = true
		return report, nil
	}

	report.UnsupportedReasons = []string{
		"XFA packet family is not limited to static template/datasets packets",
		"XFA edit/render support is not implemented for this packet family",
	}
	for _, packet := range metadata {
		if packet.HasDecodeError && packet.DecodeError != "" {
			report.UnsupportedReasons = append(report.UnsupportedReasons, fmt.Sprintf("XFA packet %d decode error: %s", packet.Index, packet.DecodeError))
		}
		if packet.UnsafeXML && packet.XMLParseError != "" {
			report.UnsupportedReasons = append(report.UnsupportedReasons, fmt.Sprintf("XFA packet %d unsafe XML: %s", packet.Index, packet.XMLParseError))
		} else if packet.XMLParseError != "" {
			report.UnsupportedReasons = append(report.UnsupportedReasons, fmt.Sprintf("XFA packet %d XML parse error: %s", packet.Index, packet.XMLParseError))
		}
	}
	return report, nil
}

func inspectXFADynamicMarkersFromPacket(packetIndex int, packet xfaPacket) []XFADynamicMarker {
	decoder := xml.NewDecoder(strings.NewReader(packet.text))
	stack := make([]xfaDynamicFrame, 0)
	markers := make([]XFADynamicMarker, 0)
	for {
		token, err := decoder.Token()
		if err == io.EOF {
			break
		}
		if err != nil {
			return markers
		}
		switch t := token.(type) {
		case xml.StartElement:
			path := xfaDynamicPath(stack, t.Name.Local)
			if xfaDynamicFrameInside(stack, "template") {
				markers = append(markers, inspectXFATemplateDynamicElementMarkers(packetIndex, packet, t, path)...)
			}
			stack = append(stack, xfaDynamicFrame{localName: t.Name.Local, path: path})
		case xml.CharData:
			if len(stack) > 0 {
				stack[len(stack)-1].text.Write([]byte(t))
			}
		case xml.EndElement:
			if len(stack) == 0 {
				return markers
			}
			frame := stack[len(stack)-1]
			stack = stack[:len(stack)-1]
			if frame.localName == "dynamicRender" {
				value := strings.TrimSpace(frame.text.String())
				if xfaDynamicRenderRequiresRenderer(value) {
					markers = append(markers, XFADynamicMarker{
						PacketIndex: packetIndex,
						Label:       packet.label,
						PacketKind:  packet.kind,
						Path:        frame.path,
						Reason:      fmt.Sprintf("config dynamicRender=%q", value),
					})
				}
			}
		case xml.Directive:
			directive := strings.TrimSpace(strings.ToUpper(string(t)))
			if strings.HasPrefix(directive, "DOCTYPE") || strings.HasPrefix(directive, "ENTITY") {
				return markers
			}
		}
	}
	return markers
}

func inspectXFATemplateDynamicElementMarkers(packetIndex int, packet xfaPacket, element xml.StartElement, path string) []XFADynamicMarker {
	markers := xfaTemplateDynamicMarkers(packetIndex, packet, element, path)
	if presence := xfaXMLAttr(element, "presence"); presence != "" && !strings.EqualFold(presence, "visible") {
		markers = append(markers, XFADynamicMarker{
			PacketIndex: packetIndex,
			Label:       packet.label,
			PacketKind:  packet.kind,
			Path:        path,
			Reason:      fmt.Sprintf("template presence=%q", presence),
		})
	}
	return markers
}
