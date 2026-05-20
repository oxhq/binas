package pdf

import "testing"

func TestTextStateTracksFontLeadingAndPosition(t *testing.T) {
	tracker := newPDFTextStateTracker()

	if !tracker.Apply("BT") {
		t.Fatal("BT was not applied")
	}
	if !tracker.Apply("Tf", "/F1", "12.5") {
		t.Fatal("Tf was not applied")
	}
	if !tracker.Apply("TL", "14") {
		t.Fatal("TL was not applied")
	}
	if !tracker.Apply("Td", "10", "20") {
		t.Fatal("Td was not applied")
	}
	if !tracker.Apply("T*") {
		t.Fatal("T* was not applied")
	}

	got := tracker.Snapshot()
	if !got.InTextObject {
		t.Fatal("expected tracker to be inside a text object")
	}
	if got.FontName != "F1" || got.FontSize != 12.5 {
		t.Fatalf("font state = %q %.2f, want F1 12.5", got.FontName, got.FontSize)
	}
	if got.Leading != 14 {
		t.Fatalf("leading = %.2f, want 14", got.Leading)
	}
	if got.X != 10 || got.Y != 6 {
		t.Fatalf("position = %.2f %.2f, want 10 6", got.X, got.Y)
	}
}

func TestTextStateTDUpdatesLeadingAndMovesPosition(t *testing.T) {
	tracker := newPDFTextStateTracker()
	tracker.Apply("BT")

	if !tracker.Apply("TD", "3", "-18") {
		t.Fatal("TD was not applied")
	}
	got := tracker.Snapshot()
	if got.Leading != 18 {
		t.Fatalf("leading = %.2f, want 18", got.Leading)
	}
	if got.X != 3 || got.Y != -18 {
		t.Fatalf("position after TD = %.2f %.2f, want 3 -18", got.X, got.Y)
	}

	if !tracker.Apply("T*") {
		t.Fatal("T* was not applied")
	}
	got = tracker.Snapshot()
	if got.X != 3 || got.Y != -36 {
		t.Fatalf("position after T* = %.2f %.2f, want 3 -36", got.X, got.Y)
	}
}

func TestTextStateResetsAtBTAndET(t *testing.T) {
	tracker := newPDFTextStateTracker()
	tracker.Apply("BT")
	tracker.Apply("Tf", "/F2", "9")
	tracker.Apply("TL", "10")
	tracker.Apply("Td", "5", "6")

	if !tracker.Apply("ET") {
		t.Fatal("ET was not applied")
	}
	got := tracker.Snapshot()
	if got != (pdfTextStateSnapshot{}) {
		t.Fatalf("state after ET = %+v, want zero snapshot", got)
	}

	tracker.Apply("BT")
	got = tracker.Snapshot()
	want := pdfTextStateSnapshot{InTextObject: true}
	if got != want {
		t.Fatalf("state after fresh BT = %+v, want %+v", got, want)
	}
}

func TestTextStateTracksSpacingScalingRiseRenderingAndMatrix(t *testing.T) {
	tracker := newPDFTextStateTracker()
	tracker.Apply("BT")

	tests := []struct {
		name     string
		operator string
		operands []string
		assert   func(t *testing.T, got pdfTextStateSnapshot)
	}{
		{
			name:     "character spacing",
			operator: "Tc",
			operands: []string{"1.5"},
			assert: func(t *testing.T, got pdfTextStateSnapshot) {
				t.Helper()
				if got.CharSpacing != 1.5 {
					t.Fatalf("char spacing = %.2f, want 1.5", got.CharSpacing)
				}
			},
		},
		{
			name:     "word spacing",
			operator: "Tw",
			operands: []string{"2.25"},
			assert: func(t *testing.T, got pdfTextStateSnapshot) {
				t.Helper()
				if got.WordSpacing != 2.25 {
					t.Fatalf("word spacing = %.2f, want 2.25", got.WordSpacing)
				}
			},
		},
		{
			name:     "horizontal scaling",
			operator: "Tz",
			operands: []string{"80"},
			assert: func(t *testing.T, got pdfTextStateSnapshot) {
				t.Helper()
				if got.HorizontalScaling != 80 {
					t.Fatalf("horizontal scaling = %.2f, want 80", got.HorizontalScaling)
				}
			},
		},
		{
			name:     "text rise",
			operator: "Ts",
			operands: []string{"-3.5"},
			assert: func(t *testing.T, got pdfTextStateSnapshot) {
				t.Helper()
				if got.TextRise != -3.5 {
					t.Fatalf("text rise = %.2f, want -3.5", got.TextRise)
				}
			},
		},
		{
			name:     "rendering mode",
			operator: "Tr",
			operands: []string{"2"},
			assert: func(t *testing.T, got pdfTextStateSnapshot) {
				t.Helper()
				if got.RenderingMode != 2 {
					t.Fatalf("rendering mode = %d, want 2", got.RenderingMode)
				}
			},
		},
		{
			name:     "text matrix",
			operator: "Tm",
			operands: []string{"1", "0", "0.25", "1", "72", "144"},
			assert: func(t *testing.T, got pdfTextStateSnapshot) {
				t.Helper()
				wantMatrix := [6]float64{1, 0, 0.25, 1, 72, 144}
				if got.TextMatrix != wantMatrix {
					t.Fatalf("text matrix = %+v, want %+v", got.TextMatrix, wantMatrix)
				}
				if got.X != 72 || got.Y != 144 {
					t.Fatalf("position after Tm = %.2f %.2f, want 72 144", got.X, got.Y)
				}
			},
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			if !tracker.Apply(tt.operator, tt.operands...) {
				t.Fatalf("%s was not applied", tt.operator)
			}
			tt.assert(t, tracker.Snapshot())
		})
	}
}

func TestTextStateRejectsMalformedNewOperatorOperandsWithoutMutation(t *testing.T) {
	tests := []struct {
		name     string
		operator string
		before   func(*pdfTextStateTracker)
		operands []string
	}{
		{
			name:     "character spacing",
			operator: "Tc",
			before: func(tracker *pdfTextStateTracker) {
				tracker.Apply("Tc", "1")
			},
			operands: []string{"bad"},
		},
		{
			name:     "word spacing",
			operator: "Tw",
			before: func(tracker *pdfTextStateTracker) {
				tracker.Apply("Tw", "2")
			},
			operands: []string{"bad"},
		},
		{
			name:     "horizontal scaling",
			operator: "Tz",
			before: func(tracker *pdfTextStateTracker) {
				tracker.Apply("Tz", "90")
			},
			operands: []string{"bad"},
		},
		{
			name:     "text rise",
			operator: "Ts",
			before: func(tracker *pdfTextStateTracker) {
				tracker.Apply("Ts", "4")
			},
			operands: []string{"bad"},
		},
		{
			name:     "rendering mode",
			operator: "Tr",
			before: func(tracker *pdfTextStateTracker) {
				tracker.Apply("Tr", "1")
			},
			operands: []string{"bad"},
		},
		{
			name:     "text matrix",
			operator: "Tm",
			before: func(tracker *pdfTextStateTracker) {
				tracker.Apply("Tm", "1", "0", "0", "1", "10", "20")
			},
			operands: []string{"1", "0", "bad", "1", "30", "40"},
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			tracker := newPDFTextStateTracker()
			tracker.Apply("BT")
			tt.before(tracker)
			before := tracker.Snapshot()

			if tracker.Apply(tt.operator, tt.operands...) {
				t.Fatalf("%s with malformed operands was applied", tt.operator)
			}
			if got := tracker.Snapshot(); got != before {
				t.Fatalf("state mutated after malformed %s: got %+v, want %+v", tt.operator, got, before)
			}
		})
	}
}
