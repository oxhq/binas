use std::{collections::BTreeMap, fmt};

use aes::{
    Aes256,
    cipher::{BlockDecryptMut, KeyIvInit, block_padding::Pkcs7},
};
use cbc::Decryptor;
use cms::{
    builder::{
        ContentEncryptionAlgorithm, EnvelopedDataBuilder, KeyEncryptionInfo,
        KeyTransRecipientInfoBuilder,
    },
    cert::IssuerAndSerialNumber,
    content_info::ContentInfo,
    enveloped_data::{EnvelopedData, RecipientIdentifier, RecipientInfo},
};
use der::{Any, AnyRef, Decode, Encode, asn1::OctetString};
use rsa::{
    Pkcs1v15Encrypt, RsaPrivateKey, RsaPublicKey,
    pkcs8::{DecodePrivateKey, DecodePublicKey},
    rand_core::OsRng,
};
use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};
use sha2::Sha256;
use x509_cert::Certificate;

use crate::{
    DecryptionReport, DecryptionVerification, EncryptionReport, OpenOptions, PdfDocument,
    PdfEngine, PdfError,
    encryption::{
        CryptMethod, StandardSecurity, decrypt_objects, encrypt_objects, remove_encryption,
        write_encrypted_pdf,
    },
    parser::{IndirectObject, ObjectRef, ParsedDocument, Value},
    security::inspect_encryption,
};

const ID_DATA: der::asn1::ObjectIdentifier =
    der::asn1::ObjectIdentifier::new_unwrap("1.2.840.113549.1.7.1");
const ID_ENVELOPED_DATA: der::asn1::ObjectIdentifier =
    der::asn1::ObjectIdentifier::new_unwrap("1.2.840.113549.1.7.3");
const RSA_ENCRYPTION: der::asn1::ObjectIdentifier =
    der::asn1::ObjectIdentifier::new_unwrap("1.2.840.113549.1.1.1");
const AES_256_CBC: der::asn1::ObjectIdentifier =
    der::asn1::ObjectIdentifier::new_unwrap("2.16.840.1.101.3.4.1.42");

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PublicKeyEncryptionMethod {
    Rc4,
    AesV2,
    AesV3,
}

impl PublicKeyEncryptionMethod {
    fn crypt(self) -> CryptMethod {
        match self {
            Self::Rc4 => CryptMethod::Rc4,
            Self::AesV2 => CryptMethod::AesV2,
            Self::AesV3 => CryptMethod::AesV3,
        }
    }

    fn key_length(self) -> usize {
        match self {
            Self::Rc4 | Self::AesV2 => 16,
            Self::AesV3 => 32,
        }
    }

    fn version(self) -> i64 {
        match self {
            Self::Rc4 | Self::AesV2 => 4,
            Self::AesV3 => 5,
        }
    }
}

#[derive(Clone, Deserialize)]
pub struct PublicKeyEncryptionOptions {
    pub method: PublicKeyEncryptionMethod,
    pub recipient_certificates_der: Vec<Vec<u8>>,
    pub permissions: i32,
}

impl fmt::Debug for PublicKeyEncryptionOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PublicKeyEncryptionOptions")
            .field("method", &self.method)
            .field("recipient_count", &self.recipient_certificates_der.len())
            .field("permissions", &self.permissions)
            .finish()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct PublicKeyEncryptionOutcome {
    pub bytes: Vec<u8>,
    pub report: EncryptionReport,
    pub verification: PublicKeyEncryptionVerification,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PublicKeyEncryptionVerification {
    pub passed: bool,
    pub encrypted_reparsed: bool,
    pub page_count_unchanged: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PublicKeyDecryptionOutcome {
    pub bytes: Vec<u8>,
    pub report: DecryptionReport,
    pub verification: DecryptionVerification,
    pub permissions: i32,
}

struct OpenedRecipient {
    seed: [u8; 20],
    permissions: i32,
}

struct PublicKeySecurity {
    method: CryptMethod,
    version: i64,
    key_length: usize,
    encrypt_metadata: bool,
    recipients: Vec<Vec<u8>>,
    encrypt_object: Option<ObjectRef>,
}

struct Secret(Vec<u8>);

impl Drop for Secret {
    fn drop(&mut self) {
        self.0.fill(0);
    }
}

impl PdfDocument {
    pub fn encrypt_public_key(
        &self,
        options: PublicKeyEncryptionOptions,
    ) -> Result<PublicKeyEncryptionOutcome, PdfError> {
        if inspect_encryption(self)?.encrypted {
            return Err(PdfError::unsafe_rewrite(
                "encrypt_public_key requires an unencrypted input PDF",
            ));
        }
        validate_recipients(
            &options.recipient_certificates_der,
            self.parsed().limits.max_container_items,
            self.parsed().limits.max_stream_bytes,
        )?;
        let baseline_bytes = self.canonicalize()?.bytes;
        let baseline = PdfEngine::new(self.engine_config().clone())
            .open(&baseline_bytes, OpenOptions::default())?;
        let old_pages = baseline.page_count()?;
        let mut seed = [0_u8; 20];
        getrandom::fill(&mut seed)
            .map_err(|_| PdfError::verification("secure random generation failed"))?;
        let recipients = options
            .recipient_certificates_der
            .iter()
            .map(|certificate| {
                make_recipient(
                    &seed,
                    options.permissions,
                    certificate,
                    &baseline.parsed().limits,
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        let file_key = Secret(derive_file_key(
            &seed,
            &recipients,
            true,
            options.method.key_length(),
        ));
        seed.fill(0);
        let encrypt_ref = allocate_encrypt_ref(&baseline)?;
        let security = StandardSecurity::for_public_key(
            options.method.key_length(),
            options.method.crypt(),
            true,
            Some(encrypt_ref),
        );
        let mut parsed = baseline.parsed().clone();
        encrypt_objects(&mut parsed, &security, &file_key.0)?;
        install_public_key_encryption(&mut parsed, options.method, &recipients, encrypt_ref)?;
        let bytes = write_encrypted_pdf(&baseline, &parsed)?;
        let encrypted = PdfEngine::new(self.engine_config().clone())
            .open(&bytes, OpenOptions::default())
            .map_err(|error| {
                PdfError::verification(format!("encrypted output did not reparse: {error}"))
            })?;
        let encrypted_reparsed = inspect_encryption(&encrypted)?.encrypted;
        let page_count_unchanged = old_pages == baseline.page_count()?;
        let verification = PublicKeyEncryptionVerification {
            passed: encrypted_reparsed && page_count_unchanged,
            encrypted_reparsed,
            page_count_unchanged,
        };
        Ok(PublicKeyEncryptionOutcome {
            report: EncryptionReport {
                operation: "encrypt_public_key".into(),
                revision: options.method.version(),
                crypt_filter: options.method.crypt().label().into(),
                permissions: options.permissions,
                input_bytes: self.source_len(),
                output_bytes: bytes.len(),
            },
            bytes,
            verification,
        })
    }

    pub fn decrypt_public_key(
        &self,
        recipient_certificate_der: &[u8],
        recipient_private_key_pkcs8_der: &[u8],
    ) -> Result<PublicKeyDecryptionOutcome, PdfError> {
        let public_key = parse_public_key_security(self.parsed())?;
        let PublicKeySecurity {
            method,
            version,
            key_length,
            encrypt_metadata,
            recipients,
            encrypt_object,
        } = public_key;
        let limits = &self.parsed().limits;
        if recipient_certificate_der.len() > limits.max_stream_bytes
            || recipient_private_key_pkcs8_der.len() > limits.max_stream_bytes
        {
            return Err(PdfError::limit(
                "public-key credential exceeds max_stream_bytes",
            ));
        }
        let opened = recipients
            .iter()
            .find_map(|recipient| {
                open_recipient(
                    recipient,
                    recipient_certificate_der,
                    recipient_private_key_pkcs8_der,
                    limits,
                )
                .transpose()
            })
            .transpose()?
            .ok_or_else(|| {
                PdfError::unsafe_rewrite("certificate is not an authorized recipient")
            })?;
        let file_key = Secret(derive_file_key(
            &opened.seed,
            &recipients,
            encrypt_metadata,
            key_length,
        ));
        let security =
            StandardSecurity::for_public_key(key_length, method, encrypt_metadata, encrypt_object);
        let mut parsed = self.parsed().clone();
        decrypt_objects(&mut parsed, &security, &file_key.0)?;
        crate::parser::finish_compressed_objects(&mut parsed)?;
        let old_pages = self.with_parsed(parsed.clone()).page_count()?;
        remove_encryption(&mut parsed, encrypt_object)?;
        let plain_document = self.with_parsed(parsed);
        let canonical = plain_document.canonicalize()?;
        let reopened = PdfEngine::new(self.engine_config().clone())
            .open(&canonical.bytes, OpenOptions::default())?;
        let encryption_removed = !inspect_encryption(&reopened)?.encrypted;
        let page_count_unchanged = reopened.page_count()? == old_pages;
        let verification = DecryptionVerification {
            passed: encryption_removed && page_count_unchanged,
            reparsed: true,
            encryption_removed,
            page_count_unchanged,
        };
        if !verification.passed {
            return Err(PdfError::verification(
                "decrypted output failed post-write verification",
            ));
        }
        Ok(PublicKeyDecryptionOutcome {
            report: DecryptionReport {
                operation: "decrypt_public_key".into(),
                revision: version,
                crypt_filter: method.label().into(),
                input_bytes: self.source_len(),
                output_bytes: canonical.bytes.len(),
            },
            bytes: canonical.bytes,
            verification,
            permissions: opened.permissions,
        })
    }
}

impl PdfEngine {
    pub fn decrypt_public_key_input(
        &self,
        input: &[u8],
        recipient_certificate_der: &[u8],
        recipient_private_key_pkcs8_der: &[u8],
        options: OpenOptions,
    ) -> Result<PublicKeyDecryptionOutcome, PdfError> {
        if options.repair {
            return Err(PdfError::unsupported("PDF repair mode is not implemented"));
        }
        self.open_skeleton(input)?
            .decrypt_public_key(recipient_certificate_der, recipient_private_key_pkcs8_der)
    }

    pub fn open_with_public_key(
        &self,
        input: &[u8],
        recipient_certificate_der: &[u8],
        recipient_private_key_pkcs8_der: &[u8],
        options: OpenOptions,
    ) -> Result<PdfDocument, PdfError> {
        if options.repair {
            return Err(PdfError::unsupported("PDF repair mode is not implemented"));
        }
        let plaintext = self.decrypt_public_key_input(
            input,
            recipient_certificate_der,
            recipient_private_key_pkcs8_der,
            OpenOptions::default(),
        )?;
        self.open(&plaintext.bytes, options)
    }
}

fn make_recipient(
    seed: &[u8; 20],
    permissions: i32,
    certificate_der: &[u8],
    limits: &crate::Limits,
) -> Result<Vec<u8>, PdfError> {
    if certificate_der.len() > limits.max_stream_bytes {
        return Err(PdfError::limit(
            "recipient certificate exceeds max_stream_bytes",
        ));
    }
    let certificate = Certificate::from_der(certificate_der)
        .map_err(|_| PdfError::unsupported("recipient certificate is not valid DER X.509"))?;
    let public_key_der = certificate
        .tbs_certificate
        .subject_public_key_info
        .to_der()
        .map_err(|_| PdfError::unsupported("recipient public key is not valid DER"))?;
    let public_key = RsaPublicKey::from_public_key_der(&public_key_der).map_err(|_| {
        PdfError::unsupported("recipient certificate does not contain an RSA public key")
    })?;
    let rid = RecipientIdentifier::IssuerAndSerialNumber(IssuerAndSerialNumber {
        issuer: certificate.tbs_certificate.issuer.clone(),
        serial_number: certificate.tbs_certificate.serial_number.clone(),
    });
    let mut recipient_rng = OsRng;
    let recipient = KeyTransRecipientInfoBuilder::new(
        rid,
        KeyEncryptionInfo::Rsa(public_key),
        &mut recipient_rng,
    )
    .map_err(|_| PdfError::verification("CMS recipient initialization failed"))?;
    let mut payload = Vec::with_capacity(24);
    payload.extend_from_slice(seed);
    payload.extend_from_slice(&permissions.to_le_bytes());
    let content = {
        let mut envelope =
            EnvelopedDataBuilder::new(None, &payload, ContentEncryptionAlgorithm::Aes256Cbc, None)
                .map_err(|_| PdfError::verification("CMS envelope initialization failed"))?;
        envelope
            .add_recipient_info(recipient)
            .map_err(|_| PdfError::verification("CMS recipient insertion failed"))?;
        let mut content_rng = OsRng;
        let enveloped = envelope
            .build_with_rng(&mut content_rng)
            .map_err(|_| PdfError::verification("CMS recipient encryption failed"))?;
        let encoded = enveloped
            .to_der()
            .map_err(|_| PdfError::verification("CMS envelope encoding failed"))?;
        Any::from(
            AnyRef::try_from(encoded.as_slice())
                .map_err(|_| PdfError::verification("CMS envelope encoding failed"))?,
        )
    };
    payload.fill(0);
    ContentInfo {
        content_type: ID_ENVELOPED_DATA,
        content,
    }
    .to_der()
    .map_err(|_| PdfError::verification("CMS ContentInfo encoding failed"))
}

fn open_recipient(
    envelope_der: &[u8],
    certificate_der: &[u8],
    private_key_der: &[u8],
    limits: &crate::Limits,
) -> Result<Option<OpenedRecipient>, PdfError> {
    if envelope_der.len() > limits.max_stream_bytes {
        return Err(PdfError::limit("CMS recipient exceeds max_stream_bytes"));
    }
    let certificate = Certificate::from_der(certificate_der)
        .map_err(|_| PdfError::unsafe_rewrite("recipient credential could not be opened"))?;
    let content = ContentInfo::from_der(envelope_der)
        .map_err(|_| PdfError::unsafe_rewrite("CMS recipient is malformed"))?;
    if content.content_type != ID_ENVELOPED_DATA {
        return Err(PdfError::unsupported(
            "recipient CMS content is not EnvelopedData",
        ));
    }
    let enveloped_der = content
        .content
        .to_der()
        .map_err(|_| PdfError::unsafe_rewrite("CMS recipient is malformed"))?;
    let enveloped = EnvelopedData::from_der(&enveloped_der)
        .map_err(|_| PdfError::unsafe_rewrite("CMS recipient is malformed"))?;
    if enveloped.recip_infos.0.len() > limits.max_container_items {
        return Err(PdfError::limit(
            "CMS recipient count exceeds max_container_items",
        ));
    }
    let expected = IssuerAndSerialNumber {
        issuer: certificate.tbs_certificate.issuer,
        serial_number: certificate.tbs_certificate.serial_number,
    };
    let Some(ktri) = enveloped.recip_infos.0.iter().find_map(|info| match info {
        RecipientInfo::Ktri(ktri)
            if matches!(&ktri.rid, RecipientIdentifier::IssuerAndSerialNumber(actual) if actual == &expected) => Some(ktri),
        _ => None,
    }) else {
        return Ok(None);
    };
    if ktri.key_enc_alg.oid != RSA_ENCRYPTION
        || enveloped.encrypted_content.content_type != ID_DATA
        || enveloped.encrypted_content.content_enc_alg.oid != AES_256_CBC
    {
        return Err(PdfError::unsupported(
            "CMS recipient uses an unsupported algorithm",
        ));
    }
    let private_key = RsaPrivateKey::from_pkcs8_der(private_key_der)
        .map_err(|_| PdfError::unsafe_rewrite("recipient credential could not be opened"))?;
    let key = Secret(
        private_key
            .decrypt(Pkcs1v15Encrypt, ktri.enc_key.as_bytes())
            .map_err(|_| PdfError::unsafe_rewrite("recipient credential could not be opened"))?,
    );
    if key.0.len() != 32 {
        return Err(PdfError::unsafe_rewrite(
            "recipient credential could not be opened",
        ));
    }
    let params = enveloped
        .encrypted_content
        .content_enc_alg
        .parameters
        .as_ref()
        .ok_or_else(|| PdfError::unsafe_rewrite("CMS recipient AES IV is missing"))?;
    let iv_der = params
        .to_der()
        .map_err(|_| PdfError::unsafe_rewrite("CMS recipient AES IV is malformed"))?;
    let iv = OctetString::from_der(&iv_der)
        .map_err(|_| PdfError::unsafe_rewrite("CMS recipient AES IV is malformed"))?;
    let ciphertext = enveloped
        .encrypted_content
        .encrypted_content
        .as_ref()
        .ok_or_else(|| PdfError::unsafe_rewrite("CMS recipient content is missing"))?;
    let payload = Decryptor::<Aes256>::new_from_slices(&key.0, iv.as_bytes())
        .map_err(|_| PdfError::unsafe_rewrite("recipient credential could not be opened"))?
        .decrypt_padded_vec_mut::<Pkcs7>(ciphertext.as_bytes())
        .map_err(|_| PdfError::unsafe_rewrite("recipient credential could not be opened"))?;
    if payload.len() != 24 {
        return Err(PdfError::unsafe_rewrite(
            "CMS recipient payload has invalid length",
        ));
    }
    let mut seed = [0_u8; 20];
    seed.copy_from_slice(&payload[..20]);
    let permissions = i32::from_le_bytes(payload[20..].try_into().unwrap());
    Ok(Some(OpenedRecipient { seed, permissions }))
}

fn derive_file_key(
    seed: &[u8; 20],
    recipients: &[Vec<u8>],
    encrypt_metadata: bool,
    key_length: usize,
) -> Vec<u8> {
    fn update(hash: &mut impl Digest, seed: &[u8; 20], recipients: &[Vec<u8>], metadata: bool) {
        hash.update(seed);
        for recipient in recipients {
            hash.update(recipient);
        }
        if !metadata {
            hash.update([0xff; 4]);
        }
    }
    if key_length == 32 {
        let mut hash = Sha256::new();
        update(&mut hash, seed, recipients, encrypt_metadata);
        hash.finalize().to_vec()
    } else {
        let mut hash = Sha1::new();
        update(&mut hash, seed, recipients, encrypt_metadata);
        hash.finalize()[..key_length].to_vec()
    }
}

fn validate_recipients(
    recipients: &[Vec<u8>],
    max_items: usize,
    max_bytes: usize,
) -> Result<(), PdfError> {
    if recipients.is_empty() {
        return Err(PdfError::unsupported(
            "public-key encryption requires at least one recipient",
        ));
    }
    if recipients.len() > max_items {
        return Err(PdfError::limit(
            "recipient count exceeds max_container_items",
        ));
    }
    if recipients.iter().any(|value| value.len() > max_bytes) {
        return Err(PdfError::limit(
            "recipient certificate exceeds max_stream_bytes",
        ));
    }
    Ok(())
}

fn allocate_encrypt_ref(document: &PdfDocument) -> Result<ObjectRef, PdfError> {
    let number = document
        .parsed()
        .objects
        .keys()
        .map(|reference| reference.number)
        .max()
        .unwrap_or(0)
        .checked_add(1)
        .ok_or_else(|| PdfError::limit("encryption object number overflows"))?;
    if document.parsed().objects.len() >= document.parsed().limits.max_objects {
        return Err(PdfError::limit(
            "encryption object allocation exceeds limits",
        ));
    }
    Ok(ObjectRef {
        number,
        generation: 0,
    })
}

fn install_public_key_encryption(
    parsed: &mut ParsedDocument,
    method: PublicKeyEncryptionMethod,
    recipients: &[Vec<u8>],
    reference: ObjectRef,
) -> Result<(), PdfError> {
    let key_bits = i64::try_from(method.key_length() * 8)
        .map_err(|_| PdfError::limit("public-key key length exceeds i64"))?;
    let recipient_values = recipients.iter().cloned().map(Value::String).collect();
    let mut filter = BTreeMap::new();
    filter.insert(
        b"CFM".to_vec(),
        Value::Name(method.crypt().label().as_bytes().to_vec()),
    );
    filter.insert(b"AuthEvent".to_vec(), Value::Name(b"DocOpen".to_vec()));
    filter.insert(b"Length".to_vec(), Value::Integer(key_bits));
    filter.insert(b"Recipients".to_vec(), Value::Array(recipient_values));
    filter.insert(b"EncryptMetadata".to_vec(), Value::Bool(true));
    let mut filters = BTreeMap::new();
    filters.insert(b"DefaultCryptFilter".to_vec(), Value::Dict(filter));
    let mut dictionary = BTreeMap::new();
    dictionary.insert(b"Filter".to_vec(), Value::Name(b"Adobe.PubSec".to_vec()));
    dictionary.insert(
        b"SubFilter".to_vec(),
        Value::Name(b"adbe.pkcs7.s5".to_vec()),
    );
    dictionary.insert(b"V".to_vec(), Value::Integer(method.version()));
    dictionary.insert(b"Length".to_vec(), Value::Integer(key_bits));
    dictionary.insert(
        b"StmF".to_vec(),
        Value::Name(b"DefaultCryptFilter".to_vec()),
    );
    dictionary.insert(
        b"StrF".to_vec(),
        Value::Name(b"DefaultCryptFilter".to_vec()),
    );
    dictionary.insert(b"CF".to_vec(), Value::Dict(filters));
    parsed.objects.insert(
        reference,
        IndirectObject {
            value: Value::Dict(dictionary),
            stream: None,
            stream_offset: 0,
            offset: 0,
        },
    );
    let Value::Dict(trailer) = &mut parsed.trailer else {
        return Err(PdfError::unsupported("trailer must be a dictionary"));
    };
    trailer.insert(b"Encrypt".to_vec(), Value::Ref(reference));
    trailer.insert(
        b"Size".to_vec(),
        Value::Integer(i64::from(reference.number) + 1),
    );
    Ok(())
}

fn parse_public_key_security(parsed: &ParsedDocument) -> Result<PublicKeySecurity, PdfError> {
    let Value::Dict(trailer) = &parsed.trailer else {
        return Err(PdfError::unsupported("trailer must be a dictionary"));
    };
    let encrypt_value = trailer
        .get(b"Encrypt".as_slice())
        .ok_or_else(|| PdfError::unsafe_rewrite("PDF is not encrypted"))?;
    let (encrypt, encrypt_object) = match encrypt_value {
        Value::Dict(value) => (value, None),
        Value::Ref(reference) => match &parsed.object(*reference)?.value {
            Value::Dict(value) => (value, Some(*reference)),
            _ => {
                return Err(PdfError::unsupported(
                    "encryption object must be a dictionary",
                ));
            }
        },
        _ => {
            return Err(PdfError::unsupported(
                "trailer /Encrypt must be a dictionary or reference",
            ));
        }
    };
    if !matches!(encrypt.get(b"Filter".as_slice()), Some(Value::Name(value)) if value == b"Adobe.PubSec")
    {
        return Err(PdfError::unsupported(
            "unsupported public-key security handler",
        ));
    }
    let sub_filter = match encrypt.get(b"SubFilter".as_slice()) {
        Some(Value::Name(value)) => value.as_slice(),
        _ => {
            return Err(PdfError::unsupported(
                "public-key /SubFilter must be a name",
            ));
        }
    };
    let version = match encrypt.get(b"V".as_slice()) {
        Some(Value::Integer(value)) => *value,
        _ => return Err(PdfError::unsupported("public-key encryption is missing /V")),
    };
    let (method, key_length, encrypt_metadata, values) = if matches!(
        sub_filter,
        b"adbe.pkcs7.s3" | b"adbe.pkcs7.s4"
    ) {
        let key_bits = match (version, encrypt.get(b"Length".as_slice())) {
            (1, None | Some(Value::Integer(40))) => 40,
            (2, None) => 40,
            (2, Some(Value::Integer(value))) if (40..=128).contains(value) && value % 8 == 0 => {
                *value
            }
            _ => {
                return Err(PdfError::unsupported(
                    "legacy public-key encryption requires V=1/V=2 RC4 with a 40-128 bit key",
                ));
            }
        };
        let Value::Array(values) = encrypt.get(b"Recipients".as_slice()).ok_or_else(|| {
            PdfError::unsupported("legacy public-key encryption is missing /Recipients")
        })?
        else {
            return Err(PdfError::unsupported(
                "public-key /Recipients must be an array",
            ));
        };
        (
            CryptMethod::Rc4,
            usize::try_from(key_bits / 8).unwrap(),
            true,
            values,
        )
    } else if sub_filter == b"adbe.pkcs7.s5" {
        for name in [b"StmF".as_slice(), b"StrF".as_slice()] {
            if !matches!(encrypt.get(name), Some(Value::Name(value)) if value == b"DefaultCryptFilter")
            {
                return Err(PdfError::unsupported(
                    "document public-key encryption requires /DefaultCryptFilter for strings and streams",
                ));
            }
        }
        let Value::Dict(filters) = encrypt
            .get(b"CF".as_slice())
            .ok_or_else(|| PdfError::unsupported("public-key encryption is missing /CF"))?
        else {
            return Err(PdfError::unsupported("public-key /CF must be a dictionary"));
        };
        let Value::Dict(filter) =
            filters
                .get(b"DefaultCryptFilter".as_slice())
                .ok_or_else(|| {
                    PdfError::unsupported("public-key encryption is missing /DefaultCryptFilter")
                })?
        else {
            return Err(PdfError::unsupported(
                "public-key crypt filter must be a dictionary",
            ));
        };
        let (method, key_length) = match (
            version,
            filter.get(b"CFM".as_slice()),
            filter.get(b"Length".as_slice()),
        ) {
            (4, Some(Value::Name(value)), Some(Value::Integer(128))) if value == b"V2" => {
                (CryptMethod::Rc4, 16)
            }
            (4, Some(Value::Name(value)), Some(Value::Integer(128))) if value == b"AESV2" => {
                (CryptMethod::AesV2, 16)
            }
            (5, Some(Value::Name(value)), Some(Value::Integer(256))) if value == b"AESV3" => {
                (CryptMethod::AesV3, 32)
            }
            _ => {
                return Err(PdfError::unsupported(
                    "public-key crypt filter must be V=4 V2/AESV2-128 or V=5 AESV3-256",
                ));
            }
        };
        let encrypt_metadata = match filter.get(b"EncryptMetadata".as_slice()) {
            None => true,
            Some(Value::Bool(value)) => *value,
            Some(_) => {
                return Err(PdfError::unsupported(
                    "public-key /EncryptMetadata must be a Boolean",
                ));
            }
        };
        let Value::Array(values) = filter.get(b"Recipients".as_slice()).ok_or_else(|| {
            PdfError::unsupported("public-key crypt filter is missing /Recipients")
        })?
        else {
            return Err(PdfError::unsupported(
                "public-key /Recipients must be an array",
            ));
        };
        (method, key_length, encrypt_metadata, values)
    } else {
        return Err(PdfError::unsupported("unsupported public-key /SubFilter"));
    };
    if values.is_empty() || values.len() > parsed.limits.max_container_items {
        return Err(PdfError::limit(
            "public-key recipient count is invalid or exceeds limits",
        ));
    }
    let recipients = values
        .iter()
        .map(|value| match value {
            Value::String(value) if value.len() <= parsed.limits.max_stream_bytes => {
                Ok(value.clone())
            }
            Value::String(_) => Err(PdfError::limit("CMS recipient exceeds max_stream_bytes")),
            _ => Err(PdfError::unsupported(
                "public-key recipient must be a byte string",
            )),
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(PublicKeySecurity {
        method,
        version,
        key_length,
        encrypt_metadata,
        recipients,
        encrypt_object,
    })
}
