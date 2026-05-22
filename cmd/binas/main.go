package main

import (
	"encoding/json"
	"errors"
	"flag"
	"fmt"
	"os"
	"strconv"
	"strings"

	"github.com/oxhq/binas/pkg/adapters/pdf"
	"github.com/oxhq/binas/pkg/core"
)

func main() {
	if err := run(os.Args[1:]); err != nil {
		fmt.Fprintln(os.Stderr, "binas:", err)
		os.Exit(1)
	}
}

func run(args []string) error {
	if len(args) == 0 {
		return errors.New("expected command: inspect, query, edit, overlay, ocr, form, annot, xfa, signature, validate")
	}
	switch args[0] {
	case "inspect":
		return inspect(args[1:])
	case "query":
		return query(args[1:])
	case "edit":
		return edit(args[1:])
	case "overlay":
		return overlay(args[1:])
	case "ocr":
		return ocr(args[1:])
	case "form":
		return form(args[1:])
	case "annot":
		return annot(args[1:])
	case "xfa":
		return xfa(args[1:])
	case "signature":
		return signature(args[1:])
	case "validate":
		return validate(args[1:])
	default:
		return fmt.Errorf("unknown command %q", args[0])
	}
}

func ocr(args []string) error {
	if len(args) == 0 {
		return errors.New("expected ocr command: text-layer-plan")
	}
	switch args[0] {
	case "text-layer-plan":
		return ocrTextLayerPlan(args[1:])
	default:
		return fmt.Errorf("unknown ocr command %q", args[0])
	}
}

func ocrTextLayerPlan(args []string) error {
	fs := flag.NewFlagSet("ocr text-layer-plan", flag.ContinueOnError)
	format := fs.String("format", "pdf", "input format")
	pageIndex := fs.Int("page-index", 0, "zero-based page index")
	text := fs.String("text", "", "OCR text")
	xMin := fs.Float64("x-min", 0, "OCR text bounding box minimum x")
	yMin := fs.Float64("y-min", 0, "OCR text bounding box minimum y")
	xMax := fs.Float64("x-max", 0, "OCR text bounding box maximum x")
	yMax := fs.Float64("y-max", 0, "OCR text bounding box maximum y")
	confidence := fs.Float64("confidence", 0, "OCR confidence from 0 to 1")
	asJSON := fs.Bool("json", false, "write JSON")
	if err := fs.Parse(reorderFlags(args, map[string]bool{"json": true}, map[string]bool{
		"format":     true,
		"page-index": true,
		"text":       true,
		"x-min":      true,
		"y-min":      true,
		"x-max":      true,
		"y-max":      true,
		"confidence": true,
	})); err != nil {
		return err
	}
	if fs.NArg() != 1 {
		return errors.New("ocr text-layer-plan requires one input file")
	}
	if strings.ToLower(*format) != "pdf" {
		return fmt.Errorf("ocr text-layer-plan is unsupported for format %q", *format)
	}
	input, err := os.ReadFile(fs.Arg(0))
	if err != nil {
		return err
	}
	plan, err := pdf.PlanExplicitOCRTextLayer(input, pdf.OCRTextLayerOptions{
		PageIndex: *pageIndex,
		Text:      *text,
		Box: pdf.OCRTextLayerBox{
			XMin: *xMin,
			YMin: *yMin,
			XMax: *xMax,
			YMax: *yMax,
		},
		Confidence: *confidence,
	})
	if err != nil {
		return err
	}
	if *asJSON {
		return writeJSON(plan)
	}
	fmt.Printf("planned %s page=%d confidence=%g\n", plan.Operation, plan.PageIndex, plan.Confidence)
	return nil
}

func signature(args []string) error {
	if len(args) == 0 {
		return errors.New("expected signature command: inspect")
	}
	switch args[0] {
	case "inspect":
		return signatureInspect(args[1:])
	default:
		return fmt.Errorf("unknown signature command %q", args[0])
	}
}

func signatureInspect(args []string) error {
	fs := flag.NewFlagSet("signature inspect", flag.ContinueOnError)
	format := fs.String("format", "pdf", "input format")
	asJSON := fs.Bool("json", false, "write JSON")
	if err := fs.Parse(reorderFlags(args, map[string]bool{"json": true}, map[string]bool{"format": true})); err != nil {
		return err
	}
	if fs.NArg() != 1 {
		return errors.New("signature inspect requires one input file")
	}
	if strings.ToLower(*format) != "pdf" {
		return fmt.Errorf("signature inspect is unsupported for format %q", *format)
	}
	input, err := os.ReadFile(fs.Arg(0))
	if err != nil {
		return err
	}
	signature := pdf.SecurityMetadataForInput(input).Signature
	if *asJSON {
		return writeJSON(signature)
	}
	parts := []string{
		fmt.Sprintf("present=%t", signature.Present),
		fmt.Sprintf("byte_range_status=%s", signature.ByteRangeStatus),
		fmt.Sprintf("byte_range_count=%d", signature.ByteRangeCount),
		fmt.Sprintf("byte_range_total_ranges=%d", signature.ByteRangeTotalRanges),
		fmt.Sprintf("byte_range_covered_bytes=%d", signature.ByteRangeCoveredBytes),
		fmt.Sprintf("signature_container=%s", signature.SignatureContainer),
		fmt.Sprintf("digest_algorithm=%s", signature.DigestAlgorithm),
		fmt.Sprintf("digest_algorithm_status=%s", signature.DigestAlgorithmStatus),
		fmt.Sprintf("cryptographic_validation_status=%s", signature.CryptographicValidationStatus),
	}
	if signature.ObjectNumber != nil && signature.ObjectGeneration != nil {
		parts = append(parts, fmt.Sprintf("object=%d %d R", *signature.ObjectNumber, *signature.ObjectGeneration))
	}
	if signature.Filter != "" {
		parts = append(parts, fmt.Sprintf("filter=%s", signature.Filter))
	}
	if signature.SubFilter != "" {
		parts = append(parts, fmt.Sprintf("sub_filter=%s", signature.SubFilter))
	}
	if signature.ContentsByteLength != nil {
		parts = append(parts, fmt.Sprintf("contents_byte_length=%d", *signature.ContentsByteLength))
	}
	if signature.SigningTime != "" {
		parts = append(parts, fmt.Sprintf("signing_time=%q", signature.SigningTime))
	}
	fmt.Println("signature " + strings.Join(parts, " "))
	return nil
}

func overlay(args []string) error {
	if len(args) == 0 {
		return errors.New("expected overlay command: text")
	}
	switch args[0] {
	case "text":
		return overlayText(args[1:])
	default:
		return fmt.Errorf("unknown overlay command %q", args[0])
	}
}

func overlayText(args []string) error {
	fs := flag.NewFlagSet("overlay text", flag.ContinueOnError)
	format := fs.String("format", "pdf", "input format")
	pageIndex := fs.Int("page-index", 0, "zero-based page index")
	text := fs.String("text", "", "overlay text")
	x := fs.Float64("x", 0, "overlay x position")
	y := fs.Float64("y", 0, "overlay y position")
	fontSize := fs.Float64("font-size", 12, "overlay font size")
	outputPath := fs.String("o", "", "output file")
	asJSON := fs.Bool("json", false, "write JSON")
	if err := fs.Parse(reorderFlags(args, map[string]bool{"json": true}, map[string]bool{"format": true, "page-index": true, "text": true, "x": true, "y": true, "font-size": true, "o": true})); err != nil {
		return err
	}
	if fs.NArg() != 1 {
		return errors.New("overlay text requires one input file")
	}
	if strings.ToLower(*format) != "pdf" {
		return fmt.Errorf("overlay text is unsupported for format %q", *format)
	}
	if *text == "" {
		return errors.New("overlay text requires --text")
	}
	if *outputPath == "" {
		return errors.New("overlay text requires -o")
	}
	input, err := os.ReadFile(fs.Arg(0))
	if err != nil {
		return err
	}
	output, report, verification, err := pdf.ApplyExplicitOverlayStamp(input, pdf.ExplicitOverlayStampOptions{
		PageIndex: *pageIndex,
		Text:      *text,
		X:         *x,
		Y:         *y,
		FontSize:  *fontSize,
	})
	if err != nil {
		return err
	}
	return writeOverlayTextResult(output, report, verification, *outputPath, *asJSON)
}

func writeOverlayTextResult(output []byte, report core.Report, verification core.Verification, outputPath string, asJSON bool) error {
	if err := os.WriteFile(outputPath, output, 0644); err != nil {
		return err
	}
	report.OutputPath = outputPath
	report.Verification = &verification
	result := map[string]any{"operation": report.Edit, "report": report, "verification": verification}
	if asJSON {
		return writeJSON(result)
	}
	fmt.Println("overlaid", strconv.Quote(outputPath))
	return nil
}

func form(args []string) error {
	if len(args) == 0 {
		return errors.New("expected form command: list, set")
	}
	switch args[0] {
	case "list":
		return formList(args[1:])
	case "set":
		return formSet(args[1:])
	default:
		return fmt.Errorf("unknown form command %q", args[0])
	}
}

func formList(args []string) error {
	fs := flag.NewFlagSet("form list", flag.ContinueOnError)
	format := fs.String("format", "pdf", "input format")
	asJSON := fs.Bool("json", false, "write JSON")
	if err := fs.Parse(reorderFlags(args, map[string]bool{"json": true}, map[string]bool{"format": true})); err != nil {
		return err
	}
	if fs.NArg() != 1 {
		return errors.New("form list requires one input file")
	}
	if strings.ToLower(*format) != "pdf" {
		return fmt.Errorf("form list is unsupported for format %q", *format)
	}
	input, err := os.ReadFile(fs.Arg(0))
	if err != nil {
		return err
	}
	fields, err := pdf.ListFormFields(input)
	if err != nil {
		return err
	}
	result := map[string]any{"fields": fields, "count": len(fields)}
	if *asJSON {
		return writeJSON(result)
	}
	for _, field := range fields {
		object := "direct"
		if field.ObjectNumber != nil && field.ObjectGeneration != nil {
			object = fmt.Sprintf("%d %d R", *field.ObjectNumber, *field.ObjectGeneration)
		}
		value := ""
		if field.Value != nil {
			value = *field.Value
		}
		parts := []string{
			fmt.Sprintf("%d", field.Index),
			object,
			fmt.Sprintf("name=%q", field.Name),
			fmt.Sprintf("field_type=%s", field.FieldType),
			fmt.Sprintf("value=%q", value),
			fmt.Sprintf("kids=%d", field.KidCount),
			fmt.Sprintf("button_widget_appearance_proof=%t", field.ButtonWidgetAppearanceProof),
		}
		if field.Flags != nil {
			parts = append(parts, fmt.Sprintf("flags=%d", *field.Flags))
		}
		if len(field.FlagNames) > 0 {
			parts = append(parts, fmt.Sprintf("flag_names=%q", field.FlagNames))
		}
		if len(field.TypeFlagNames) > 0 {
			parts = append(parts, fmt.Sprintf("type_flag_names=%q", field.TypeFlagNames))
		}
		if field.AlternateName != nil {
			parts = append(parts, fmt.Sprintf("alternate_name=%q", *field.AlternateName))
		}
		if field.MappingName != nil {
			parts = append(parts, fmt.Sprintf("mapping_name=%q", *field.MappingName))
		}
		if field.DefaultValue != nil {
			parts = append(parts, fmt.Sprintf("default_value=%q", *field.DefaultValue))
		}
		if len(field.Options) > 0 {
			parts = append(parts, fmt.Sprintf("options_count=%d", len(field.Options)), fmt.Sprintf("options=%q", field.Options))
		}
		if len(field.ButtonStates) > 0 {
			parts = append(parts, fmt.Sprintf("button_states=%q", field.ButtonStates))
		}
		fmt.Println(strings.Join(parts, " "))
	}
	return nil
}

func formSet(args []string) error {
	fs := flag.NewFlagSet("form set", flag.ContinueOnError)
	format := fs.String("format", "pdf", "input format")
	field := fs.String("field", "", "AcroForm field name")
	value := fs.String("value", "", "new field value")
	matchIndex := optionalIntFlag{}
	fs.Var(&matchIndex, "match-index", "zero-based field match index")
	outputPath := fs.String("o", "", "output file")
	asJSON := fs.Bool("json", false, "write JSON")
	regenerateAppearance := fs.Bool("regenerate-appearance", false, "regenerate simple text/choice widget appearances")
	if err := fs.Parse(reorderFlags(args, map[string]bool{"json": true, "regenerate-appearance": true}, map[string]bool{"format": true, "field": true, "value": true, "match-index": true, "o": true})); err != nil {
		return err
	}
	if fs.NArg() != 1 {
		return errors.New("form set requires one input file")
	}
	if strings.ToLower(*format) != "pdf" {
		return fmt.Errorf("form set is unsupported for format %q", *format)
	}
	if *field == "" {
		return errors.New("form set requires --field")
	}
	if *outputPath == "" {
		return errors.New("form set requires -o")
	}
	input, err := os.ReadFile(fs.Arg(0))
	if err != nil {
		return err
	}
	var selected *int
	if matchIndex.set {
		selected = &matchIndex.value
	}
	output, report, verification, err := pdf.ApplyFormFieldEditWithOptions(input, *field, *value, pdf.FormFieldEditOptions{
		MatchIndex:           selected,
		RegenerateAppearance: *regenerateAppearance,
	})
	if err != nil {
		if *asJSON && *regenerateAppearance && isUnsupportedAppearanceError(err) {
			if jsonErr := writeUnsupportedAppearanceErrorJSON(err); jsonErr != nil {
				return jsonErr
			}
		}
		return err
	}
	return writeFormSetResult(output, report, verification, *outputPath, *asJSON)
}

func writeFormSetResult(output []byte, report pdf.FormFieldEditReport, verification pdf.FormFieldEditVerification, outputPath string, asJSON bool) error {
	if err := os.WriteFile(outputPath, output, 0644); err != nil {
		return err
	}
	report.OutputPath = outputPath
	report.Report.Verification = &core.Verification{ReparseOK: verification.ReparseOK}
	result := map[string]any{"report": report, "verification": verification}
	if asJSON {
		return writeJSON(result)
	}
	fmt.Println("edited", strconv.Quote(outputPath))
	return nil
}

func annot(args []string) error {
	if len(args) == 0 {
		return errors.New("expected annot command: list, set-contents")
	}
	switch args[0] {
	case "list":
		return annotList(args[1:])
	case "set-contents":
		return annotSetContents(args[1:])
	default:
		return fmt.Errorf("unknown annot command %q", args[0])
	}
}

func annotList(args []string) error {
	fs := flag.NewFlagSet("annot list", flag.ContinueOnError)
	format := fs.String("format", "pdf", "input format")
	asJSON := fs.Bool("json", false, "write JSON")
	if err := fs.Parse(reorderFlags(args, map[string]bool{"json": true}, map[string]bool{"format": true})); err != nil {
		return err
	}
	if fs.NArg() != 1 {
		return errors.New("annot list requires one input file")
	}
	if strings.ToLower(*format) != "pdf" {
		return fmt.Errorf("annot list is unsupported for format %q", *format)
	}
	input, err := os.ReadFile(fs.Arg(0))
	if err != nil {
		return err
	}
	annotations, err := pdf.ListAnnotationCandidates(input)
	if err != nil {
		return err
	}
	result := map[string]any{"annotations": annotations, "count": len(annotations)}
	if *asJSON {
		return writeJSON(result)
	}
	for _, annotation := range annotations {
		object := "direct"
		if annotation.ObjectNumber != nil && annotation.ObjectGeneration != nil {
			object = fmt.Sprintf("%d %d R", *annotation.ObjectNumber, *annotation.ObjectGeneration)
		}
		parts := []string{
			fmt.Sprintf("%d", annotation.Index),
			object,
			fmt.Sprintf("subtype=%s", annotation.Subtype),
			fmt.Sprintf("contents=%q", annotation.Contents),
			fmt.Sprintf("has_appearance=%t", annotation.HasAppearance),
		}
		if annotation.PageIndex != nil {
			parts = append(parts, fmt.Sprintf("page_index=%d", *annotation.PageIndex))
		}
		if annotation.PageObjectNumber != nil && annotation.PageObjectGeneration != nil {
			parts = append(parts, fmt.Sprintf("page_object=%d %d R", *annotation.PageObjectNumber, *annotation.PageObjectGeneration))
		}
		if len(annotation.Rect) > 0 {
			parts = append(parts, fmt.Sprintf("rect=%s", formatFloat64Slice(annotation.Rect)))
		}
		if len(annotation.Color) > 0 {
			parts = append(parts, fmt.Sprintf("color=%s", formatFloat64Slice(annotation.Color)))
		}
		if len(annotation.Border) > 0 {
			parts = append(parts, fmt.Sprintf("border=%s", formatFloat64Slice(annotation.Border)))
		}
		if annotation.QuadPointsCount > 0 {
			parts = append(parts, fmt.Sprintf("quad_points_count=%d", annotation.QuadPointsCount))
		}
		if annotation.Name != "" {
			parts = append(parts, fmt.Sprintf("name=%q", annotation.Name))
		}
		if annotation.Modified != "" {
			parts = append(parts, fmt.Sprintf("modified=%q", annotation.Modified))
		}
		if annotation.Title != "" {
			parts = append(parts, fmt.Sprintf("title=%q", annotation.Title))
		}
		if annotation.Flags != 0 || len(annotation.FlagNames) > 0 {
			parts = append(parts, fmt.Sprintf("flags=%d", annotation.Flags))
		}
		if len(annotation.FlagNames) > 0 {
			parts = append(parts, fmt.Sprintf("flag_names=%q", annotation.FlagNames))
		}
		fmt.Println(strings.Join(parts, " "))
	}
	return nil
}

func formatFloat64Slice(values []float64) string {
	parts := make([]string, 0, len(values))
	for _, value := range values {
		parts = append(parts, strconv.FormatFloat(value, 'f', -1, 64))
	}
	return "[" + strings.Join(parts, " ") + "]"
}

func annotSetContents(args []string) error {
	fs := flag.NewFlagSet("annot set-contents", flag.ContinueOnError)
	format := fs.String("format", "pdf", "input format")
	index := fs.Int("index", 0, "zero-based annotation index")
	contents := fs.String("contents", "", "new annotation contents")
	removeAppearance := fs.Bool("remove-appearance", false, "remove stale annotation /AP after updating /Contents")
	regenerateAppearance := fs.Bool("regenerate-appearance", false, "regenerate a basic annotation /AP /N appearance stream after updating /Contents")
	outputPath := fs.String("o", "", "output file")
	asJSON := fs.Bool("json", false, "write JSON")
	if err := fs.Parse(reorderFlags(args, map[string]bool{"json": true, "remove-appearance": true, "regenerate-appearance": true}, map[string]bool{"format": true, "index": true, "contents": true, "o": true})); err != nil {
		return err
	}
	if *removeAppearance && *regenerateAppearance {
		return errors.New("use only one of --remove-appearance or --regenerate-appearance")
	}
	if fs.NArg() != 1 {
		return errors.New("annot set-contents requires one input file")
	}
	if strings.ToLower(*format) != "pdf" {
		return fmt.Errorf("annot set-contents is unsupported for format %q", *format)
	}
	if *outputPath == "" {
		return errors.New("annot set-contents requires -o")
	}
	input, err := os.ReadFile(fs.Arg(0))
	if err != nil {
		return err
	}
	output, report, verification, err := pdf.ApplyAnnotationContentsEdit(input, *index, *contents, pdf.AnnotationContentsEditOptions{
		RemoveAppearance:     *removeAppearance,
		RegenerateAppearance: *regenerateAppearance,
	})
	if err != nil {
		if *asJSON && *regenerateAppearance && isUnsupportedAppearanceError(err) {
			if jsonErr := writeUnsupportedAppearanceErrorJSON(err); jsonErr != nil {
				return jsonErr
			}
		}
		return err
	}
	if err := os.WriteFile(*outputPath, output, 0644); err != nil {
		return err
	}
	report.OutputPath = *outputPath
	result := map[string]any{"report": report, "verification": verification}
	if *asJSON {
		return writeJSON(result)
	}
	fmt.Println("edited", strconv.Quote(*outputPath))
	return nil
}

func xfa(args []string) error {
	if len(args) == 0 {
		return errors.New("expected xfa command: list, datasets, mappings, dataset-set, replace")
	}
	switch args[0] {
	case "list":
		return xfaList(args[1:])
	case "datasets":
		return xfaDatasets(args[1:])
	case "mappings":
		return xfaMappings(args[1:])
	case "dataset-set":
		return xfaDatasetSet(args[1:])
	case "replace":
		return xfaReplace(args[1:])
	default:
		return fmt.Errorf("unknown xfa command %q", args[0])
	}
}

func xfaList(args []string) error {
	fs := flag.NewFlagSet("xfa list", flag.ContinueOnError)
	format := fs.String("format", "pdf", "input format")
	packetKind := fs.String("packet-kind", "", "filter XFA packets by conservative packet kind")
	label := fs.String("label", "", "filter XFA packets by array label")
	asJSON := fs.Bool("json", false, "write JSON")
	if err := fs.Parse(reorderFlags(args, map[string]bool{"json": true}, map[string]bool{"format": true, "packet-kind": true, "label": true})); err != nil {
		return err
	}
	if fs.NArg() != 1 {
		return errors.New("xfa list requires one input file")
	}
	if strings.ToLower(*format) != "pdf" {
		return fmt.Errorf("xfa list is unsupported for format %q", *format)
	}
	input, err := os.ReadFile(fs.Arg(0))
	if err != nil {
		return err
	}
	packets, err := pdf.ListXFAPacketsWithOptions(input, pdf.XFAPacketListOptions{
		Selector: pdf.XFASelector{PacketKind: *packetKind, Label: *label},
	})
	if err != nil {
		return err
	}
	result := map[string]any{"packets": packets, "count": len(packets)}
	if *asJSON {
		return writeJSON(result)
	}
	for _, packet := range packets {
		object := "direct"
		if packet.ObjectNumber != nil && packet.ObjectGeneration != nil {
			object = fmt.Sprintf("%d %d R", *packet.ObjectNumber, *packet.ObjectGeneration)
		}
		label := "-"
		if packet.Label != "" {
			label = packet.Label
		}
		parts := []string{
			fmt.Sprintf("%d", packet.Index),
			object,
			fmt.Sprintf("label=%s", label),
			fmt.Sprintf("stream=%t", packet.IsStream),
			fmt.Sprintf("length=%d", packet.TextLength),
			fmt.Sprintf("byte_length=%d", packet.ByteLength),
			fmt.Sprintf("preview=%q", packet.Preview),
		}
		if packet.PacketKind != "" {
			parts = append(parts, fmt.Sprintf("packet_kind=%s", packet.PacketKind))
		}
		if packet.Filter != "" {
			parts = append(parts, fmt.Sprintf("filter=%s", packet.Filter))
		}
		if packet.DecodeParms != "" {
			parts = append(parts, fmt.Sprintf("decode_parms=%q", packet.DecodeParms))
		}
		if packet.DecodeError != "" {
			parts = append(parts, fmt.Sprintf("decode_error=%q", packet.DecodeError))
		}
		fmt.Println(strings.Join(parts, " "))
	}
	return nil
}

func xfaDatasets(args []string) error {
	fs := flag.NewFlagSet("xfa datasets", flag.ContinueOnError)
	format := fs.String("format", "pdf", "input format")
	packetKind := fs.String("packet-kind", "", "filter XFA dataset packets by conservative packet kind")
	label := fs.String("label", "", "filter XFA dataset packets by array label")
	asJSON := fs.Bool("json", false, "write JSON")
	if err := fs.Parse(reorderFlags(args, map[string]bool{"json": true}, map[string]bool{"format": true, "packet-kind": true, "label": true})); err != nil {
		return err
	}
	if fs.NArg() != 1 {
		return errors.New("xfa datasets requires one input file")
	}
	if strings.ToLower(*format) != "pdf" {
		return fmt.Errorf("xfa datasets is unsupported for format %q", *format)
	}
	input, err := os.ReadFile(fs.Arg(0))
	if err != nil {
		return err
	}
	fields, err := pdf.ListXFADatasetFieldsWithOptions(input, pdf.XFADatasetFieldListOptions{
		Selector: pdf.XFASelector{PacketKind: *packetKind, Label: *label},
	})
	if err != nil {
		return err
	}
	result := map[string]any{"fields": fields, "count": len(fields)}
	if *asJSON {
		return writeJSON(result)
	}
	for _, field := range fields {
		fmt.Printf("%s=%s\n", field.Path, field.Value)
	}
	return nil
}

func xfaMappings(args []string) error {
	fs := flag.NewFlagSet("xfa mappings", flag.ContinueOnError)
	format := fs.String("format", "pdf", "input format")
	asJSON := fs.Bool("json", false, "write JSON")
	if err := fs.Parse(reorderFlags(args, map[string]bool{"json": true}, map[string]bool{"format": true})); err != nil {
		return err
	}
	if fs.NArg() != 1 {
		return errors.New("xfa mappings requires one input file")
	}
	if strings.ToLower(*format) != "pdf" {
		return fmt.Errorf("xfa mappings is unsupported for format %q", *format)
	}
	input, err := os.ReadFile(fs.Arg(0))
	if err != nil {
		return err
	}
	mappings, err := pdf.ListXFATemplateDatasetMappings(input)
	if err != nil {
		return err
	}
	result := map[string]any{"mappings": mappings, "count": len(mappings)}
	if *asJSON {
		return writeJSON(result)
	}
	for _, mapping := range mappings {
		label := "-"
		if mapping.Label != "" {
			label = mapping.Label
		}
		fmt.Printf("%s=%s field=%s template_packet=%d dataset_packet=%d label=%s\n",
			mapping.DatasetPath,
			mapping.Value,
			mapping.FieldName,
			mapping.TemplatePacketIndex,
			mapping.DatasetPacketIndex,
			label,
		)
	}
	return nil
}

func xfaDatasetSet(args []string) error {
	fs := flag.NewFlagSet("xfa dataset-set", flag.ContinueOnError)
	format := fs.String("format", "pdf", "input format")
	path := fs.String("path", "", "XFA dataset field path")
	value := fs.String("value", "", "replacement XFA dataset field value")
	packetKind := fs.String("packet-kind", "", "restrict update to a conservative XFA packet kind")
	label := fs.String("label", "", "restrict update to an XFA array label")
	outputPath := fs.String("o", "", "output file")
	asJSON := fs.Bool("json", false, "write JSON")
	if err := fs.Parse(reorderFlags(args, map[string]bool{"json": true}, map[string]bool{"format": true, "path": true, "value": true, "packet-kind": true, "label": true, "o": true})); err != nil {
		return err
	}
	if fs.NArg() != 1 {
		return errors.New("xfa dataset-set requires one input file")
	}
	if strings.ToLower(*format) != "pdf" {
		return fmt.Errorf("xfa dataset-set is unsupported for format %q", *format)
	}
	if strings.TrimSpace(*path) == "" {
		return errors.New("xfa dataset-set requires --path")
	}
	if *outputPath == "" {
		return errors.New("xfa dataset-set requires -o")
	}
	input, err := os.ReadFile(fs.Arg(0))
	if err != nil {
		return err
	}
	output, report, verification, err := pdf.ApplyXFADatasetFieldUpdateWithOptions(input, *path, *value, pdf.XFADatasetFieldUpdateOptions{
		Selector: pdf.XFASelector{PacketKind: *packetKind, Label: *label},
	})
	if err != nil {
		return err
	}
	return writeSemanticEditResult(output, report, verification, *outputPath, *asJSON)
}

func xfaReplace(args []string) error {
	fs := flag.NewFlagSet("xfa replace", flag.ContinueOnError)
	format := fs.String("format", "pdf", "input format")
	text := fs.String("text", "", "old XFA text")
	replace := fs.String("replace", "", "replacement XFA text")
	packetKind := fs.String("packet-kind", "", "restrict replacement to a conservative XFA packet kind")
	label := fs.String("label", "", "restrict replacement to an XFA array label")
	matchIndex := optionalIntFlag{}
	fs.Var(&matchIndex, "match-index", "zero-based XFA match index")
	outputPath := fs.String("o", "", "output file")
	asJSON := fs.Bool("json", false, "write JSON")
	if err := fs.Parse(reorderFlags(args, map[string]bool{"json": true}, map[string]bool{"format": true, "text": true, "replace": true, "packet-kind": true, "label": true, "match-index": true, "o": true})); err != nil {
		return err
	}
	if fs.NArg() != 1 {
		return errors.New("xfa replace requires one input file")
	}
	if strings.ToLower(*format) != "pdf" {
		return fmt.Errorf("xfa replace is unsupported for format %q", *format)
	}
	if *text == "" || *replace == "" {
		return errors.New("xfa replace requires --text and --replace")
	}
	if *outputPath == "" {
		return errors.New("xfa replace requires -o")
	}
	input, err := os.ReadFile(fs.Arg(0))
	if err != nil {
		return err
	}
	var selected *int
	if matchIndex.set {
		selected = &matchIndex.value
	}
	output, report, verification, err := pdf.ApplyXFAReplaceWithOptions(input, *text, *replace, pdf.XFAReplaceOptions{
		MatchIndex: selected,
		Selector:   pdf.XFASelector{PacketKind: *packetKind, Label: *label},
	})
	if err != nil {
		return err
	}
	return writeSemanticEditResult(output, report, verification, *outputPath, *asJSON)
}

func writeSemanticEditResult(output []byte, report core.Report, verification core.Verification, outputPath string, asJSON bool) error {
	if err := os.WriteFile(outputPath, output, 0644); err != nil {
		return err
	}
	report.OutputPath = outputPath
	report.Verification = &verification
	result := map[string]any{"report": report, "verification": verification}
	if asJSON {
		return writeJSON(result)
	}
	fmt.Println("edited", strconv.Quote(outputPath))
	return nil
}

type validationResult struct {
	Format        string              `json:"format"`
	Valid         bool                `json:"valid"`
	Errors        []string            `json:"errors"`
	Warnings      []string            `json:"warnings"`
	Root          any                 `json:"root,omitempty"`
	Streams       []core.Node         `json:"streams,omitempty"`
	StreamFilters *streamFilterReport `json:"stream_filters,omitempty"`
}

type streamFilterReport struct {
	Total              int `json:"total"`
	EditableTargets    int `json:"editable_targets"`
	PassThroughStreams int `json:"pass_through_streams"`
	UnsupportedTargets int `json:"unsupported_targets"`
}

type optionalIntFlag struct {
	value int
	set   bool
}

func (f *optionalIntFlag) Set(raw string) error {
	value, err := strconv.Atoi(raw)
	if err != nil {
		return fmt.Errorf("invalid integer %q", raw)
	}
	f.value = value
	f.set = true
	return nil
}

func (f *optionalIntFlag) String() string {
	if f == nil || !f.set {
		return ""
	}
	return strconv.Itoa(f.value)
}

type optionalStringFlag struct {
	value string
	set   bool
}

func (f *optionalStringFlag) Set(raw string) error {
	f.value = raw
	f.set = true
	return nil
}

func (f *optionalStringFlag) String() string {
	if f == nil || !f.set {
		return ""
	}
	return f.value
}

type metaFlag struct {
	values map[string]any
}

func (f *metaFlag) Set(raw string) error {
	key, value, ok := strings.Cut(raw, "=")
	if !ok || key == "" {
		return fmt.Errorf("invalid --meta %q: expected key=value", raw)
	}
	if f.values == nil {
		f.values = make(map[string]any)
	}
	f.values[key] = value
	return nil
}

func (f *metaFlag) String() string {
	if f == nil || len(f.values) == 0 {
		return ""
	}
	parts := make([]string, 0, len(f.values))
	for key, value := range f.values {
		parts = append(parts, fmt.Sprintf("%s=%v", key, value))
	}
	return strings.Join(parts, ",")
}

func validate(args []string) error {
	fs := flag.NewFlagSet("validate", flag.ContinueOnError)
	format := fs.String("format", "pdf", "input format")
	password := fs.String("password", "", "PDF password for explicit encrypted-PDF parsing")
	asJSON := fs.Bool("json", false, "write JSON")
	failOnInvalid := fs.Bool("fail-on-invalid", false, "return an error when validation fails")
	if err := fs.Parse(reorderFlags(args, map[string]bool{"json": true, "fail-on-invalid": true}, map[string]bool{"format": true, "password": true})); err != nil {
		return err
	}
	if fs.NArg() != 1 {
		return errors.New("validate requires one input file")
	}
	adapter, err := adapterFor(*format)
	if err != nil {
		return err
	}
	input, err := os.ReadFile(fs.Arg(0))
	if err != nil {
		return err
	}
	result := validationResult{
		Format:   strings.ToLower(*format),
		Valid:    true,
		Errors:   []string{},
		Warnings: []string{},
	}
	tree, err := parseInputWithPassword(adapter, input, *format, core.ParseOptions{Strict: true}, *password)
	if tree != nil {
		if tree.Format != "" {
			result.Format = tree.Format
		}
		if root, ok := tree.Node(tree.Root); ok {
			result.Root = root.Value
		}
		result.Streams = streamNodes(tree)
		result.StreamFilters = streamFilterReportFor(result.Streams)
		result.Warnings = append(result.Warnings, validationWarnings(tree)...)
	}
	if strings.ToLower(*format) == "pdf" {
		result.Root = rootWithSecurityMetadata(result.Root, input)
	}
	if err != nil {
		result.Valid = false
		result.Errors = append(result.Errors, err.Error())
	}
	if *asJSON {
		if err := writeJSON(result); err != nil {
			return err
		}
		if *failOnInvalid && !result.Valid {
			return validationFailureError(result)
		}
		return nil
	}
	if result.Valid {
		fmt.Printf("%s valid=true\n", result.Format)
		return nil
	}
	fmt.Printf("%s valid=false errors=%s\n", result.Format, strings.Join(result.Errors, "; "))
	if *failOnInvalid {
		return validationFailureError(result)
	}
	return nil
}

func validationWarnings(tree *core.Tree) []string {
	if tree == nil {
		return nil
	}
	root, ok := tree.Node(tree.Root)
	if !ok {
		return nil
	}
	value, ok := root.Value.(map[string]any)
	if !ok {
		return nil
	}
	boundaries, ok := value["boundaries"].(map[string]any)
	if !ok {
		return nil
	}
	warnings := make([]string, 0, 4)
	if boolBoundary(boundaries, "has_acroform") {
		warnings = append(warnings, "PDF boundary detected: AcroForm field values require form set; simple text/choice widget appearances require --regenerate-appearance")
	}
	if boolBoundary(boundaries, "has_annotations") {
		warnings = append(warnings, "PDF boundary detected: annotation /Contents updates require annot set-contents; simple text-like appearances require --regenerate-appearance")
	}
	if boolBoundary(boundaries, "has_font_markers") || boolBoundary(boundaries, "has_cmap_markers") || boolBoundary(boundaries, "has_tounicode_cmap") || boolBoundary(boundaries, "has_cid_font_markers") {
		warnings = append(warnings, "PDF boundary detected: font/CMap support is limited to page font-scoped ToUnicode CMaps for simple Tf flows, CMap-backed TJ arrays, and one unambiguous fallback; glyph metrics and layout are not verified")
	}
	return warnings
}

func boolBoundary(boundaries map[string]any, key string) bool {
	value, ok := boundaries[key]
	if !ok {
		return false
	}
	got, ok := value.(bool)
	return ok && got
}

func validationFailureError(result validationResult) error {
	if len(result.Errors) == 0 {
		return errors.New("validation failed")
	}
	return fmt.Errorf("validation failed: %s", strings.Join(result.Errors, "; "))
}

func parseInputWithPassword(adapter core.Adapter, input []byte, format string, opts core.ParseOptions, password string) (*core.Tree, error) {
	if strings.ToLower(format) == "pdf" && password == "" {
		if err := pdf.CheckSecurity(input, pdf.SecurityOptions{}); err != nil {
			return nil, err
		}
	}
	if strings.ToLower(format) == "pdf" && password != "" {
		tree, err := pdf.ParseWithPassword(input, opts, password)
		if err != nil && errors.Is(err, pdf.ErrEncryptedPDFUnsupportedAlgorithm) {
			if securityErr := pdf.CheckSecurity(input, pdf.SecurityOptions{Password: password}); securityErr != nil {
				return tree, securityErr
			}
		}
		return tree, err
	}
	return adapter.Parse(input, opts)
}

func rootWithSecurityMetadata(root any, input []byte) any {
	security := pdf.SecurityMetadataForInput(input)
	if !security.HasSecurityBoundary() {
		return root
	}
	value, _ := root.(map[string]any)
	if value == nil {
		value = make(map[string]any)
	} else {
		copyValue := make(map[string]any, len(value)+1)
		for key, item := range value {
			copyValue[key] = item
		}
		value = copyValue
	}
	value["security"] = security
	return value
}

func inspect(args []string) error {
	fs := flag.NewFlagSet("inspect", flag.ContinueOnError)
	format := fs.String("format", "pdf", "input format")
	password := fs.String("password", "", "PDF password for explicit encrypted-PDF parsing")
	asJSON := fs.Bool("json", false, "write JSON")
	if err := fs.Parse(reorderFlags(args, map[string]bool{"json": true}, map[string]bool{"format": true, "password": true})); err != nil {
		return err
	}
	if fs.NArg() != 1 {
		return errors.New("inspect requires one input file")
	}
	adapter, err := adapterFor(*format)
	if err != nil {
		return err
	}
	input, err := os.ReadFile(fs.Arg(0))
	if err != nil {
		return err
	}
	tree, parseErr := parseInputWithPassword(adapter, input, *format, core.ParseOptions{Strict: true}, *password)
	if parseErr != nil && tree == nil {
		if *asJSON && strings.ToLower(*format) == "pdf" {
			result := map[string]any{
				"format":      "pdf",
				"nodes":       0,
				"root":        rootWithSecurityMetadata(nil, input),
				"parse_error": parseErr.Error(),
			}
			return writeJSON(result)
		}
		return parseErr
	}
	if tree == nil {
		return errors.New("inspect produced no parse tree")
	}
	root, _ := tree.Node(tree.Root)
	streams := streamNodes(tree)
	result := map[string]any{
		"format": tree.Format,
		"nodes":  len(tree.Nodes),
		"root":   rootWithSecurityMetadata(root.Value, input),
	}
	if len(streams) > 0 {
		result["streams"] = streams
		result["stream_filters"] = streamFilterReportFor(streams)
	}
	if parseErr != nil {
		result["parse_error"] = parseErr.Error()
	}
	if *asJSON {
		return writeJSON(result)
	}
	fmt.Printf("%s nodes=%d\n", tree.Format, len(tree.Nodes))
	return nil
}

func query(args []string) error {
	fs := flag.NewFlagSet("query", flag.ContinueOnError)
	format := fs.String("format", "pdf", "input format")
	kind := fs.String("kind", pdf.KindTextShow, "node kind")
	text := fs.String("text", "", "node text")
	password := fs.String("password", "", "PDF password for explicit encrypted-PDF parsing")
	matchIndex := optionalIntFlag{}
	fs.Var(&matchIndex, "match-index", "zero-based match index")
	meta := metaFlag{}
	fs.Var(&meta, "meta", "exact metadata filter as key=value (repeatable)")
	asJSON := fs.Bool("json", false, "write JSON")
	if err := fs.Parse(reorderFlags(args, map[string]bool{"json": true}, map[string]bool{"format": true, "kind": true, "text": true, "password": true, "match-index": true, "meta": true})); err != nil {
		return err
	}
	if fs.NArg() != 1 {
		return errors.New("query requires one input file")
	}
	adapter, err := adapterFor(*format)
	if err != nil {
		return err
	}
	input, err := os.ReadFile(fs.Arg(0))
	if err != nil {
		return err
	}
	tree, err := parseInputWithPassword(adapter, input, *format, core.ParseOptions{Strict: true}, *password)
	if err != nil {
		return err
	}
	selector := core.Match{Kind: *kind, Text: *text, Meta: meta.values}
	allMatches := tree.QueryAll(selector)
	matches, selectedIndex, err := selectMatchIndex(allMatches, matchIndex)
	if err != nil {
		return err
	}
	if *asJSON {
		result := map[string]any{"matches": matches, "count": len(matches)}
		if selectedIndex != nil {
			result["match_index"] = *selectedIndex
			result["total_matches"] = len(allMatches)
		}
		return writeJSON(result)
	}
	for _, match := range matches {
		fmt.Printf("%d %s %d:%d %v\n", match.ID, match.Kind, match.Span.Start, match.Span.End, match.Value)
	}
	return nil
}

func streamNodes(tree *core.Tree) []core.Node {
	if tree == nil {
		return nil
	}
	streams := make([]core.Node, 0)
	for _, node := range tree.Nodes {
		if node.Kind == pdf.KindStream {
			streams = append(streams, node)
		}
	}
	return streams
}

func streamFilterReportFor(streams []core.Node) *streamFilterReport {
	if len(streams) == 0 {
		return nil
	}
	report := streamFilterReport{Total: len(streams)}
	for _, stream := range streams {
		if stream.Meta["unsupported"] != nil && metaBool(stream.Meta, "filter_target") {
			report.UnsupportedTargets++
			continue
		}
		if metaBool(stream.Meta, "filter_editable") && metaBool(stream.Meta, "filter_target") {
			report.EditableTargets++
		}
		if metaBool(stream.Meta, "filter_pass_through") {
			report.PassThroughStreams++
		}
		if !metaBool(stream.Meta, "filter_editable") && metaBool(stream.Meta, "filter_target") {
			report.UnsupportedTargets++
		}
	}
	return &report
}

func metaBool(meta map[string]any, key string) bool {
	value, ok := meta[key]
	if !ok {
		return false
	}
	got, ok := value.(bool)
	return ok && got
}

func editPlanErrorWithStreamBoundaries(err error, tree *core.Tree) error {
	unsupported := unsupportedTargetStreamNodes(tree)
	if err == nil || len(unsupported) == 0 {
		return err
	}
	reasons := make([]string, 0, len(unsupported))
	for _, stream := range unsupported {
		if reason, _ := stream.Meta["unsupported"].(string); reason != "" {
			reasons = append(reasons, reason)
			continue
		}
		if capability, _ := stream.Meta["filter_capability"].(string); capability != "" {
			reasons = append(reasons, capability)
		}
	}
	if len(reasons) == 0 {
		return err
	}
	return fmt.Errorf("%w; unsupported stream filter targets present: %s", err, strings.Join(reasons, "; "))
}

func writeEditErrorJSON(err error, tree *core.Tree) error {
	unsupported := unsupportedTargetStreamNodes(tree)
	if len(unsupported) == 0 {
		return nil
	}
	return writeJSON(map[string]any{
		"error":               err.Error(),
		"edit_status":         "unsupported",
		"fallback_used":       false,
		"unsupported_streams": unsupported,
		"stream_filters":      streamFilterReportFor(streamNodes(tree)),
	})
}

func unsupportedTargetStreamNodes(tree *core.Tree) []core.Node {
	streams := streamNodes(tree)
	unsupported := make([]core.Node, 0)
	for _, stream := range streams {
		if metaBool(stream.Meta, "filter_target") && (stream.Meta["unsupported"] != nil || !metaBool(stream.Meta, "filter_editable")) {
			unsupported = append(unsupported, stream)
		}
	}
	return unsupported
}

func edit(args []string) error {
	fs := flag.NewFlagSet("edit", flag.ContinueOnError)
	format := fs.String("format", "pdf", "input format")
	kind := fs.String("kind", pdf.KindTextShow, "node kind")
	text := fs.String("text", "", "node text")
	replace := fs.String("replace", "", "replacement text")
	rewrite := fs.String("rewrite", "auto", "rewrite mode: auto, surgical, canonical, preserve-structure")
	layoutModeFlag := fs.String("layout-mode", "allow-width-change", "layout mode: preserve-width, allow-width-change, reflow-line")
	password := fs.String("password", "", "PDF password for explicit encrypted-PDF canonical rewrites")
	matchIndex := optionalIntFlag{}
	legacyIndex := optionalIntFlag{}
	fs.Var(&matchIndex, "match-index", "zero-based match index")
	fs.Var(&legacyIndex, "index", "deprecated alias for --match-index")
	verify := fs.String("verify", "reparse,old-gone,new-selectable", "comma-separated invariants")
	allowSignatureInvalidation := fs.Bool("allow-signature-invalidation", false, "allow canonical rewrites that invalidate digital signatures")
	signatureModeFlag := optionalStringFlag{}
	fs.Var(&signatureModeFlag, "signature-mode", "digital signature mode: refuse, invalidate, preserve-incremental")
	outputPath := fs.String("o", "", "output file")
	asJSON := fs.Bool("json", false, "write JSON")
	if err := fs.Parse(reorderFlags(args, map[string]bool{"json": true, "allow-signature-invalidation": true}, map[string]bool{"format": true, "kind": true, "text": true, "replace": true, "rewrite": true, "layout-mode": true, "password": true, "match-index": true, "index": true, "verify": true, "signature-mode": true, "o": true})); err != nil {
		return err
	}
	selectedFlag, err := coalesceMatchIndexFlags(matchIndex, legacyIndex)
	if err != nil {
		return err
	}
	signatureMode, err := coalesceSignatureMode(signatureModeFlag, *allowSignatureInvalidation)
	if err != nil {
		return err
	}
	if fs.NArg() != 1 {
		return errors.New("edit requires one input file")
	}
	if *text == "" || *replace == "" {
		return errors.New("edit requires --text and --replace")
	}
	if *outputPath == "" {
		return errors.New("edit requires -o")
	}
	rewriteMode, err := parseRewriteMode(*rewrite)
	if err != nil {
		return err
	}
	layoutMode, err := parseLayoutMode(*layoutModeFlag)
	if err != nil {
		return err
	}
	if err := enforceLayoutMode(layoutMode, nil); err != nil && layoutMode == layoutModeReflowLine {
		return err
	}
	if *allowSignatureInvalidation && rewriteMode == "surgical" {
		return errors.New("--allow-signature-invalidation requires --rewrite canonical or --rewrite auto selecting the canonical path")
	}
	if signatureMode == pdf.SignatureInvalidationInvalidate && rewriteMode == "surgical" {
		return errors.New("signature invalidation requires --rewrite canonical or --rewrite auto selecting the canonical path")
	}
	if signatureMode == pdf.SignatureInvalidationPreserveIncremental {
		if rewriteMode != "auto" {
			return errors.New("signature preservation requires --rewrite auto because it uses append-only incremental updates")
		}
		if *password != "" {
			return errors.New("signature-preserving incremental rewrite does not support encrypted PDFs")
		}
	}
	adapter, err := adapterFor(*format)
	if err != nil {
		return err
	}
	input, err := os.ReadFile(fs.Arg(0))
	if err != nil {
		return err
	}
	if signatureMode == pdf.SignatureInvalidationPreserveIncremental {
		return incrementalSignaturePreservingEdit(input, canonicalEditRequest{
			Format:        *format,
			Kind:          *kind,
			Text:          *text,
			Replace:       *replace,
			SelectedFlag:  selectedFlag,
			Verify:        *verify,
			LayoutMode:    layoutMode,
			SignatureMode: signatureMode,
			OutputPath:    *outputPath,
			WriteJSON:     *asJSON,
		})
	}
	if strings.ToLower(*format) == "pdf" && *password != "" {
		if rewriteMode == "surgical" {
			return errors.New("surgical rewrite does not support encrypted PDFs; use --rewrite auto or --rewrite canonical with --password")
		}
		if rewriteMode == "preserve-structure" {
			return errors.New("preserve-structure rewrite does not support encrypted PDFs; use --rewrite auto or --rewrite canonical with --password")
		}
		return canonicalEdit(input, canonicalEditRequest{
			Format:        *format,
			Kind:          *kind,
			Text:          *text,
			Replace:       *replace,
			Password:      *password,
			SelectedFlag:  selectedFlag,
			Verify:        *verify,
			LayoutMode:    layoutMode,
			SignatureMode: signatureMode,
			OutputPath:    *outputPath,
			WriteJSON:     *asJSON,
		})
	}
	if rewriteMode == "canonical" || rewriteMode == "preserve-structure" {
		return canonicalEdit(input, canonicalEditRequest{
			Format:        *format,
			Kind:          *kind,
			Text:          *text,
			Replace:       *replace,
			Password:      *password,
			SelectedFlag:  selectedFlag,
			Verify:        *verify,
			LayoutMode:    layoutMode,
			SignatureMode: signatureMode,
			WriterMode:    pdfWriterModeForRewrite(rewriteMode),
			OutputPath:    *outputPath,
			WriteJSON:     *asJSON,
		})
	}
	tree, err := adapter.Parse(input, core.ParseOptions{Strict: true})
	if err != nil {
		if rewriteMode == "auto" && strings.ToLower(*format) == "pdf" {
			return canonicalEdit(input, canonicalEditRequest{
				Format:        *format,
				Kind:          *kind,
				Text:          *text,
				Replace:       *replace,
				Password:      *password,
				SelectedFlag:  selectedFlag,
				Verify:        *verify,
				LayoutMode:    layoutMode,
				SignatureMode: signatureMode,
				OutputPath:    *outputPath,
				WriteJSON:     *asJSON,
			})
		}
		return err
	}
	if rewriteMode == "surgical" && pdf.NeedsCanonicalRewrite(tree) {
		return errors.New("surgical rewrite does not support PDFs with xref streams or object streams; use --rewrite auto or --rewrite canonical")
	}
	if rewriteMode == "auto" && pdf.NeedsCanonicalRewrite(tree) {
		return canonicalEdit(input, canonicalEditRequest{
			Format:        *format,
			Kind:          *kind,
			Text:          *text,
			Replace:       *replace,
			Password:      *password,
			SelectedFlag:  selectedFlag,
			Verify:        *verify,
			LayoutMode:    layoutMode,
			SignatureMode: signatureMode,
			OutputPath:    *outputPath,
			WriteJSON:     *asJSON,
		})
	}
	if *allowSignatureInvalidation {
		return errors.New("--allow-signature-invalidation requires --rewrite canonical or --rewrite auto selecting the canonical path")
	}
	if signatureMode == pdf.SignatureInvalidationInvalidate {
		return errors.New("signature invalidation requires --rewrite canonical or --rewrite auto selecting the canonical path")
	}
	selector := core.Match{Kind: *kind, Text: *text}
	allMatches := tree.QueryAll(selector)
	_, selectedIndex, err := selectMatchIndex(allMatches, selectedFlag)
	if err != nil {
		return err
	}
	if selectedIndex == nil && len(allMatches) > 1 {
		return fmt.Errorf("selector matched %d nodes; pass --match-index N (zero-based, 0..%d) to choose one", len(allMatches), len(allMatches)-1)
	}
	if selectedIndex != nil {
		selector.MatchIndex = selectedIndex
	}
	plan, err := adapter.PlanEdit(tree, selector, core.Mutation{Replace: *replace})
	if err != nil {
		err = editPlanErrorWithStreamBoundaries(err, tree)
		if *asJSON {
			if jsonErr := writeEditErrorJSON(err, tree); jsonErr != nil {
				return jsonErr
			}
		}
		return err
	}
	if err := enforceLayoutMode(layoutMode, plan.Meta); err != nil {
		return err
	}
	plan.Invariants = parseInvariants(*verify)
	output, report, err := adapter.Apply(input, plan)
	if err != nil {
		return err
	}
	verification, err := adapter.Verify(output, plan)
	if err != nil {
		return err
	}
	if err := verificationError(verification, plan.Invariants); err != nil {
		return err
	}
	if err := os.WriteFile(*outputPath, output, 0644); err != nil {
		return err
	}
	if selectedIndex != nil {
		report.MatchIndex = selectedIndex
	}
	addLayoutModeToReportMeta(&report, layoutMode)
	report.OutputPath = *outputPath
	report.Verification = &verification
	result := map[string]any{"layout_mode": layoutMode, "report": report, "verification": verification}
	if *asJSON {
		return writeJSON(result)
	}
	fmt.Println("edited", strconv.Quote(*outputPath))
	return nil
}

type canonicalEditRequest struct {
	Format        string
	Kind          string
	Text          string
	Replace       string
	Password      string
	SelectedFlag  optionalIntFlag
	Verify        string
	LayoutMode    layoutMode
	SignatureMode pdf.SignatureInvalidationMode
	WriterMode    pdf.PDFWriterMode
	OutputPath    string
	WriteJSON     bool
}

func parseRewriteMode(raw string) (string, error) {
	mode := strings.ToLower(strings.TrimSpace(raw))
	switch mode {
	case "auto", "surgical", "canonical", "preserve-structure":
		return mode, nil
	default:
		return "", fmt.Errorf("unsupported rewrite mode %q (expected auto, surgical, canonical, or preserve-structure)", raw)
	}
}

func pdfWriterModeForRewrite(rewriteMode string) pdf.PDFWriterMode {
	if rewriteMode == "preserve-structure" {
		return pdf.PDFWriterModePreserveStructure
	}
	return pdf.PDFWriterModeCanonical
}

type layoutMode string

const (
	layoutModePreserveWidth      layoutMode = "preserve-width"
	layoutModeAllowWidthChange   layoutMode = "allow-width-change"
	layoutModeReflowLine         layoutMode = "reflow-line"
	layoutProofStatusWidthProven            = "width_proven"
)

func parseLayoutMode(raw string) (layoutMode, error) {
	mode := layoutMode(strings.ToLower(strings.TrimSpace(raw)))
	switch mode {
	case layoutModePreserveWidth, layoutModeAllowWidthChange, layoutModeReflowLine:
		return mode, nil
	default:
		return "", fmt.Errorf("unsupported layout mode %q (expected preserve-width, allow-width-change, or reflow-line)", raw)
	}
}

func enforceLayoutMode(mode layoutMode, meta map[string]any) error {
	switch mode {
	case layoutModeAllowWidthChange:
		return nil
	case layoutModeReflowLine:
		return errors.New("layout mode reflow-line is not supported yet")
	case layoutModePreserveWidth:
		if meta != nil && fmt.Sprint(meta["layout_proof"]) == layoutProofStatusWidthProven {
			return nil
		}
		proof := "unknown"
		if meta != nil {
			if value, ok := meta["layout_proof"]; ok && fmt.Sprint(value) != "" {
				proof = fmt.Sprint(value)
			}
		}
		return fmt.Errorf("layout mode preserve-width refused: layout_proof=%s (expected width_proven)", proof)
	default:
		return fmt.Errorf("unsupported layout mode %q", mode)
	}
}

func addLayoutModeToReportMeta(report *core.Report, mode layoutMode) {
	if report.Meta == nil {
		report.Meta = map[string]any{}
	}
	report.Meta["layout_mode"] = string(mode)
}

func canonicalEdit(input []byte, req canonicalEditRequest) error {
	if strings.ToLower(req.Format) != "pdf" {
		return fmt.Errorf("canonical rewrite is unsupported for format %q", req.Format)
	}
	if req.WriterMode == pdf.PDFWriterModePreserveStructure {
		if req.Password != "" {
			return errors.New("preserve-structure rewrite does not support encrypted PDFs; use --rewrite auto or --rewrite canonical with --password")
		}
		if req.SignatureMode == pdf.SignatureInvalidationInvalidate {
			return errors.New("preserve-structure rewrite does not support explicit signature invalidation; use --rewrite canonical when invalidating signatures")
		}
	}
	invariants := parseInvariants(req.Verify)
	selector := core.Match{Kind: req.Kind, Text: req.Text}
	if req.SelectedFlag.set {
		index := req.SelectedFlag.value
		selector.MatchIndex = &index
	}
	apply := pdf.ApplyCanonicalEdit
	if req.SignatureMode == pdf.SignatureInvalidationInvalidate {
		apply = pdf.ApplyCanonicalEditInvalidatingSignatures
	}
	var (
		output       []byte
		report       core.Report
		verification core.Verification
		err          error
	)
	if req.Password != "" {
		if req.SignatureMode == pdf.SignatureInvalidationInvalidate {
			output, report, verification, err = pdf.ApplyCanonicalEditWithPasswordInvalidatingSignatures(input, req.Password, selector, core.Mutation{Replace: req.Replace}, invariants)
		} else {
			output, report, verification, err = pdf.ApplyCanonicalEditWithPassword(input, req.Password, selector, core.Mutation{Replace: req.Replace}, invariants)
		}
	} else {
		if req.WriterMode == pdf.PDFWriterModePreserveStructure {
			output, report, verification, err = pdf.ApplyCanonicalEditWithWriterMode(input, req.WriterMode, selector, core.Mutation{Replace: req.Replace}, invariants)
		} else {
			output, report, verification, err = apply(input, selector, core.Mutation{Replace: req.Replace}, invariants)
		}
	}
	if err != nil {
		return err
	}
	if err := enforceLayoutMode(req.LayoutMode, report.Meta); err != nil {
		return err
	}
	if err := verificationError(verification, invariants); err != nil {
		return err
	}
	if err := os.WriteFile(req.OutputPath, output, 0644); err != nil {
		return err
	}
	addLayoutModeToReportMeta(&report, req.LayoutMode)
	report.OutputPath = req.OutputPath
	report.Verification = &verification
	result := map[string]any{"layout_mode": req.LayoutMode, "report": report, "verification": verification}
	if req.SignatureMode == pdf.SignatureInvalidationInvalidate {
		result["signature_invalidation"] = "digital signatures invalidated; not preserved or re-signed"
	}
	if req.WriteJSON {
		return writeJSON(result)
	}
	fmt.Println("edited", strconv.Quote(req.OutputPath))
	return nil
}

func incrementalSignaturePreservingEdit(input []byte, req canonicalEditRequest) error {
	if strings.ToLower(req.Format) != "pdf" {
		return fmt.Errorf("signature-preserving incremental rewrite is unsupported for format %q", req.Format)
	}
	invariants := parseInvariants(req.Verify)
	selector := core.Match{Kind: req.Kind, Text: req.Text}
	if req.SelectedFlag.set {
		index := req.SelectedFlag.value
		selector.MatchIndex = &index
	}
	output, report, verification, signaturePreservation, err := pdf.ApplyIncrementalTextEditPreservingSignatures(input, selector, core.Mutation{Replace: req.Replace}, invariants)
	if err != nil {
		return err
	}
	if err := enforceLayoutMode(req.LayoutMode, report.Meta); err != nil {
		return err
	}
	if err := verificationError(verification, invariants); err != nil {
		return err
	}
	if !signaturePreservation.OriginalBytesPreserved || !signaturePreservation.SignedByteRangesUnchanged {
		return errors.New("signature-preserving incremental verification failed")
	}
	if err := os.WriteFile(req.OutputPath, output, 0644); err != nil {
		return err
	}
	addLayoutModeToReportMeta(&report, req.LayoutMode)
	report.OutputPath = req.OutputPath
	report.Verification = &verification
	result := map[string]any{
		"layout_mode":            req.LayoutMode,
		"report":                 report,
		"verification":           verification,
		"signature_preservation": signaturePreservation,
	}
	if req.WriteJSON {
		return writeJSON(result)
	}
	fmt.Println("edited", strconv.Quote(req.OutputPath))
	return nil
}

func coalesceMatchIndexFlags(matchIndex, legacyIndex optionalIntFlag) (optionalIntFlag, error) {
	if matchIndex.set && legacyIndex.set {
		return optionalIntFlag{}, errors.New("use only one of --match-index or --index")
	}
	if matchIndex.set {
		return matchIndex, nil
	}
	return legacyIndex, nil
}

func coalesceSignatureMode(flag optionalStringFlag, allowInvalidation bool) (pdf.SignatureInvalidationMode, error) {
	mode := pdf.SignatureInvalidationRefuse
	if flag.set {
		parsed, err := parseSignatureMode(flag.value)
		if err != nil {
			return "", err
		}
		mode = parsed
	}
	if allowInvalidation {
		if flag.set && mode != pdf.SignatureInvalidationInvalidate {
			return "", errors.New("use only one of --allow-signature-invalidation or --signature-mode")
		}
		mode = pdf.SignatureInvalidationInvalidate
	}
	return mode, nil
}

func parseSignatureMode(raw string) (pdf.SignatureInvalidationMode, error) {
	switch pdf.SignatureInvalidationMode(strings.ToLower(strings.TrimSpace(raw))) {
	case pdf.SignatureInvalidationRefuse:
		return pdf.SignatureInvalidationRefuse, nil
	case pdf.SignatureInvalidationInvalidate:
		return pdf.SignatureInvalidationInvalidate, nil
	case pdf.SignatureInvalidationPreserveIncremental:
		return pdf.SignatureInvalidationPreserveIncremental, nil
	default:
		return "", fmt.Errorf("unsupported signature mode %q (expected refuse, invalidate, or preserve-incremental)", raw)
	}
}

func selectMatchIndex(matches []core.Node, matchIndex optionalIntFlag) ([]core.Node, *int, error) {
	if !matchIndex.set {
		return matches, nil, nil
	}
	index := matchIndex.value
	if index < 0 {
		return nil, nil, errors.New("match index cannot be negative (zero-based)")
	}
	if index >= len(matches) {
		return nil, nil, fmt.Errorf("match index %d out of range for %d matches (zero-based)", index, len(matches))
	}
	return []core.Node{matches[index]}, &index, nil
}

func verificationError(verification core.Verification, invariants []core.Invariant) error {
	failed := make([]string, 0)
	for _, invariant := range invariants {
		switch invariant {
		case core.InvariantReparse:
			if !verification.ReparseOK {
				failed = append(failed, string(invariant))
			}
		case core.InvariantOldGone:
			if !verification.OldTextRemoved {
				failed = append(failed, string(invariant))
			}
		case core.InvariantNewSelectable:
			if !verification.NewSelectable {
				failed = append(failed, string(invariant))
			}
		case core.InvariantPageUnchanged:
			if !verification.PageUnchanged {
				failed = append(failed, string(invariant))
			}
		}
	}
	if len(failed) > 0 {
		return fmt.Errorf("verification failed for %s: %+v", strings.Join(failed, ","), verification)
	}
	return nil
}

func adapterFor(format string) (core.Adapter, error) {
	switch strings.ToLower(format) {
	case "pdf":
		return pdf.NewAdapter(), nil
	default:
		return nil, fmt.Errorf("unsupported format %q", format)
	}
}

func parseInvariants(raw string) []core.Invariant {
	if raw == "" {
		return nil
	}
	parts := strings.Split(raw, ",")
	out := make([]core.Invariant, 0, len(parts))
	for _, part := range parts {
		part = strings.TrimSpace(part)
		if part != "" {
			out = append(out, core.Invariant(part))
		}
	}
	return out
}

func writeJSON(v any) error {
	enc := json.NewEncoder(os.Stdout)
	enc.SetIndent("", "  ")
	return enc.Encode(v)
}

func isUnsupportedAppearanceError(err error) bool {
	if err == nil {
		return false
	}
	message := err.Error()
	return strings.HasPrefix(message, "unsupported AcroForm appearance regeneration") ||
		strings.HasPrefix(message, "cannot regenerate annotation appearance")
}

func writeUnsupportedAppearanceErrorJSON(err error) error {
	reason := err.Error()
	return writeJSON(map[string]any{
		"error":              reason,
		"appearance_status":  "unsupported",
		"unsupported_reason": reason,
	})
}

func reorderFlags(args []string, boolFlags, valueFlags map[string]bool) []string {
	flags := make([]string, 0, len(args))
	positionals := make([]string, 0, len(args))
	for i := 0; i < len(args); i++ {
		arg := args[i]
		if !strings.HasPrefix(arg, "-") || arg == "-" {
			positionals = append(positionals, arg)
			continue
		}
		flags = append(flags, arg)
		name := strings.TrimLeft(arg, "-")
		if eq := strings.IndexByte(name, '='); eq >= 0 {
			name = name[:eq]
		}
		if boolFlags[name] || strings.Contains(arg, "=") {
			continue
		}
		if valueFlags[name] && i+1 < len(args) {
			i++
			flags = append(flags, args[i])
		}
	}
	return append(flags, positionals...)
}
