//! Read-only census of resources used by supported content operators.
//!
//! Every invocation is retained, including hidden content and inactive appearance
//! states. Completeness describes this traversal, not rendering, font-program
//! validation, future form input, conformance, or permission to prune objects.
//! Image CTMs/DPI are optional: Type3 glyph layout and pattern tiling are not
//! inferred. Clipping does not change the full-image sampling density reported.
//! Returned records reserve logical storage until the report is dropped; parser
//! scratch, shared source/store retention and allocator capacity are separate.
use std::collections::{BTreeMap, BTreeSet};

use binas_core::{OperationContext, Reservation, RevisionId, SnapshotId};

use crate::{
    PdfDocument, PdfError, PdfErrorCode, PdfReferenceTarget,
    content::operations::{ContentOperand, ContentOperation, OperationState, read_operations_part},
    object::{ObjectRef, Value},
    parser::{ParseBudget, decode_stream},
    store::ObjectStore,
};

type Matrix = [f64; 6];
const IDENTITY: Matrix = [1., 0., 0., 1., 0., 0.];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PdfResourceKind {
    Font,
    Image,
    Form,
    ExtGState,
    ColorSpace,
    Pattern,
    Shading,
    Properties,
    Appearance,
    Type3Glyph,
    SoftMask,
    InlineImage,
}

#[derive(Debug)]
pub struct PdfResourceUse {
    pub page_index: usize,
    pub page: PdfReferenceTarget,
    /// The content stream containing the operator, not a source-byte range.
    pub stream: PdfReferenceTarget,
    pub kind: PdfResourceKind,
    /// True for an image/inline-image placement or image mask. A soft-mask
    /// transparency group is a Form and has this field set to false.
    pub is_image: bool,
    pub target: Option<PdfReferenceTarget>,
    pub name: Vec<u8>,
    pub invocation_path: Vec<PdfReferenceTarget>,
    pub ctm: Option<Matrix>,
    /// Encoded PDF font codes, never guessed Unicode or font-program glyph IDs.
    pub encoded_text: Option<Vec<u8>>,
    /// Full-image axis sampling density in dots per inch; excludes clipping.
    pub image_dpi: Option<[f64; 2]>,
    pub conditional: bool,
    pub appearance: bool,
    pub decoded_offset: usize,
}

#[derive(Debug)]
pub struct PdfUsageGap {
    pub page_index: Option<usize>,
    pub stream: Option<PdfReferenceTarget>,
    pub code: &'static str,
    pub detail: String,
}

#[derive(Debug)]
pub struct PdfResourceUsage {
    snapshot: SnapshotId,
    coverage_complete: bool,
    pub page_indices: Vec<usize>,
    pub uses: Vec<PdfResourceUse>,
    pub gaps: Vec<PdfUsageGap>,
    /// Successfully lexed content streams. Indirect image payloads are not
    /// decoded; locating an inline image's end may require bounded decoding.
    pub scanned_streams: Vec<PdfReferenceTarget>,
    _store: ObjectStore,
    _reservations: Vec<Reservation>,
}

impl PdfResourceUsage {
    pub fn snapshot(&self) -> SnapshotId {
        self.snapshot
    }

    /// Whether all selected content paths had supported resource traversal.
    /// This is observational evidence, never a mutation/optimization authority.
    pub fn complete(&self) -> bool {
        self.coverage_complete
    }
}

impl PdfDocument {
    pub fn resource_usage(&self, context: &OperationContext) -> Result<PdfResourceUsage, PdfError> {
        census(
            self,
            SnapshotId::new(self.source_id(), RevisionId::INITIAL),
            None,
            context,
        )
    }

    /// Enumerates the page tree but decodes content only on the selected page
    /// and its recursively used Forms, glyphs, and annotation appearances.
    pub fn page_resource_usage(
        &self,
        page_index: usize,
        context: &OperationContext,
    ) -> Result<PdfResourceUsage, PdfError> {
        census(
            self,
            SnapshotId::new(self.source_id(), RevisionId::INITIAL),
            Some(page_index),
            context,
        )
    }
}

pub(crate) fn census(
    document: &PdfDocument,
    snapshot: SnapshotId,
    selected: Option<usize>,
    context: &OperationContext,
) -> Result<PdfResourceUsage, PdfError> {
    context.charge_work(1).map_err(PdfError::from_context)?;
    if snapshot.source() != document.source_id() {
        return Err(PdfError::selection(
            "resource census source does not match revision",
        ));
    }
    let document = document.with_cancellation(context.cancellation().clone());
    let pages = document.page_refs()?;
    if selected.is_some_and(|index| index >= pages.len()) {
        return Err(PdfError::selection(
            "resource census page index is out of range",
        ));
    }
    let mut walker = Walker {
        document: &document,
        context,
        budget: ParseBudget::for_document(document.parsed()),
        active: Vec::new(),
        seen_streams: BTreeSet::new(),
        visits: 0,
        report: PdfResourceUsage {
            snapshot,
            coverage_complete: false,
            page_indices: Vec::new(),
            uses: Vec::new(),
            gaps: Vec::new(),
            scanned_streams: Vec::new(),
            _store: document.parsed().objects.clone(),
            _reservations: Vec::new(),
        },
    };
    if dict_get(&document.parsed().trailer, b"Encrypt")
        .is_some_and(|value| !matches!(value, Value::Null))
    {
        walker.gap(
            None,
            "pdf.usage.encryption",
            "Encrypted content must be opened with its security context before resource analysis.",
        )?;
        return Ok(walker.report);
    }
    for (index, page) in pages.into_iter().enumerate() {
        walker.work(1)?;
        if selected.is_some_and(|selected| selected != index) {
            continue;
        }
        walker.reserve(std::mem::size_of::<usize>())?;
        walker.report.page_indices.push(index);
        walker.page(index, page)?;
    }
    walker.report.coverage_complete = walker.report.gaps.is_empty();
    context.checkpoint().map_err(PdfError::from_context)?;
    Ok(walker.report)
}

#[derive(Clone, Copy)]
struct Binding<'a> {
    value: &'a Value,
    target: Option<ObjectRef>,
}

#[derive(Clone)]
struct Font<'a> {
    binding: Binding<'a>,
    name: Vec<u8>,
    selection: Option<usize>,
    type3: bool,
}

#[derive(Clone)]
struct Graphics<'a> {
    ctm: Option<Matrix>,
    font: Option<Font<'a>>,
}
impl Default for Graphics<'_> {
    fn default() -> Self {
        Self {
            ctm: Some(IDENTITY),
            font: None,
        }
    }
}

#[derive(Clone)]
struct Frame {
    page_index: usize,
    page: ObjectRef,
    path: Vec<ObjectRef>,
    unit: Option<f64>,
    conditional: bool,
    appearance: bool,
}
#[derive(Clone, Copy)]
struct Site<'a> {
    frame: &'a Frame,
    stream: ObjectRef,
    offset: usize,
}

struct Walker<'a> {
    document: &'a PdfDocument,
    context: &'a OperationContext,
    budget: ParseBudget,
    active: Vec<ObjectRef>,
    seen_streams: BTreeSet<ObjectRef>,
    visits: usize,
    report: PdfResourceUsage,
}

impl<'a> Walker<'a> {
    fn work(&self, units: usize) -> Result<(), PdfError> {
        self.context
            .charge_work(units as u64)
            .map_err(PdfError::from_context)?;
        self.budget.charge_work(units)
    }
    fn reserve(&mut self, bytes: usize) -> Result<(), PdfError> {
        let bytes = bytes
            .checked_add(std::mem::size_of::<Reservation>())
            .ok_or_else(|| PdfError::limit("resource census record size overflows"))?;
        self.report._reservations.push(
            self.context
                .reserve(bytes as u64)
                .map_err(PdfError::from_context)?,
        );
        Ok(())
    }
    fn gap(
        &mut self,
        site: Option<Site<'_>>,
        code: &'static str,
        detail: &str,
    ) -> Result<(), PdfError> {
        self.work(1)?;
        if self.report.gaps.len() >= self.document.parsed().limits.max_container_items {
            return Err(PdfError::limit("resource census gaps exceed limit"));
        }
        self.reserve(std::mem::size_of::<PdfUsageGap>().saturating_add(detail.len()))?;
        self.report.gaps.push(PdfUsageGap {
            page_index: site.map(|s| s.frame.page_index),
            stream: site.map(|s| target(s.stream)),
            code,
            detail: detail.to_owned(),
        });
        Ok(())
    }
    fn recover(&mut self, site: Site<'_>, error: PdfError) -> Result<(), PdfError> {
        match error.code {
            PdfErrorCode::ResourceLimit | PdfErrorCode::Cancelled => Err(error),
            _ => self.gap(Some(site), "pdf.usage.unsupported_resource", &error.message),
        }
    }
    fn record(
        &mut self,
        site: Site<'_>,
        state: &Graphics<'_>,
        kind: PdfResourceKind,
        binding: Option<Binding<'_>>,
        name: &[u8],
        encoded: Option<&[u8]>,
    ) -> Result<usize, PdfError> {
        self.work(1)?;
        if self.report.uses.len() >= self.document.parsed().limits.max_container_items {
            return Err(PdfError::limit("resource census uses exceed limit"));
        }
        let bytes = std::mem::size_of::<PdfResourceUse>()
            .saturating_add(name.len())
            .saturating_add(encoded.map_or(0, <[u8]>::len))
            .saturating_add(
                site.frame
                    .path
                    .len()
                    .saturating_mul(std::mem::size_of::<PdfReferenceTarget>()),
            );
        self.reserve(bytes)?;
        let index = self.report.uses.len();
        self.report.uses.push(PdfResourceUse {
            page_index: site.frame.page_index,
            page: target(site.frame.page),
            stream: target(site.stream),
            kind,
            is_image: matches!(kind, PdfResourceKind::Image | PdfResourceKind::InlineImage),
            target: binding.and_then(|b| b.target).map(target),
            name: name.to_vec(),
            invocation_path: site.frame.path.iter().copied().map(target).collect(),
            ctm: state.ctm,
            encoded_text: encoded.map(<[u8]>::to_vec),
            image_dpi: None,
            conditional: site.frame.conditional,
            appearance: site.frame.appearance,
            decoded_offset: site.offset,
        });
        Ok(index)
    }
    fn resolve(&self, mut value: &'a Value) -> Result<&'a Value, PdfError> {
        let mut seen = BTreeSet::new();
        for _ in 0..=self.document.parsed().limits.max_parser_depth {
            self.work(1)?;
            let Value::Ref(reference) = value else {
                return Ok(value);
            };
            if !seen.insert(*reference) {
                return Err(PdfError::unsupported("cycle in resource reference chain"));
            }
            value = &self.document.parsed().object(*reference)?.value;
        }
        Err(PdfError::limit("resource reference depth exceeds limit"))
    }
    fn binding(&self, value: &'a Value) -> Result<Binding<'a>, PdfError> {
        let target = match value {
            Value::Ref(reference) => Some(*reference),
            _ => None,
        };
        Ok(Binding {
            value: self.resolve(value)?,
            target,
        })
    }
    fn named(
        &self,
        resources: Option<&'a Value>,
        category: &[u8],
        name: &[u8],
    ) -> Result<Binding<'a>, PdfError> {
        let resources = self.resolve(
            resources.ok_or_else(|| PdfError::unsupported("content has no resource dictionary"))?,
        )?;
        let entries = dict_get(resources, category)
            .ok_or_else(|| PdfError::unsupported("resource category is absent"))?;
        let entries = self.resolve(entries)?;
        let value = dict_get(entries, name)
            .ok_or_else(|| PdfError::unsupported("named content resource is absent"))?;
        let binding = self.binding(value)?;
        if matches!(binding.value, Value::Null) {
            return Err(PdfError::unsupported("named resource resolves to null"));
        }
        Ok(binding)
    }
    fn matrix(&self, value: Option<&'a Value>) -> Result<Matrix, PdfError> {
        let Some(value) = value else {
            return Ok(IDENTITY);
        };
        let Value::Array(values) = self.resolve(value)? else {
            return Err(PdfError::unsupported("resource matrix is not an array"));
        };
        if values.len() != 6 {
            return Err(PdfError::unsupported("resource matrix needs six numbers"));
        }
        let mut matrix = IDENTITY;
        for (index, value) in values.iter().enumerate() {
            matrix[index] = number(self.resolve(value)?)
                .ok_or_else(|| PdfError::unsupported("resource matrix has an invalid number"))?;
        }
        Ok(matrix)
    }
    fn page(&mut self, page_index: usize, page: ObjectRef) -> Result<(), PdfError> {
        let value = &self.document.parsed().object(page)?.value;
        let unit = match dict_get(value, b"UserUnit") {
            None => Some(1.),
            Some(value) => number(self.resolve(value)?).filter(|unit| *unit > 0.),
        };
        let frame = Frame {
            page_index,
            page,
            path: Vec::new(),
            unit,
            conditional: false,
            appearance: false,
        };
        let site = Site {
            frame: &frame,
            stream: page,
            offset: 0,
        };
        let resources = self.document.inherited_resources(page)?;
        if let Err(error) = self.group_color(site, resources, &Graphics::default(), value) {
            self.recover(site, error)?;
        }
        let refs = match self.document.page_content_refs(page) {
            Ok(refs) => refs,
            Err(error) => {
                self.recover(site, error)?;
                Vec::new()
            }
        };
        self.sequence(&refs, resources, &frame, Graphics::default())?;
        if let Some(annots) = dict_get(value, b"Annots") {
            let result = self.appearances(annots, resources, &frame);
            if let Err(error) = result {
                self.recover(site, error)?;
            }
        }
        Ok(())
    }
    fn sequence(
        &mut self,
        refs: &[ObjectRef],
        resources: Option<&'a Value>,
        frame: &Frame,
        mut state: Graphics<'a>,
    ) -> Result<(), PdfError> {
        let mut lexer = OperationState::new();
        let mut stack = Vec::new();
        let mut marked = Vec::new();
        let mut conditional = frame.conditional;
        let mut text = false;
        let mut compatibility = 0usize;
        // The initial paint color is DeviceGray. Its replacement context is not
        // tracked through every paint operator yet; never omit that dependency
        // while describing the name census as complete.
        if let Some(resources) = resources
            && let Some(spaces) = dict_get(self.resolve(resources)?, b"ColorSpace")
            && let Some(default) = dict_get(self.resolve(spaces)?, b"DefaultGray")
            && !matches!(self.resolve(default)?, Value::Null)
            && let Some(stream) = refs.first().copied()
        {
            let site = Site {
                frame,
                stream,
                offset: 0,
            };
            self.gap(Some(site), "pdf.usage.initial_color", "Implicit initial device-gray replacement across paint contexts is not fully interpreted; explicit selections are still recorded.")?;
        }
        for (part, reference) in refs.iter().copied().enumerate() {
            self.work(1)?;
            self.visits = self
                .visits
                .checked_add(1)
                .ok_or_else(|| PdfError::limit("resource stream visit count overflows"))?;
            if self.visits > self.document.parsed().limits.max_container_items {
                return Err(PdfError::limit("resource stream visits exceed limit"));
            }
            let site = Site {
                frame,
                stream: reference,
                offset: 0,
            };
            let object = self.document.parsed().object(reference)?;
            let Some(stream) = &object.stream else {
                self.gap(
                    Some(site),
                    "pdf.usage.missing_stream",
                    "Content reference is not a stream.",
                )?;
                return Ok(());
            };
            let decoded = match decode_stream(
                &object.value,
                stream,
                &self.document.parsed().limits,
                &mut self.budget,
            ) {
                Ok(decoded) => decoded,
                Err(error) if error.code == PdfErrorCode::UnsupportedFeature => {
                    self.recover(site, error)?;
                    return Ok(());
                }
                Err(error) => return Err(error),
            };
            self.work(decoded.len().saturating_add(1))?;
            let operations = match read_operations_part(
                &decoded,
                part,
                &mut lexer,
                &self.document.parsed().limits,
                &mut self.budget,
            ) {
                Ok(operations) => operations,
                Err(error) if error.code == PdfErrorCode::UnsupportedFeature => {
                    self.recover(site, error)?;
                    return Ok(());
                }
                Err(error) => return Err(error),
            };
            if self.seen_streams.insert(reference) {
                self.reserve(std::mem::size_of::<PdfReferenceTarget>())?;
                self.report.scanned_streams.push(target(reference));
            }
            for operation in operations {
                self.work(1)?;
                debug_assert_eq!(operation.part_index, part);
                let mut local_frame = frame.clone();
                local_frame.conditional = conditional;
                let site = Site {
                    frame: &local_frame,
                    stream: reference,
                    offset: operation.decoded_offset,
                };
                let operands = &operation.operands;
                let result = match operation.operator.as_slice() {
                    b"q" if operands.is_empty() => {
                        if stack.len() >= self.document.parsed().limits.max_parser_depth {
                            return Err(PdfError::limit(
                                "resource graphics state depth exceeds limit",
                            ));
                        }
                        stack.push(state.clone());
                        Ok(())
                    }
                    b"Q" if operands.is_empty() => match stack.pop() {
                        Some(saved) => {
                            state = saved;
                            Ok(())
                        }
                        None => Err(PdfError::unsupported("Q has no matching q")),
                    },
                    b"cm" => {
                        if let Some(matrix) = operand_matrix(operands) {
                            state.ctm = state.ctm.and_then(|ctm| multiply(ctm, matrix));
                            Ok(())
                        } else {
                            Err(PdfError::unsupported("cm requires six finite numbers"))
                        }
                    }
                    b"Tf" => self.select_font(site, resources, &mut state, operands),
                    b"Tj" | b"TJ" | b"'" | b"\"" => {
                        if text {
                            self.show_text(site, &mut state, &operation)
                        } else {
                            Err(PdfError::unsupported("text showing outside BT/ET"))
                        }
                    }
                    b"BT" if operands.is_empty() && !text => {
                        text = true;
                        Ok(())
                    }
                    b"ET" if operands.is_empty() && text => {
                        text = false;
                        Ok(())
                    }
                    b"Do" => self.xobject(site, resources, &state, operands),
                    b"gs" => self.ext_state(site, resources, &mut state, operands),
                    b"CS" | b"cs" => self.color_space(site, resources, &state, operands),
                    b"RG" | b"rg" if plain_operator(&operation.operator, operands) => {
                        self.color_name(site, resources, &state, b"DeviceRGB", 0, false)
                    }
                    b"G" | b"g" if plain_operator(&operation.operator, operands) => {
                        self.color_name(site, resources, &state, b"DeviceGray", 0, false)
                    }
                    b"K" | b"k" if plain_operator(&operation.operator, operands) => {
                        self.color_name(site, resources, &state, b"DeviceCMYK", 0, false)
                    }
                    b"SCN" | b"scn" => self.pattern(site, resources, &state, operands),
                    b"sh" => self.shading(site, resources, &state, operands),
                    b"BDC" | b"BMC" => {
                        if marked.len() >= self.document.parsed().limits.max_parser_depth {
                            return Err(PdfError::limit("marked content depth exceeds limit"));
                        }
                        marked.push(conditional);
                        match self.marked(site, resources, &state, &operation) {
                            Ok(is_conditional) => {
                                conditional |= is_conditional;
                                Ok(())
                            }
                            Err(error) => {
                                conditional = true;
                                Err(error)
                            }
                        }
                    }
                    b"EMC" if operands.is_empty() => match marked.pop() {
                        Some(saved) => {
                            conditional = saved;
                            Ok(())
                        }
                        None => Err(PdfError::unsupported(
                            "EMC has no matching marked-content start",
                        )),
                    },
                    b"DP" => self.marked(site, resources, &state, &operation).map(|_| ()),
                    b"MP" if matches!(operands.as_slice(), [ContentOperand::Name(_)]) => Ok(()),
                    b"BI" => {
                        self.inline_image(site, resources, &state, operation.inline_image.as_ref())
                    }
                    b"BX" if operands.is_empty() => {
                        compatibility += 1;
                        Ok(())
                    }
                    b"EX" if operands.is_empty() && compatibility > 0 => {
                        compatibility -= 1;
                        Ok(())
                    }
                    operator if plain_operator(operator, operands) => Ok(()),
                    _ => Err(PdfError::unsupported(
                        "unknown content operator or unsupported operands",
                    )),
                };
                if let Err(error) = result {
                    self.recover(site, error)?;
                    // An unknown extension can alter arbitrary graphics state.
                    // Continue discovering resources without trusting its CTM/font.
                    state.ctm = None;
                    state.font = None;
                }
            }
        }
        if let Some(reference) = refs.last().copied() {
            let site = Site {
                frame,
                stream: reference,
                offset: 0,
            };
            if let Err(error) = lexer.finish() {
                self.recover(site, error)?;
            }
            if !stack.is_empty() || !marked.is_empty() || text || compatibility != 0 {
                self.gap(Some(site), "pdf.usage.unbalanced_state", "Content ends with an unbalanced graphics, text, marked-content or compatibility scope.")?;
            }
        }
        Ok(())
    }
    fn inline_image(
        &mut self,
        site: Site<'_>,
        resources: Option<&'a Value>,
        state: &Graphics<'a>,
        image: Option<&crate::content::InlineImage>,
    ) -> Result<(), PdfError> {
        let image = image
            .ok_or_else(|| PdfError::unsupported("inline image boundaries are unavailable"))?;
        let name: &[u8] = match image.color_space {
            crate::content::InlineColorSpace::Gray => b"DeviceGray",
            crate::content::InlineColorSpace::Rgb => b"DeviceRGB",
            crate::content::InlineColorSpace::Cmyk => b"DeviceCMYK",
        };
        self.color_name(site, resources, state, name, 0, false)?;
        let index = self.record(site, state, PdfResourceKind::InlineImage, None, b"", None)?;
        self.report.uses[index].image_dpi = dpi(
            state.ctm,
            site.frame.unit,
            f64::from(image.width),
            f64::from(image.height),
        );
        Ok(())
    }
    fn select_font(
        &mut self,
        site: Site<'_>,
        resources: Option<&'a Value>,
        state: &mut Graphics<'a>,
        operands: &[ContentOperand],
    ) -> Result<(), PdfError> {
        let [ContentOperand::Name(name), ContentOperand::Number(_)] = operands else {
            return Err(PdfError::unsupported("Tf requires font name and size"));
        };
        let binding = self.named(resources, b"Font", name)?;
        if !matches!(binding.value, Value::Dict(_)) {
            return Err(PdfError::unsupported("font resource is not a dictionary"));
        }
        let type3 = self.font_is_type3(binding.value)?;
        let selection = self.record(
            site,
            state,
            PdfResourceKind::Font,
            Some(binding),
            name,
            None,
        )?;
        state.font = Some(Font {
            binding,
            name: name.clone(),
            selection: Some(selection),
            type3,
        });
        Ok(())
    }
    fn font_is_type3(&self, font: &'a Value) -> Result<bool, PdfError> {
        let subtype = dict_get(font, b"Subtype")
            .ok_or_else(|| PdfError::unsupported("font subtype is absent"))?;
        match self.resolve(subtype)? {
            Value::Name(name) if name == b"Type3" => Ok(true),
            Value::Name(name)
                if matches!(
                    name.as_slice(),
                    b"Type0" | b"Type1" | b"MMType1" | b"TrueType"
                ) =>
            {
                Ok(false)
            }
            _ => Err(PdfError::unsupported(
                "font subtype is unsupported for resource traversal",
            )),
        }
    }
    fn show_text(
        &mut self,
        site: Site<'_>,
        state: &mut Graphics<'a>,
        operation: &ContentOperation,
    ) -> Result<(), PdfError> {
        let items: &[ContentOperand] =
            match (operation.operator.as_slice(), operation.operands.as_slice()) {
                (b"TJ", [ContentOperand::Array(items)]) => items,
                (b"Tj" | b"'", items @ [ContentOperand::Bytes { .. }]) => items,
                (
                    b"\"",
                    [
                        ContentOperand::Number(_),
                        ContentOperand::Number(_),
                        item @ ContentOperand::Bytes { .. },
                    ],
                ) => std::slice::from_ref(item),
                _ => return Err(PdfError::unsupported("text showing has invalid operands")),
            };
        let mut font = state
            .font
            .clone()
            .ok_or_else(|| PdfError::unsupported("text font identity is unavailable"))?;
        for item in items {
            self.work(1)?;
            let bytes = match item {
                ContentOperand::Bytes { bytes, .. } => bytes.as_slice(),
                ContentOperand::Number(_) => continue,
                _ => return Err(PdfError::unsupported("TJ contains an unsupported item")),
            };
            self.work(bytes.len())?;
            if let Some(index) = font.selection.take().filter(|index| {
                let record = &self.report.uses[*index];
                record.encoded_text.is_none()
                    && record.invocation_path.iter().copied().eq(site
                        .frame
                        .path
                        .iter()
                        .copied()
                        .map(target))
                    && record.conditional == site.frame.conditional
                    && record.appearance == site.frame.appearance
            }) {
                self.reserve(bytes.len())?;
                let record = &mut self.report.uses[index];
                record.encoded_text = Some(bytes.to_vec());
                // The selection and first showing can occur in different parts.
                record.stream = target(site.stream);
                record.decoded_offset = site.offset;
                record.ctm = state.ctm;
            } else {
                self.record(
                    site,
                    state,
                    PdfResourceKind::Font,
                    Some(font.binding),
                    &font.name,
                    Some(bytes),
                )?;
            }
            if font.type3 {
                self.type3(site, state, font.binding, bytes)?;
            }
        }
        state.font = Some(font);
        Ok(())
    }
    fn xobject(
        &mut self,
        site: Site<'_>,
        resources: Option<&'a Value>,
        state: &Graphics<'a>,
        operands: &[ContentOperand],
    ) -> Result<(), PdfError> {
        let name = one_name(operands)?;
        let binding = self.named(resources, b"XObject", name)?;
        match dict_name(binding.value, b"Subtype") {
            Some(b"Image") => self.image(
                site,
                resources,
                state,
                binding,
                name,
                PdfResourceKind::Image,
            ),
            Some(b"Form") => {
                self.record(
                    site,
                    state,
                    PdfResourceKind::Form,
                    Some(binding),
                    name,
                    None,
                )?;
                self.form(site, resources, state.clone(), binding, false)
            }
            _ => Err(PdfError::unsupported(
                "XObject subtype is unsupported for resource traversal",
            )),
        }
    }
    fn enter(&mut self, reference: ObjectRef) -> Result<(), PdfError> {
        self.work(1)?;
        if self.active.contains(&reference) {
            return Err(PdfError::unsupported("cycle in resource invocation graph"));
        }
        if self.active.len() >= self.document.parsed().limits.max_parser_depth {
            return Err(PdfError::limit("resource invocation depth exceeds limit"));
        }
        self.active.push(reference);
        Ok(())
    }
    fn form(
        &mut self,
        site: Site<'_>,
        inherited: Option<&'a Value>,
        mut state: Graphics<'a>,
        binding: Binding<'a>,
        appearance: bool,
    ) -> Result<(), PdfError> {
        let reference = binding
            .target
            .ok_or_else(|| PdfError::unsupported("invoked stream has no indirect identity"))?;
        let matrix = self.matrix(dict_get(binding.value, b"Matrix"))?;
        if [b"Ref".as_slice(), b"OPI"]
            .iter()
            .any(|key| dict_get(binding.value, key).is_some_and(|v| !matches!(v, Value::Null)))
        {
            self.gap(Some(site), "pdf.usage.external_form", "External reference or OPI substitute content is not traversed; only the local Form is inspected.")?;
        }
        state.ctm = state.ctm.and_then(|ctm| multiply(ctm, matrix));
        let resources = match dict_get(binding.value, b"Resources") {
            None => inherited,
            Some(value) => match self.resolve(value)? {
                Value::Null => inherited,
                value @ Value::Dict(_) => Some(value),
                _ => return Err(PdfError::unsupported("Form Resources is not a dictionary")),
            },
        };
        let mut frame = site.frame.clone();
        self.group_color(site, resources, &state, binding.value)?;
        frame.path.push(reference);
        frame.appearance |= appearance;
        frame.conditional |=
            dict_get(binding.value, b"OC").is_some_and(|v| !matches!(v, Value::Null));
        self.enter(reference)?;
        let result = self.sequence(&[reference], resources, &frame, state);
        self.active.pop();
        result
    }
    fn image(
        &mut self,
        site: Site<'_>,
        resources: Option<&'a Value>,
        state: &Graphics<'a>,
        binding: Binding<'a>,
        name: &[u8],
        kind: PdfResourceKind,
    ) -> Result<(), PdfError> {
        let mut image_frame = site.frame.clone();
        image_frame.conditional |=
            dict_get(binding.value, b"OC").is_some_and(|v| !matches!(v, Value::Null));
        let site = Site {
            frame: &image_frame,
            ..site
        };
        let reference = binding
            .target
            .ok_or_else(|| PdfError::unsupported("image has no indirect identity"))?;
        if self.document.parsed().object(reference)?.stream.is_none() {
            return Err(PdfError::unsupported("image resource is not a stream"));
        }
        if dict_name(binding.value, b"Subtype") != Some(b"Image".as_slice()) {
            return Err(PdfError::unsupported("image dependency is not an image"));
        }
        let index = self.record(site, state, kind, Some(binding), name, None)?;
        self.report.uses[index].is_image = true;
        // Pixel dimensions are integer counts. A real-valued declaration must
        // not manufacture a sampling-density pass for malformed image metadata.
        let dimensions = match (
            dict_get(binding.value, b"Width"),
            dict_get(binding.value, b"Height"),
        ) {
            (Some(Value::Integer(width)), Some(Value::Integer(height)))
                if *width > 0 && *height > 0 =>
            {
                Some((*width as f64, *height as f64))
            }
            _ => None,
        };
        self.report.uses[index].image_dpi =
            dimensions.and_then(|(w, h)| dpi(state.ctm, site.frame.unit, w, h));
        self.report.uses[index].conditional |=
            dict_get(binding.value, b"OC").is_some_and(|v| !matches!(v, Value::Null));
        self.enter(reference)?;
        let result = (|| {
            if [b"F".as_slice(), b"OPI"]
                .iter()
                .any(|key| dict_get(binding.value, key).is_some_and(|v| !matches!(v, Value::Null)))
            {
                self.gap(
                    Some(site),
                    "pdf.usage.external_image",
                    "External image data or OPI replacement is not inspected.",
                )?;
            }
            if let Some(space) = dict_get(binding.value, b"ColorSpace") {
                self.color_value(site, resources, state, space, 0, false)?;
            } else if dict_get(binding.value, b"ImageMask") != Some(&Value::Bool(true)) {
                return Err(PdfError::unsupported("image color space is absent"));
            }
            for key in [b"SMask".as_slice(), b"Mask"] {
                if let Some(value) = dict_get(binding.value, key) {
                    self.work(1)?;
                    let dependency = self.binding(value)?;
                    match dependency.value {
                        Value::Null => {}
                        Value::Name(name) if key == b"SMask" && name == b"None" => {}
                        Value::Array(items)
                            if key == b"Mask" && items.iter().all(|v| number(v).is_some()) => {}
                        _ => self.image(
                            site,
                            resources,
                            state,
                            dependency,
                            key,
                            PdfResourceKind::SoftMask,
                        )?,
                    }
                }
            }
            if let Some(value) = dict_get(binding.value, b"Alternates") {
                let Value::Array(alternates) = self.resolve(value)? else {
                    return Err(PdfError::unsupported("image Alternates is not an array"));
                };
                for alternate in alternates {
                    self.work(1)?;
                    let alternate = self.resolve(alternate)?;
                    let value = dict_get(alternate, b"Image").ok_or_else(|| {
                        PdfError::unsupported("alternate image has no Image reference")
                    })?;
                    self.image(
                        site,
                        resources,
                        state,
                        self.binding(value)?,
                        b"Alternates",
                        PdfResourceKind::Image,
                    )?;
                }
            }
            Ok(())
        })();
        self.active.pop();
        result
    }
    fn type3(
        &mut self,
        site: Site<'_>,
        state: &Graphics<'a>,
        font: Binding<'a>,
        bytes: &[u8],
    ) -> Result<(), PdfError> {
        let encoding = dict_get(font.value, b"Encoding")
            .ok_or_else(|| PdfError::unsupported("Type3 Encoding is absent"))?;
        let encoding = self.resolve(encoding)?;
        let differences = dict_get(encoding, b"Differences").ok_or_else(|| {
            PdfError::unsupported("Type3 encoding requires explicit Differences for used codes")
        })?;
        let Value::Array(differences) = self.resolve(differences)? else {
            return Err(PdfError::unsupported("Type3 Differences is not an array"));
        };
        let mut names = BTreeMap::new();
        let mut code = None;
        for item in differences {
            self.work(1)?;
            match item {
                Value::Integer(value) if (0..=255).contains(value) => code = Some(*value as u16),
                Value::Name(name) => {
                    let current = code.filter(|value| *value <= 255).ok_or_else(|| {
                        PdfError::unsupported("Type3 Differences code is invalid")
                    })?;
                    if names.insert(current as u8, name.as_slice()).is_some() {
                        return Err(PdfError::unsupported(
                            "Type3 Differences assigns a code twice",
                        ));
                    }
                    code = Some(current + 1);
                }
                _ => {
                    return Err(PdfError::unsupported(
                        "Type3 Differences has an unsupported item",
                    ));
                }
            }
        }
        let procs = self.resolve(
            dict_get(font.value, b"CharProcs")
                .ok_or_else(|| PdfError::unsupported("Type3 CharProcs is absent"))?,
        )?;
        let resources = match dict_get(font.value, b"Resources") {
            Some(value) if !matches!(self.resolve(value)?, Value::Null) => {
                Some(self.resolve(value)?)
            }
            _ => self.document.inherited_resources(site.frame.page)?,
        };
        for byte in bytes {
            self.work(1)?;
            let name = names.get(byte).ok_or_else(|| {
                PdfError::unsupported("used Type3 code lacks an explicit glyph-name mapping")
            })?;
            let value = dict_get(procs, name)
                .ok_or_else(|| PdfError::unsupported("used Type3 glyph has no CharProc"))?;
            let glyph = self.binding(value)?;
            self.record(
                site,
                state,
                PdfResourceKind::Type3Glyph,
                Some(glyph),
                name,
                None,
            )?;
            let mut glyph_state = state.clone();
            // Font/text metrics and glyph advances are a separate interpreter.
            glyph_state.ctm = None;
            self.form(site, resources, glyph_state, glyph, false)?;
        }
        Ok(())
    }
    fn group_color(
        &mut self,
        site: Site<'_>,
        resources: Option<&'a Value>,
        state: &Graphics<'a>,
        value: &'a Value,
    ) -> Result<(), PdfError> {
        if let Some(group) = dict_get(value, b"Group") {
            let group = self.resolve(group)?;
            if !matches!(group, Value::Null | Value::Dict(_)) {
                return Err(PdfError::unsupported(
                    "transparency group is not a dictionary",
                ));
            }
            if let Some(space) = dict_get(group, b"CS") {
                self.color_value(site, resources, state, space, 0, false)?;
            }
        }
        Ok(())
    }
    fn shading(
        &mut self,
        site: Site<'_>,
        resources: Option<&'a Value>,
        state: &Graphics<'a>,
        operands: &[ContentOperand],
    ) -> Result<(), PdfError> {
        let name = one_name(operands)?;
        let binding = self.named(resources, b"Shading", name)?;
        self.record(
            site,
            state,
            PdfResourceKind::Shading,
            Some(binding),
            name,
            None,
        )?;
        let space = dict_get(binding.value, b"ColorSpace")
            .ok_or_else(|| PdfError::unsupported("shading color space is absent"))?;
        self.color_value(site, resources, state, space, 0, false)
    }
    fn color_space(
        &mut self,
        site: Site<'_>,
        resources: Option<&'a Value>,
        state: &Graphics<'a>,
        operands: &[ContentOperand],
    ) -> Result<(), PdfError> {
        let name = one_name(operands)?;
        self.color_name(site, resources, state, name, 0, false)
    }
    fn color_name(
        &mut self,
        site: Site<'_>,
        resources: Option<&'a Value>,
        state: &Graphics<'a>,
        name: &[u8],
        depth: usize,
        suppress_defaults: bool,
    ) -> Result<(), PdfError> {
        self.work(1)?;
        if depth > self.document.parsed().limits.max_parser_depth {
            return Err(PdfError::limit("color resource depth exceeds limit"));
        }
        let default = match name {
            b"DeviceGray" => Some(b"DefaultGray".as_slice()),
            b"DeviceRGB" => Some(b"DefaultRGB".as_slice()),
            b"DeviceCMYK" => Some(b"DefaultCMYK".as_slice()),
            _ => None,
        };
        if default.is_some() || name == b"Pattern" {
            self.record(site, state, PdfResourceKind::ColorSpace, None, name, None)?;
            // Defaults replace explicitly used device spaces. A replacement is
            // not recursively subject to the same replacement rule.
            if !suppress_defaults
                && let (Some(default), Some(resources)) = (default, resources)
                && let Some(spaces) = dict_get(self.resolve(resources)?, b"ColorSpace")
                && let Some(value) = dict_get(self.resolve(spaces)?, default)
            {
                let binding = self.binding(value)?;
                if !matches!(binding.value, Value::Null) {
                    self.record(
                        site,
                        state,
                        PdfResourceKind::ColorSpace,
                        Some(binding),
                        default,
                        None,
                    )?;
                    self.color_value(site, Some(resources), state, value, depth + 1, true)?;
                }
            }
            return Ok(());
        }
        let binding = self.named(resources, b"ColorSpace", name)?;
        self.record(
            site,
            state,
            PdfResourceKind::ColorSpace,
            Some(binding),
            name,
            None,
        )?;
        self.color_value(
            site,
            resources,
            state,
            binding.value,
            depth + 1,
            suppress_defaults,
        )
    }
    fn color_value(
        &mut self,
        site: Site<'_>,
        resources: Option<&'a Value>,
        state: &Graphics<'a>,
        value: &'a Value,
        depth: usize,
        suppress_defaults: bool,
    ) -> Result<(), PdfError> {
        self.work(1)?;
        if depth > self.document.parsed().limits.max_parser_depth {
            return Err(PdfError::limit("color resource depth exceeds limit"));
        }
        let value = self.resolve(value)?;
        match value {
            Value::Name(name) => {
                self.color_name(site, resources, state, name, depth, suppress_defaults)
            }
            Value::Array(values) => match values.as_slice() {
                [Value::Name(family), parameters]
                    if matches!(family.as_slice(), b"CalGray" | b"CalRGB" | b"Lab") =>
                {
                    if !matches!(self.resolve(parameters)?, Value::Dict(_)) {
                        return Err(PdfError::unsupported(
                            "calibrated color parameters are not a dictionary",
                        ));
                    }
                    Ok(())
                }
                [Value::Name(family), profile] if family == b"ICCBased" => {
                    let binding = self.binding(profile)?;
                    let reference = binding.target.ok_or_else(|| {
                        PdfError::unsupported("ICC profile is not an indirect stream")
                    })?;
                    if self.document.parsed().object(reference)?.stream.is_none() {
                        return Err(PdfError::unsupported("ICC profile has no stream"));
                    }
                    self.record(
                        site,
                        state,
                        PdfResourceKind::ColorSpace,
                        Some(binding),
                        b"ICCBased",
                        None,
                    )?;
                    if let Some(alternate) = dict_get(binding.value, b"Alternate") {
                        self.color_value(
                            site,
                            resources,
                            state,
                            alternate,
                            depth + 1,
                            suppress_defaults,
                        )?;
                    }
                    Ok(())
                }
                [Value::Name(family), base, Value::Integer(_), _] if family == b"Indexed" => {
                    self.color_value(site, resources, state, base, depth + 1, suppress_defaults)?;
                    self.gap(
                        Some(site),
                        "pdf.usage.complex_color",
                        "Indexed lookup and default color interpretation require further analysis.",
                    )
                }
                [Value::Name(family), _, alternate, _, ..]
                    if matches!(family.as_slice(), b"Separation" | b"DeviceN") =>
                {
                    self.color_value(
                        site,
                        resources,
                        state,
                        alternate,
                        depth + 1,
                        suppress_defaults,
                    )?;
                    self.gap(Some(site), "pdf.usage.complex_color", "Spot-color tint functions and DeviceN attributes are not fully interpreted.")
                }
                [Value::Name(family)] if family == b"Pattern" => Ok(()),
                [Value::Name(family), base] if family == b"Pattern" => {
                    self.color_value(site, resources, state, base, depth + 1, suppress_defaults)
                }
                _ => Err(PdfError::unsupported(
                    "color-space array is unsupported or malformed",
                )),
            },
            _ => Err(PdfError::unsupported(
                "color space is neither a name nor a supported array",
            )),
        }
    }
    fn pattern(
        &mut self,
        site: Site<'_>,
        resources: Option<&'a Value>,
        state: &Graphics<'a>,
        operands: &[ContentOperand],
    ) -> Result<(), PdfError> {
        let Some(ContentOperand::Name(name)) = operands.last() else {
            return if operands
                .iter()
                .all(|v| matches!(v, ContentOperand::Number(_)))
            {
                Ok(())
            } else {
                Err(PdfError::unsupported("SCN/scn operands are unsupported"))
            };
        };
        if !operands[..operands.len() - 1]
            .iter()
            .all(|v| matches!(v, ContentOperand::Number(_)))
        {
            return Err(PdfError::unsupported(
                "pattern color operands are unsupported",
            ));
        }
        let binding = self.named(resources, b"Pattern", name)?;
        self.record(
            site,
            state,
            PdfResourceKind::Pattern,
            Some(binding),
            name,
            None,
        )?;
        // Pattern matrices use initial user space, and tiling can invoke resources
        // many times. Report the gap even when its resource closure is inspected.
        self.gap(
            Some(site),
            "pdf.usage.pattern_placements",
            "Pattern tile placements and shading dependencies are not fully interpreted.",
        )?;
        if dict_get(binding.value, b"PatternType") == Some(&Value::Integer(1)) {
            let mut pattern_state = state.clone();
            pattern_state.ctm = None;
            self.form(site, resources, pattern_state, binding, false)?;
        }
        Ok(())
    }
    fn marked(
        &mut self,
        site: Site<'_>,
        resources: Option<&'a Value>,
        state: &Graphics<'a>,
        operation: &ContentOperation,
    ) -> Result<bool, PdfError> {
        match operation.operands.as_slice() {
            [ContentOperand::Name(_)] if operation.operator == b"BMC" => Ok(false),
            [ContentOperand::Name(tag), ContentOperand::Name(name)]
                if operation.operator != b"BMC" =>
            {
                let binding = self.named(resources, b"Properties", name)?;
                self.record(
                    site,
                    state,
                    PdfResourceKind::Properties,
                    Some(binding),
                    name,
                    None,
                )?;
                Ok(tag == b"OC")
            }
            _ => Err(PdfError::unsupported(
                "marked-content property dictionary or operands are unsupported",
            )),
        }
    }
    fn ext_state(
        &mut self,
        site: Site<'_>,
        resources: Option<&'a Value>,
        state: &mut Graphics<'a>,
        operands: &[ContentOperand],
    ) -> Result<(), PdfError> {
        let name = one_name(operands)?;
        let binding = self.named(resources, b"ExtGState", name)?;
        let Value::Dict(entries) = binding.value else {
            return Err(PdfError::unsupported("ExtGState is not a dictionary"));
        };
        self.record(
            site,
            state,
            PdfResourceKind::ExtGState,
            Some(binding),
            name,
            None,
        )?;
        for key in entries.keys() {
            self.work(1)?;
            if !matches!(
                key.as_slice(),
                b"Type"
                    | b"LW"
                    | b"LC"
                    | b"LJ"
                    | b"ML"
                    | b"D"
                    | b"RI"
                    | b"OP"
                    | b"op"
                    | b"OPM"
                    | b"Font"
                    | b"BG"
                    | b"BG2"
                    | b"UCR"
                    | b"UCR2"
                    | b"TR"
                    | b"TR2"
                    | b"HT"
                    | b"FL"
                    | b"SM"
                    | b"SA"
                    | b"BM"
                    | b"SMask"
                    | b"CA"
                    | b"ca"
                    | b"AIS"
                    | b"TK"
                    | b"UseBlackPtComp"
                    | b"HTO"
            ) {
                self.gap(Some(site), "pdf.usage.extgstate_extension", "An unknown graphics-state extension may introduce additional resource effects.")?;
            }
        }
        if let Some(value) = entries.get(b"Font".as_slice()) {
            let Value::Array(values) = self.resolve(value)? else {
                return Err(PdfError::unsupported("ExtGState Font is not an array"));
            };
            let [font, size] = values.as_slice() else {
                return Err(PdfError::unsupported(
                    "ExtGState Font needs a font and size",
                ));
            };
            if number(self.resolve(size)?).is_none() {
                return Err(PdfError::unsupported("ExtGState font size is invalid"));
            }
            let binding = self.binding(font)?;
            if !matches!(binding.value, Value::Dict(_)) {
                return Err(PdfError::unsupported("ExtGState font is not a dictionary"));
            }
            let type3 = self.font_is_type3(binding.value)?;
            let selection =
                self.record(site, state, PdfResourceKind::Font, Some(binding), b"", None)?;
            state.font = Some(Font {
                binding,
                name: Vec::new(),
                selection: Some(selection),
                type3,
            });
        }
        if let Some(value) = entries.get(b"SMask".as_slice()) {
            let mask = self.resolve(value)?;
            if !matches!(mask, Value::Null) && !matches!(mask, Value::Name(name) if name == b"None")
            {
                let group = dict_get(mask, b"G")
                    .ok_or_else(|| PdfError::unsupported("soft mask has no group"))?;
                let group = self.binding(group)?;
                self.record(
                    site,
                    state,
                    PdfResourceKind::SoftMask,
                    Some(group),
                    b"SMask",
                    None,
                )?;
                self.form(site, resources, state.clone(), group, false)?;
            }
        }
        Ok(())
    }
    fn appearances(
        &mut self,
        annots: &'a Value,
        resources: Option<&'a Value>,
        frame: &Frame,
    ) -> Result<(), PdfError> {
        let Value::Array(annots) = self.resolve(annots)? else {
            return Err(PdfError::unsupported("Annots is not an array"));
        };
        for annot in annots {
            self.work(1)?;
            let annot = self.binding(annot)?;
            if !matches!(annot.value, Value::Dict(_)) {
                return Err(PdfError::unsupported("annotation is not a dictionary"));
            }
            let mut annotation_frame = frame.clone();
            annotation_frame.appearance = true;
            annotation_frame.conditional |=
                dict_get(annot.value, b"OC").is_some_and(|value| !matches!(value, Value::Null));
            let site = Site {
                frame: &annotation_frame,
                stream: annot.target.unwrap_or(frame.page),
                offset: 0,
            };
            let Some(ap) = dict_get(annot.value, b"AP") else {
                continue;
            };
            let ap = self.resolve(ap)?;
            if matches!(ap, Value::Null) {
                continue;
            }
            if !matches!(ap, Value::Dict(_)) {
                return Err(PdfError::unsupported("annotation AP is not a dictionary"));
            }
            for key in [b"N".as_slice(), b"R", b"D"] {
                let Some(value) = dict_get(ap, key) else {
                    continue;
                };
                let binding = self.binding(value)?;
                let has_stream = binding.target.is_some_and(|r| {
                    self.document
                        .parsed()
                        .objects
                        .get(&r)
                        .is_some_and(|object| object.stream.is_some())
                });
                if has_stream {
                    self.appearance(site, resources, annot.value, binding, key)?;
                } else if let Value::Dict(states) = binding.value {
                    for (name, value) in states {
                        self.work(1)?;
                        self.appearance(site, resources, annot.value, self.binding(value)?, name)?;
                    }
                } else if !matches!(binding.value, Value::Null) {
                    return Err(PdfError::unsupported(
                        "AP value is neither a stream nor a state dictionary",
                    ));
                }
            }
        }
        Ok(())
    }
    fn appearance(
        &mut self,
        site: Site<'_>,
        resources: Option<&'a Value>,
        annot: &'a Value,
        binding: Binding<'a>,
        name: &[u8],
    ) -> Result<(), PdfError> {
        let matrix = self.matrix(dict_get(binding.value, b"Matrix"))?;
        let rect = self.rectangle(dict_get(annot, b"Rect"))?;
        let bbox = self.rectangle(dict_get(binding.value, b"BBox"))?;
        let ctm = rect
            .zip(bbox)
            .and_then(|(rect, bbox)| appearance_mapping(rect, bbox, matrix));
        let state = Graphics { ctm, font: None };
        self.record(
            site,
            &state,
            PdfResourceKind::Appearance,
            Some(binding),
            name,
            None,
        )?;
        self.form(site, resources, state, binding, true)
    }
    fn rectangle(&self, value: Option<&'a Value>) -> Result<Option<[f64; 4]>, PdfError> {
        let Some(value) = value else {
            return Ok(None);
        };
        let Value::Array(values) = self.resolve(value)? else {
            return Ok(None);
        };
        if values.len() != 4 {
            return Ok(None);
        }
        let mut rect = [0.; 4];
        for (index, value) in values.iter().enumerate() {
            let Some(value) = number(self.resolve(value)?) else {
                return Ok(None);
            };
            rect[index] = value;
        }
        Ok(Some(rect))
    }
}

fn target(reference: ObjectRef) -> PdfReferenceTarget {
    PdfReferenceTarget {
        number: reference.number,
        generation: reference.generation,
    }
}
fn dict_get<'a>(value: &'a Value, key: &[u8]) -> Option<&'a Value> {
    match value {
        Value::Dict(entries) => entries.get(key),
        _ => None,
    }
}
fn dict_name<'a>(value: &'a Value, key: &[u8]) -> Option<&'a [u8]> {
    match dict_get(value, key) {
        Some(Value::Name(name)) => Some(name),
        _ => None,
    }
}
fn number(value: &Value) -> Option<f64> {
    match value {
        Value::Integer(v) => Some(*v as f64),
        Value::Real(v) if v.is_finite() => Some(*v),
        _ => None,
    }
}
fn one_name(operands: &[ContentOperand]) -> Result<&[u8], PdfError> {
    match operands {
        [ContentOperand::Name(name)] => Ok(name),
        _ => Err(PdfError::unsupported("operator requires one resource name")),
    }
}
fn operand_matrix(operands: &[ContentOperand]) -> Option<Matrix> {
    if operands.len() != 6 {
        return None;
    }
    let mut matrix = IDENTITY;
    for (index, operand) in operands.iter().enumerate() {
        let ContentOperand::Number(value) = operand else {
            return None;
        };
        matrix[index] = *value;
    }
    matrix.iter().all(|v| v.is_finite()).then_some(matrix)
}
fn multiply(a: Matrix, b: Matrix) -> Option<Matrix> {
    let matrix = [
        a[0] * b[0] + a[2] * b[1],
        a[1] * b[0] + a[3] * b[1],
        a[0] * b[2] + a[2] * b[3],
        a[1] * b[2] + a[3] * b[3],
        a[0] * b[4] + a[2] * b[5] + a[4],
        a[1] * b[4] + a[3] * b[5] + a[5],
    ];
    matrix.iter().all(|v| v.is_finite()).then_some(matrix)
}
fn dpi(ctm: Option<Matrix>, unit: Option<f64>, width: f64, height: f64) -> Option<[f64; 2]> {
    let m = ctm?;
    let unit = unit?;
    let nx = m[0].hypot(m[1]);
    let ny = m[2].hypot(m[3]);
    let x = nx * unit;
    let y = ny * unit;
    let cross = (m[0] / nx) * (m[3] / ny) - (m[1] / nx) * (m[2] / ny);
    if width <= 0. || height <= 0. || x <= 0. || y <= 0. || !cross.is_finite() || cross == 0. {
        return None;
    }
    let dpi = [width / x * 72., height / y * 72.];
    dpi.iter().all(|v| v.is_finite() && *v > 0.).then_some(dpi)
}
fn appearance_mapping(rect: [f64; 4], bbox: [f64; 4], matrix: Matrix) -> Option<Matrix> {
    let mut bounds = [
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    ];
    for (x, y) in [
        (bbox[0], bbox[1]),
        (bbox[0], bbox[3]),
        (bbox[2], bbox[1]),
        (bbox[2], bbox[3]),
    ] {
        let x1 = matrix[0] * x + matrix[2] * y + matrix[4];
        let y1 = matrix[1] * x + matrix[3] * y + matrix[5];
        if !x1.is_finite() || !y1.is_finite() {
            return None;
        }
        bounds[0] = bounds[0].min(x1);
        bounds[1] = bounds[1].min(y1);
        bounds[2] = bounds[2].max(x1);
        bounds[3] = bounds[3].max(y1);
    }
    let sx = (rect[2] - rect[0]).abs() / (bounds[2] - bounds[0]);
    let sy = (rect[3] - rect[1]).abs() / (bounds[3] - bounds[1]);
    let result = [
        sx,
        0.,
        0.,
        sy,
        rect[0].min(rect[2]) - bounds[0] * sx,
        rect[1].min(rect[3]) - bounds[1] * sy,
    ];
    result.iter().all(|v| v.is_finite()).then_some(result)
}
fn plain_operator(operator: &[u8], operands: &[ContentOperand]) -> bool {
    let count = match operator {
        b"h" | b"S" | b"s" | b"f" | b"F" | b"f*" | b"B" | b"B*" | b"b" | b"b*" | b"n" | b"W"
        | b"W*" | b"T*" => 0,
        b"w" | b"J" | b"j" | b"M" | b"i" | b"Tc" | b"Tw" | b"Tz" | b"TL" | b"Tr" | b"Ts" | b"G"
        | b"g" => 1,
        b"m" | b"l" | b"Td" | b"TD" | b"d0" => 2,
        b"RG" | b"rg" => 3,
        b"v" | b"y" | b"re" | b"K" | b"k" => 4,
        b"c" | b"Tm" | b"d1" => 6,
        b"ri" => return matches!(operands, [ContentOperand::Name(_)]),
        b"d" => {
            return matches!(operands, [ContentOperand::Array(items), ContentOperand::Number(_)] if items.iter().all(|v| matches!(v, ContentOperand::Number(_))));
        }
        b"SC" | b"sc" => {
            return operands
                .iter()
                .all(|v| matches!(v, ContentOperand::Number(_)));
        }
        _ => return false,
    };
    operands.len() == count
        && operands
            .iter()
            .all(|v| matches!(v, ContentOperand::Number(_)))
}

#[cfg(test)]
mod geometry_tests {
    use super::*;
    #[test]
    fn overflowing_products_do_not_disguise_a_singular_image_transform() {
        assert!(
            dpi(
                Some([1e200, 1e200, 1e200, 1e200, 0., 0.]),
                Some(1.),
                100.,
                100.
            )
            .is_none()
        );
        assert!(dpi(Some([0.; 6]), Some(1.), 100., 100.).is_none());
        assert_eq!(
            dpi(Some([0., 2., -4., 0., 10., 20.]), Some(2.), 100., 200.),
            Some([1800., 1800.])
        );
    }
    #[test]
    fn rotated_appearance_maps_its_transformed_bbox_to_the_annotation_rect() {
        let matrix = [0., 1., -1., 0., 10., 20.];
        let mapping =
            appearance_mapping([100., 200., 140., 220.], [0., 0., 2., 4.], matrix).unwrap();
        assert_eq!(
            multiply(mapping, matrix),
            Some([0., 10., -10., 0., 140., 200.])
        );
        assert!(appearance_mapping([0., 0., 1., 1.], [0.; 4], matrix).is_none());
    }
}
