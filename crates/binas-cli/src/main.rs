use std::{fs, path::PathBuf, process::ExitCode};

use binas_pdf::{
    AnnotationContentsMutationRequest, AnnotationCreateRequest, AnnotationRemoveRequest,
    AnnotationSubtype, BlankPageSize, EncodedImageReplacementRequest, EncryptionMetadata,
    ExternalSignatureFieldOptions, ExternalSignaturePlan, ExternalSignaturePlanDescriptor,
    FilteredTextEditRequest, FontTextEditRequest, FormFieldCreateRequest, FormFieldKind,
    FormFieldRemoveRequest, FormValueMutationRequest, FreeTextAppearanceRequest, ImageColorSpace,
    ImageDecodeParams, ImageFilter, ImageMaskPolicy, ImageReplacementRequest,
    IncrementalTextEditRequest, OcrParseLimits, OpenOptions, OverlayStampRequest, PageTransform,
    PdfDocument, PdfEngine, PdfErrorCode, PublicKeyEncryptionMethod, PublicKeyEncryptionOptions,
    SignatureTrustOptions, StandardEncryptionOptions, StandardEncryptionRevision,
    StreamMutationRequest, SurgicalTextEditRequest, TextFieldAppearanceRequest, TextOverlayRequest,
    XfaDatasetField, XfaDatasetSetRequest, XfaPacket, XfaReplaceRequest, inspect_encryption,
    inspect_signatures, inspect_signatures_with_options, list_annotations, list_form_fields,
    list_xfa_dataset_fields, list_xfa_packets, parse_alto_xml, parse_ocr_json,
};
use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "binas")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Inspect {
        input: PathBuf,
        #[arg(long, default_value = "pdf")]
        format: String,
        #[arg(long)]
        password: Option<String>,
        #[arg(long)]
        repair: bool,
        #[arg(long)]
        json: bool,
    },
    Profile {
        input: PathBuf,
        #[arg(long)]
        json: bool,
    },
    ExtractText {
        input: PathBuf,
        #[arg(long)]
        password: Option<String>,
        #[arg(long)]
        repair: bool,
        #[arg(long)]
        json: bool,
    },
    Query {
        input: PathBuf,
        #[arg(long, default_value = "pdf")]
        format: String,
        #[arg(long, default_value = "pdf.content.text_show")]
        kind: String,
        #[arg(long)]
        text: String,
        #[arg(long)]
        password: Option<String>,
        #[arg(long)]
        repair: bool,
        #[arg(long)]
        match_index: Option<usize>,
        #[arg(long = "meta")]
        meta: Vec<String>,
        #[arg(long)]
        json: bool,
    },
    Edit {
        input: PathBuf,
        #[arg(long, default_value = "pdf")]
        format: String,
        #[arg(long)]
        password: Option<String>,
        #[arg(long)]
        text: String,
        #[arg(long)]
        replace: String,
        #[arg(long, default_value_t = 0)]
        match_index: usize,
        #[arg(long, default_value = "auto")]
        rewrite: String,
        #[arg(short = 'o')]
        output: PathBuf,
        #[arg(long)]
        json: bool,
    },
    Form {
        #[command(subcommand)]
        command: FormCommand,
    },
    Annot {
        #[command(subcommand)]
        command: AnnotCommand,
    },
    Signature {
        #[command(subcommand)]
        command: SignatureCommand,
    },
    Page {
        #[command(subcommand)]
        command: PageCommand,
    },
    Xfa {
        #[command(subcommand)]
        command: XfaCommand,
    },
    Stream {
        #[command(subcommand)]
        command: StreamCommand,
    },
    Image {
        #[command(subcommand)]
        command: ImageCommand,
    },
    Ocr {
        #[command(subcommand)]
        command: OcrCommand,
    },
    Overlay {
        #[command(subcommand)]
        command: OverlayCommand,
    },
    Validate {
        input: PathBuf,
        #[arg(long, default_value = "pdf")]
        format: String,
        #[arg(long)]
        password: Option<String>,
        #[arg(long)]
        repair: bool,
        #[arg(long)]
        json: bool,
        #[arg(long)]
        fail_on_invalid: bool,
    },
    Encrypt {
        input: PathBuf,
        #[arg(long, default_value = "pdf")]
        format: String,
        #[arg(long, default_value = "r4-aesv2")]
        revision: String,
        #[arg(long)]
        user_password: String,
        #[arg(long)]
        owner_password: String,
        #[arg(long, default_value_t = -4)]
        permissions: i32,
        #[arg(short = 'o')]
        output: PathBuf,
        #[arg(long)]
        json: bool,
    },
    Decrypt {
        input: PathBuf,
        #[arg(long, default_value = "pdf")]
        format: String,
        #[arg(long)]
        password: String,
        #[arg(short = 'o')]
        output: PathBuf,
        #[arg(long)]
        json: bool,
    },
    ChangePassword {
        input: PathBuf,
        #[arg(long)]
        old_password: String,
        #[arg(long)]
        new_user_password: String,
        #[arg(long)]
        new_owner_password: String,
        #[arg(short = 'o')]
        output: PathBuf,
        #[arg(long)]
        json: bool,
    },
    EncryptPublicKey {
        input: PathBuf,
        #[arg(long, default_value = "pdf")]
        format: String,
        #[arg(long, default_value = "aesv2")]
        method: String,
        #[arg(long = "recipient", required = true)]
        recipients: Vec<PathBuf>,
        #[arg(long, default_value_t = -4)]
        permissions: i32,
        #[arg(short = 'o')]
        output: PathBuf,
        #[arg(long)]
        json: bool,
    },
    DecryptPublicKey {
        input: PathBuf,
        #[arg(long, default_value = "pdf")]
        format: String,
        #[arg(long)]
        certificate: PathBuf,
        #[arg(long)]
        private_key: PathBuf,
        #[arg(short = 'o')]
        output: PathBuf,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Debug, Subcommand)]
enum PageCommand {
    Create {
        #[arg(long)]
        width: f64,
        #[arg(long)]
        height: f64,
        #[arg(short = 'o')]
        output: PathBuf,
    },
    Extract {
        input: PathBuf,
        #[arg(long, value_delimiter = ',', required = true)]
        pages: Vec<usize>,
        #[arg(short = 'o')]
        output: PathBuf,
        #[arg(long)]
        json: bool,
    },
    Insert {
        input: PathBuf,
        #[arg(long)]
        source: PathBuf,
        #[arg(long)]
        index: usize,
        #[arg(long = "source-pages", value_delimiter = ',', required = true)]
        source_pages: Vec<usize>,
        #[arg(short = 'o')]
        output: PathBuf,
        #[arg(long)]
        json: bool,
    },
    Merge {
        #[arg(required = true, num_args = 2..)]
        inputs: Vec<PathBuf>,
        #[arg(short = 'o')]
        output: PathBuf,
        #[arg(long)]
        json: bool,
    },
    Transform {
        input: PathBuf,
        #[arg(long, value_delimiter = ',', required = true)]
        pages: Vec<usize>,
        #[arg(long)]
        rotation: Option<i32>,
        #[arg(long, num_args = 4)]
        media_box: Option<Vec<f64>>,
        #[arg(long, num_args = 4)]
        crop_box: Option<Vec<f64>>,
        #[arg(long, num_args = 2)]
        translate: Option<Vec<f64>>,
        #[arg(long, num_args = 2)]
        scale: Option<Vec<f64>>,
        #[arg(short = 'o')]
        output: PathBuf,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Debug, Subcommand)]
enum XfaCommand {
    List {
        input: PathBuf,
        #[arg(long)]
        json: bool,
    },
    Datasets {
        input: PathBuf,
        #[arg(long)]
        json: bool,
    },
    DatasetSet {
        input: PathBuf,
        #[arg(long)]
        path: String,
        #[arg(long)]
        value: String,
        #[arg(short = 'o')]
        output: PathBuf,
        #[arg(long)]
        json: bool,
    },
    Replace {
        input: PathBuf,
        #[arg(long, default_value_t = 0)]
        packet_index: usize,
        #[arg(long)]
        text: String,
        #[arg(long)]
        replace: String,
        #[arg(short = 'o')]
        output: PathBuf,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Debug, Subcommand)]
enum StreamCommand {
    Mutate {
        input: PathBuf,
        #[arg(long)]
        object: u32,
        #[arg(long, default_value_t = 0)]
        generation: u16,
        #[arg(long)]
        data: PathBuf,
        #[arg(short = 'o')]
        output: PathBuf,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Debug, Subcommand)]
enum ImageCommand {
    ReplaceEncoded {
        input: PathBuf,
        #[arg(long)]
        object: u32,
        #[arg(long, default_value_t = 0)]
        generation: u16,
        #[arg(long)]
        data: PathBuf,
        #[arg(long, default_value = "reject")]
        mask_policy: String,
        #[arg(short = 'o')]
        output: PathBuf,
        #[arg(long)]
        json: bool,
    },
    Replace {
        input: PathBuf,
        #[arg(long)]
        object: u32,
        #[arg(long, default_value_t = 0)]
        generation: u16,
        #[arg(long)]
        data: PathBuf,
        #[arg(long)]
        width: u32,
        #[arg(long)]
        height: u32,
        #[arg(long, default_value_t = 8)]
        bits: u8,
        #[arg(long)]
        color_space: String,
        #[arg(long)]
        filter: String,
        #[arg(long)]
        predictor: Option<u8>,
        #[arg(long, default_value = "reject")]
        mask_policy: String,
        #[arg(short = 'o')]
        output: PathBuf,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Debug, Subcommand)]
enum OcrCommand {
    Apply {
        input: PathBuf,
        #[arg(long)]
        source: PathBuf,
        #[arg(long, default_value = "json")]
        source_format: String,
        #[arg(short = 'o')]
        output: PathBuf,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Debug, Subcommand)]
enum OverlayCommand {
    Stamp {
        input: PathBuf,
        #[arg(long, value_delimiter = ',', required = true)]
        pages: Vec<usize>,
        #[arg(long)]
        content: PathBuf,
        #[arg(long, num_args = 4)]
        bbox: Vec<f64>,
        #[arg(long, num_args = 6)]
        matrix: Vec<f64>,
        #[arg(long)]
        opacity: Option<f64>,
        #[arg(short = 'o')]
        output: PathBuf,
        #[arg(long)]
        json: bool,
    },
    Text {
        input: PathBuf,
        #[arg(long)]
        page_index: usize,
        #[arg(long)]
        text: String,
        #[arg(long)]
        x: f64,
        #[arg(long)]
        y: f64,
        #[arg(long)]
        font_size: f64,
        #[arg(short = 'o')]
        output: PathBuf,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Debug, Subcommand)]
enum FormCommand {
    List {
        input: PathBuf,
        #[arg(long, default_value = "pdf")]
        format: String,
        #[arg(long)]
        json: bool,
    },
    Set {
        input: PathBuf,
        #[arg(long, default_value = "pdf")]
        format: String,
        #[arg(long)]
        field: String,
        #[arg(long)]
        value: String,
        #[arg(long, default_value_t = 0)]
        match_index: usize,
        #[arg(long)]
        regenerate_appearance: bool,
        #[arg(short = 'o')]
        output: PathBuf,
        #[arg(long)]
        json: bool,
    },
    Create {
        input: PathBuf,
        #[arg(long)]
        name: String,
        #[arg(long)]
        kind: String,
        #[arg(long, default_value_t = 0)]
        page: usize,
        #[arg(long, num_args = 4)]
        rect: Vec<f64>,
        #[arg(long, default_value = "")]
        value: String,
        #[arg(long = "option")]
        options: Vec<String>,
        #[arg(short = 'o')]
        output: PathBuf,
        #[arg(long)]
        json: bool,
    },
    Remove {
        input: PathBuf,
        #[arg(long)]
        field: String,
        #[arg(long, default_value_t = 0)]
        match_index: usize,
        #[arg(short = 'o')]
        output: PathBuf,
        #[arg(long)]
        json: bool,
    },
    Flatten {
        input: PathBuf,
        #[arg(short = 'o')]
        output: PathBuf,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Debug, Subcommand)]
enum AnnotCommand {
    List {
        input: PathBuf,
        #[arg(long, default_value = "pdf")]
        format: String,
        #[arg(long)]
        json: bool,
    },
    SetContents {
        input: PathBuf,
        #[arg(long, default_value = "pdf")]
        format: String,
        #[arg(long)]
        index: usize,
        #[arg(long)]
        contents: String,
        #[arg(long)]
        regenerate_appearance: bool,
        #[arg(short = 'o')]
        output: PathBuf,
        #[arg(long)]
        json: bool,
    },
    Create {
        input: PathBuf,
        #[arg(long)]
        subtype: String,
        #[arg(long, default_value_t = 0)]
        page: usize,
        #[arg(long, num_args = 4)]
        rect: Vec<f64>,
        #[arg(long, default_value = "")]
        contents: String,
        #[arg(long, value_delimiter = ',')]
        quad_points: Vec<f64>,
        #[arg(long, default_value = "")]
        uri: String,
        #[arg(short = 'o')]
        output: PathBuf,
        #[arg(long)]
        json: bool,
    },
    Remove {
        input: PathBuf,
        #[arg(long)]
        index: usize,
        #[arg(short = 'o')]
        output: PathBuf,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Debug, Subcommand)]
enum SignatureCommand {
    Inspect {
        input: PathBuf,
        #[arg(long, default_value = "pdf")]
        format: String,
        #[arg(long = "trust-root")]
        trust_roots: Vec<PathBuf>,
        #[arg(long)]
        system_trust: bool,
        #[arg(long = "trust-intermediate")]
        trust_intermediates: Vec<PathBuf>,
        #[arg(long = "aia-intermediate")]
        fetched_intermediates: Vec<PathBuf>,
        #[arg(long = "crl")]
        crls: Vec<PathBuf>,
        #[arg(long = "ocsp")]
        ocsp_responses: Vec<PathBuf>,
        #[arg(long = "tsa-root")]
        tsa_roots: Vec<PathBuf>,
        #[arg(long)]
        tsa_system_trust: bool,
        #[arg(long = "tsa-intermediate")]
        tsa_intermediates: Vec<PathBuf>,
        #[arg(long = "tsa-aia-intermediate")]
        tsa_fetched_intermediates: Vec<PathBuf>,
        #[arg(long = "tsa-crl")]
        tsa_crls: Vec<PathBuf>,
        #[arg(long = "tsa-ocsp")]
        tsa_ocsp_responses: Vec<PathBuf>,
        #[arg(long)]
        validation_time_unix: Option<u64>,
        #[arg(long)]
        json: bool,
    },
    Prepare {
        input: PathBuf,
        #[arg(long, default_value_t = 16_384)]
        reserve: usize,
        #[arg(long)]
        field: Option<String>,
        #[arg(long, default_value_t = 0)]
        page: usize,
        #[arg(long, num_args = 4)]
        rect: Vec<f64>,
        #[arg(short = 'o')]
        output: PathBuf,
        #[arg(long)]
        plan: PathBuf,
        #[arg(long)]
        digest: PathBuf,
    },
    Apply {
        input: PathBuf,
        #[arg(long)]
        plan: PathBuf,
        #[arg(long)]
        cms: PathBuf,
        #[arg(short = 'o')]
        output: PathBuf,
    },
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("binas: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<(), String> {
    match cli.command {
        Command::Profile { input, json } => {
            let bytes = fs::read(input).map_err(|error| error.to_string())?;
            let document = PdfEngine::default()
                .open(&bytes, OpenOptions::default())
                .map_err(|error| error.to_string())?;
            let profile = document
                .capability_profile()
                .map_err(|error| error.to_string())?;
            if json {
                println!(
                    "{}",
                    serde_json::to_string(&profile).map_err(|error| error.to_string())?
                );
            } else {
                println!(
                    "PDF {} pages={} objects={} signatures={} fields={} annotations={}",
                    profile.pdf_version,
                    profile.page_count,
                    profile.object_count,
                    profile.signature_count,
                    profile.form_field_count,
                    profile.annotation_count,
                );
                for operation in profile.operations {
                    println!(
                        "{} {:?}: {}",
                        operation.operation, operation.decision, operation.reason
                    );
                }
            }
            Ok(())
        }
        Command::ExtractText {
            input,
            password,
            repair,
            json,
        } => {
            let bytes = fs::read(input).map_err(|error| error.to_string())?;
            let (document, _) = open_pdf(&bytes, password.as_deref(), OpenOptions { repair })?;
            let extraction = document
                .extract_text_spans()
                .map_err(|error| error.to_string())?;
            if json {
                println!(
                    "{}",
                    serde_json::to_string(&extraction).map_err(|error| error.to_string())?
                );
            } else {
                for span in extraction.spans {
                    println!("{}\t{}", span.page_index, span.text);
                }
                for warning in extraction.warnings {
                    eprintln!("warning: {}", warning.message);
                }
            }
            Ok(())
        }
        Command::Inspect {
            input,
            format,
            password,
            repair,
            json,
        } => {
            require_pdf(&format)?;
            let bytes = fs::read(input).map_err(|error| error.to_string())?;
            let original = PdfEngine::default()
                .open(&bytes, OpenOptions { repair })
                .map_err(|error| error.to_string())?;
            let signatures = inspect_signatures(&original).map_err(|error| error.to_string())?;
            let (document, encryption) =
                open_pdf(&bytes, password.as_deref(), OpenOptions { repair })?;
            let result = document.inspect().map_err(|error| error.to_string())?;
            if json {
                println!(
                    "{}",
                    serde_json::json!({
                        "format": "pdf",
                        "nodes": result.object_count,
                        "version": result.version,
                        "pages": result.page_count,
                        "xref_revisions": result.xref_revisions,
                        "encryption": encryption,
                        "signatures": signatures,
                    })
                );
            } else {
                println!(
                    "pdf nodes={} encrypted={} signatures={}",
                    result.object_count,
                    encryption.encrypted,
                    signatures.len()
                );
            }
            Ok(())
        }
        Command::Form {
            command:
                FormCommand::Set {
                    input,
                    format,
                    field,
                    value,
                    match_index,
                    regenerate_appearance,
                    output,
                    json,
                },
        } => {
            require_pdf(&format)?;
            let bytes = fs::read(input).map_err(|error| error.to_string())?;
            let document = PdfEngine::default()
                .open(&bytes, OpenOptions::default())
                .map_err(|error| error.to_string())?;
            let (edited, report, verification) = if regenerate_appearance {
                let outcome = document
                    .regenerate_text_field_appearance(TextFieldAppearanceRequest {
                        field_name: field,
                        value,
                        match_index,
                    })
                    .map_err(|error| error.to_string())?;
                outcome_parts(outcome.bytes, outcome.report, outcome.verification)?
            } else {
                let outcome = document
                    .set_form_field_value(FormValueMutationRequest {
                        field_name: field,
                        value,
                        match_index,
                    })
                    .map_err(|error| error.to_string())?;
                outcome_parts(outcome.bytes, outcome.report, outcome.verification)?
            };
            fs::write(&output, &edited).map_err(|error| error.to_string())?;
            if json {
                println!(
                    "{}",
                    serde_json::json!({
                        "output": output,
                        "report": report,
                        "verification": verification,
                    })
                );
            } else {
                println!("wrote {}", output.display());
            }
            Ok(())
        }
        Command::Query {
            input,
            format,
            kind,
            text,
            password,
            repair,
            match_index,
            meta,
            json,
        } => {
            require_pdf(&format)?;
            if kind != "pdf.content.text_show" {
                return Err(format!("unsupported query kind {kind:?}"));
            }
            for filter in &meta {
                validate_query_meta(filter)?;
            }
            let bytes = fs::read(input).map_err(|error| error.to_string())?;
            let (document, _) = open_pdf(&bytes, password.as_deref(), OpenOptions { repair })?;
            let all_matches = document
                .query_text_all(&text)
                .map_err(|error| error.to_string())?
                .into_iter()
                .filter(|value| {
                    meta.iter().all(|filter| {
                        value.operator == filter.split_once('=').expect("validated filter").1
                    })
                })
                .collect::<Vec<_>>();
            let total_matches = all_matches.len();
            let matches = match match_index {
                Some(index) => vec![all_matches.get(index).cloned().ok_or_else(|| {
                    format!(
                        "match index {index} out of range for {total_matches} matches (zero-based)"
                    )
                })?],
                None => all_matches,
            };
            if json {
                let mut output = serde_json::json!({ "count": matches.len(), "matches": matches });
                if let Some(index) = match_index {
                    output["match_index"] = index.into();
                    output["total_matches"] = total_matches.into();
                }
                println!("{output}");
            } else {
                println!("matches={}", matches.len());
            }
            Ok(())
        }
        Command::Form {
            command:
                FormCommand::Create {
                    input,
                    name,
                    kind,
                    page,
                    rect,
                    value,
                    options,
                    output,
                    json,
                },
        } => {
            let bytes = fs::read(input).map_err(|error| error.to_string())?;
            let document = PdfEngine::default()
                .open(&bytes, OpenOptions::default())
                .map_err(|error| error.to_string())?;
            let outcome = document
                .create_form_field(FormFieldCreateRequest {
                    name,
                    page_index: page,
                    rect: fixed_array(Some(rect), "rect")?
                        .ok_or_else(|| "--rect is required".to_string())?,
                    kind: form_field_kind(&kind)?,
                    value,
                    options,
                })
                .map_err(|error| error.to_string())?;
            write_outcome(
                &output,
                outcome.bytes,
                outcome.report,
                outcome.verification,
                json,
            )
        }
        Command::Form {
            command:
                FormCommand::Remove {
                    input,
                    field,
                    match_index,
                    output,
                    json,
                },
        } => {
            let bytes = fs::read(input).map_err(|error| error.to_string())?;
            let document = PdfEngine::default()
                .open(&bytes, OpenOptions::default())
                .map_err(|error| error.to_string())?;
            let outcome = document
                .remove_form_field(FormFieldRemoveRequest {
                    field_name: field,
                    match_index,
                })
                .map_err(|error| error.to_string())?;
            write_outcome(
                &output,
                outcome.bytes,
                outcome.report,
                outcome.verification,
                json,
            )
        }
        Command::Form {
            command:
                FormCommand::Flatten {
                    input,
                    output,
                    json,
                },
        } => {
            let bytes = fs::read(input).map_err(|error| error.to_string())?;
            let document = PdfEngine::default()
                .open(&bytes, OpenOptions::default())
                .map_err(|error| error.to_string())?;
            let outcome = document
                .flatten_form_fields()
                .map_err(|error| error.to_string())?;
            write_outcome(
                &output,
                outcome.bytes,
                outcome.report,
                outcome.verification,
                json,
            )
        }
        Command::Validate {
            input,
            format,
            password,
            repair,
            json,
            fail_on_invalid,
        } => {
            require_pdf(&format)?;
            let bytes = fs::read(input).map_err(|error| error.to_string())?;
            let result = open_pdf(&bytes, password.as_deref(), OpenOptions { repair })
                .and_then(|(document, _)| document.validate().map_err(|error| error.to_string()));
            let valid = result.as_ref().is_ok_and(|report| report.valid);
            if json {
                let output = match result {
                    Ok(report) => serde_json::json!({
                        "format": "pdf",
                        "valid": true,
                        "errors": [],
                        "warnings": [],
                        "objects": report.object_count,
                        "pages": report.page_count,
                    }),
                    Err(error) => serde_json::json!({
                        "format": "pdf",
                        "valid": false,
                        "errors": [error.to_string()],
                        "warnings": [],
                    }),
                };
                println!("{output}");
            } else {
                println!("pdf valid={valid}");
            }
            if fail_on_invalid && !valid {
                return Err("validation failed".into());
            }
            Ok(())
        }
        Command::Edit {
            input,
            format,
            password,
            text,
            replace,
            match_index,
            rewrite,
            output,
            json,
        } => {
            require_pdf(&format)?;
            let bytes = fs::read(input).map_err(|error| error.to_string())?;
            let (document, _) = open_pdf(&bytes, password.as_deref(), OpenOptions::default())?;
            let (edited, report, verification) =
                edit_text(&document, &text, &replace, match_index, &rewrite)?;
            fs::write(&output, &edited).map_err(|error| error.to_string())?;
            if json {
                println!(
                    "{}",
                    serde_json::json!({
                        "output": output,
                        "report": report,
                        "verification": verification,
                    })
                );
            } else {
                println!("wrote {} bytes to {}", edited.len(), output.display());
            }
            Ok(())
        }
        Command::Form {
            command:
                FormCommand::List {
                    input,
                    format,
                    json,
                },
        } => {
            require_pdf(&format)?;
            let bytes = fs::read(input).map_err(|error| error.to_string())?;
            let document = PdfEngine::default()
                .open(&bytes, OpenOptions::default())
                .map_err(|error| error.to_string())?;
            let fields = list_form_fields(&document).map_err(|error| error.to_string())?;
            if json {
                println!(
                    "{}",
                    serde_json::json!({ "count": fields.len(), "fields": fields })
                );
            } else {
                for field in fields {
                    println!(
                        "{} name={:?} type={:?} value={:?}",
                        field.index, field.name, field.field_type, field.value
                    );
                }
            }
            Ok(())
        }
        Command::Annot {
            command:
                AnnotCommand::List {
                    input,
                    format,
                    json,
                },
        } => {
            require_pdf(&format)?;
            let bytes = fs::read(input).map_err(|error| error.to_string())?;
            let document = PdfEngine::default()
                .open(&bytes, OpenOptions::default())
                .map_err(|error| error.to_string())?;
            let annotations = list_annotations(&document).map_err(|error| error.to_string())?;
            if json {
                println!(
                    "{}",
                    serde_json::json!({ "count": annotations.len(), "annotations": annotations })
                );
            } else {
                for annotation in annotations {
                    println!(
                        "{} page={} subtype={} contents={:?}",
                        annotation.index,
                        annotation.page_index,
                        annotation.subtype,
                        annotation.contents
                    );
                }
            }
            Ok(())
        }
        Command::Signature {
            command:
                SignatureCommand::Prepare {
                    input,
                    reserve,
                    field,
                    page,
                    rect,
                    output,
                    plan,
                    digest,
                },
        } => {
            let bytes = fs::read(input).map_err(|error| error.to_string())?;
            let document = PdfEngine::default()
                .open(&bytes, OpenOptions::default())
                .map_err(|error| error.to_string())?;
            let rect = if rect.is_empty() {
                [0.0; 4]
            } else {
                fixed_array(Some(rect), "rect")?
                    .ok_or_else(|| "--rect requires four numbers".to_string())?
            };
            let prepared = document
                .prepare_external_signature_with_field(
                    reserve,
                    ExternalSignatureFieldOptions {
                        field_name: field,
                        page_index: page,
                        rect,
                    },
                )
                .map_err(|error| error.to_string())?;
            fs::write(&output, &prepared.bytes).map_err(|error| error.to_string())?;
            fs::write(&digest, &prepared.digest_to_sign).map_err(|error| error.to_string())?;
            fs::write(
                &plan,
                serde_json::to_vec_pretty(&prepared.descriptor())
                    .map_err(|error| error.to_string())?,
            )
            .map_err(|error| error.to_string())?;
            println!("prepared {}", output.display());
            Ok(())
        }
        Command::Signature {
            command:
                SignatureCommand::Apply {
                    input,
                    plan,
                    cms,
                    output,
                },
        } => {
            let bytes = fs::read(input).map_err(|error| error.to_string())?;
            let descriptor: ExternalSignaturePlanDescriptor =
                serde_json::from_slice(&fs::read(plan).map_err(|error| error.to_string())?)
                    .map_err(|error| error.to_string())?;
            let prepared = ExternalSignaturePlan::from_prepared_pdf(bytes, descriptor)
                .map_err(|error| error.to_string())?;
            let cms = fs::read(cms).map_err(|error| error.to_string())?;
            let applied = prepared
                .apply_cms(&cms)
                .map_err(|error| error.to_string())?;
            fs::write(&output, applied.bytes).map_err(|error| error.to_string())?;
            println!("wrote {}", output.display());
            Ok(())
        }
        Command::Signature {
            command:
                SignatureCommand::Inspect {
                    input,
                    format,
                    trust_roots,
                    system_trust,
                    trust_intermediates,
                    fetched_intermediates,
                    crls,
                    ocsp_responses,
                    tsa_roots,
                    tsa_system_trust,
                    tsa_intermediates,
                    tsa_fetched_intermediates,
                    tsa_crls,
                    tsa_ocsp_responses,
                    validation_time_unix,
                    json,
                },
        } => {
            require_pdf(&format)?;
            let bytes = fs::read(input).map_err(|error| error.to_string())?;
            let document = PdfEngine::default()
                .open(&bytes, OpenOptions::default())
                .map_err(|error| error.to_string())?;
            let trust = SignatureTrustOptions {
                roots_der: read_der_files(&trust_roots, "trust root")?,
                os_roots_der: native_roots(system_trust)?,
                intermediates_der: read_der_files(&trust_intermediates, "trust intermediate")?,
                fetched_intermediates_der: read_der_files(
                    &fetched_intermediates,
                    "AIA intermediate",
                )?,
                crls_der: read_der_files(&crls, "CRL")?,
                ocsp_responses_der: read_der_files(&ocsp_responses, "OCSP response")?,
                validation_time_unix,
                tsa_roots_der: read_der_files(&tsa_roots, "TSA root")?,
                tsa_os_roots_der: native_roots(tsa_system_trust)?,
                tsa_intermediates_der: read_der_files(&tsa_intermediates, "TSA intermediate")?,
                tsa_fetched_intermediates_der: read_der_files(
                    &tsa_fetched_intermediates,
                    "TSA AIA intermediate",
                )?,
                tsa_crls_der: read_der_files(&tsa_crls, "TSA CRL")?,
                tsa_ocsp_responses_der: read_der_files(&tsa_ocsp_responses, "TSA OCSP response")?,
            };
            let signatures = inspect_signatures_with_options(&document, &trust)
                .map_err(|error| error.to_string())?;
            if json {
                println!(
                    "{}",
                    serde_json::json!({ "count": signatures.len(), "signatures": signatures })
                );
            } else {
                for signature in signatures {
                    println!(
                        "{} object={} {} R covered_end={} later_bytes={} cms_verified={}",
                        signature.index,
                        signature.object_number,
                        signature.object_generation,
                        signature.covered_end,
                        signature.later_bytes,
                        signature.cms_verified
                    );
                }
            }
            Ok(())
        }
        Command::Annot {
            command:
                AnnotCommand::Create {
                    input,
                    subtype,
                    page,
                    rect,
                    contents,
                    quad_points,
                    uri,
                    output,
                    json,
                },
        } => {
            let bytes = fs::read(input).map_err(|error| error.to_string())?;
            let document = PdfEngine::default()
                .open(&bytes, OpenOptions::default())
                .map_err(|error| error.to_string())?;
            let outcome = document
                .create_annotation(AnnotationCreateRequest {
                    page_index: page,
                    subtype: annotation_subtype(&subtype)?,
                    rect: fixed_array(Some(rect), "rect")?
                        .ok_or_else(|| "--rect is required".to_string())?,
                    contents,
                    quad_points,
                    uri,
                })
                .map_err(|error| error.to_string())?;
            write_outcome(
                &output,
                outcome.bytes,
                outcome.report,
                outcome.verification,
                json,
            )
        }
        Command::Annot {
            command:
                AnnotCommand::Remove {
                    input,
                    index,
                    output,
                    json,
                },
        } => {
            let bytes = fs::read(input).map_err(|error| error.to_string())?;
            let document = PdfEngine::default()
                .open(&bytes, OpenOptions::default())
                .map_err(|error| error.to_string())?;
            let outcome = document
                .remove_annotation(AnnotationRemoveRequest {
                    annotation_index: index,
                })
                .map_err(|error| error.to_string())?;
            write_outcome(
                &output,
                outcome.bytes,
                outcome.report,
                outcome.verification,
                json,
            )
        }
        Command::Annot {
            command:
                AnnotCommand::SetContents {
                    input,
                    format,
                    index,
                    contents,
                    regenerate_appearance,
                    output,
                    json,
                },
        } => {
            require_pdf(&format)?;
            let bytes = fs::read(input).map_err(|error| error.to_string())?;
            let document = PdfEngine::default()
                .open(&bytes, OpenOptions::default())
                .map_err(|error| error.to_string())?;
            let (bytes, report, verification) = if regenerate_appearance {
                let outcome = document
                    .regenerate_free_text_appearance(FreeTextAppearanceRequest {
                        annotation_index: index,
                        contents,
                    })
                    .map_err(|error| error.to_string())?;
                (
                    outcome.bytes,
                    serde_json::to_value(outcome.report).map_err(|error| error.to_string())?,
                    serde_json::to_value(outcome.verification)
                        .map_err(|error| error.to_string())?,
                )
            } else {
                let outcome = document
                    .set_annotation_contents(AnnotationContentsMutationRequest {
                        annotation_index: index,
                        contents,
                    })
                    .map_err(|error| error.to_string())?;
                (
                    outcome.bytes,
                    serde_json::to_value(outcome.report).map_err(|error| error.to_string())?,
                    serde_json::to_value(outcome.verification)
                        .map_err(|error| error.to_string())?,
                )
            };
            fs::write(&output, &bytes).map_err(|error| error.to_string())?;
            if json {
                println!(
                    "{}",
                    serde_json::json!({
                        "output": output,
                        "report": report,
                        "verification": verification,
                    })
                );
            } else {
                println!("wrote {}", output.display());
            }
            Ok(())
        }
        Command::Xfa { command } => {
            let engine = PdfEngine::default();
            match command {
                XfaCommand::List { input, json } => {
                    let bytes = fs::read(input).map_err(|error| error.to_string())?;
                    let document = engine
                        .open(&bytes, OpenOptions::default())
                        .map_err(|error| error.to_string())?;
                    let packets = list_xfa_packets(&document).map_err(|error| error.to_string())?;
                    let output = render_xfa_packets(&packets, json)?;
                    if !output.is_empty() {
                        println!("{output}");
                    }
                    Ok(())
                }
                XfaCommand::Datasets { input, json } => {
                    let bytes = fs::read(input).map_err(|error| error.to_string())?;
                    let document = engine
                        .open(&bytes, OpenOptions::default())
                        .map_err(|error| error.to_string())?;
                    let fields =
                        list_xfa_dataset_fields(&document).map_err(|error| error.to_string())?;
                    let output = render_xfa_dataset_fields(&fields, json)?;
                    if !output.is_empty() {
                        println!("{output}");
                    }
                    Ok(())
                }
                XfaCommand::DatasetSet {
                    input,
                    path,
                    value,
                    output,
                    json,
                } => {
                    let bytes = fs::read(input).map_err(|error| error.to_string())?;
                    let document = engine
                        .open(&bytes, OpenOptions::default())
                        .map_err(|error| error.to_string())?;
                    let outcome = document
                        .set_xfa_dataset_field(XfaDatasetSetRequest { path, value })
                        .map_err(|error| error.to_string())?;
                    write_verified_outcome(
                        &output,
                        outcome.bytes,
                        outcome.report,
                        outcome.verification,
                        json,
                    )
                }
                XfaCommand::Replace {
                    input,
                    packet_index,
                    text,
                    replace,
                    output,
                    json,
                } => {
                    let bytes = fs::read(input).map_err(|error| error.to_string())?;
                    let document = engine
                        .open(&bytes, OpenOptions::default())
                        .map_err(|error| error.to_string())?;
                    let outcome = document
                        .replace_xfa_text(XfaReplaceRequest {
                            old_text: text,
                            new_text: replace,
                            packet_index,
                        })
                        .map_err(|error| error.to_string())?;
                    write_outcome(
                        &output,
                        outcome.bytes,
                        outcome.report,
                        outcome.verification,
                        json,
                    )
                }
            }
        }
        Command::Image { command } => match command {
            ImageCommand::ReplaceEncoded {
                input,
                object,
                generation,
                data,
                mask_policy,
                output,
                json,
            } => {
                let bytes = fs::read(input).map_err(|error| error.to_string())?;
                let encoded_bytes = fs::read(data).map_err(|error| error.to_string())?;
                let document = PdfEngine::default()
                    .open(&bytes, OpenOptions::default())
                    .map_err(|error| error.to_string())?;
                let outcome = document
                    .replace_image_xobject_encoded(EncodedImageReplacementRequest {
                        object_number: object,
                        object_generation: generation,
                        encoded_bytes,
                        mask_policy: image_mask_policy(&mask_policy)?,
                    })
                    .map_err(|error| error.to_string())?;
                write_outcome(
                    &output,
                    outcome.bytes,
                    outcome.report,
                    outcome.verification,
                    json,
                )
            }
            ImageCommand::Replace {
                input,
                object,
                generation,
                data,
                width,
                height,
                bits,
                color_space,
                filter,
                predictor,
                mask_policy,
                output,
                json,
            } => {
                let bytes = fs::read(input).map_err(|error| error.to_string())?;
                let encoded_bytes = fs::read(data).map_err(|error| error.to_string())?;
                let color_space = image_color_space(&color_space)?;
                let filter = image_filter(&filter)?;
                let decode_params = predictor.map(|predictor| ImageDecodeParams {
                    predictor,
                    colors: image_components(color_space),
                    bits_per_component: bits,
                    columns: width,
                });
                let document = PdfEngine::default()
                    .open(&bytes, OpenOptions::default())
                    .map_err(|error| error.to_string())?;
                let outcome = document
                    .replace_image_xobject(ImageReplacementRequest {
                        object_number: object,
                        object_generation: generation,
                        encoded_bytes,
                        width,
                        height,
                        bits_per_component: bits,
                        color_space,
                        filter,
                        decode_params,
                        mask_policy: image_mask_policy(&mask_policy)?,
                    })
                    .map_err(|error| error.to_string())?;
                write_outcome(
                    &output,
                    outcome.bytes,
                    outcome.report,
                    outcome.verification,
                    json,
                )
            }
        },
        Command::Ocr { command } => match command {
            OcrCommand::Apply {
                input,
                source,
                source_format,
                output,
                json,
            } => {
                let mut bytes = fs::read(input).map_err(|error| error.to_string())?;
                let source = fs::read(source).map_err(|error| error.to_string())?;
                let requests = match source_format.to_ascii_lowercase().as_str() {
                    "json" => vec![
                        parse_ocr_json(&source, OcrParseLimits::default())
                            .map_err(|error| error.to_string())?,
                    ],
                    "alto" | "xml" => parse_alto_xml(&source, OcrParseLimits::default())
                        .map_err(|error| error.to_string())?,
                    _ => return Err("--source-format must be json or alto".into()),
                };
                let mut results = Vec::with_capacity(requests.len());
                for request in requests {
                    let document = PdfEngine::default()
                        .open(&bytes, OpenOptions::default())
                        .map_err(|error| error.to_string())?;
                    let plan = document
                        .plan_ocr_text_layer(request)
                        .map_err(|error| error.to_string())?;
                    let outcome = document
                        .apply_ocr_text_layer(&plan)
                        .map_err(|error| error.to_string())?;
                    results.push(serde_json::json!({
                        "report": outcome.report,
                        "verification": outcome.verification,
                    }));
                    bytes = outcome.bytes;
                }
                fs::write(&output, &bytes).map_err(|error| error.to_string())?;
                if json {
                    println!(
                        "{}",
                        serde_json::json!({ "output": output, "results": results })
                    );
                } else {
                    let verification =
                        serde_json::to_string(&results).map_err(|error| error.to_string())?;
                    println!(
                        "wrote {} bytes to {}\nverification {verification}",
                        bytes.len(),
                        output.display()
                    );
                }
                Ok(())
            }
        },
        Command::Overlay { command } => match command {
            OverlayCommand::Stamp {
                input,
                pages,
                content,
                bbox,
                matrix,
                opacity,
                output,
                json,
            } => {
                let bytes = fs::read(input).map_err(|error| error.to_string())?;
                let form_content = fs::read(content).map_err(|error| error.to_string())?;
                let document = PdfEngine::default()
                    .open(&bytes, OpenOptions::default())
                    .map_err(|error| error.to_string())?;
                let outcome = document
                    .place_overlay_stamp(OverlayStampRequest {
                        page_indices: pages,
                        form_content,
                        bbox: fixed_array(Some(bbox), "bbox")?
                            .ok_or_else(|| "--bbox is required".to_string())?,
                        transform: fixed_array(Some(matrix), "matrix")?
                            .ok_or_else(|| "--matrix is required".to_string())?,
                        opacity,
                    })
                    .map_err(|error| error.to_string())?;
                write_outcome(
                    &output,
                    outcome.bytes,
                    outcome.report,
                    outcome.verification,
                    json,
                )
            }
            OverlayCommand::Text {
                input,
                page_index,
                text,
                x,
                y,
                font_size,
                output,
                json,
            } => {
                let bytes = fs::read(input).map_err(|error| error.to_string())?;
                let document = PdfEngine::default()
                    .open(&bytes, OpenOptions::default())
                    .map_err(|error| error.to_string())?;
                let outcome = document
                    .place_text_overlay(TextOverlayRequest {
                        page_index,
                        text,
                        x,
                        y,
                        font_size,
                    })
                    .map_err(|error| error.to_string())?;
                write_verified_outcome(
                    &output,
                    outcome.bytes,
                    outcome.report,
                    outcome.verification,
                    json,
                )
            }
        },
        Command::Stream { command } => match command {
            StreamCommand::Mutate {
                input,
                object,
                generation,
                data,
                output,
                json,
            } => {
                let bytes = fs::read(input).map_err(|error| error.to_string())?;
                let decoded_bytes = fs::read(data).map_err(|error| error.to_string())?;
                let document = PdfEngine::default()
                    .open(&bytes, OpenOptions::default())
                    .map_err(|error| error.to_string())?;
                let outcome = document
                    .mutate_stream(StreamMutationRequest {
                        object_number: object,
                        object_generation: generation,
                        decoded_bytes,
                    })
                    .map_err(|error| error.to_string())?;
                write_outcome(
                    &output,
                    outcome.bytes,
                    outcome.report,
                    outcome.verification,
                    json,
                )
            }
        },
        Command::Page { command } => {
            let engine = PdfEngine::default();
            match command {
                PageCommand::Create {
                    width,
                    height,
                    output,
                } => {
                    let bytes = engine
                        .create_blank_pdf(&[BlankPageSize { width, height }])
                        .map_err(|error| error.to_string())?;
                    fs::write(&output, bytes).map_err(|error| error.to_string())?;
                    println!("wrote {}", output.display());
                    Ok(())
                }
                PageCommand::Extract {
                    input,
                    pages,
                    output,
                    json,
                } => {
                    let bytes = fs::read(input).map_err(|error| error.to_string())?;
                    let document = engine
                        .open(&bytes, OpenOptions::default())
                        .map_err(|error| error.to_string())?;
                    let outcome = document
                        .extract_pages(&pages)
                        .map_err(|error| error.to_string())?;
                    write_outcome(
                        &output,
                        outcome.bytes,
                        outcome.report,
                        outcome.verification,
                        json,
                    )
                }
                PageCommand::Insert {
                    input,
                    source,
                    index,
                    source_pages,
                    output,
                    json,
                } => {
                    let bytes = fs::read(input).map_err(|error| error.to_string())?;
                    let source_bytes = fs::read(source).map_err(|error| error.to_string())?;
                    let document = engine
                        .open(&bytes, OpenOptions::default())
                        .map_err(|error| error.to_string())?;
                    let source = engine
                        .open(&source_bytes, OpenOptions::default())
                        .map_err(|error| error.to_string())?;
                    let outcome = document
                        .insert_pages(index, &source, &source_pages)
                        .map_err(|error| error.to_string())?;
                    write_outcome(
                        &output,
                        outcome.bytes,
                        outcome.report,
                        outcome.verification,
                        json,
                    )
                }
                PageCommand::Merge {
                    inputs,
                    output,
                    json,
                } => {
                    let bytes = inputs
                        .iter()
                        .map(fs::read)
                        .collect::<Result<Vec<_>, _>>()
                        .map_err(|error| error.to_string())?;
                    let documents = bytes
                        .iter()
                        .map(|bytes| engine.open(bytes, OpenOptions::default()))
                        .collect::<Result<Vec<_>, _>>()
                        .map_err(|error| error.to_string())?;
                    let sources = documents.iter().skip(1).collect::<Vec<_>>();
                    let outcome = documents[0]
                        .merge_pages(&sources)
                        .map_err(|error| error.to_string())?;
                    write_outcome(
                        &output,
                        outcome.bytes,
                        outcome.report,
                        outcome.verification,
                        json,
                    )
                }
                PageCommand::Transform {
                    input,
                    pages,
                    rotation,
                    media_box,
                    crop_box,
                    translate,
                    scale,
                    output,
                    json,
                } => {
                    let bytes = fs::read(input).map_err(|error| error.to_string())?;
                    let document = engine
                        .open(&bytes, OpenOptions::default())
                        .map_err(|error| error.to_string())?;
                    let transform = PageTransform {
                        rotation_degrees: rotation,
                        media_box: fixed_array(media_box, "media-box")?,
                        crop_box: fixed_array(crop_box, "crop-box")?,
                        translate: fixed_array(translate, "translate")?,
                        scale: fixed_array(scale, "scale")?,
                    };
                    let outcome = document
                        .transform_pages(&pages, transform)
                        .map_err(|error| error.to_string())?;
                    write_outcome(
                        &output,
                        outcome.bytes,
                        outcome.report,
                        outcome.verification,
                        json,
                    )
                }
            }
        }
        Command::Encrypt {
            input,
            format,
            revision,
            user_password,
            owner_password,
            permissions,
            output,
            json,
        } => {
            require_pdf(&format)?;
            let bytes = fs::read(input).map_err(|error| error.to_string())?;
            let document = PdfEngine::default()
                .open(&bytes, OpenOptions::default())
                .map_err(|error| error.to_string())?;
            let outcome = document
                .encrypt_standard(StandardEncryptionOptions {
                    revision: encryption_revision(&revision)?,
                    user_password,
                    owner_password,
                    permissions,
                })
                .map_err(|error| error.to_string())?;
            fs::write(&output, &outcome.bytes).map_err(|error| error.to_string())?;
            if json {
                println!(
                    "{}",
                    serde_json::json!({
                        "output": output,
                        "report": outcome.report,
                        "verification": outcome.verification,
                    })
                );
            } else {
                println!("wrote {}", output.display());
            }
            Ok(())
        }
        Command::Decrypt {
            input,
            format,
            password,
            output,
            json,
        } => {
            require_pdf(&format)?;
            let bytes = fs::read(input).map_err(|error| error.to_string())?;
            let outcome = PdfEngine::default()
                .decrypt_input_to_plain(&bytes, &password, OpenOptions::default())
                .map_err(|error| error.to_string())?;
            fs::write(&output, &outcome.bytes).map_err(|error| error.to_string())?;
            if json {
                println!(
                    "{}",
                    serde_json::json!({
                        "output": output,
                        "report": outcome.report,
                        "verification": outcome.verification,
                    })
                );
            } else {
                println!("wrote {}", output.display());
            }
            Ok(())
        }
        Command::ChangePassword {
            input,
            old_password,
            new_user_password,
            new_owner_password,
            output,
            json,
        } => {
            let bytes = fs::read(input).map_err(|error| error.to_string())?;
            let outcome = PdfEngine::default()
                .change_standard_passwords_input(
                    &bytes,
                    &old_password,
                    &new_user_password,
                    &new_owner_password,
                    OpenOptions::default(),
                )
                .map_err(|error| error.to_string())?;
            write_outcome(
                &output,
                outcome.bytes,
                outcome.report,
                outcome.verification,
                json,
            )
        }
        Command::EncryptPublicKey {
            input,
            format,
            method,
            recipients,
            permissions,
            output,
            json,
        } => {
            require_pdf(&format)?;
            let bytes = fs::read(input).map_err(|error| error.to_string())?;
            let document = PdfEngine::default()
                .open(&bytes, OpenOptions::default())
                .map_err(|error| error.to_string())?;
            let outcome = document
                .encrypt_public_key(PublicKeyEncryptionOptions {
                    method: public_key_method(&method)?,
                    recipient_certificates_der: read_der_files(&recipients, "recipient")?,
                    permissions,
                })
                .map_err(|error| error.to_string())?;
            write_outcome(
                &output,
                outcome.bytes,
                outcome.report,
                outcome.verification,
                json,
            )
        }
        Command::DecryptPublicKey {
            input,
            format,
            certificate,
            private_key,
            output,
            json,
        } => {
            require_pdf(&format)?;
            let bytes = fs::read(input).map_err(|error| error.to_string())?;
            let certificate = fs::read(certificate).map_err(|error| error.to_string())?;
            let private_key = fs::read(private_key).map_err(|error| error.to_string())?;
            let outcome = PdfEngine::default()
                .decrypt_public_key_input(
                    &bytes,
                    &certificate,
                    &private_key,
                    OpenOptions::default(),
                )
                .map_err(|error| error.to_string())?;
            write_outcome(
                &output,
                outcome.bytes,
                outcome.report,
                outcome.verification,
                json,
            )
        }
    }
}

fn edit_text(
    document: &PdfDocument,
    old_text: &str,
    replacement: &str,
    match_index: usize,
    rewrite: &str,
) -> Result<(Vec<u8>, serde_json::Value, serde_json::Value), String> {
    let surgical = || {
        document.surgical_text_edit(SurgicalTextEditRequest {
            old_text: old_text.into(),
            replacement: replacement.into(),
            match_index,
        })
    };
    let incremental = || {
        document.incremental_text_edit(IncrementalTextEditRequest {
            old_text: old_text.into(),
            replacement: replacement.into(),
            match_index,
        })
    };
    let filtered = || {
        document.filtered_text_edit(FilteredTextEditRequest {
            old_text: old_text.into(),
            replacement: replacement.into(),
            match_index,
        })
    };
    let font = || {
        document.font_text_edit(FontTextEditRequest {
            old_text: old_text.into(),
            replacement: replacement.into(),
            match_index,
        })
    };
    match rewrite {
        "surgical" => {
            let outcome = surgical().map_err(|error| error.to_string())?;
            outcome_parts(outcome.bytes, outcome.report, outcome.verification)
        }
        "incremental" => {
            let outcome = incremental().map_err(|error| error.to_string())?;
            outcome_parts(outcome.bytes, outcome.report, outcome.verification)
        }
        "filtered-incremental" => {
            let outcome = filtered().map_err(|error| error.to_string())?;
            outcome_parts(outcome.bytes, outcome.report, outcome.verification)
        }
        "font-incremental" => {
            let outcome = font().map_err(|error| error.to_string())?;
            outcome_parts(outcome.bytes, outcome.report, outcome.verification)
        }
        "auto"
            if document
                .query_text(old_text, match_index)
                .map_err(|error| error.to_string())?
                .font_name
                .is_some() =>
        {
            let outcome = font().map_err(|error| error.to_string())?;
            outcome_parts(outcome.bytes, outcome.report, outcome.verification)
        }
        "auto" => match surgical() {
            Ok(outcome) => outcome_parts(outcome.bytes, outcome.report, outcome.verification),
            Err(error) if error.code == PdfErrorCode::UnsafeRewrite => match incremental() {
                Ok(outcome) => outcome_parts(outcome.bytes, outcome.report, outcome.verification),
                Err(error) if error.code == PdfErrorCode::UnsafeRewrite => {
                    let outcome = filtered().map_err(|error| error.to_string())?;
                    outcome_parts(outcome.bytes, outcome.report, outcome.verification)
                }
                Err(error) => Err(error.to_string()),
            },
            Err(error) => Err(error.to_string()),
        },
        other => Err(format!("unsupported rewrite mode {other:?}")),
    }
}

fn write_outcome(
    output: &PathBuf,
    bytes: Vec<u8>,
    report: impl serde::Serialize,
    verification: impl serde::Serialize,
    json: bool,
) -> Result<(), String> {
    fs::write(output, bytes).map_err(|error| error.to_string())?;
    if json {
        println!(
            "{}",
            serde_json::json!({ "output": output, "report": report, "verification": verification })
        );
    } else {
        println!("wrote {}", output.display());
    }
    Ok(())
}

fn write_verified_outcome(
    output: &PathBuf,
    bytes: Vec<u8>,
    report: impl serde::Serialize,
    verification: impl serde::Serialize,
    json: bool,
) -> Result<(), String> {
    let message = if json {
        serde_json::to_string(
            &serde_json::json!({ "output": output, "report": report, "verification": verification }),
        )
        .map_err(|error| error.to_string())?
    } else {
        let verification =
            serde_json::to_string(&verification).map_err(|error| error.to_string())?;
        format!("wrote {}\nverification {verification}", output.display())
    };
    fs::write(output, bytes).map_err(|error| error.to_string())?;
    println!("{message}");
    Ok(())
}

fn outcome_parts(
    bytes: Vec<u8>,
    report: impl serde::Serialize,
    verification: impl serde::Serialize,
) -> Result<(Vec<u8>, serde_json::Value, serde_json::Value), String> {
    Ok((
        bytes,
        serde_json::to_value(report).map_err(|error| error.to_string())?,
        serde_json::to_value(verification).map_err(|error| error.to_string())?,
    ))
}

fn read_der_files(paths: &[PathBuf], label: &str) -> Result<Vec<Vec<u8>>, String> {
    paths
        .iter()
        .map(|path| {
            fs::read(path)
                .map_err(|error| format!("failed to read {label} {}: {error}", path.display()))
        })
        .collect()
}

fn native_roots(enabled: bool) -> Result<Vec<Vec<u8>>, String> {
    if !enabled {
        return Ok(Vec::new());
    }
    let result = rustls_native_certs::load_native_certs();
    if result.certs.is_empty() {
        return Err(format!(
            "system trust store returned no usable certificates ({} load errors)",
            result.errors.len()
        ));
    }
    Ok(result
        .certs
        .into_iter()
        .map(|certificate| certificate.as_ref().to_vec())
        .collect())
}

fn fixed_array<const N: usize>(
    values: Option<Vec<f64>>,
    label: &str,
) -> Result<Option<[f64; N]>, String> {
    values
        .map(|values| {
            values
                .try_into()
                .map_err(|_| format!("--{label} requires exactly {N} numbers"))
        })
        .transpose()
}

fn require_pdf(format: &str) -> Result<(), String> {
    if format.eq_ignore_ascii_case("pdf") {
        Ok(())
    } else {
        Err(format!("unsupported format {format:?}"))
    }
}

fn validate_query_meta(filter: &str) -> Result<(), String> {
    let (key, _) = filter
        .split_once('=')
        .ok_or_else(|| format!("invalid --meta {filter:?}; expected key=value"))?;
    match key {
        "operator" => Ok(()),
        _ => Err(format!("unsupported query metadata key {key:?}")),
    }
}

fn open_pdf(
    bytes: &[u8],
    password: Option<&str>,
    options: OpenOptions,
) -> Result<(PdfDocument, EncryptionMetadata), String> {
    let engine = PdfEngine::default();
    if options.repair {
        let document = engine
            .open(bytes, options)
            .map_err(|error| error.to_string())?;
        let encryption = inspect_encryption(&document).map_err(|error| error.to_string())?;
        if encryption.encrypted {
            return Err("repair mode does not support encrypted PDFs".into());
        }
        return Ok((document, encryption));
    }
    let encryption = engine
        .inspect_encryption_input(bytes, OpenOptions::default())
        .map_err(|error| error.to_string())?;
    let document = match password {
        Some(password) if encryption.encrypted => engine
            .open_with_password(bytes, password, OpenOptions::default())
            .map_err(|error| error.to_string())?,
        _ => engine
            .open(bytes, OpenOptions::default())
            .map_err(|error| error.to_string())?,
    };
    Ok((document, encryption))
}

fn encryption_revision(value: &str) -> Result<StandardEncryptionRevision, String> {
    let value = value.to_ascii_lowercase();
    if let Some(bits) = value.strip_prefix("r3-rc4-") {
        let bits = bits
            .parse::<u16>()
            .map_err(|_| "R3 RC4 key length must be 40..=128 bits in 8-bit steps".to_string())?;
        if (40..=128).contains(&bits) && bits % 8 == 0 {
            return Ok(StandardEncryptionRevision::R3Rc4(bits));
        }
        return Err("R3 RC4 key length must be 40..=128 bits in 8-bit steps".into());
    }
    match value.as_str() {
        "r2-rc4" => Ok(StandardEncryptionRevision::R2Rc4),
        "r4-rc4" => Ok(StandardEncryptionRevision::R4Rc4),
        "r4-aesv2" => Ok(StandardEncryptionRevision::R4AesV2),
        "r5-aes256" => Ok(StandardEncryptionRevision::R5Aes256),
        "r6-aes256" => Ok(StandardEncryptionRevision::R6Aes256),
        _ => Err(format!(
            "unsupported encryption revision {value:?}; expected r2-rc4, r3-rc4-<40..128>, r4-rc4, r4-aesv2, r5-aes256, or r6-aes256"
        )),
    }
}

fn public_key_method(value: &str) -> Result<PublicKeyEncryptionMethod, String> {
    match value.to_ascii_lowercase().as_str() {
        "rc4" => Ok(PublicKeyEncryptionMethod::Rc4),
        "aesv2" => Ok(PublicKeyEncryptionMethod::AesV2),
        "aesv3" => Ok(PublicKeyEncryptionMethod::AesV3),
        _ => Err(format!(
            "unsupported public-key encryption method {value:?}; expected rc4, aesv2, or aesv3"
        )),
    }
}

fn form_field_kind(value: &str) -> Result<FormFieldKind, String> {
    match value.to_ascii_lowercase().as_str() {
        "text" => Ok(FormFieldKind::Text),
        "checkbox" => Ok(FormFieldKind::Checkbox),
        "radio" => Ok(FormFieldKind::Radio),
        "choice" => Ok(FormFieldKind::Choice),
        "signature" => Ok(FormFieldKind::Signature),
        _ => Err(format!(
            "unsupported form field kind {value:?}; expected text, checkbox, radio, choice, or signature"
        )),
    }
}

fn annotation_subtype(value: &str) -> Result<AnnotationSubtype, String> {
    match value.to_ascii_lowercase().as_str() {
        "text" => Ok(AnnotationSubtype::Text),
        "free-text" | "freetext" => Ok(AnnotationSubtype::FreeText),
        "square" => Ok(AnnotationSubtype::Square),
        "circle" => Ok(AnnotationSubtype::Circle),
        "link" => Ok(AnnotationSubtype::Link),
        "highlight" => Ok(AnnotationSubtype::Highlight),
        "underline" => Ok(AnnotationSubtype::Underline),
        "strike-out" | "strikeout" => Ok(AnnotationSubtype::StrikeOut),
        _ => Err(format!("unsupported annotation subtype {value:?}")),
    }
}

fn render_xfa_dataset_fields(fields: &[XfaDatasetField], json: bool) -> Result<String, String> {
    if json {
        return serde_json::to_string(&serde_json::json!({
            "fields": fields,
            "count": fields.len(),
        }))
        .map_err(|error| error.to_string());
    }
    Ok(fields
        .iter()
        .map(|field| format!("{}={}", field.path, field.value))
        .collect::<Vec<_>>()
        .join("\n"))
}

fn render_xfa_packets(packets: &[XfaPacket], json: bool) -> Result<String, String> {
    if json {
        return serde_json::to_string(&serde_json::json!({
            "packets": packets,
            "count": packets.len(),
        }))
        .map_err(|error| error.to_string());
    }
    Ok(packets
        .iter()
        .map(|packet| {
            format!(
                "index={} label={} object={}:{} bytes={} root={} unsafe_xml={}",
                packet.index,
                packet.label,
                packet.object_number,
                packet.object_generation,
                packet.byte_length,
                packet.root_element.as_deref().unwrap_or("-"),
                packet.unsafe_xml,
            )
        })
        .collect::<Vec<_>>()
        .join("\n"))
}

fn image_color_space(value: &str) -> Result<ImageColorSpace, String> {
    match value.to_ascii_lowercase().as_str() {
        "gray" | "device-gray" => Ok(ImageColorSpace::DeviceGray),
        "rgb" | "device-rgb" => Ok(ImageColorSpace::DeviceRgb),
        "cmyk" | "device-cmyk" => Ok(ImageColorSpace::DeviceCmyk),
        _ => Err(format!("unsupported image color space {value:?}")),
    }
}

fn image_components(color_space: ImageColorSpace) -> u8 {
    match color_space {
        ImageColorSpace::DeviceGray => 1,
        ImageColorSpace::DeviceRgb => 3,
        ImageColorSpace::DeviceCmyk => 4,
    }
}

fn image_filter(value: &str) -> Result<ImageFilter, String> {
    match value.to_ascii_lowercase().as_str() {
        "raw" => Ok(ImageFilter::Raw),
        "flate" => Ok(ImageFilter::Flate),
        "jpeg" => Ok(ImageFilter::Jpeg),
        "jpx" => Ok(ImageFilter::Jpx),
        _ => Err(format!("unsupported image filter {value:?}")),
    }
}

fn image_mask_policy(value: &str) -> Result<ImageMaskPolicy, String> {
    match value.to_ascii_lowercase().as_str() {
        "reject" => Ok(ImageMaskPolicy::Reject),
        "preserve" | "preserve-compatible" => Ok(ImageMaskPolicy::PreserveCompatible),
        _ => Err(format!("unsupported image mask policy {value:?}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_rust_cli_commands() {
        assert!(Cli::try_parse_from(["binas", "extract-text", "input.pdf", "--json"]).is_ok());
        assert!(
            Cli::try_parse_from(["binas", "inspect", "broken.pdf", "--repair", "--json"]).is_ok()
        );
        assert!(Cli::try_parse_from(["binas", "profile", "input.pdf", "--json"]).is_ok());
        assert!(
            Cli::try_parse_from([
                "binas",
                "ocr",
                "apply",
                "input.pdf",
                "--source",
                "ocr.json",
                "--source-format",
                "json",
                "-o",
                "output.pdf",
                "--json",
            ])
            .is_ok()
        );
        assert!(
            Cli::try_parse_from([
                "binas",
                "overlay",
                "text",
                "input.pdf",
                "--page-index",
                "0",
                "--text",
                "APPROVED",
                "--x",
                "12",
                "--y",
                "24",
                "--font-size",
                "12",
                "-o",
                "output.pdf",
                "--json",
            ])
            .is_ok()
        );
        assert!(
            Cli::try_parse_from([
                "binas",
                "overlay",
                "stamp",
                "input.pdf",
                "--pages",
                "0,2",
                "--content",
                "stamp.bin",
                "--bbox",
                "0",
                "0",
                "100",
                "50",
                "--matrix",
                "1",
                "0",
                "0",
                "1",
                "10",
                "20",
                "--opacity",
                "0.5",
                "-o",
                "output.pdf",
            ])
            .is_ok()
        );
        assert!(
            Cli::try_parse_from([
                "binas",
                "image",
                "replace-encoded",
                "input.pdf",
                "--object",
                "8",
                "--data",
                "image.jpg",
                "-o",
                "output.pdf",
            ])
            .is_ok()
        );
        assert!(
            Cli::try_parse_from([
                "binas",
                "image",
                "replace",
                "input.pdf",
                "--object",
                "8",
                "--data",
                "image.jpg",
                "--width",
                "640",
                "--height",
                "480",
                "--color-space",
                "rgb",
                "--filter",
                "jpeg",
                "-o",
                "output.pdf",
            ])
            .is_ok()
        );
        assert!(
            Cli::try_parse_from([
                "binas",
                "inspect",
                "input.pdf",
                "--password",
                "secret",
                "--json"
            ])
            .is_ok()
        );
        assert!(
            Cli::try_parse_from([
                "binas",
                "form",
                "create",
                "input.pdf",
                "--name",
                "approved",
                "--kind",
                "checkbox",
                "--rect",
                "0",
                "0",
                "20",
                "20",
                "-o",
                "output.pdf"
            ])
            .is_ok()
        );
        assert!(
            Cli::try_parse_from(["binas", "form", "flatten", "input.pdf", "-o", "output.pdf"])
                .is_ok()
        );
        assert!(Cli::try_parse_from(["binas", "form", "list", "input.pdf", "--json"]).is_ok());
        assert!(Cli::try_parse_from(["binas", "annot", "list", "input.pdf", "--json"]).is_ok());
        assert!(
            Cli::try_parse_from([
                "binas",
                "page",
                "create",
                "--width",
                "612",
                "--height",
                "792",
                "-o",
                "blank.pdf",
            ])
            .is_ok()
        );
        assert!(
            Cli::try_parse_from([
                "binas",
                "page",
                "extract",
                "input.pdf",
                "--pages",
                "0,2",
                "-o",
                "pages.pdf"
            ])
            .is_ok()
        );
        assert!(Cli::try_parse_from(["binas", "xfa", "list", "input.pdf", "--json"]).is_ok());
        assert!(Cli::try_parse_from(["binas", "xfa", "datasets", "input.pdf", "--json"]).is_ok());
        assert!(
            Cli::try_parse_from([
                "binas",
                "xfa",
                "dataset-set",
                "input.pdf",
                "--path",
                "form.name",
                "--value",
                "Alice",
                "-o",
                "output.pdf",
                "--json",
            ])
            .is_ok()
        );
        assert!(
            Cli::try_parse_from([
                "binas",
                "xfa",
                "dataset-set",
                "input.pdf",
                "--path",
                "form.name",
                "--value",
                "Alice",
            ])
            .is_err()
        );
        assert!(
            Cli::try_parse_from([
                "binas",
                "stream",
                "mutate",
                "input.pdf",
                "--object",
                "4",
                "--data",
                "stream.bin",
                "-o",
                "output.pdf"
            ])
            .is_ok()
        );
        assert!(
            Cli::try_parse_from([
                "binas",
                "signature",
                "inspect",
                "input.pdf",
                "--trust-root",
                "root.der",
                "--system-trust",
                "--trust-root",
                "other-root.der",
                "--trust-intermediate",
                "intermediate.der",
                "--aia-intermediate",
                "fetched.der",
                "--crl",
                "issuer.crl",
                "--ocsp",
                "issuer.ocsp",
                "--tsa-root",
                "tsa-root.der",
                "--tsa-system-trust",
                "--tsa-aia-intermediate",
                "tsa-fetched.der",
                "--tsa-crl",
                "tsa.crl",
                "--tsa-ocsp",
                "tsa.ocsp",
                "--validation-time-unix",
                "1700000000",
                "--json",
            ])
            .is_ok()
        );
        assert!(
            Cli::try_parse_from([
                "binas",
                "form",
                "set",
                "input.pdf",
                "--field",
                "name",
                "--value",
                "new",
                "-o",
                "output.pdf",
            ])
            .is_ok()
        );
        assert!(
            Cli::try_parse_from([
                "binas",
                "annot",
                "set-contents",
                "input.pdf",
                "--index",
                "0",
                "--contents",
                "new",
                "--regenerate-appearance",
                "-o",
                "output.pdf",
            ])
            .is_ok()
        );
        assert!(
            Cli::try_parse_from([
                "binas",
                "encrypt",
                "input.pdf",
                "--revision",
                "r6-aes256",
                "--user-password",
                "user",
                "--owner-password",
                "owner",
                "-o",
                "encrypted.pdf",
                "--json",
            ])
            .is_ok()
        );
        assert!(
            Cli::try_parse_from([
                "binas",
                "encrypt-public-key",
                "input.pdf",
                "--recipient",
                "recipient.der",
                "-o",
                "encrypted.pdf",
            ])
            .is_ok()
        );
        assert!(
            Cli::try_parse_from([
                "binas",
                "decrypt-public-key",
                "encrypted.pdf",
                "--certificate",
                "recipient.der",
                "--private-key",
                "recipient.pk8",
                "-o",
                "plain.pdf",
            ])
            .is_ok()
        );
        assert!(
            Cli::try_parse_from([
                "binas",
                "decrypt",
                "encrypted.pdf",
                "--password",
                "user",
                "-o",
                "plain.pdf",
            ])
            .is_ok()
        );
        assert!(
            Cli::try_parse_from([
                "binas",
                "query",
                "input.pdf",
                "--text",
                "hello",
                "--kind",
                "pdf.content.text_show",
                "--match-index",
                "0",
                "--meta",
                "page=1",
                "--json",
            ])
            .is_ok()
        );
        assert!(
            Cli::try_parse_from(["binas", "validate", "input.pdf", "--fail-on-invalid",]).is_ok()
        );
        assert!(
            Cli::try_parse_from([
                "binas",
                "edit",
                "input.pdf",
                "--text",
                "old",
                "--replace",
                "new value",
                "--rewrite",
                "incremental",
                "-o",
                "output.pdf",
                "--json",
            ])
            .is_ok()
        );
    }

    #[test]
    fn runs_shared_cli_command_paths_against_a_fixture() {
        let base = std::env::temp_dir().join(format!("binas-cli-legacy-{}", std::process::id()));
        let input = base.with_extension("input.pdf");
        let output = base.with_extension("output.pdf");
        fs::write(&input, test_pdf()).unwrap();

        let input_arg = input.display().to_string();
        let output_arg = output.display().to_string();
        for args in [
            vec!["binas", "inspect", &input_arg, "--json"],
            vec!["binas", "query", &input_arg, "--text", "hello", "--json"],
            vec!["binas", "validate", &input_arg, "--json"],
            vec!["binas", "profile", &input_arg, "--json"],
            vec![
                "binas",
                "edit",
                &input_arg,
                "--text",
                "hello",
                "--replace",
                "world",
                "-o",
                &output_arg,
                "--json",
            ],
        ] {
            run(Cli::try_parse_from(args).unwrap()).unwrap();
        }

        let edited_bytes = fs::read(&output).unwrap();
        let edited = PdfEngine::default()
            .open(&edited_bytes, OpenOptions::default())
            .unwrap();
        assert_eq!(edited.query_text_all("world").unwrap().len(), 1);
        let _ = fs::remove_file(input);
        let _ = fs::remove_file(output);
    }

    #[test]
    fn flattens_form_fields_and_fails_closed_without_them() {
        let base =
            std::env::temp_dir().join(format!("binas-cli-flatten-form-{}", std::process::id()));
        let input = base.with_extension("input.pdf");
        let output = base.with_extension("output.pdf");
        let no_fields = base.with_extension("no-fields.pdf");
        let rejected = base.with_extension("rejected.pdf");
        let _ = fs::remove_file(&output);
        let _ = fs::remove_file(&rejected);

        let document = PdfEngine::default()
            .open(&test_pdf(), OpenOptions::default())
            .unwrap();
        let form = document
            .create_form_field(FormFieldCreateRequest {
                name: "approved".into(),
                page_index: 0,
                rect: [10.0, 10.0, 90.0, 30.0],
                kind: FormFieldKind::Text,
                value: "yes".into(),
                options: Vec::new(),
            })
            .unwrap();
        fs::write(&input, form.bytes).unwrap();

        let input_arg = input.display().to_string();
        let output_arg = output.display().to_string();
        run(Cli::try_parse_from([
            "binas",
            "form",
            "flatten",
            &input_arg,
            "-o",
            &output_arg,
            "--json",
        ])
        .unwrap())
        .unwrap();
        let flattened = PdfEngine::default()
            .open(&fs::read(&output).unwrap(), OpenOptions::default())
            .unwrap();
        assert!(list_form_fields(&flattened).unwrap().is_empty());
        assert_eq!(flattened.inspect().unwrap().page_count, 1);

        fs::write(&no_fields, test_pdf()).unwrap();
        let no_fields_arg = no_fields.display().to_string();
        let rejected_arg = rejected.display().to_string();
        assert!(
            run(Cli::try_parse_from([
                "binas",
                "form",
                "flatten",
                &no_fields_arg,
                "-o",
                &rejected_arg,
            ])
            .unwrap())
            .is_err()
        );
        assert!(!rejected.exists());

        for path in [input, output, no_fields] {
            let _ = fs::remove_file(path);
        }
    }

    #[test]
    fn creates_a_blank_pdf_and_rejects_invalid_dimensions_or_missing_output() {
        let output =
            std::env::temp_dir().join(format!("binas-cli-blank-{}.pdf", std::process::id()));
        let rejected = std::env::temp_dir().join(format!(
            "binas-cli-blank-rejected-{}.pdf",
            std::process::id()
        ));
        let _ = fs::remove_file(&output);
        let _ = fs::remove_file(&rejected);

        let output_arg = output.display().to_string();
        run(Cli::try_parse_from([
            "binas",
            "page",
            "create",
            "--width",
            "612",
            "--height",
            "792",
            "-o",
            &output_arg,
        ])
        .unwrap())
        .unwrap();
        let reopened = PdfEngine::default()
            .open(&fs::read(&output).unwrap(), OpenOptions::default())
            .unwrap();
        assert_eq!(reopened.inspect().unwrap().page_count, 1);

        let rejected_arg = rejected.display().to_string();
        assert!(
            run(Cli::try_parse_from([
                "binas",
                "page",
                "create",
                "--width",
                "0",
                "--height",
                "792",
                "-o",
                &rejected_arg,
            ])
            .unwrap())
            .is_err()
        );
        assert!(!rejected.exists());
        assert!(
            Cli::try_parse_from([
                "binas", "page", "create", "--width", "612", "--height", "792",
            ])
            .is_err()
        );

        let _ = fs::remove_file(output);
    }

    #[test]
    fn prepares_an_external_signature_plan_and_refuses_unverified_cms() {
        let base = std::env::temp_dir().join(format!(
            "binas-cli-external-signature-{}",
            std::process::id()
        ));
        let input = base.with_extension("input.pdf");
        let prepared = base.with_extension("prepared.pdf");
        let plan = base.with_extension("plan.json");
        let digest = base.with_extension("digest.bin");
        let cms = base.with_extension("cms.der");
        let output = base.with_extension("signed.pdf");
        let _ = fs::remove_file(&output);
        fs::write(&input, test_pdf()).unwrap();

        let input_arg = input.display().to_string();
        let prepared_arg = prepared.display().to_string();
        let plan_arg = plan.display().to_string();
        let digest_arg = digest.display().to_string();
        run(Cli::try_parse_from([
            "binas",
            "signature",
            "prepare",
            &input_arg,
            "--reserve",
            "512",
            "-o",
            &prepared_arg,
            "--plan",
            &plan_arg,
            "--digest",
            &digest_arg,
        ])
        .unwrap())
        .unwrap();

        let descriptor: ExternalSignaturePlanDescriptor =
            serde_json::from_slice(&fs::read(&plan).unwrap()).unwrap();
        assert_eq!(descriptor.digest_algorithm, "sha256");
        assert_eq!(fs::read(&digest).unwrap(), descriptor.digest_to_sign);
        ExternalSignaturePlan::from_prepared_pdf(fs::read(&prepared).unwrap(), descriptor).unwrap();

        fs::write(&cms, b"not-a-detached-cms").unwrap();
        let cms_arg = cms.display().to_string();
        let output_arg = output.display().to_string();
        let error = run(Cli::try_parse_from([
            "binas",
            "signature",
            "apply",
            &prepared_arg,
            "--plan",
            &plan_arg,
            "--cms",
            &cms_arg,
            "-o",
            &output_arg,
        ])
        .unwrap())
        .unwrap_err();
        assert!(error.contains("applied CMS failed detached signature verification"));
        assert!(!output.exists());

        for path in [input, prepared, plan, digest, cms] {
            let _ = fs::remove_file(path);
        }
    }

    #[test]
    fn runs_text_overlay_cli_command_against_a_fixture() {
        let base =
            std::env::temp_dir().join(format!("binas-cli-text-overlay-{}", std::process::id()));
        let input = base.with_extension("input.pdf");
        let json_output = base.with_extension("json-output.pdf");
        let text_output = base.with_extension("text-output.pdf");
        fs::write(&input, test_pdf()).unwrap();

        let input_arg = input.display().to_string();
        let json_output_arg = json_output.display().to_string();
        run(Cli::try_parse_from([
            "binas",
            "overlay",
            "text",
            &input_arg,
            "--page-index",
            "0",
            "--text",
            "APPROVED",
            "--x",
            "12",
            "--y",
            "24",
            "--font-size",
            "12",
            "-o",
            &json_output_arg,
            "--json",
        ])
        .unwrap())
        .unwrap();
        let json_bytes = fs::read(&json_output).unwrap();
        assert!(
            json_bytes
                .windows(b"/BaseFont /Helvetica".len())
                .any(|window| window == b"/BaseFont /Helvetica")
        );
        assert_eq!(
            open_pdf(&json_bytes, None, OpenOptions::default())
                .unwrap()
                .0
                .inspect()
                .unwrap()
                .page_count,
            1
        );

        let text_output_arg = text_output.display().to_string();
        run(Cli::try_parse_from([
            "binas",
            "overlay",
            "text",
            &input_arg,
            "--page-index",
            "0",
            "--text",
            "APPROVED",
            "--x",
            "12",
            "--y",
            "24",
            "--font-size",
            "12",
            "-o",
            &text_output_arg,
        ])
        .unwrap())
        .unwrap();
        let text_bytes = fs::read(&text_output).unwrap();
        assert!(
            text_bytes
                .windows(b"(APPROVED) Tj".len())
                .any(|window| window == b"(APPROVED) Tj")
        );

        let _ = fs::remove_file(input);
        let _ = fs::remove_file(json_output);
        let _ = fs::remove_file(text_output);
    }

    #[test]
    fn runs_ocr_apply_cli_with_json_and_alto_sources() {
        let base = std::env::temp_dir().join(format!("binas-cli-ocr-{}", std::process::id()));
        let input = base.with_extension("input.pdf");
        let json_source = base.with_extension("json-source.json");
        let alto_source = base.with_extension("alto-source.xml");
        let json_output = base.with_extension("json-output.pdf");
        let text_output = base.with_extension("text-output.pdf");
        fs::write(&input, test_pdf()).unwrap();
        fs::write(
            &json_source,
            br#"{"page_index":0,"source_width":100,"source_height":100,"boxes":[{"text":"JSON OCR","x":10,"y":20,"width":30,"height":10}]}"#,
        )
        .unwrap();
        fs::write(
            &alto_source,
            br#"<alto><Layout><Page WIDTH="100" HEIGHT="100"><PrintSpace><TextBlock><TextLine><String CONTENT="ALTO OCR" HPOS="10" VPOS="20" WIDTH="30" HEIGHT="10"/></TextLine></TextBlock></PrintSpace></Page></Layout></alto>"#,
        )
        .unwrap();

        let input_arg = input.display().to_string();
        let json_source_arg = json_source.display().to_string();
        let json_output_arg = json_output.display().to_string();
        run(Cli::try_parse_from([
            "binas",
            "ocr",
            "apply",
            &input_arg,
            "--source",
            &json_source_arg,
            "--source-format",
            "json",
            "-o",
            &json_output_arg,
            "--json",
        ])
        .unwrap())
        .unwrap();
        let document = PdfEngine::default()
            .open(&fs::read(&json_output).unwrap(), OpenOptions::default())
            .unwrap();
        assert_eq!(document.query_text_all("JSON OCR").unwrap().len(), 1);

        let alto_source_arg = alto_source.display().to_string();
        let text_output_arg = text_output.display().to_string();
        run(Cli::try_parse_from([
            "binas",
            "ocr",
            "apply",
            &input_arg,
            "--source",
            &alto_source_arg,
            "--source-format",
            "alto",
            "-o",
            &text_output_arg,
        ])
        .unwrap())
        .unwrap();
        let document = PdfEngine::default()
            .open(&fs::read(&text_output).unwrap(), OpenOptions::default())
            .unwrap();
        assert_eq!(document.query_text_all("ALTO OCR").unwrap().len(), 1);

        let _ = fs::remove_file(input);
        let _ = fs::remove_file(json_source);
        let _ = fs::remove_file(alto_source);
        let _ = fs::remove_file(json_output);
        let _ = fs::remove_file(text_output);
    }

    #[test]
    fn ocr_apply_cli_fails_closed_for_an_invalid_source() {
        let base =
            std::env::temp_dir().join(format!("binas-cli-ocr-reject-{}", std::process::id()));
        let input = base.with_extension("input.pdf");
        let source = base.with_extension("source.json");
        let output = base.with_extension("output.pdf");
        let _ = fs::remove_file(&output);
        fs::write(&input, test_pdf()).unwrap();
        fs::write(&source, b"{}").unwrap();

        let input_arg = input.display().to_string();
        let source_arg = source.display().to_string();
        let output_arg = output.display().to_string();
        assert!(
            run(Cli::try_parse_from([
                "binas",
                "ocr",
                "apply",
                &input_arg,
                "--source",
                &source_arg,
                "--source-format",
                "json",
                "-o",
                &output_arg,
            ])
            .unwrap())
            .is_err()
        );
        assert!(!output.exists());

        let _ = fs::remove_file(input);
        let _ = fs::remove_file(source);
    }

    #[test]
    fn text_overlay_cli_fails_closed_for_an_invalid_font_size() {
        let base = std::env::temp_dir().join(format!(
            "binas-cli-text-overlay-reject-{}",
            std::process::id()
        ));
        let input = base.with_extension("input.pdf");
        let output = base.with_extension("output.pdf");
        let _ = fs::remove_file(&output);
        fs::write(&input, test_pdf()).unwrap();

        let input_arg = input.display().to_string();
        let output_arg = output.display().to_string();
        assert!(
            run(Cli::try_parse_from([
                "binas",
                "overlay",
                "text",
                &input_arg,
                "--page-index",
                "0",
                "--text",
                "APPROVED",
                "--x",
                "12",
                "--y",
                "24",
                "--font-size",
                "0",
                "-o",
                &output_arg,
            ])
            .unwrap())
            .is_err()
        );
        assert!(!output.exists());

        let _ = fs::remove_file(input);
    }

    #[test]
    fn runs_xfa_read_only_cli_commands_against_a_static_fixture() {
        let input =
            std::env::temp_dir().join(format!("binas-cli-xfa-datasets-{}.pdf", std::process::id()));
        fs::write(&input, test_xfa_dataset_pdf()).unwrap();
        let input_arg = input.display().to_string();

        run(Cli::try_parse_from(["binas", "xfa", "list", &input_arg, "--json"]).unwrap()).unwrap();
        run(Cli::try_parse_from(["binas", "xfa", "list", &input_arg]).unwrap()).unwrap();
        run(Cli::try_parse_from(["binas", "xfa", "datasets", &input_arg, "--json"]).unwrap())
            .unwrap();
        run(Cli::try_parse_from(["binas", "xfa", "datasets", &input_arg]).unwrap()).unwrap();

        let _ = fs::remove_file(input);
    }

    #[test]
    fn runs_xfa_dataset_set_cli_command_against_a_static_fixture() {
        let base =
            std::env::temp_dir().join(format!("binas-cli-xfa-dataset-set-{}", std::process::id()));
        let input = base.with_extension("input.pdf");
        let json_output = base.with_extension("json-output.pdf");
        let text_output = base.with_extension("text-output.pdf");
        fs::write(&input, test_xfa_dataset_pdf()).unwrap();

        let input_arg = input.display().to_string();
        let json_output_arg = json_output.display().to_string();
        run(Cli::try_parse_from([
            "binas",
            "xfa",
            "dataset-set",
            &input_arg,
            "--path",
            "form.name",
            "--value",
            "Ana & Co",
            "-o",
            &json_output_arg,
            "--json",
        ])
        .unwrap())
        .unwrap();
        let document = PdfEngine::default()
            .open(&fs::read(&json_output).unwrap(), OpenOptions::default())
            .unwrap();
        assert_eq!(
            document.get_xfa_dataset_field("form.name").unwrap().value,
            "Ana & Co"
        );

        let text_output_arg = text_output.display().to_string();
        run(Cli::try_parse_from([
            "binas",
            "xfa",
            "dataset-set",
            &input_arg,
            "--path",
            "form.name",
            "--value",
            "Bea",
            "-o",
            &text_output_arg,
        ])
        .unwrap())
        .unwrap();
        let document = PdfEngine::default()
            .open(&fs::read(&text_output).unwrap(), OpenOptions::default())
            .unwrap();
        assert_eq!(
            document.get_xfa_dataset_field("form.name").unwrap().value,
            "Bea"
        );

        let _ = fs::remove_file(input);
        let _ = fs::remove_file(json_output);
        let _ = fs::remove_file(text_output);
    }

    #[test]
    fn xfa_dataset_set_cli_fails_closed_for_a_missing_path() {
        let base = std::env::temp_dir().join(format!(
            "binas-cli-xfa-dataset-set-reject-{}",
            std::process::id()
        ));
        let input = base.with_extension("input.pdf");
        let output = base.with_extension("output.pdf");
        let _ = fs::remove_file(&output);
        fs::write(&input, test_xfa_dataset_pdf()).unwrap();

        let input_arg = input.display().to_string();
        let output_arg = output.display().to_string();
        assert!(
            run(Cli::try_parse_from([
                "binas",
                "xfa",
                "dataset-set",
                &input_arg,
                "--path",
                "form.missing",
                "--value",
                "Bea",
                "-o",
                &output_arg,
            ])
            .unwrap(),)
            .is_err()
        );
        assert!(!output.exists());

        let _ = fs::remove_file(input);
    }

    #[test]
    fn renders_xfa_dataset_fields_as_tested_json_and_text() {
        let fields = [XfaDatasetField {
            path: "form.name".into(),
            value: "Alice".into(),
            packet_index: 0,
            object_number: 6,
            object_generation: 0,
        }];
        let json: serde_json::Value =
            serde_json::from_str(&render_xfa_dataset_fields(&fields, true).unwrap()).unwrap();
        assert_eq!(json["count"], 1);
        assert_eq!(json["fields"][0]["path"], "form.name");
        assert_eq!(json["fields"][0]["value"], "Alice");
        assert_eq!(
            render_xfa_dataset_fields(&fields, false).unwrap(),
            "form.name=Alice"
        );
    }

    #[test]
    fn renders_xfa_packets_as_tested_json_and_text() {
        let packets = [XfaPacket {
            index: 0,
            label: "datasets".into(),
            object_number: 6,
            object_generation: 0,
            root_element: Some("xfa:datasets".into()),
            unsafe_xml: false,
            byte_length: 42,
            preview: "<xfa:datasets/>".into(),
        }];
        let json: serde_json::Value =
            serde_json::from_str(&render_xfa_packets(&packets, true).unwrap()).unwrap();
        assert_eq!(json["count"], 1);
        assert_eq!(json["packets"][0]["label"], "datasets");
        assert_eq!(json["packets"][0]["preview"], "<xfa:datasets/>");
        assert_eq!(
            render_xfa_packets(&packets, false).unwrap(),
            "index=0 label=datasets object=6:0 bytes=42 root=xfa:datasets unsafe_xml=false"
        );
    }

    #[test]
    fn rejects_other_formats() {
        assert!(validate_query_meta("operator=TJ").is_ok());
        assert!(validate_query_meta("operator").is_err());
        assert!(validate_query_meta("page=1").is_err());
        assert!(require_pdf("PDF").is_ok());
        assert_eq!(
            require_pdf("png").unwrap_err(),
            "unsupported format \"png\""
        );
        assert_eq!(
            encryption_revision("R4-AESV2").unwrap(),
            StandardEncryptionRevision::R4AesV2
        );
        assert_eq!(
            encryption_revision("R3-RC4-80").unwrap(),
            StandardEncryptionRevision::R3Rc4(80)
        );
        assert_eq!(
            public_key_method("AESV3").unwrap(),
            PublicKeyEncryptionMethod::AesV3
        );
        assert!(native_roots(false).unwrap().is_empty());
        assert!(encryption_revision("r6").is_err());
    }

    #[test]
    fn password_open_decrypts_without_putting_secrets_in_reports_or_errors() {
        let input = test_pdf();
        let document = PdfEngine::default()
            .open(&input, OpenOptions::default())
            .unwrap();
        let encrypted = document
            .encrypt_standard(StandardEncryptionOptions {
                revision: StandardEncryptionRevision::R4AesV2,
                user_password: "cli-user-secret".into(),
                owner_password: "cli-owner-secret".into(),
                permissions: -4,
            })
            .unwrap();
        let report = serde_json::to_string(&encrypted.report).unwrap();
        assert!(!report.contains("cli-user-secret"));
        assert!(!report.contains("cli-owner-secret"));

        let (plain, original_encryption) = open_pdf(
            &encrypted.bytes,
            Some("cli-user-secret"),
            OpenOptions::default(),
        )
        .unwrap();
        assert!(original_encryption.encrypted);
        assert_eq!(plain.query_text_all("hello").unwrap().len(), 1);

        let rejected = open_pdf(
            &encrypted.bytes,
            Some("do-not-echo-this"),
            OpenOptions::default(),
        )
        .unwrap_err();
        assert!(!rejected.contains("do-not-echo-this"));
    }

    fn test_pdf() -> Vec<u8> {
        let stream = b"BT (hello) Tj ET";
        let objects = [
            b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
            b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
            b"<< /Type /Page /Parent 2 0 R /Contents 4 0 R /MediaBox [0 0 100 100] >>".to_vec(),
            {
                let mut value = format!("<< /Length {} >>\nstream\n", stream.len()).into_bytes();
                value.extend_from_slice(stream);
                value.extend_from_slice(b"\nendstream");
                value
            },
        ];
        let mut bytes = b"%PDF-1.7\n".to_vec();
        let mut offsets = Vec::new();
        for (index, object) in objects.iter().enumerate() {
            offsets.push(bytes.len());
            bytes.extend_from_slice(format!("{} 0 obj\n", index + 1).as_bytes());
            bytes.extend_from_slice(object);
            bytes.extend_from_slice(b"\nendobj\n");
        }
        let xref = bytes.len();
        bytes.extend_from_slice(b"xref\n0 5\n0000000000 65535 f \n");
        for offset in offsets {
            bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        bytes.extend_from_slice(
            format!("trailer\n<< /Size 5 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes(),
        );
        bytes
    }

    fn test_xfa_dataset_pdf() -> Vec<u8> {
        let dataset = br#"<xfa:datasets xmlns:xfa="http://www.xfa.org/schema/xfa-data/1.0/"><xfa:data><form><name>Alice</name></form></xfa:data></xfa:datasets>"#;
        let objects = [
            b"<< /Type /Catalog /Pages 2 0 R /AcroForm 5 0 R >>".to_vec(),
            b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] >>".to_vec(),
            b"null".to_vec(),
            b"<< /XFA [(datasets) 6 0 R] >>".to_vec(),
            [
                format!("<< /Length {} >>\nstream\n", dataset.len()).into_bytes(),
                dataset.to_vec(),
                b"\nendstream".to_vec(),
            ]
            .concat(),
        ];
        let mut bytes = b"%PDF-1.7\n".to_vec();
        let mut offsets = Vec::new();
        for (index, object) in objects.iter().enumerate() {
            offsets.push(bytes.len());
            bytes.extend_from_slice(format!("{} 0 obj\n", index + 1).as_bytes());
            bytes.extend_from_slice(object);
            bytes.extend_from_slice(b"\nendobj\n");
        }
        let xref = bytes.len();
        bytes.extend_from_slice(b"xref\n0 7\n0000000000 65535 f \n");
        for offset in offsets {
            bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        bytes.extend_from_slice(
            format!("trailer\n<< /Size 7 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes(),
        );
        bytes
    }
}
