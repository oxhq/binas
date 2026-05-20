package pdf

import "testing"

func TestParseDefaultAppearanceExtractsFontAndSize(t *testing.T) {
	appearance, ok := parseDefaultAppearance("/Helv 10 Tf")
	if !ok {
		t.Fatal("parseDefaultAppearance ok = false, want true")
	}
	if appearance.FontResourceName != "Helv" {
		t.Fatalf("font resource name = %q, want Helv", appearance.FontResourceName)
	}
	if appearance.FontSize != 10 {
		t.Fatalf("font size = %v, want 10", appearance.FontSize)
	}
}

func TestParseDefaultAppearanceExtractsGrayscaleFill(t *testing.T) {
	appearance, ok := parseDefaultAppearance("/Helv 10 Tf 0.25 g")
	if !ok {
		t.Fatal("parseDefaultAppearance ok = false, want true")
	}
	if appearance.FillGray == nil || *appearance.FillGray != 0.25 {
		t.Fatalf("fill gray = %v, want 0.25", appearance.FillGray)
	}
}

func TestParseDefaultAppearanceExtractsRGBFill(t *testing.T) {
	appearance, ok := parseDefaultAppearance("/F1 12 Tf 1 0 0 rg")
	if !ok {
		t.Fatal("parseDefaultAppearance ok = false, want true")
	}
	if appearance.FillRGB == nil || *appearance.FillRGB != [3]float64{1, 0, 0} {
		t.Fatalf("fill rgb = %v, want [1 0 0]", appearance.FillRGB)
	}
}

func TestParseDefaultAppearanceExtractsTextMatrix(t *testing.T) {
	appearance, ok := parseDefaultAppearance("/F1 12 Tf 1 0 0 1 4 7 Tm")
	if !ok {
		t.Fatal("parseDefaultAppearance ok = false, want true")
	}
	if appearance.TextMatrix == nil || *appearance.TextMatrix != [6]float64{1, 0, 0, 1, 4, 7} {
		t.Fatalf("text matrix = %v, want [1 0 0 1 4 7]", appearance.TextMatrix)
	}
}

func TestParseDefaultAppearanceToleratesExtraOperators(t *testing.T) {
	appearance, ok := parseDefaultAppearance("q 0.5 w /F1 12 Tf 1 0 0 rg Q")
	if !ok {
		t.Fatal("parseDefaultAppearance ok = false, want true")
	}
	if appearance.FontResourceName != "F1" || appearance.FontSize != 12 {
		t.Fatalf("font = %q %v, want F1 12", appearance.FontResourceName, appearance.FontSize)
	}
	if appearance.FillRGB == nil || *appearance.FillRGB != [3]float64{1, 0, 0} {
		t.Fatalf("fill rgb = %v, want [1 0 0]", appearance.FillRGB)
	}
}

func TestParseDefaultAppearanceFailsClosedForMalformedFontSize(t *testing.T) {
	if appearance, ok := parseDefaultAppearance("/Helv nope Tf 0 g"); ok {
		t.Fatalf("parseDefaultAppearance ok = true with appearance %+v, want false", appearance)
	}
}

func TestParseDefaultAppearanceFailsClosedForMalformedTextMatrix(t *testing.T) {
	if appearance, ok := parseDefaultAppearance("/Helv 10 Tf 1 0 0 nope 4 7 Tm"); ok {
		t.Fatalf("parseDefaultAppearance ok = true with appearance %+v, want false", appearance)
	}
}
