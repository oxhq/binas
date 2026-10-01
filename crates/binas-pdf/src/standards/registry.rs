//! Published edition identities, frozen registry schema; normative rule and
//! amendment/errata inventories are not yet complete. See docs/standards-registry.md.
use std::{error::Error, fmt};
pub const STANDARDS_REGISTRY_VERSION: &str = "binas.standards.registry.v1";
pub const REGISTRY_REFERENCE_DATE: &str = "2026-09-07";

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum StandardTarget {
    PdfA1b,
    PdfA1a,
    PdfA2b,
    PdfA2u,
    PdfA2a,
    PdfA3b,
    PdfA3u,
    PdfA3a,
    PdfA4,
    PdfA4f,
    PdfA4e,
    PdfUa1,
    PdfUa2,
    PdfX1a,
    PdfX3,
    PdfX4,
    PdfX6,
    PdfX4p,
    PdfX5g,
    PdfX5n,
    PdfX5pg,
    PdfX6n,
    PdfX6p,
    PdfE1,
    PdfVt1,
    PdfVt3,
    PdfVt2,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TargetCapability {
    ConversionPlanned,
    CheckOnlyPlanned,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Coverage {
    Unenumerated,
    Partial,
    Complete,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TargetDependency {
    None,
    Required(StandardTarget),
    AnyOf(&'static [StandardTarget]),
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NormativeBaseline {
    pub publication: &'static str,
    pub base_pdf: &'static str,
    pub named_supplements: &'static [&'static str],
    /// False also when named_supplements is empty: absence is not proof of no errata.
    pub amendments_and_errata_frozen: bool,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TargetDefinition {
    pub target: StandardTarget,
    pub id: &'static str,
    pub baseline: NormativeBaseline,
    pub roadmap_capability: TargetCapability,
    pub dependency: TargetDependency,
    pub semantic_review_required: bool,
    pub normative_rule_inventory: Coverage,
}
const TARGETS: [StandardTarget; 27] = [
    StandardTarget::PdfA1b,
    StandardTarget::PdfA1a,
    StandardTarget::PdfA2b,
    StandardTarget::PdfA2u,
    StandardTarget::PdfA2a,
    StandardTarget::PdfA3b,
    StandardTarget::PdfA3u,
    StandardTarget::PdfA3a,
    StandardTarget::PdfA4,
    StandardTarget::PdfA4f,
    StandardTarget::PdfA4e,
    StandardTarget::PdfUa1,
    StandardTarget::PdfUa2,
    StandardTarget::PdfX1a,
    StandardTarget::PdfX3,
    StandardTarget::PdfX4,
    StandardTarget::PdfX6,
    StandardTarget::PdfX4p,
    StandardTarget::PdfX5g,
    StandardTarget::PdfX5n,
    StandardTarget::PdfX5pg,
    StandardTarget::PdfX6n,
    StandardTarget::PdfX6p,
    StandardTarget::PdfE1,
    StandardTarget::PdfVt1,
    StandardTarget::PdfVt3,
    StandardTarget::PdfVt2,
];
pub fn target_registry() -> impl ExactSizeIterator<Item = TargetDefinition> {
    TARGETS.into_iter().map(target_definition)
}
pub fn target_definition(target: StandardTarget) -> TargetDefinition {
    use StandardTarget::*;
    let (id, publication, base_pdf) = match target {
        PdfA1b => ("pdfa-1b", "ISO 19005-1:2005", "PDF 1.4"),
        PdfA1a => ("pdfa-1a", "ISO 19005-1:2005", "PDF 1.4"),
        PdfA2b => ("pdfa-2b", "ISO 19005-2:2011", "ISO 32000-1:2008"),
        PdfA2u => ("pdfa-2u", "ISO 19005-2:2011", "ISO 32000-1:2008"),
        PdfA2a => ("pdfa-2a", "ISO 19005-2:2011", "ISO 32000-1:2008"),
        PdfA3b => ("pdfa-3b", "ISO 19005-3:2012", "ISO 32000-1:2008"),
        PdfA3u => ("pdfa-3u", "ISO 19005-3:2012", "ISO 32000-1:2008"),
        PdfA3a => ("pdfa-3a", "ISO 19005-3:2012", "ISO 32000-1:2008"),
        PdfA4 => ("pdfa-4", "ISO 19005-4:2020", "ISO 32000-2:2020"),
        PdfA4f => ("pdfa-4f", "ISO 19005-4:2020", "ISO 32000-2:2020"),
        PdfA4e => ("pdfa-4e", "ISO 19005-4:2020", "ISO 32000-2:2020"),
        PdfUa1 => ("pdfua-1", "ISO 14289-1:2014", "ISO 32000-1:2008"),
        PdfUa2 => ("pdfua-2", "ISO 14289-2:2024", "ISO 32000-2:2020"),
        PdfX1a => ("pdfx-1a-2003", "ISO 15930-4:2003", "PDF 1.4"),
        PdfX3 => ("pdfx-3-2003", "ISO 15930-6:2003", "PDF 1.4"),
        PdfX4 => ("pdfx-4", "ISO 15930-7:2010", "PDF 1.6"),
        PdfX6 => ("pdfx-6", "ISO 15930-9:2020", "ISO 32000-2:2020"),
        PdfX4p => ("pdfx-4p", "ISO 15930-7:2010", "PDF 1.6"),
        PdfX5g => ("pdfx-5g", "ISO 15930-8:2010", "PDF 1.6"),
        PdfX5n => ("pdfx-5n", "ISO 15930-8:2010", "PDF 1.6"),
        PdfX5pg => ("pdfx-5pg", "ISO 15930-8:2010", "PDF 1.6"),
        PdfX6n => ("pdfx-6n", "ISO 15930-9:2020", "ISO 32000-2:2020"),
        PdfX6p => ("pdfx-6p", "ISO 15930-9:2020", "ISO 32000-2:2020"),
        PdfE1 => ("pdfe-1", "ISO 24517-1:2008", "PDF 1.6"),
        PdfVt1 => ("pdfvt-1", "ISO 16612-2:2010", "ISO 15930-7:2010"),
        PdfVt3 => ("pdfvt-3", "ISO 16612-3:2020", "ISO 15930-9:2020"),
        PdfVt2 => (
            "pdfvt-2",
            "ISO 16612-2:2010",
            "ISO 15930-7:2010 or ISO 15930-8:2010",
        ),
    };
    TargetDefinition {
        target,
        id,
        baseline: NormativeBaseline {
            publication,
            base_pdf,
            named_supplements: if matches!(target, PdfA1a | PdfA1b) {
                &["ISO 19005-1:2005/Cor 1:2007", "ISO 19005-1:2005/Cor 2:2011"]
            } else {
                &[]
            },
            amendments_and_errata_frozen: false,
        },
        roadmap_capability: if matches!(
            target,
            PdfX4p | PdfX5g | PdfX5n | PdfX5pg | PdfX6n | PdfX6p | PdfVt2
        ) {
            TargetCapability::CheckOnlyPlanned
        } else {
            TargetCapability::ConversionPlanned
        },
        dependency: match target {
            PdfVt1 => TargetDependency::Required(PdfX4),
            PdfVt3 => TargetDependency::Required(PdfX6),
            PdfVt2 => TargetDependency::AnyOf(&[PdfX4p, PdfX5g, PdfX5pg]),
            _ => TargetDependency::None,
        },
        semantic_review_required: matches!(target, PdfUa1 | PdfUa2 | PdfA1a | PdfA2a | PdfA3a),
        normative_rule_inventory: Coverage::Unenumerated,
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UnsupportedTarget(pub String);
impl fmt::Display for UnsupportedTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unsupported standards target: {}", self.0)
    }
}
impl Error for UnsupportedTarget {}
pub fn parse_target(id: &str) -> Result<StandardTarget, UnsupportedTarget> {
    if id.len() > 160 {
        return Err(UnsupportedTarget("identifier exceeds 160 bytes".into()));
    }
    target_registry()
        .find(|item| item.id == id)
        .map(|item| item.target)
        .ok_or_else(|| UnsupportedTarget(id.into()))
}

/// Packaging identity is separate from PDF subset conformance and XML/business
/// validation. No packaging creation/check capability is implemented here.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PackagingTarget {
    FacturX,
    Zugferd,
    XrechnungAssociated,
    OrderX,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PackagingDefinition {
    pub target: PackagingTarget,
    pub reference_version: &'static str,
    pub publisher_reference: &'static str,
    pub normative_rule_inventory: Coverage,
    pub schema_and_business_rule_manifest_frozen: bool,
}
pub fn packaging_registry() -> [PackagingDefinition; 4] {
    use PackagingTarget::*;
    [
        (
            FacturX,
            "1.09.2",
            "https://fnfe-mpe.org/factur-x/factur-x-et-zugferd/",
        ),
        (
            Zugferd,
            "2.5.2",
            "https://www.ferd-net.de/downloads/veroeffentlichungen",
        ),
        (
            XrechnungAssociated,
            "XML specification 3.0.2; PDF packaging is separately scoped",
            "https://xeinkauf.de/xrechnung/versionen-und-bundles/",
        ),
        (
            OrderX,
            "1.0 (2021-04-13)",
            "https://www.ferd-net.de/standards/order-x",
        ),
    ]
    .map(
        |(target, reference_version, publisher_reference)| PackagingDefinition {
            target,
            reference_version,
            publisher_reference,
            normative_rule_inventory: Coverage::Unenumerated,
            schema_and_business_rule_manifest_frozen: false,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn full_matrix_is_unique_planned_and_independent() {
        let items: Vec<_> = target_registry().collect();
        assert_eq!(items.len(), 27);
        let ids: std::collections::BTreeSet<_> = items.iter().map(|item| item.id).collect();
        assert_eq!(ids.len(), 27);
        assert_eq!(
            items
                .iter()
                .filter(|item| item.roadmap_capability == TargetCapability::CheckOnlyPlanned)
                .count(),
            7
        );
        for item in items {
            assert_eq!(parse_target(item.id).unwrap(), item.target);
            assert_eq!(item.normative_rule_inventory, Coverage::Unenumerated);
            assert!(!item.baseline.amendments_and_errata_frozen);
        }
        assert_eq!(
            target_definition(StandardTarget::PdfUa1).dependency,
            TargetDependency::None
        );
        assert_eq!(
            target_definition(StandardTarget::PdfVt1).dependency,
            TargetDependency::Required(StandardTarget::PdfX4)
        );
        assert_eq!(
            target_definition(StandardTarget::PdfVt3).dependency,
            TargetDependency::Required(StandardTarget::PdfX6)
        );
        assert!(parse_target("pdfa-5").is_err());
        assert!(parse_target("factur-x").is_err());
        assert_eq!(packaging_registry().len(), 4);
    }
}
