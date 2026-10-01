use binas_core::{OperationContext, Reservation, SnapshotId};

use super::ImageResolutionProfile;
use crate::{
    PdfDocument, PdfError, PdfReferenceTarget, PdfResourceUsage, PdfResourceUse, PdfRevision,
    PdfUsageGap,
};

pub const IMAGE_RESOLUTION_CHECK_CODE: &str = "pdf.preflight.image.minimum_axis_dpi";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ImagePreflightScope {
    Document,
    Page(usize),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ImageResolutionStatus {
    Passed,
    Failed,
    NotChecked,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ImagePreflightResult {
    Passed,
    Failed,
    Incomplete,
    NotApplicable,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ImagePreflightCoverage {
    pub image_placements: usize,
    pub checked_placements: usize,
    /// Resource traversal coverage; independent of known image geometry.
    pub census_complete: bool,
}

/// A borrowed immutable view of one image placement and its check result.
/// The resource retains its page/object identity, invocation path, AP/OC flags,
/// optional full-image-axis DPI, and decoded operator location.
#[derive(Clone, Copy, Debug)]
pub struct ImageResolutionFinding<'a> {
    pub resource: &'a PdfResourceUse,
    pub code: &'static str,
    pub status: ImageResolutionStatus,
    pub minimum_dpi: f64,
    pub below_minimum_axes: Option<[bool; 2]>,
    pub detail: &'static str,
}

#[derive(Debug)]
struct StoredFinding {
    use_index: usize,
    status: ImageResolutionStatus,
    below_minimum_axes: Option<[bool; 2]>,
}

/// Observational preflight evidence, not a PDF standards conformance report.
/// Public construction from a caller-supplied census is deliberately unavailable.
#[derive(Debug)]
pub struct ImagePreflightReport {
    profile: ImageResolutionProfile,
    scope: ImagePreflightScope,
    result: ImagePreflightResult,
    coverage: ImagePreflightCoverage,
    usage: PdfResourceUsage,
    findings: Vec<StoredFinding>,
    _reservation: Reservation,
}

impl ImagePreflightReport {
    pub fn snapshot(&self) -> SnapshotId {
        self.usage.snapshot()
    }
    pub fn profile(&self) -> ImageResolutionProfile {
        self.profile
    }
    pub fn scope(&self) -> ImagePreflightScope {
        self.scope
    }
    pub fn result(&self) -> ImagePreflightResult {
        self.result
    }
    pub fn coverage(&self) -> ImagePreflightCoverage {
        self.coverage
    }
    pub fn page_indices(&self) -> &[usize] {
        &self.usage.page_indices
    }
    pub fn census_gaps(&self) -> &[PdfUsageGap] {
        &self.usage.gaps
    }
    pub fn scanned_streams(&self) -> &[PdfReferenceTarget] {
        &self.usage.scanned_streams
    }
    pub fn findings(&self) -> impl ExactSizeIterator<Item = ImageResolutionFinding<'_>> {
        self.findings.iter().map(|finding| ImageResolutionFinding {
            resource: &self.usage.uses[finding.use_index],
            code: IMAGE_RESOLUTION_CHECK_CODE,
            status: finding.status,
            minimum_dpi: self.profile.minimum_dpi(),
            below_minimum_axes: finding.below_minimum_axes,
            detail: match finding.status {
                ImageResolutionStatus::Passed => "Both full-image axes meet the minimum DPI.",
                ImageResolutionStatus::Failed => "At least one full-image axis is below the minimum DPI.",
                ImageResolutionStatus::NotChecked => "Image DPI is unavailable because placement geometry or image dimensions are not established; no resolution pass is inferred.",
            },
        })
    }
}

pub fn inspect_document_images(
    document: &PdfDocument,
    profile: &ImageResolutionProfile,
    scope: ImagePreflightScope,
    context: &OperationContext,
) -> Result<ImagePreflightReport, PdfError> {
    let usage = match scope {
        ImagePreflightScope::Document => document.resource_usage(context)?,
        ImagePreflightScope::Page(index) => document.page_resource_usage(index, context)?,
    };
    evaluate(usage, *profile, scope, context)
}

pub fn inspect_revision_images(
    revision: &PdfRevision,
    profile: &ImageResolutionProfile,
    scope: ImagePreflightScope,
    context: &OperationContext,
) -> Result<ImagePreflightReport, PdfError> {
    let usage = match scope {
        ImagePreflightScope::Document => revision.resource_usage(context)?,
        ImagePreflightScope::Page(index) => revision.page_resource_usage(index, context)?,
    };
    evaluate(usage, *profile, scope, context)
}

fn evaluate(
    usage: PdfResourceUsage,
    profile: ImageResolutionProfile,
    scope: ImagePreflightScope,
    context: &OperationContext,
) -> Result<ImagePreflightReport, PdfError> {
    let mut count = 0usize;
    for resource in &usage.uses {
        context.charge_work(1).map_err(PdfError::from_context)?;
        count += usize::from(resource.is_image);
    }
    let storage = count
        .checked_mul(std::mem::size_of::<StoredFinding>())
        .and_then(|bytes| bytes.checked_add(std::mem::size_of::<ImagePreflightReport>()))
        .ok_or_else(|| PdfError::limit("image preflight report size overflows"))?;
    let reservation = context
        .reserve(storage as u64)
        .map_err(PdfError::from_context)?;
    let mut findings = Vec::with_capacity(count);
    let mut checked = 0usize;
    let mut failed = false;
    for (use_index, resource) in usage.uses.iter().enumerate() {
        context.charge_work(1).map_err(PdfError::from_context)?;
        if !resource.is_image {
            continue;
        }
        let below_minimum_axes = resource
            .image_dpi
            .filter(|dpi| dpi.iter().all(|axis| axis.is_finite() && *axis > 0.0))
            .map(|dpi| {
                [
                    dpi[0] < profile.minimum_dpi(),
                    dpi[1] < profile.minimum_dpi(),
                ]
            });
        let status = match below_minimum_axes {
            Some(axes) => {
                checked += 1;
                if axes[0] || axes[1] {
                    failed = true;
                    ImageResolutionStatus::Failed
                } else {
                    ImageResolutionStatus::Passed
                }
            }
            None => ImageResolutionStatus::NotChecked,
        };
        findings.push(StoredFinding {
            use_index,
            status,
            below_minimum_axes,
        });
    }
    let coverage = ImagePreflightCoverage {
        image_placements: count,
        checked_placements: checked,
        census_complete: usage.complete(),
    };
    let result = if failed {
        ImagePreflightResult::Failed
    } else if !usage.complete() || checked != count {
        ImagePreflightResult::Incomplete
    } else if count == 0 {
        ImagePreflightResult::NotApplicable
    } else {
        ImagePreflightResult::Passed
    };
    context.checkpoint().map_err(PdfError::from_context)?;
    Ok(ImagePreflightReport {
        profile,
        scope,
        result,
        coverage,
        usage,
        findings,
        _reservation: reservation,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{OpenOptions, PdfEngine, PdfErrorCode, PdfResourceKind, PdfRevisionRequest};
    use binas_core::{BudgetLimits, CancellationToken, ResourceBudget};
    use std::collections::BTreeMap;

    fn context() -> OperationContext {
        OperationContext::new(
            ResourceBudget::new(BudgetLimits {
                max_live_bytes: 32 * 1024 * 1024,
                max_work_units: 64 * 1024 * 1024,
            }),
            CancellationToken::new(),
        )
    }
    fn stream(data: &[u8], entries: &str) -> Vec<u8> {
        let mut bytes = format!("<< /Length {} {entries} >>\nstream\n", data.len()).into_bytes();
        bytes.extend_from_slice(data);
        bytes.extend_from_slice(b"\nendstream");
        bytes
    }
    fn objects(content: &[u8]) -> BTreeMap<u32, Vec<u8>> {
        BTreeMap::from([
            (1, b"<< /Type /Catalog /Pages 2 0 R >>".to_vec()),
            (2, b"<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 200 200] >>".to_vec()),
            (3, b"<< /Type /Page /Parent 2 0 R /Contents 4 0 R /Resources << /XObject << /Im 5 0 R /Fm 6 0 R >> /Font << /F1 7 0 R >> /Properties << /Layer 8 0 R >> >> >>".to_vec()),
            (4, stream(content, "")),
            (5, stream(&[0x40], "/Type /XObject /Subtype /Image /Width 1 /Height 1 /BitsPerComponent 8 /ColorSpace /DeviceGray")),
            (6, stream(b"/Im Do", "/Type /XObject /Subtype /Form /BBox [0 0 1 1] /Resources << /XObject << /Im 5 0 R >> >>")),
            (7, b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_vec()),
            (8, b"<< /Type /OCG /Name (hidden) >>".to_vec()),
        ])
    }
    fn open(objects: BTreeMap<u32, Vec<u8>>) -> PdfDocument {
        let size = objects.keys().last().unwrap() + 1;
        let mut bytes = b"%PDF-1.7\n".to_vec();
        let mut offsets = BTreeMap::new();
        for (number, body) in objects {
            offsets.insert(number, bytes.len());
            bytes.extend_from_slice(format!("{number} 0 obj\n").as_bytes());
            bytes.extend(body);
            bytes.extend_from_slice(b"\nendobj\n");
        }
        let xref = bytes.len();
        bytes.extend_from_slice(format!("xref\n0 {size}\n0000000000 65535 f \n").as_bytes());
        for number in 1..size {
            bytes.extend_from_slice(
                match offsets.get(&number) {
                    Some(offset) => format!("{offset:010} 00000 n \n"),
                    None => "0000000000 00000 f \n".into(),
                }
                .as_bytes(),
            );
        }
        bytes.extend_from_slice(
            format!("trailer\n<< /Size {size} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n")
                .as_bytes(),
        );
        PdfEngine::default()
            .open_owned(bytes, OpenOptions::default())
            .unwrap()
    }
    fn inspect(document: &PdfDocument, threshold: f64) -> ImagePreflightReport {
        inspect_document_images(
            document,
            &ImageResolutionProfile::new(threshold).unwrap(),
            ImagePreflightScope::Document,
            &context(),
        )
        .unwrap()
    }

    #[test]
    fn shared_image_placements_check_both_axes_and_threshold_equality() {
        let document = open(objects(b"/Im Do q 2 0 0 0.5 0 0 cm /Im Do Q"));
        let report = inspect(&document, 72.0);
        assert_eq!(report.result(), ImagePreflightResult::Failed);
        assert_eq!(
            report.coverage(),
            ImagePreflightCoverage {
                image_placements: 2,
                checked_placements: 2,
                census_complete: true
            }
        );
        let findings: Vec<_> = report.findings().collect();
        assert_eq!(findings[0].status, ImageResolutionStatus::Passed);
        assert_eq!(findings[0].below_minimum_axes, Some([false, false]));
        assert_eq!(findings[1].status, ImageResolutionStatus::Failed);
        assert_eq!(findings[1].resource.image_dpi, Some([36.0, 144.0]));
        assert_eq!(findings[1].below_minimum_axes, Some([true, false]));
        assert_eq!(findings[0].resource.target, findings[1].resource.target);
        assert_eq!(findings[1].resource.page.number, 3);
        assert_eq!(findings[1].code, IMAGE_RESOLUTION_CHECK_CODE);
        assert_eq!(
            inspect(&document, 36.0).result(),
            ImagePreflightResult::Passed
        );
    }

    #[test]
    fn unknown_geometry_and_census_gaps_remain_incomplete_but_known_failure_wins() {
        let unknown = open(objects(b"q 0 0 0 0 0 0 cm /Im Do Q"));
        let report = inspect(&unknown, 72.0);
        assert_eq!(report.result(), ImagePreflightResult::Incomplete);
        assert_eq!(report.coverage().checked_placements, 0);
        let finding = report.findings().next().unwrap();
        assert_eq!(finding.status, ImageResolutionStatus::NotChecked);
        assert_eq!(finding.below_minimum_axes, None);
        assert!(!finding.detail.is_empty());

        let gap = open(objects(b"/Im Do MysteryOperator"));
        let incomplete = inspect(&gap, 72.0);
        assert_eq!(incomplete.result(), ImagePreflightResult::Incomplete);
        assert!(!incomplete.coverage().census_complete);
        assert!(!incomplete.census_gaps().is_empty());
        let failed = inspect(&gap, 100.0);
        assert_eq!(failed.result(), ImagePreflightResult::Failed);
        assert!(!failed.census_gaps().is_empty());
        assert_eq!(
            inspect(&open(objects(b"")), 72.0).result(),
            ImagePreflightResult::NotApplicable
        );
        assert_eq!(
            inspect(&open(objects(b"MysteryOperator")), 72.0).result(),
            ImagePreflightResult::Incomplete
        );
    }

    #[test]
    fn type3_images_keep_unknown_geometry_instead_of_guessed_glyph_dpi() {
        let mut source = objects(b"BT /F1 12 Tf (A) Tj ET");
        source.insert(7, b"<< /Type /Font /Subtype /Type3 /FontBBox [0 0 600 700] /FontMatrix [0.001 0 0 0.001 0 0] /CharProcs << /A 6 0 R >> /Encoding << /Differences [65 /A] >> /FirstChar 65 /LastChar 65 /Widths [600] /Resources << /XObject << /Im 5 0 R >> >> >>".to_vec());
        source.insert(6, stream(b"600 0 0 0 600 700 d1 /Im Do", ""));
        let report = inspect(&open(source), 36.0);
        assert_eq!(report.result(), ImagePreflightResult::Incomplete);
        assert_eq!(report.findings().len(), 1);
        assert_eq!(
            report.findings().next().unwrap().status,
            ImageResolutionStatus::NotChecked
        );
    }

    #[test]
    fn hidden_content_all_appearance_states_and_image_masks_are_checked() {
        let mut source = objects(b"/OC /Layer BDC /Im Do EMC");
        source.insert(3, b"<< /Type /Page /Parent 2 0 R /Contents 4 0 R /Annots [9 0 R] /Resources << /XObject << /Im 5 0 R >> /Properties << /Layer 8 0 R >> >> >>".to_vec());
        source.insert(5, stream(&[0x40], "/Type /XObject /Subtype /Image /Width 1 /Height 1 /BitsPerComponent 8 /ColorSpace /DeviceGray /SMask 13 0 R"));
        source.insert(9, b"<< /Type /Annot /Subtype /Stamp /Rect [0 0 1 1] /OC 8 0 R /AS /On /AP << /N << /On 10 0 R /Off 11 0 R >> /R 12 0 R /D 10 0 R >> >>".to_vec());
        for number in [10, 11, 12] {
            source.insert(number, stream(b"/Im Do", "/Type /XObject /Subtype /Form /BBox [0 0 1 1] /Resources << /XObject << /Im 5 0 R >> >>"));
        }
        source.insert(13, stream(&[0x80], "/Type /XObject /Subtype /Image /Width 1 /Height 1 /BitsPerComponent 8 /ColorSpace /DeviceGray"));
        let report = inspect(&open(source), 72.0);
        assert_eq!(report.result(), ImagePreflightResult::Passed);
        assert_eq!(report.findings().len(), 10);
        assert!(report.findings().all(|f| f.resource.conditional));
        assert_eq!(
            report.findings().filter(|f| f.resource.appearance).count(),
            8
        );
        assert_eq!(
            report
                .findings()
                .filter(|f| f.resource.kind == PdfResourceKind::SoftMask)
                .count(),
            5
        );
    }

    #[test]
    fn transparency_group_is_not_an_image_but_its_images_and_inline_images_are() {
        let mut source = objects(b"/GS gs BI /W 1 /H 1 /CS /G /BPC 8 ID a EI");
        source.insert(3, b"<< /Type /Page /Parent 2 0 R /Contents 4 0 R /Resources << /ExtGState << /GS << /SMask << /S /Luminosity /G 6 0 R >> >> >> >> >>".to_vec());
        let report = inspect(&open(source), 72.0);
        assert_eq!(report.result(), ImagePreflightResult::Passed);
        assert_eq!(report.findings().len(), 2);
        assert!(
            report
                .findings()
                .all(|f| f.resource.kind != PdfResourceKind::SoftMask)
        );
        assert!(
            report
                .findings()
                .any(|f| f.resource.kind == PdfResourceKind::InlineImage)
        );
    }

    #[test]
    fn callers_cannot_supply_mutated_census_evidence_and_report_memory_is_released() {
        let document = open(objects(b"/Im Do"));
        let mut untrusted = document.resource_usage(&context()).unwrap();
        untrusted.uses.clear();
        assert_eq!(
            inspect(&document, 100.0).result(),
            ImagePreflightResult::Failed
        );
        let context = context();
        let before = context.budget().snapshot();
        let profile = ImageResolutionProfile::new(72.0).unwrap();
        let report =
            inspect_document_images(&document, &profile, ImagePreflightScope::Document, &context)
                .unwrap();
        let held = context.budget().snapshot();
        assert!(held.live_bytes > before.live_bytes);
        assert!(held.work_units > before.work_units);
        drop(report);
        assert_eq!(context.budget().snapshot().live_bytes, before.live_bytes);
        assert_eq!(context.budget().snapshot().work_units, held.work_units);
        context.cancellation().cancel();
        assert_eq!(
            inspect_document_images(&document, &profile, ImagePreflightScope::Document, &context)
                .unwrap_err()
                .code,
            PdfErrorCode::Cancelled
        );
        let limited = OperationContext::new(
            ResourceBudget::new(BudgetLimits {
                max_live_bytes: 0,
                max_work_units: 64 * 1024 * 1024,
            }),
            CancellationToken::new(),
        );
        assert_eq!(
            inspect_document_images(&document, &profile, ImagePreflightScope::Document, &limited)
                .unwrap_err()
                .code,
            PdfErrorCode::ResourceLimit
        );
        assert_eq!(limited.budget().snapshot().live_bytes, 0);
    }

    #[test]
    fn selective_and_retained_preflight_bind_page_order_and_committed_snapshot() {
        let mut source = objects(b"/Im Do");
        source.insert(
            2,
            b"<< /Type /Pages /Kids [3 0 R 9 0 R] /Count 2 /MediaBox [0 0 200 200] >>".to_vec(),
        );
        source.insert(
            9,
            b"<< /Type /Page /Parent 2 0 R /Contents 10 0 R >>".to_vec(),
        );
        source.insert(10, stream(b"MysteryOperator", ""));
        let document = open(source);
        let context = context();
        let profile = ImageResolutionProfile::new(72.0).unwrap();
        let before =
            inspect_document_images(&document, &profile, ImagePreflightScope::Page(0), &context)
                .unwrap();
        assert_eq!(before.result(), ImagePreflightResult::Passed);
        assert_eq!(before.page_indices(), [0]);
        assert!(!before.scanned_streams().iter().any(|r| r.number == 10));
        let mut revision = PdfRevision::new(document);
        let plan = revision
            .plan(
                PdfRevisionRequest {
                    reorder_pages: Some(vec![1, 0]),
                    ..Default::default()
                },
                &context,
            )
            .unwrap();
        let receipt = revision.commit(&plan, &context).unwrap();
        let after =
            inspect_revision_images(&revision, &profile, ImagePreflightScope::Page(1), &context)
                .unwrap();
        assert_eq!(after.snapshot(), receipt.after);
        assert_ne!(before.snapshot(), after.snapshot());
        assert_eq!(before.snapshot().source(), after.snapshot().source());
        assert_eq!(after.findings().next().unwrap().resource.page.number, 3);
        assert_eq!(after.findings().next().unwrap().resource.page_index, 1);
        assert_eq!(before.findings().next().unwrap().resource.page_index, 0);
        assert_eq!(
            inspect_revision_images(&revision, &profile, ImagePreflightScope::Document, &context)
                .unwrap()
                .result(),
            ImagePreflightResult::Incomplete
        );
    }
}
