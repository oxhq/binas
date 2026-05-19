package pdf

const (
	pdfFilterCCITTFaxDecode = "CCITTFaxDecode"
	pdfFilterDCTDecode      = "DCTDecode"
	pdfFilterJBIG2Decode    = "JBIG2Decode"
	pdfFilterJPXDecode      = "JPXDecode"
)

type pdfStreamFilterCapabilityClass string

const (
	pdfStreamFilterCapabilityIdentityPassThrough pdfStreamFilterCapabilityClass = "identity_pass_through"
	pdfStreamFilterCapabilityEditableReversible  pdfStreamFilterCapabilityClass = "editable_reversible"
	pdfStreamFilterCapabilityPassThroughImage    pdfStreamFilterCapabilityClass = "pass_through_image"
	pdfStreamFilterCapabilityUnsupportedTarget   pdfStreamFilterCapabilityClass = "unsupported_target"
)

type pdfStreamFilterCapability struct {
	Class       pdfStreamFilterCapabilityClass
	Chain       []string
	Editable    bool
	PassThrough bool
	Target      bool
}

func addPDFStreamFilterCapabilityMetadata(meta map[string]any, filter string) {
	if meta == nil {
		return
	}
	capability := classifyPDFStreamFilterCapability(filter)
	meta["filter_capability"] = string(capability.Class)
	meta["filter_editable"] = capability.Editable
	meta["filter_pass_through"] = capability.PassThrough
	meta["filter_target"] = capability.Target
}

func classifyPDFStreamFilterCapability(filter string) pdfStreamFilterCapability {
	chain := normalizePDFStreamFilterCapabilityChain(parsePDFStreamFilterChain(filter))
	if len(chain) == 0 {
		return pdfStreamFilterCapability{
			Class:       pdfStreamFilterCapabilityIdentityPassThrough,
			PassThrough: true,
		}
	}
	if pdfStreamFilterChainIsEditableReversible(chain) {
		return pdfStreamFilterCapability{
			Class:    pdfStreamFilterCapabilityEditableReversible,
			Chain:    chain,
			Editable: true,
			Target:   true,
		}
	}
	if pdfStreamFilterChainIsImagePassThrough(chain) {
		return pdfStreamFilterCapability{
			Class:       pdfStreamFilterCapabilityPassThroughImage,
			Chain:       chain,
			PassThrough: true,
		}
	}
	return pdfStreamFilterCapability{
		Class:  pdfStreamFilterCapabilityUnsupportedTarget,
		Chain:  chain,
		Target: true,
	}
}

func normalizePDFStreamFilterCapabilityChain(filters []string) []string {
	if len(filters) == 0 {
		return nil
	}
	out := make([]string, 0, len(filters))
	for _, filter := range filters {
		out = append(out, normalizePDFStreamFilterCapabilityName(filter))
	}
	return out
}

func normalizePDFStreamFilterCapabilityName(filter string) string {
	filter = normalizePDFStreamFilter(filter)
	switch filter {
	case "CCF":
		return pdfFilterCCITTFaxDecode
	case "DCT":
		return pdfFilterDCTDecode
	default:
		return filter
	}
}

func pdfStreamFilterChainIsEditableReversible(filters []string) bool {
	if len(filters) == 0 {
		return false
	}
	for _, filter := range filters {
		if !isEditableReversiblePDFStreamFilter(filter) {
			return false
		}
	}
	return true
}

func isEditableReversiblePDFStreamFilter(filter string) bool {
	switch filter {
	case pdfFilterASCII85Decode, pdfFilterASCIIHexDecode, pdfFilterFlateDecode, pdfFilterLZWDecode, pdfFilterRunLengthDecode:
		return true
	default:
		return false
	}
}

func pdfStreamFilterChainIsImagePassThrough(filters []string) bool {
	if len(filters) == 0 {
		return false
	}
	for _, filter := range filters {
		if !isImagePassThroughPDFStreamFilter(filter) {
			return false
		}
	}
	return true
}

func isImagePassThroughPDFStreamFilter(filter string) bool {
	switch filter {
	case pdfFilterCCITTFaxDecode, pdfFilterDCTDecode, pdfFilterJBIG2Decode, pdfFilterJPXDecode:
		return true
	default:
		return false
	}
}
