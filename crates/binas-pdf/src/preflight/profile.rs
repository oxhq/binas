use binas_core::OperationContext;
use serde::Deserialize;

use crate::PdfError;

pub const IMAGE_RESOLUTION_PROFILE_VERSION: u32 = 1;
pub const IMAGE_RESOLUTION_PROFILE_MAX_JSON_BYTES: usize = 4096;

/// One minimum full-image-axis sampling density, in dots per inch.
/// Construction and JSON import both require a finite, strictly positive value.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ImageResolutionProfile {
    minimum_dpi: f64,
}

impl ImageResolutionProfile {
    pub fn new(minimum_dpi: f64) -> Result<Self, PdfError> {
        if !minimum_dpi.is_finite() || minimum_dpi <= 0.0 {
            return Err(PdfError::unsupported(
                "minimum image DPI must be finite and strictly positive",
            ));
        }
        Ok(Self { minimum_dpi })
    }

    pub fn minimum_dpi(self) -> f64 {
        self.minimum_dpi
    }

    pub fn version(self) -> u32 {
        IMAGE_RESOLUTION_PROFILE_VERSION
    }

    /// Imports one fixed-shape profile; no expression language or repair actions.
    /// The byte cap and cumulative work charge precede parsing or retained copies.
    pub fn from_json(bytes: &[u8], context: &OperationContext) -> Result<Self, PdfError> {
        context.checkpoint().map_err(PdfError::from_context)?;
        if bytes.len() > IMAGE_RESOLUTION_PROFILE_MAX_JSON_BYTES {
            return Err(PdfError::limit(
                "image preflight profile exceeds 4096 bytes",
            ));
        }
        context
            .charge_work(bytes.len() as u64 + 1)
            .map_err(PdfError::from_context)?;
        // The private wire value owns at most the bounded input's string data.
        // Reserve that logical storage before serde creates its strings.
        let _parsing = context
            .reserve((bytes.len() + std::mem::size_of::<WireProfile>()) as u64)
            .map_err(PdfError::from_context)?;
        let wire: WireProfile = serde_json::from_slice(bytes).map_err(|error| {
            PdfError::syntax(format!("invalid image preflight profile: {error}"), 0)
        })?;
        if wire.version != IMAGE_RESOLUTION_PROFILE_VERSION {
            return Err(PdfError::unsupported(
                "image preflight profile version is unsupported",
            ));
        }
        if wire.operator != "minimum_image_axis_dpi" {
            return Err(PdfError::unsupported(
                "image preflight operator is unsupported",
            ));
        }
        if wire.unit != "dpi" {
            return Err(PdfError::unsupported("image preflight unit must be dpi"));
        }
        context.checkpoint().map_err(PdfError::from_context)?;
        Self::new(wire.minimum)
    }
}

// Deliberately private: Deserialize on the validated public profile would bypass
// its constructor. The only valid shape is a root object with these four scalars.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireProfile {
    version: u32,
    operator: String,
    unit: String,
    minimum: f64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PdfErrorCode;
    use binas_core::{BudgetLimits, CancellationToken, ResourceBudget};

    fn context(work: u64) -> OperationContext {
        OperationContext::new(
            ResourceBudget::new(BudgetLimits {
                max_live_bytes: 1024,
                max_work_units: work,
            }),
            CancellationToken::new(),
        )
    }

    const VALID: &[u8] =
        br#"{"version":1,"operator":"minimum_image_axis_dpi","unit":"dpi","minimum":150}"#;

    #[test]
    fn profile_requires_finite_positive_dpi_and_one_known_check() {
        let profile = ImageResolutionProfile::from_json(VALID, &context(4096)).unwrap();
        assert_eq!(profile.minimum_dpi(), 150.0);
        assert_eq!(profile.version(), 1);
        for invalid in [0.0, -0.0, -1.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert!(ImageResolutionProfile::new(invalid).is_err());
        }
        for changed in [
            String::from_utf8(VALID.to_vec())
                .unwrap()
                .replace("\"version\":1", "\"version\":2"),
            String::from_utf8(VALID.to_vec())
                .unwrap()
                .replace("minimum_image_axis_dpi", "execute"),
            String::from_utf8(VALID.to_vec())
                .unwrap()
                .replace("\"dpi\"", "\"dpcm\""),
            String::from_utf8(VALID.to_vec())
                .unwrap()
                .replace("150", "-1"),
        ] {
            assert_eq!(
                ImageResolutionProfile::from_json(changed.as_bytes(), &context(4096))
                    .unwrap_err()
                    .code,
                PdfErrorCode::UnsupportedFeature
            );
        }
    }

    #[test]
    fn json_rejects_unknown_duplicate_nested_and_trailing_data() {
        let valid = std::str::from_utf8(VALID).unwrap();
        for invalid in [
            valid.replace("\"minimum\":150", "\"minimum\":150,\"expression\":\"true\""),
            valid.replace("\"minimum\":150", "\"minimum\":150,\"minimum\":1"),
            valid.replace("150", "{\"minimum\":150}"),
            valid.replace("150", "[150]"),
            valid.replace("150", "1e999"),
            valid.replace("150", "null"),
            format!("{valid} {{}}"),
            "[]".into(),
        ] {
            assert_eq!(
                ImageResolutionProfile::from_json(invalid.as_bytes(), &context(4096))
                    .unwrap_err()
                    .code,
                PdfErrorCode::InvalidSyntax
            );
        }
    }

    #[test]
    fn import_bounds_work_and_cancellation_precede_parsing() {
        let context = context(VALID.len() as u64 + 1);
        ImageResolutionProfile::from_json(VALID, &context).unwrap();
        let spent = context.budget().snapshot().work_units;
        assert_eq!(context.budget().snapshot().live_bytes, 0);
        assert_eq!(
            ImageResolutionProfile::from_json(VALID, &context)
                .unwrap_err()
                .code,
            PdfErrorCode::ResourceLimit
        );
        assert_eq!(context.budget().snapshot().work_units, spent);
        let oversized = vec![b' '; IMAGE_RESOLUTION_PROFILE_MAX_JSON_BYTES + 1];
        assert_eq!(
            ImageResolutionProfile::from_json(&oversized, &context)
                .unwrap_err()
                .code,
            PdfErrorCode::ResourceLimit
        );
        assert_eq!(context.budget().snapshot().work_units, spent);
        context.cancellation().cancel();
        assert_eq!(
            ImageResolutionProfile::from_json(b"malformed", &context)
                .unwrap_err()
                .code,
            PdfErrorCode::Cancelled
        );
        let no_memory = OperationContext::new(
            ResourceBudget::new(BudgetLimits {
                max_live_bytes: 0,
                max_work_units: 4096,
            }),
            CancellationToken::new(),
        );
        assert_eq!(
            ImageResolutionProfile::from_json(VALID, &no_memory)
                .unwrap_err()
                .code,
            PdfErrorCode::ResourceLimit
        );
        assert_eq!(no_memory.budget().snapshot().live_bytes, 0);
    }
}
