use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::{
    PdfDocument, PdfError,
    annotations::list_annotations,
    forms::list_form_fields,
    parser::Value,
    security::{EncryptionMetadata, inspect_encryption},
    signatures::inspect_signatures,
    xfa::{inspect_xfa_dynamic, list_xfa_packets},
};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityDecision {
    Supported,
    Conditional,
    Refused,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct OperationCapability {
    pub operation: String,
    pub decision: CapabilityDecision,
    pub reason: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct DocumentCapabilityProfile {
    pub profile_version: u32,
    pub pdf_version: String,
    pub object_count: usize,
    pub page_count: usize,
    pub xref_revisions: usize,
    pub filter_names: Vec<String>,
    pub form_field_count: usize,
    pub annotation_count: usize,
    pub xfa_packet_count: usize,
    pub xfa_dynamic: bool,
    pub encryption: EncryptionMetadata,
    pub signature_count: usize,
    pub cryptographically_verified_signature_count: usize,
    pub operations: Vec<OperationCapability>,
}

impl DocumentCapabilityProfile {
    pub const VERSION: u32 = 1;

    pub fn operation(&self, name: &str) -> Option<&OperationCapability> {
        self.operations
            .iter()
            .find(|capability| capability.operation == name)
    }
}

impl PdfDocument {
    pub fn capability_profile(&self) -> Result<DocumentCapabilityProfile, PdfError> {
        let inspected = self.inspect()?;
        let encryption = inspect_encryption(self)?;
        let signatures = inspect_signatures(self)?;
        let signed = !signatures.is_empty();
        let fields = list_form_fields(self)?;
        let annotations = list_annotations(self)?;
        let xfa = inspect_xfa_dynamic(self)?;
        let xfa_packets = list_xfa_packets(self)?;
        let filter_names = filter_names(self)?;
        let operations =
            operation_capabilities(&encryption, signed, !xfa_packets.is_empty(), xfa.dynamic);
        Ok(DocumentCapabilityProfile {
            profile_version: Self::capability_profile_version(),
            pdf_version: inspected.version,
            object_count: inspected.object_count,
            page_count: inspected.page_count,
            xref_revisions: inspected.xref_revisions,
            filter_names,
            form_field_count: fields.len(),
            annotation_count: annotations.len(),
            xfa_packet_count: xfa_packets.len(),
            xfa_dynamic: xfa.dynamic,
            encryption,
            signature_count: signatures.len(),
            cryptographically_verified_signature_count: signatures
                .iter()
                .filter(|signature| signature.cms_verified)
                .count(),
            operations,
        })
    }

    const fn capability_profile_version() -> u32 {
        DocumentCapabilityProfile::VERSION
    }
}

fn filter_names(document: &PdfDocument) -> Result<Vec<String>, PdfError> {
    fn visit(
        value: &Value,
        output: &mut BTreeSet<String>,
        depth: usize,
        max_depth: usize,
        max_items: usize,
    ) -> Result<(), PdfError> {
        if depth > max_depth {
            return Err(PdfError::limit(
                "capability filter scan exceeds depth limit",
            ));
        }
        match value {
            Value::Array(values) => {
                for value in values {
                    visit(value, output, depth + 1, max_depth, max_items)?;
                }
            }
            Value::Dict(dictionary) => {
                if let Some(filter) = dictionary.get(b"Filter".as_slice()) {
                    match filter {
                        Value::Name(name) => {
                            output.insert(String::from_utf8_lossy(name).into_owned());
                        }
                        Value::Array(filters) => {
                            for filter in filters {
                                if let Value::Name(name) = filter {
                                    output.insert(String::from_utf8_lossy(name).into_owned());
                                }
                            }
                        }
                        _ => {}
                    }
                    if output.len() > max_items {
                        return Err(PdfError::limit(
                            "capability filter count exceeds container limit",
                        ));
                    }
                }
                for value in dictionary.values() {
                    visit(value, output, depth + 1, max_depth, max_items)?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    let mut output = BTreeSet::new();
    for object in document.parsed().objects.values() {
        visit(
            &object.value,
            &mut output,
            0,
            document.parsed().limits.max_parser_depth,
            document.parsed().limits.max_container_items,
        )?;
    }
    Ok(output.into_iter().collect())
}

fn operation_capabilities(
    encryption: &EncryptionMetadata,
    signed: bool,
    has_xfa: bool,
    dynamic_xfa: bool,
) -> Vec<OperationCapability> {
    let mutation_refusal = if encryption.encrypted {
        Some("encrypted PDFs require authenticated decryption before rewriting")
    } else if signed {
        Some("signed PDFs require an explicit signature-preservation policy")
    } else {
        None
    };
    let mut output = vec![
        capability(
            "inspect",
            CapabilityDecision::Supported,
            "document parsed within configured resource limits",
        ),
        capability(
            "query_text",
            CapabilityDecision::Conditional,
            "supported when page content filters, fonts, and character mappings are decodable",
        ),
        mutation_capability("canonicalize", mutation_refusal),
        mutation_capability("page_operations", mutation_refusal),
        mutation_capability("annotation_mutation", mutation_refusal),
        mutation_capability("stream_mutation", mutation_refusal),
        mutation_capability("image_replacement", mutation_refusal),
    ];
    output.push(if let Some(reason) = mutation_refusal {
        capability("form_lifecycle", CapabilityDecision::Refused, reason)
    } else if has_xfa {
        capability(
            "form_lifecycle",
            CapabilityDecision::Refused,
            "AcroForm lifecycle mutation refuses PDFs containing XFA",
        )
    } else {
        capability(
            "form_lifecycle",
            CapabilityDecision::Supported,
            "AcroForm lifecycle mutation is available with operation-specific validation",
        )
    });
    output.push(if encryption.encrypted {
        capability(
            "ocr_text_layer",
            CapabilityDecision::Refused,
            "OCR text layers require authenticated decryption first",
        )
    } else if signed {
        capability(
            "ocr_text_layer",
            CapabilityDecision::Refused,
            "OCR text layers refuse signed PDFs",
        )
    } else if dynamic_xfa {
        capability(
            "ocr_text_layer",
            CapabilityDecision::Refused,
            "OCR text layers refuse dynamic XFA PDFs",
        )
    } else {
        capability(
            "ocr_text_layer",
            CapabilityDecision::Conditional,
            "supported for unrotated pages with caller-supplied bounded ASCII text boxes",
        )
    });
    output.push(if encryption.encrypted {
        capability(
            "external_signing",
            CapabilityDecision::Refused,
            "external signing requires an unencrypted PDF",
        )
    } else {
        capability(
            "external_signing",
            CapabilityDecision::Supported,
            "caller supplies detached CMS; signer and network access are outside Binas",
        )
    });
    output.push(if encryption.encrypted {
        capability(
            "standard_encrypt",
            CapabilityDecision::Refused,
            "standard encryption requires an unencrypted input PDF",
        )
    } else if signed {
        capability(
            "standard_encrypt",
            CapabilityDecision::Refused,
            "standard encryption would rewrite signed bytes",
        )
    } else {
        capability(
            "standard_encrypt",
            CapabilityDecision::Supported,
            "Standard Security revisions R2 through R6 are available",
        )
    });
    output.push(decryption_capability(encryption));
    output
}

fn decryption_capability(encryption: &EncryptionMetadata) -> OperationCapability {
    if !encryption.encrypted {
        return capability(
            "decrypt",
            CapabilityDecision::Refused,
            "document is not encrypted",
        );
    }
    match encryption.filter.as_deref() {
        Some("Standard") if matches!(encryption.revision, Some(2..=6)) => capability(
            "decrypt",
            CapabilityDecision::Supported,
            "caller must supply a valid Standard Security user or owner password",
        ),
        Some("Adobe.PubSec") => capability(
            "decrypt",
            CapabilityDecision::Supported,
            "caller must supply a matching recipient certificate and private key",
        ),
        _ => capability(
            "decrypt",
            CapabilityDecision::Refused,
            "encryption security handler or revision is unsupported",
        ),
    }
}

fn mutation_capability(operation: &str, refusal: Option<&str>) -> OperationCapability {
    match refusal {
        Some(reason) => capability(operation, CapabilityDecision::Refused, reason),
        None => capability(
            operation,
            CapabilityDecision::Conditional,
            "available when operation-specific structure and filter checks pass",
        ),
    }
}

fn capability(operation: &str, decision: CapabilityDecision, reason: &str) -> OperationCapability {
    OperationCapability {
        operation: operation.into(),
        decision,
        reason: reason.into(),
    }
}
