use std::collections::BTreeMap;

use aes::{
    Aes128, Aes256,
    cipher::{
        BlockDecrypt, BlockDecryptMut, BlockEncrypt, BlockEncryptMut, KeyInit as AesKeyInit,
        KeyIvInit,
        block_padding::{NoPadding, Pkcs7},
    },
};
use cbc::{Decryptor, Encryptor};
use md5::{Digest, Md5};
use rc4::{
    Rc4, StreamCipher,
    consts::{U5, U6, U7, U8, U9, U10, U11, U12, U13, U14, U15, U16},
};
use serde::{Deserialize, Serialize};
use sha2::{Sha256, Sha384, Sha512};
use subtle::ConstantTimeEq;

use crate::{
    OpenOptions, PdfDocument, PdfEngine, PdfError, PdfErrorCode,
    parser::{IndirectObject, ObjectRef, ParsedDocument, Value},
    security::inspect_encryption,
    writer::{Output, dict_integer, require_classic_offset, write_name, write_object, write_value},
};

const PASSWORD_PADDING: [u8; 32] = [
    0x28, 0xbf, 0x4e, 0x5e, 0x4e, 0x75, 0x8a, 0x41, 0x64, 0x00, 0x4e, 0x56, 0xff, 0xfa, 0x01, 0x08,
    0x2e, 0x2e, 0x00, 0xb6, 0xd0, 0x68, 0x3e, 0x80, 0x2f, 0x0c, 0xa9, 0xfe, 0x64, 0x53, 0x69, 0x7a,
];

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StandardEncryptionRevision {
    R2Rc4,
    R3Rc4(u16),
    R4Rc4,
    R4AesV2,
    R5Aes256,
    R6Aes256,
}

impl StandardEncryptionRevision {
    fn revision(self) -> i64 {
        match self {
            Self::R2Rc4 => 2,
            Self::R3Rc4(_) => 3,
            Self::R4Rc4 | Self::R4AesV2 => 4,
            Self::R5Aes256 => 5,
            Self::R6Aes256 => 6,
        }
    }

    fn method(self) -> CryptMethod {
        match self {
            Self::R2Rc4 | Self::R3Rc4(_) | Self::R4Rc4 => CryptMethod::Rc4,
            Self::R4AesV2 => CryptMethod::AesV2,
            Self::R5Aes256 | Self::R6Aes256 => CryptMethod::AesV3,
        }
    }

    fn key_length(self) -> Result<usize, PdfError> {
        match self {
            Self::R2Rc4 => Ok(5),
            Self::R3Rc4(bits) => validate_key_bits(3, i64::from(bits)),
            Self::R4Rc4 | Self::R4AesV2 => Ok(16),
            Self::R5Aes256 | Self::R6Aes256 => Ok(32),
        }
    }
}

#[derive(Clone, Deserialize)]
pub struct StandardEncryptionOptions {
    pub revision: StandardEncryptionRevision,
    pub user_password: String,
    pub owner_password: String,
    pub permissions: i32,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct EncryptionReport {
    pub operation: String,
    pub revision: i64,
    pub crypt_filter: String,
    pub permissions: i32,
    pub input_bytes: usize,
    pub output_bytes: usize,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct EncryptionVerification {
    pub passed: bool,
    pub encrypted_reparsed: bool,
    pub user_password_authenticated: bool,
    pub page_count_unchanged: bool,
    pub text_and_object_semantics_unchanged: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct EncryptionOutcome {
    pub bytes: Vec<u8>,
    pub report: EncryptionReport,
    pub verification: EncryptionVerification,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct DecryptionReport {
    pub operation: String,
    pub revision: i64,
    pub crypt_filter: String,
    pub input_bytes: usize,
    pub output_bytes: usize,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct DecryptionVerification {
    pub passed: bool,
    pub reparsed: bool,
    pub encryption_removed: bool,
    pub page_count_unchanged: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DecryptionOutcome {
    pub bytes: Vec<u8>,
    pub report: DecryptionReport,
    pub verification: DecryptionVerification,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CryptMethod {
    Rc4,
    AesV2,
    AesV3,
}

impl CryptMethod {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Rc4 => "V2",
            Self::AesV2 => "AESV2",
            Self::AesV3 => "AESV3",
        }
    }
}

#[derive(Clone)]
pub(crate) struct StandardSecurity {
    version: i64,
    revision: i64,
    key_length: usize,
    owner_key: Vec<u8>,
    user_key: Vec<u8>,
    owner_encrypted_key: Vec<u8>,
    user_encrypted_key: Vec<u8>,
    encrypted_permissions: Vec<u8>,
    permissions: i32,
    file_id: Vec<u8>,
    encrypt_metadata: bool,
    method: CryptMethod,
    encrypt_object: Option<ObjectRef>,
}

impl PdfDocument {
    pub fn change_standard_passwords(
        &self,
        old_password: &str,
        new_user_password: &str,
        new_owner_password: &str,
    ) -> Result<EncryptionOutcome, PdfError> {
        let security = StandardSecurity::from_parsed(self.parsed())?;
        let revision = match (security.revision, security.method) {
            (2, CryptMethod::Rc4) => StandardEncryptionRevision::R2Rc4,
            (3, CryptMethod::Rc4) => StandardEncryptionRevision::R3Rc4(
                u16::try_from(security.key_length * 8)
                    .map_err(|_| PdfError::limit("encryption key length exceeds u16"))?,
            ),
            (4, CryptMethod::Rc4) => StandardEncryptionRevision::R4Rc4,
            (4, CryptMethod::AesV2) => StandardEncryptionRevision::R4AesV2,
            (5, CryptMethod::AesV3) => StandardEncryptionRevision::R5Aes256,
            (6, CryptMethod::AesV3) => StandardEncryptionRevision::R6Aes256,
            _ => {
                return Err(PdfError::unsupported(
                    "encrypted PDF has an unsupported revision/method pair",
                ));
            }
        };
        let plain = self.decrypt_to_plain(old_password)?;
        let document = PdfEngine::new(self.engine_config().clone())
            .open(&plain.bytes, OpenOptions::default())?;
        let mut outcome = document.encrypt_standard(StandardEncryptionOptions {
            revision,
            user_password: new_user_password.into(),
            owner_password: new_owner_password.into(),
            permissions: security.permissions,
        })?;
        outcome.report.operation = "change_standard_passwords".into();
        outcome.report.input_bytes = self.source_len();
        Ok(outcome)
    }

    pub fn decrypt_to_plain(&self, password: &str) -> Result<DecryptionOutcome, PdfError> {
        let security = StandardSecurity::from_parsed(self.parsed())?;
        let prepared;
        let password = if security.revision >= 5 {
            prepared = prepare_aes256_password(password)?;
            prepared.as_slice()
        } else {
            password.as_bytes()
        };
        let file_key = security.authenticate(password)?;
        let mut parsed = self.parsed().clone();
        decrypt_objects(&mut parsed, &security, &file_key)?;
        crate::parser::finish_compressed_objects(&mut parsed)?;
        remove_encryption(&mut parsed, security.encrypt_object)?;
        let plain_document = self.with_parsed(parsed);
        let old_pages = plain_document.page_count()?;
        let canonical = plain_document.canonicalize()?;
        let reopened = PdfEngine::new(self.engine_config().clone())
            .open(&canonical.bytes, OpenOptions::default())
            .map_err(|error| {
                PdfError::verification(format!("decrypted output did not reparse: {error}"))
            })?;
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
        Ok(DecryptionOutcome {
            report: DecryptionReport {
                operation: "decrypt_to_plain".into(),
                revision: security.revision,
                crypt_filter: security.method.label().into(),
                input_bytes: self.source_len(),
                output_bytes: canonical.bytes.len(),
            },
            bytes: canonical.bytes,
            verification,
        })
    }

    pub fn encrypt_standard(
        &self,
        options: StandardEncryptionOptions,
    ) -> Result<EncryptionOutcome, PdfError> {
        if inspect_encryption(self)?.encrypted {
            return Err(PdfError::unsafe_rewrite(
                "encrypt_standard requires an unencrypted input PDF",
            ));
        }
        let baseline_bytes = self.canonicalize()?.bytes;
        let baseline = PdfEngine::new(self.engine_config().clone())
            .open(&baseline_bytes, OpenOptions::default())?;
        let old_pages = baseline.page_count()?;
        let (security, file_key, encrypt_ref) =
            StandardSecurity::new_for_write(&baseline, &options)?;
        let mut parsed = baseline.parsed().clone();
        encrypt_objects(&mut parsed, &security, &file_key)?;
        install_encryption(&mut parsed, &security, encrypt_ref)?;
        let bytes = write_encrypted_pdf(&baseline, &parsed)?;
        let encrypted = PdfEngine::new(self.engine_config().clone())
            .open(&bytes, OpenOptions::default())
            .map_err(|error| {
                PdfError::verification(format!("encrypted output did not reparse: {error}"))
            })?;
        let encrypted_reparsed = inspect_encryption(&encrypted)?.encrypted;
        let plain = encrypted.decrypt_to_plain(&options.user_password)?;
        let verified_plain = PdfEngine::new(self.engine_config().clone())
            .open(&plain.bytes, OpenOptions::default())?;
        let page_count_unchanged = verified_plain.page_count()? == old_pages;
        let text_and_object_semantics_unchanged =
            objects_semantically_equal(baseline.parsed(), verified_plain.parsed());
        let verification = EncryptionVerification {
            passed: encrypted_reparsed
                && page_count_unchanged
                && text_and_object_semantics_unchanged,
            encrypted_reparsed,
            user_password_authenticated: true,
            page_count_unchanged,
            text_and_object_semantics_unchanged,
        };
        if !verification.passed {
            return Err(PdfError::verification(
                "encrypted output failed password round-trip verification",
            ));
        }
        Ok(EncryptionOutcome {
            report: EncryptionReport {
                operation: "encrypt_standard".into(),
                revision: security.revision,
                crypt_filter: security.method.label().into(),
                permissions: security.permissions,
                input_bytes: self.source_len(),
                output_bytes: bytes.len(),
            },
            bytes,
            verification,
        })
    }
}

impl PdfEngine {
    pub fn change_standard_passwords_input(
        &self,
        input: &[u8],
        old_password: &str,
        new_user_password: &str,
        new_owner_password: &str,
        options: OpenOptions,
    ) -> Result<EncryptionOutcome, PdfError> {
        if options.repair {
            return Err(PdfError::unsupported("PDF repair mode is not implemented"));
        }
        self.open_skeleton(input)?.change_standard_passwords(
            old_password,
            new_user_password,
            new_owner_password,
        )
    }
}

#[allow(dead_code)]
pub(crate) fn decrypt_authenticated_objects(
    parsed: &mut ParsedDocument,
    password: &str,
) -> Result<StandardSecurity, PdfError> {
    let security = StandardSecurity::from_parsed(parsed)?;
    let prepared;
    let password = if security.revision >= 5 {
        prepared = prepare_aes256_password(password)?;
        prepared.as_slice()
    } else {
        password.as_bytes()
    };
    let file_key = security.authenticate(password)?;
    decrypt_objects(parsed, &security, &file_key)?;
    Ok(security)
}

impl StandardSecurity {
    #[allow(dead_code)]
    pub(crate) fn for_public_key(
        key_length: usize,
        method: CryptMethod,
        encrypt_metadata: bool,
        encrypt_object: Option<ObjectRef>,
    ) -> Self {
        Self {
            version: 0,
            revision: 0,
            key_length,
            owner_key: Vec::new(),
            user_key: Vec::new(),
            owner_encrypted_key: Vec::new(),
            user_encrypted_key: Vec::new(),
            encrypted_permissions: Vec::new(),
            permissions: 0,
            file_id: Vec::new(),
            encrypt_metadata,
            method,
            encrypt_object,
        }
    }

    fn from_parsed(parsed: &ParsedDocument) -> Result<Self, PdfError> {
        let trailer = as_dict(&parsed.trailer, "trailer")?;
        let encrypt_value = trailer
            .get(b"Encrypt".as_slice())
            .ok_or_else(|| PdfError::unsafe_rewrite("PDF is not encrypted"))?;
        let (encrypt, encrypt_object) = match encrypt_value {
            Value::Dict(value) => (value, None),
            Value::Ref(reference) => (
                as_dict(
                    &parsed.object(*reference)?.value,
                    "encryption dictionary object",
                )?,
                Some(*reference),
            ),
            _ => {
                return Err(PdfError::unsupported(
                    "trailer /Encrypt must be a dictionary or reference",
                ));
            }
        };
        require_name(encrypt, b"Filter", b"Standard")?;
        if encrypt.contains_key(b"SubFilter".as_slice()) {
            return Err(PdfError::unsupported(
                "Standard Security /SubFilter is not supported",
            ));
        }
        let version = require_integer(encrypt, b"V")?;
        let revision = require_integer(encrypt, b"R")?;
        let permissions = i32::try_from(require_integer(encrypt, b"P")?)
            .map_err(|_| PdfError::unsupported("encryption /P exceeds i32"))?;
        let owner_key = require_string(encrypt, b"O")?.to_vec();
        let user_key = require_string(encrypt, b"U")?.to_vec();
        let (owner_encrypted_key, user_encrypted_key, encrypted_permissions) =
            if matches!(revision, 5 | 6) {
                (
                    require_sized_string(encrypt, b"OE", 32)?.to_vec(),
                    require_sized_string(encrypt, b"UE", 32)?.to_vec(),
                    require_sized_string(encrypt, b"Perms", 16)?.to_vec(),
                )
            } else {
                (Vec::new(), Vec::new(), Vec::new())
            };
        let ids = match trailer.get(b"ID".as_slice()) {
            Some(Value::Array(ids)) if !ids.is_empty() => ids,
            _ => {
                return Err(PdfError::unsupported(
                    "Standard Security requires a trailer /ID array",
                ));
            }
        };
        let Value::String(file_id) = &ids[0] else {
            return Err(PdfError::unsupported(
                "Standard Security first trailer /ID must be a string",
            ));
        };
        if file_id.is_empty() {
            return Err(PdfError::unsupported(
                "Standard Security trailer /ID must not be empty",
            ));
        }
        let encrypt_metadata = match encrypt.get(b"EncryptMetadata".as_slice()) {
            None => true,
            Some(Value::Bool(value)) => *value,
            Some(_) => {
                return Err(PdfError::syntax("/EncryptMetadata is not a Boolean", 0));
            }
        };
        let (key_length, method) = match (revision, version) {
            (2, 1) => {
                require_entry_size(&owner_key, b"O", 32)?;
                require_entry_size(&user_key, b"U", 32)?;
                (5, CryptMethod::Rc4)
            }
            (3, 2) => {
                require_entry_size(&owner_key, b"O", 32)?;
                require_entry_size(&user_key, b"U", 32)?;
                let bits = match encrypt.get(b"Length".as_slice()) {
                    None => 40,
                    Some(Value::Integer(bits)) => *bits,
                    Some(_) => {
                        return Err(PdfError::unsupported(
                            "Standard Security R3 /Length must be an integer",
                        ));
                    }
                };
                (validate_key_bits(3, bits)?, CryptMethod::Rc4)
            }
            (4, 4) => {
                require_entry_size(&owner_key, b"O", 32)?;
                require_entry_size(&user_key, b"U", 32)?;
                if require_integer(encrypt, b"Length")? != 128 {
                    return Err(PdfError::unsupported(
                        "Standard Security R4 requires /Length 128",
                    ));
                }
                (16, parse_r4_crypt_filter(encrypt)?)
            }
            (5 | 6, 5) => {
                require_entry_size(&owner_key, b"O", 48)?;
                require_entry_size(&user_key, b"U", 48)?;
                if require_integer(encrypt, b"Length")? != 256 {
                    return Err(PdfError::unsupported(
                        "Standard Security R5/R6 requires /Length 256",
                    ));
                }
                (32, parse_aes_v3_crypt_filter(encrypt)?)
            }
            (2, _) | (3, _) | (4, _) | (5 | 6, _) => {
                return Err(PdfError::unsupported(format!(
                    "unsupported Standard Security R={revision} with V={version}"
                )));
            }
            _ => {
                return Err(PdfError::unsupported(format!(
                    "Standard Security revision R={revision} is not implemented"
                )));
            }
        };
        Ok(Self {
            version,
            revision,
            key_length,
            owner_key,
            user_key,
            owner_encrypted_key,
            user_encrypted_key,
            encrypted_permissions,
            permissions,
            file_id: file_id.clone(),
            encrypt_metadata,
            method,
            encrypt_object,
        })
    }

    fn new_for_write(
        document: &PdfDocument,
        options: &StandardEncryptionOptions,
    ) -> Result<(Self, Vec<u8>, ObjectRef), PdfError> {
        let mut file_id = vec![0_u8; 16];
        secure_random(&mut file_id)?;
        let revision = options.revision.revision();
        let method = options.revision.method();
        let key_length = options.revision.key_length()?;
        let modern = revision >= 5;
        let owner_key = if modern {
            Vec::new()
        } else {
            compute_owner_entry(
                options.user_password.as_bytes(),
                options.owner_password.as_bytes(),
                revision,
                key_length,
            )?
        };
        let mut security = Self {
            version: match revision {
                2 => 1,
                3 => 2,
                4 => 4,
                5 | 6 => 5,
                _ => unreachable!(),
            },
            revision,
            key_length,
            owner_key,
            user_key: Vec::new(),
            owner_encrypted_key: Vec::new(),
            user_encrypted_key: Vec::new(),
            encrypted_permissions: Vec::new(),
            permissions: options.permissions,
            file_id,
            encrypt_metadata: true,
            method,
            encrypt_object: None,
        };
        let file_key = if modern {
            let mut key = vec![0_u8; 32];
            secure_random(&mut key)?;
            let user_password = prepare_aes256_password(&options.user_password)?;
            let owner_password = prepare_aes256_password(if options.owner_password.is_empty() {
                &options.user_password
            } else {
                &options.owner_password
            })?;
            security.initialize_aes256_entries(&user_password, &owner_password, &key)?;
            key
        } else {
            let key = security.derive_file_key(options.user_password.as_bytes())?;
            let mut user_key = security.compute_user_value(&key)?;
            if matches!(revision, 3 | 4) {
                let mut tail = [0_u8; 16];
                secure_random(&mut tail)?;
                user_key.extend_from_slice(&tail);
            }
            security.user_key = user_key;
            key
        };
        let number = document
            .parsed()
            .objects
            .keys()
            .map(|reference| reference.number)
            .max()
            .unwrap_or(0)
            .checked_add(1)
            .ok_or_else(|| PdfError::limit("encryption object number overflows"))?;
        if document.parsed().objects.len() >= document.parsed().limits.max_objects
            || usize::try_from(number)
                .ok()
                .and_then(|number| number.checked_add(1))
                .is_none_or(|size| size > document.parsed().limits.max_xref_entries)
        {
            return Err(PdfError::limit(
                "encryption object allocation exceeds limits",
            ));
        }
        let reference = ObjectRef {
            number,
            generation: 0,
        };
        security.encrypt_object = Some(reference);
        Ok((security, file_key, reference))
    }

    fn authenticate(&self, password: &[u8]) -> Result<Vec<u8>, PdfError> {
        if self.revision >= 5 {
            return self.authenticate_aes256(password);
        }
        let file_key = self.derive_file_key(password)?;
        if self.user_value_matches(&file_key)? {
            return Ok(file_key);
        }
        let user_password = recover_user_password_from_owner(password, self)?;
        let owner_file_key = self.derive_file_key(&user_password)?;
        if self.user_value_matches(&owner_file_key)? {
            return Ok(owner_file_key);
        }
        Err(PdfError::unsafe_rewrite(
            "supplied password did not authenticate",
        ))
    }

    fn user_value_matches(&self, file_key: &[u8]) -> Result<bool, PdfError> {
        let expected = self.compute_user_value(file_key)?;
        let actual = if self.revision == 2 {
            self.user_key.as_slice()
        } else {
            &self.user_key[..16]
        };
        Ok(bool::from(expected.as_slice().ct_eq(actual)))
    }

    fn derive_file_key(&self, password: &[u8]) -> Result<Vec<u8>, PdfError> {
        let mut hash = Md5::new();
        hash.update(pad_password(password));
        hash.update(&self.owner_key);
        hash.update(self.permissions.to_le_bytes());
        hash.update(&self.file_id);
        if self.revision >= 4 && !self.encrypt_metadata {
            hash.update([0xff; 4]);
        }
        let mut digest = hash.finalize().to_vec();
        if self.revision >= 3 {
            for _ in 0..50 {
                digest = Md5::digest(&digest[..self.key_length]).to_vec();
            }
        }
        Ok(digest[..self.key_length].to_vec())
    }

    fn compute_user_value(&self, file_key: &[u8]) -> Result<Vec<u8>, PdfError> {
        if self.revision == 2 {
            return rc4_crypt(file_key, &PASSWORD_PADDING);
        }
        let mut hash = Md5::new();
        hash.update(PASSWORD_PADDING);
        hash.update(&self.file_id);
        let mut output = rc4_crypt(file_key, &hash.finalize())?;
        for iteration in 1..=19 {
            output = rc4_crypt(&xor_key(file_key, iteration), &output)?;
        }
        Ok(output)
    }

    fn object_key(&self, file_key: &[u8], reference: ObjectRef) -> Result<Vec<u8>, PdfError> {
        if file_key.len() != self.key_length {
            return Err(PdfError::unsupported(
                "file key length does not match encryption dictionary",
            ));
        }
        if self.method == CryptMethod::AesV3 {
            return Ok(file_key.to_vec());
        }
        let mut input = Vec::with_capacity(file_key.len() + 9);
        input.extend_from_slice(file_key);
        input.extend_from_slice(&[
            reference.number as u8,
            (reference.number >> 8) as u8,
            (reference.number >> 16) as u8,
            reference.generation as u8,
            (reference.generation >> 8) as u8,
        ]);
        if self.method == CryptMethod::AesV2 {
            input.extend_from_slice(b"sAlT");
        }
        let digest = Md5::digest(&input);
        Ok(digest[..(file_key.len() + 5).min(16)].to_vec())
    }

    fn decrypt_object(
        &self,
        file_key: &[u8],
        reference: ObjectRef,
        ciphertext: &[u8],
    ) -> Result<Vec<u8>, PdfError> {
        let key = self.object_key(file_key, reference)?;
        match self.method {
            CryptMethod::Rc4 => rc4_crypt(&key, ciphertext),
            CryptMethod::AesV2 => aes_v2_decrypt(&key, ciphertext),
            CryptMethod::AesV3 => aes_v3_decrypt(&key, ciphertext),
        }
    }

    fn encrypt_object(
        &self,
        file_key: &[u8],
        reference: ObjectRef,
        plaintext: &[u8],
    ) -> Result<Vec<u8>, PdfError> {
        let key = self.object_key(file_key, reference)?;
        match self.method {
            CryptMethod::Rc4 => rc4_crypt(&key, plaintext),
            CryptMethod::AesV2 => aes_v2_encrypt(&key, plaintext),
            CryptMethod::AesV3 => aes_v3_encrypt(&key, plaintext),
        }
    }

    fn authenticate_aes256(&self, password: &[u8]) -> Result<Vec<u8>, PdfError> {
        let user_hash = aes256_hash(self.revision, password, &self.user_key[32..40], &[])?;
        let file_key = if bool::from(user_hash.ct_eq(&self.user_key[..32])) {
            let key = aes256_hash(self.revision, password, &self.user_key[40..48], &[])?;
            aes256_cbc_no_padding_decrypt(&key, &self.user_encrypted_key)?
        } else {
            let owner_hash = aes256_hash(
                self.revision,
                password,
                &self.owner_key[32..40],
                &self.user_key,
            )?;
            if !bool::from(owner_hash.ct_eq(&self.owner_key[..32])) {
                return Err(PdfError::unsafe_rewrite(
                    "supplied password did not authenticate",
                ));
            }
            let key = aes256_hash(
                self.revision,
                password,
                &self.owner_key[40..48],
                &self.user_key,
            )?;
            aes256_cbc_no_padding_decrypt(&key, &self.owner_encrypted_key)?
        };
        if !verify_permissions(
            &file_key,
            &self.encrypted_permissions,
            self.permissions,
            self.encrypt_metadata,
        )? {
            return Err(PdfError::unsafe_rewrite(
                "encrypted permissions failed authentication",
            ));
        }
        Ok(file_key)
    }

    fn initialize_aes256_entries(
        &mut self,
        user_password: &[u8],
        owner_password: &[u8],
        file_key: &[u8],
    ) -> Result<(), PdfError> {
        let mut user_salts = [0_u8; 16];
        let mut owner_salts = [0_u8; 16];
        secure_random(&mut user_salts)?;
        secure_random(&mut owner_salts)?;
        self.user_key = aes256_hash(self.revision, user_password, &user_salts[..8], &[])?;
        self.user_key.extend_from_slice(&user_salts);
        let user_key = aes256_hash(self.revision, user_password, &user_salts[8..], &[])?;
        self.user_encrypted_key = aes256_cbc_no_padding_encrypt(&user_key, file_key)?;
        self.owner_key = aes256_hash(
            self.revision,
            owner_password,
            &owner_salts[..8],
            &self.user_key,
        )?;
        self.owner_key.extend_from_slice(&owner_salts);
        let owner_key = aes256_hash(
            self.revision,
            owner_password,
            &owner_salts[8..],
            &self.user_key,
        )?;
        self.owner_encrypted_key = aes256_cbc_no_padding_encrypt(&owner_key, file_key)?;
        self.encrypted_permissions =
            make_permissions(file_key, self.permissions, self.encrypt_metadata)?;
        Ok(())
    }
}

fn parse_r4_crypt_filter(dictionary: &BTreeMap<Vec<u8>, Value>) -> Result<CryptMethod, PdfError> {
    require_name(dictionary, b"StmF", b"StdCF")?;
    require_name(dictionary, b"StrF", b"StdCF")?;
    match dictionary.get(b"EFF".as_slice()) {
        None => {}
        Some(Value::Name(name)) if name == b"StdCF" => {}
        Some(_) => return Err(PdfError::unsupported("unsupported encryption /EFF")),
    }
    let filters = as_dict(
        dictionary
            .get(b"CF".as_slice())
            .ok_or_else(|| PdfError::unsupported("R4 encryption is missing /CF"))?,
        "encryption /CF",
    )?;
    if filters.keys().any(|name| name != b"StdCF") {
        return Err(PdfError::unsupported(
            "R4 encryption contains an unsupported crypt filter",
        ));
    }
    let standard = as_dict(
        filters
            .get(b"StdCF".as_slice())
            .ok_or_else(|| PdfError::unsupported("R4 encryption is missing /CF /StdCF"))?,
        "encryption /CF /StdCF",
    )?;
    match standard.get(b"AuthEvent".as_slice()) {
        None => {}
        Some(Value::Name(event)) if event == b"DocOpen" => {}
        Some(_) => {
            return Err(PdfError::unsupported(
                "R4 crypt filter AuthEvent must be /DocOpen",
            ));
        }
    }
    match standard.get(b"CFM".as_slice()) {
        Some(Value::Name(name)) if name == b"V2" => Ok(CryptMethod::Rc4),
        Some(Value::Name(name)) if name == b"AESV2" => {
            if let Some(length) = standard.get(b"Length".as_slice())
                && !matches!(length, Value::Integer(128))
            {
                return Err(PdfError::unsupported(
                    "AESV2 crypt filter /Length must be 128",
                ));
            }
            Ok(CryptMethod::AesV2)
        }
        Some(Value::Name(name)) => Err(PdfError::unsupported(format!(
            "unsupported R4 crypt filter /CFM /{}",
            String::from_utf8_lossy(name)
        ))),
        _ => Err(PdfError::unsupported("R4 crypt filter is missing /CFM")),
    }
}

fn parse_aes_v3_crypt_filter(
    dictionary: &BTreeMap<Vec<u8>, Value>,
) -> Result<CryptMethod, PdfError> {
    require_name(dictionary, b"StmF", b"StdCF")?;
    require_name(dictionary, b"StrF", b"StdCF")?;
    match dictionary.get(b"EFF".as_slice()) {
        None => {}
        Some(Value::Name(name)) if name == b"StdCF" => {}
        Some(_) => return Err(PdfError::unsupported("unsupported encryption /EFF")),
    }
    let filters = as_dict(
        dictionary
            .get(b"CF".as_slice())
            .ok_or_else(|| PdfError::unsupported("R5/R6 encryption is missing /CF"))?,
        "encryption /CF",
    )?;
    if filters.len() != 1 || !filters.contains_key(b"StdCF".as_slice()) {
        return Err(PdfError::unsupported(
            "R5/R6 encryption must contain exactly the /StdCF crypt filter",
        ));
    }
    let standard = as_dict(&filters[b"StdCF".as_slice()], "encryption /CF /StdCF")?;
    match standard.get(b"AuthEvent".as_slice()) {
        None => {}
        Some(Value::Name(event)) if event == b"DocOpen" => {}
        Some(_) => {
            return Err(PdfError::unsupported(
                "R5/R6 crypt filter AuthEvent must be /DocOpen",
            ));
        }
    }
    require_name(standard, b"CFM", b"AESV3")?;
    if let Some(length) = standard.get(b"Length".as_slice())
        && !matches!(length, Value::Integer(32))
    {
        return Err(PdfError::unsupported(
            "AESV3 crypt filter /Length must be 32 bytes",
        ));
    }
    Ok(CryptMethod::AesV3)
}

pub(crate) fn decrypt_objects(
    parsed: &mut ParsedDocument,
    security: &StandardSecurity,
    file_key: &[u8],
) -> Result<(), PdfError> {
    for (reference, object) in &mut parsed.objects {
        if Some(*reference) == security.encrypt_object || is_type(&object.value, b"XRef") {
            continue;
        }
        if object.stream.is_some() {
            reject_stream_crypt_filter(&object.value)?;
        }
        object.value = crypt_value(
            &object.value,
            *reference,
            security,
            file_key,
            false,
            0,
            parsed.limits.max_parser_depth,
            parsed.limits.max_token_bytes,
        )?;
        if let Some(stream) = object.stream.as_deref()
            && (security.encrypt_metadata || !is_type(&object.value, b"Metadata"))
        {
            let decrypted = security.decrypt_object(file_key, *reference, stream)?;
            if decrypted.len() > parsed.limits.max_stream_bytes {
                return Err(PdfError::limit("decrypted stream exceeds max_stream_bytes"));
            }
            object.stream = Some(decrypted);
        }
    }
    Ok(())
}

pub(crate) fn encrypt_objects(
    parsed: &mut ParsedDocument,
    security: &StandardSecurity,
    file_key: &[u8],
) -> Result<(), PdfError> {
    for (reference, object) in &mut parsed.objects {
        if is_type(&object.value, b"XRef") {
            continue;
        }
        if object.stream.is_some() {
            reject_stream_crypt_filter(&object.value)?;
        }
        object.value = crypt_value(
            &object.value,
            *reference,
            security,
            file_key,
            true,
            0,
            parsed.limits.max_parser_depth,
            parsed.limits.max_token_bytes,
        )?;
        if let Some(stream) = object.stream.as_deref()
            && (security.encrypt_metadata || !is_type(&object.value, b"Metadata"))
        {
            let encrypted = security.encrypt_object(file_key, *reference, stream)?;
            if encrypted.len() > parsed.limits.max_stream_bytes {
                return Err(PdfError::limit("encrypted stream exceeds max_stream_bytes"));
            }
            object.stream = Some(encrypted);
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn crypt_value(
    value: &Value,
    reference: ObjectRef,
    security: &StandardSecurity,
    file_key: &[u8],
    encrypt: bool,
    depth: usize,
    max_depth: usize,
    max_string: usize,
) -> Result<Value, PdfError> {
    if depth > max_depth {
        return Err(PdfError::limit("encrypted value depth exceeds limit"));
    }
    match value {
        Value::String(value) => {
            let output = if encrypt {
                security.encrypt_object(file_key, reference, value)?
            } else {
                security.decrypt_object(file_key, reference, value)?
            };
            if output.len() > max_string {
                return Err(PdfError::limit("encrypted string exceeds max_token_bytes"));
            }
            Ok(Value::String(output))
        }
        Value::Array(values) => values
            .iter()
            .map(|value| {
                crypt_value(
                    value,
                    reference,
                    security,
                    file_key,
                    encrypt,
                    depth + 1,
                    max_depth,
                    max_string,
                )
            })
            .collect::<Result<Vec<_>, _>>()
            .map(Value::Array),
        Value::Dict(dictionary) => dictionary
            .iter()
            .map(|(key, value)| {
                Ok((
                    key.clone(),
                    crypt_value(
                        value,
                        reference,
                        security,
                        file_key,
                        encrypt,
                        depth + 1,
                        max_depth,
                        max_string,
                    )?,
                ))
            })
            .collect::<Result<BTreeMap<_, _>, PdfError>>()
            .map(Value::Dict),
        _ => Ok(value.clone()),
    }
}

pub(crate) fn remove_encryption(
    parsed: &mut ParsedDocument,
    encrypt_object: Option<ObjectRef>,
) -> Result<(), PdfError> {
    let trailer = as_dict_mut(&mut parsed.trailer, "trailer")?;
    trailer.remove(b"Encrypt".as_slice());
    if let Some(reference) = encrypt_object {
        parsed.objects.remove(&reference);
    }
    Ok(())
}

fn install_encryption(
    parsed: &mut ParsedDocument,
    security: &StandardSecurity,
    reference: ObjectRef,
) -> Result<(), PdfError> {
    let mut dictionary = BTreeMap::new();
    dictionary.insert(b"Filter".to_vec(), Value::Name(b"Standard".to_vec()));
    dictionary.insert(b"V".to_vec(), Value::Integer(security.version));
    dictionary.insert(b"R".to_vec(), Value::Integer(security.revision));
    dictionary.insert(b"O".to_vec(), Value::String(security.owner_key.clone()));
    dictionary.insert(b"U".to_vec(), Value::String(security.user_key.clone()));
    dictionary.insert(
        b"P".to_vec(),
        Value::Integer(i64::from(security.permissions)),
    );
    if security.revision == 3 {
        dictionary.insert(
            b"Length".to_vec(),
            Value::Integer(
                i64::try_from(security.key_length * 8)
                    .map_err(|_| PdfError::limit("R3 key length exceeds i64"))?,
            ),
        );
    }
    if security.revision >= 4 {
        let modern = security.revision >= 5;
        dictionary.insert(
            b"Length".to_vec(),
            Value::Integer(if modern { 256 } else { 128 }),
        );
        dictionary.insert(b"EncryptMetadata".to_vec(), Value::Bool(true));
        dictionary.insert(b"StmF".to_vec(), Value::Name(b"StdCF".to_vec()));
        dictionary.insert(b"StrF".to_vec(), Value::Name(b"StdCF".to_vec()));
        let mut standard = BTreeMap::new();
        standard.insert(
            b"CFM".to_vec(),
            Value::Name(security.method.label().as_bytes().to_vec()),
        );
        standard.insert(b"AuthEvent".to_vec(), Value::Name(b"DocOpen".to_vec()));
        standard.insert(
            b"Length".to_vec(),
            Value::Integer(if modern { 32 } else { 128 }),
        );
        let mut filters = BTreeMap::new();
        filters.insert(b"StdCF".to_vec(), Value::Dict(standard));
        dictionary.insert(b"CF".to_vec(), Value::Dict(filters));
        if modern {
            dictionary.insert(
                b"OE".to_vec(),
                Value::String(security.owner_encrypted_key.clone()),
            );
            dictionary.insert(
                b"UE".to_vec(),
                Value::String(security.user_encrypted_key.clone()),
            );
            dictionary.insert(
                b"Perms".to_vec(),
                Value::String(security.encrypted_permissions.clone()),
            );
        }
    }
    parsed.objects.insert(
        reference,
        IndirectObject {
            value: Value::Dict(dictionary),
            stream: None,
            stream_offset: 0,
            offset: 0,
        },
    );
    let trailer = as_dict_mut(&mut parsed.trailer, "trailer")?;
    trailer.insert(b"Encrypt".to_vec(), Value::Ref(reference));
    trailer.insert(
        b"ID".to_vec(),
        Value::Array(vec![
            Value::String(security.file_id.clone()),
            Value::String(security.file_id.clone()),
        ]),
    );
    let size = usize::try_from(reference.number)
        .ok()
        .and_then(|number| number.checked_add(1))
        .ok_or_else(|| PdfError::limit("encrypted trailer /Size overflows"))?;
    trailer.insert(
        b"Size".to_vec(),
        Value::Integer(
            i64::try_from(size).map_err(|_| PdfError::limit("trailer /Size exceeds i64"))?,
        ),
    );
    Ok(())
}

pub(crate) fn write_encrypted_pdf(
    document: &PdfDocument,
    parsed: &ParsedDocument,
) -> Result<Vec<u8>, PdfError> {
    let mut output = Output::new(document.engine_config().limits.max_output_bytes);
    output.push(b"%PDF-")?;
    output.push(parsed.version.as_bytes())?;
    output.push(b"\n%\xE2\xE3\xCF\xD3\n")?;
    let mut offsets = BTreeMap::new();
    for (reference, object) in &parsed.objects {
        if offsets.contains_key(&reference.number) {
            return Err(PdfError::unsafe_rewrite(
                "encrypted output cannot represent duplicate object generations",
            ));
        }
        let offset = output.len();
        require_classic_offset(offset)?;
        offsets.insert(reference.number, (offset, reference.generation));
        output.formatted(format_args!(
            "{} {} obj\n",
            reference.number, reference.generation
        ))?;
        write_object(&mut output, object, parsed.limits.max_parser_depth)?;
        output.push(b"\nendobj\n")?;
    }
    let graph_size = offsets
        .keys()
        .next_back()
        .copied()
        .and_then(|number| usize::try_from(number).ok())
        .and_then(|number| number.checked_add(1))
        .unwrap_or(1);
    let size = dict_integer(&parsed.trailer, b"Size")
        .and_then(|value| usize::try_from(value).ok())
        .unwrap_or(0)
        .max(graph_size)
        .max(1);
    if size > parsed.limits.max_xref_entries {
        return Err(PdfError::limit("encrypted xref size exceeds limit"));
    }
    let xref_offset = output.len();
    require_classic_offset(xref_offset)?;
    output.formatted(format_args!("xref\n0 {size}\n"))?;
    output.push(b"0000000000 65535 f \n")?;
    for number in 1..size {
        match u32::try_from(number)
            .ok()
            .and_then(|number| offsets.get(&number))
        {
            Some((offset, generation)) => {
                output.formatted(format_args!("{offset:010} {generation:05} n \n"))?
            }
            None => output.push(b"0000000000 00000 f \n")?,
        }
    }
    output.formatted(format_args!("trailer\n<< /Size {size}"))?;
    for key in [
        b"Root".as_slice(),
        b"Info".as_slice(),
        b"ID".as_slice(),
        b"Encrypt".as_slice(),
    ] {
        if let Some(value) = dict_get(&parsed.trailer, key) {
            output.push(b" ")?;
            write_name(&mut output, key)?;
            output.push(b" ")?;
            write_value(&mut output, value, 0, parsed.limits.max_parser_depth)?;
        }
    }
    output.formatted(format_args!(">>\nstartxref\n{xref_offset}\n%%EOF\n"))?;
    Ok(output.into_bytes())
}

pub(crate) fn objects_semantically_equal(left: &ParsedDocument, right: &ParsedDocument) -> bool {
    left.objects.len() == right.objects.len()
        && left.objects.iter().all(|(reference, object)| {
            right
                .objects
                .get(reference)
                .is_some_and(|other| object.value == other.value && object.stream == other.stream)
        })
}

fn compute_owner_entry(
    user_password: &[u8],
    owner_password: &[u8],
    revision: i64,
    key_length: usize,
) -> Result<Vec<u8>, PdfError> {
    let owner_password = if owner_password.is_empty() {
        user_password
    } else {
        owner_password
    };
    let mut digest = Md5::digest(pad_password(owner_password)).to_vec();
    if revision >= 3 {
        for _ in 0..50 {
            digest = Md5::digest(&digest).to_vec();
        }
    }
    let key = &digest[..key_length];
    let mut output = rc4_crypt(key, &pad_password(user_password))?;
    if revision >= 3 {
        for iteration in 1..=19 {
            output = rc4_crypt(&xor_key(key, iteration), &output)?;
        }
    }
    Ok(output)
}

fn recover_user_password_from_owner(
    owner_password: &[u8],
    security: &StandardSecurity,
) -> Result<Vec<u8>, PdfError> {
    let mut digest = Md5::digest(pad_password(owner_password)).to_vec();
    if security.revision >= 3 {
        for _ in 0..50 {
            digest = Md5::digest(&digest).to_vec();
        }
    }
    let key = &digest[..security.key_length];
    let mut output = security.owner_key.clone();
    if security.revision == 2 {
        return rc4_crypt(key, &output);
    }
    for iteration in (0..=19).rev() {
        output = rc4_crypt(&xor_key(key, iteration), &output)?;
    }
    Ok(output)
}

fn aes_v2_encrypt(key: &[u8], plaintext: &[u8]) -> Result<Vec<u8>, PdfError> {
    let mut iv = [0_u8; 16];
    secure_random(&mut iv)?;
    let mut output = iv.to_vec();
    let encrypted = Encryptor::<Aes128>::new_from_slices(key, &iv)
        .map_err(|_| PdfError::unsupported("invalid AESV2 object key length"))?
        .encrypt_padded_vec_mut::<Pkcs7>(plaintext);
    output.extend_from_slice(&encrypted);
    Ok(output)
}

fn aes_v2_decrypt(key: &[u8], ciphertext: &[u8]) -> Result<Vec<u8>, PdfError> {
    if ciphertext.len() < 32 || !(ciphertext.len() - 16).is_multiple_of(16) {
        return Err(PdfError::syntax("malformed AESV2 object ciphertext", 0));
    }
    Decryptor::<Aes128>::new_from_slices(key, &ciphertext[..16])
        .map_err(|_| PdfError::unsupported("invalid AESV2 object key length"))?
        .decrypt_padded_vec_mut::<Pkcs7>(&ciphertext[16..])
        .map_err(|_| PdfError::syntax("malformed AESV2 object padding", 0))
}

fn aes_v3_encrypt(key: &[u8], plaintext: &[u8]) -> Result<Vec<u8>, PdfError> {
    let mut iv = [0_u8; 16];
    secure_random(&mut iv)?;
    let mut output = iv.to_vec();
    let encrypted = Encryptor::<Aes256>::new_from_slices(key, &iv)
        .map_err(|_| PdfError::unsupported("invalid AESV3 file key length"))?
        .encrypt_padded_vec_mut::<Pkcs7>(plaintext);
    output.extend_from_slice(&encrypted);
    Ok(output)
}

fn aes_v3_decrypt(key: &[u8], ciphertext: &[u8]) -> Result<Vec<u8>, PdfError> {
    if ciphertext.len() < 32 || !(ciphertext.len() - 16).is_multiple_of(16) {
        return Err(PdfError::syntax("malformed AESV3 object ciphertext", 0));
    }
    Decryptor::<Aes256>::new_from_slices(key, &ciphertext[..16])
        .map_err(|_| PdfError::unsupported("invalid AESV3 file key length"))?
        .decrypt_padded_vec_mut::<Pkcs7>(&ciphertext[16..])
        .map_err(|_| PdfError::syntax("malformed AESV3 object padding", 0))
}

fn aes256_cbc_no_padding_encrypt(key: &[u8], input: &[u8]) -> Result<Vec<u8>, PdfError> {
    if input.is_empty() || !input.len().is_multiple_of(16) {
        return Err(PdfError::syntax("AES-256 input is not whole blocks", 0));
    }
    Ok(Encryptor::<Aes256>::new_from_slices(key, &[0_u8; 16])
        .map_err(|_| PdfError::unsupported("invalid AES-256 key length"))?
        .encrypt_padded_vec_mut::<NoPadding>(input))
}

fn aes256_cbc_no_padding_decrypt(key: &[u8], input: &[u8]) -> Result<Vec<u8>, PdfError> {
    if input.is_empty() || !input.len().is_multiple_of(16) {
        return Err(PdfError::syntax("AES-256 input is not whole blocks", 0));
    }
    Decryptor::<Aes256>::new_from_slices(key, &[0_u8; 16])
        .map_err(|_| PdfError::unsupported("invalid AES-256 key length"))?
        .decrypt_padded_vec_mut::<NoPadding>(input)
        .map_err(|_| PdfError::syntax("malformed AES-256 ciphertext", 0))
}

fn aes256_hash(
    revision: i64,
    password: &[u8],
    salt: &[u8],
    user_data: &[u8],
) -> Result<Vec<u8>, PdfError> {
    if !matches!(revision, 5 | 6) || salt.len() != 8 || password.len() > 127 {
        return Err(PdfError::unsupported(
            "invalid R5/R6 password hash parameters",
        ));
    }
    if !matches!(user_data.len(), 0 | 48) {
        return Err(PdfError::unsupported(
            "invalid R5/R6 password hash user data",
        ));
    }
    let mut initial = Sha256::new();
    initial.update(password);
    initial.update(salt);
    initial.update(user_data);
    let mut key = initial.finalize().to_vec();
    if revision == 5 {
        return Ok(key);
    }
    for round in 1_u16..=287 {
        let mut sequence = Vec::with_capacity(password.len() + key.len() + user_data.len());
        sequence.extend_from_slice(password);
        sequence.extend_from_slice(&key);
        sequence.extend_from_slice(user_data);
        let encrypted = Encryptor::<Aes128>::new_from_slices(&key[..16], &key[16..32])
            .map_err(|_| PdfError::unsupported("invalid R6 intermediate key"))?
            .encrypt_padded_vec_mut::<NoPadding>(&sequence.repeat(64));
        key = match encrypted[..16]
            .iter()
            .map(|byte| usize::from(*byte))
            .sum::<usize>()
            % 3
        {
            0 => Sha256::digest(&encrypted).to_vec(),
            1 => Sha384::digest(&encrypted).to_vec(),
            _ => Sha512::digest(&encrypted).to_vec(),
        };
        if round >= 64 && u16::from(*encrypted.last().unwrap()) <= round - 32 {
            return Ok(key[..32].to_vec());
        }
    }
    Err(PdfError::verification("R6 password hash did not converge"))
}

fn make_permissions(
    key: &[u8],
    permissions: i32,
    encrypt_metadata: bool,
) -> Result<Vec<u8>, PdfError> {
    let mut block = [0_u8; 16];
    block[..4].copy_from_slice(&permissions.to_le_bytes());
    block[4..8].fill(0xff);
    block[8] = if encrypt_metadata { b'T' } else { b'F' };
    block[9..12].copy_from_slice(b"adb");
    secure_random(&mut block[12..])?;
    let cipher = Aes256::new_from_slice(key)
        .map_err(|_| PdfError::unsupported("invalid AES-256 permissions key"))?;
    cipher.encrypt_block((&mut block).into());
    Ok(block.to_vec())
}

fn verify_permissions(
    key: &[u8],
    encrypted: &[u8],
    permissions: i32,
    encrypt_metadata: bool,
) -> Result<bool, PdfError> {
    if encrypted.len() != 16 {
        return Ok(false);
    }
    let mut block = aes::Block::clone_from_slice(encrypted);
    let cipher = Aes256::new_from_slice(key)
        .map_err(|_| PdfError::unsupported("invalid AES-256 permissions key"))?;
    cipher.decrypt_block(&mut block);
    let expected_metadata = if encrypt_metadata { b'T' } else { b'F' };
    Ok(bool::from(block[..4].ct_eq(&permissions.to_le_bytes()))
        && bool::from(block[4..8].ct_eq(&[0xff; 4]))
        && block[8] == expected_metadata
        && bool::from(block[9..12].ct_eq(b"adb")))
}

fn prepare_aes256_password(password: &str) -> Result<Vec<u8>, PdfError> {
    let prepared = stringprep::saslprep(password).map_err(|_| PdfError {
        code: PdfErrorCode::InvalidSyntax,
        message: "R5/R6 password is not valid SASLprep input".into(),
        span: None,
        object: None,
    })?;
    Ok(prepared.as_bytes()[..prepared.len().min(127)].to_vec())
}

fn rc4_crypt(key: &[u8], input: &[u8]) -> Result<Vec<u8>, PdfError> {
    macro_rules! apply {
        ($size:ty) => {{
            let mut cipher = Rc4::<$size>::new_from_slice(key)
                .map_err(|_| PdfError::unsupported("invalid RC4 key length"))?;
            let mut output = input.to_vec();
            cipher.apply_keystream(&mut output);
            Ok(output)
        }};
    }
    match key.len() {
        5 => apply!(U5),
        6 => apply!(U6),
        7 => apply!(U7),
        8 => apply!(U8),
        9 => apply!(U9),
        10 => apply!(U10),
        11 => apply!(U11),
        12 => apply!(U12),
        13 => apply!(U13),
        14 => apply!(U14),
        15 => apply!(U15),
        16 => apply!(U16),
        _ => Err(PdfError::unsupported("unsupported RC4 key length")),
    }
}

fn pad_password(password: &[u8]) -> [u8; 32] {
    let mut output = [0_u8; 32];
    let length = password.len().min(32);
    output[..length].copy_from_slice(&password[..length]);
    output[length..].copy_from_slice(&PASSWORD_PADDING[..32 - length]);
    output
}

fn xor_key(key: &[u8], value: u8) -> Vec<u8> {
    key.iter().map(|byte| byte ^ value).collect()
}

fn secure_random(output: &mut [u8]) -> Result<(), PdfError> {
    getrandom::fill(output).map_err(|_| PdfError {
        code: PdfErrorCode::Internal,
        message: "secure random generation failed".into(),
        span: None,
        object: None,
    })
}

fn reject_stream_crypt_filter(value: &Value) -> Result<(), PdfError> {
    let Some(filter) = dict_get(value, b"Filter") else {
        return Ok(());
    };
    let has_crypt = match filter {
        Value::Name(name) => name == b"Crypt",
        Value::Array(values) => values
            .iter()
            .any(|value| matches!(value, Value::Name(name) if name == b"Crypt")),
        _ => false,
    };
    if has_crypt {
        return Err(PdfError::unsupported(
            "stream-level /Crypt filters are not implemented",
        ));
    }
    Ok(())
}

fn is_type(value: &Value, expected: &[u8]) -> bool {
    matches!(dict_get(value, b"Type"), Some(Value::Name(name)) if name == expected)
}

fn require_name(
    dictionary: &BTreeMap<Vec<u8>, Value>,
    key: &[u8],
    expected: &[u8],
) -> Result<(), PdfError> {
    match dictionary.get(key) {
        Some(Value::Name(value)) if value == expected => Ok(()),
        _ => Err(PdfError::unsupported(format!(
            "encryption /{} must be /{}",
            String::from_utf8_lossy(key),
            String::from_utf8_lossy(expected)
        ))),
    }
}

fn require_integer(dictionary: &BTreeMap<Vec<u8>, Value>, key: &[u8]) -> Result<i64, PdfError> {
    match dictionary.get(key) {
        Some(Value::Integer(value)) => Ok(*value),
        _ => Err(PdfError::unsupported(format!(
            "encryption /{} must be an integer",
            String::from_utf8_lossy(key)
        ))),
    }
}

fn require_string<'a>(
    dictionary: &'a BTreeMap<Vec<u8>, Value>,
    key: &[u8],
) -> Result<&'a [u8], PdfError> {
    match dictionary.get(key) {
        Some(Value::String(value)) => Ok(value),
        _ => Err(PdfError::unsupported(format!(
            "encryption /{} must be a direct string",
            String::from_utf8_lossy(key)
        ))),
    }
}

fn require_sized_string<'a>(
    dictionary: &'a BTreeMap<Vec<u8>, Value>,
    key: &[u8],
    size: usize,
) -> Result<&'a [u8], PdfError> {
    let value = require_string(dictionary, key)?;
    require_entry_size(value, key, size)?;
    Ok(value)
}

fn require_entry_size(value: &[u8], key: &[u8], size: usize) -> Result<(), PdfError> {
    if value.len() != size {
        return Err(PdfError::unsupported(format!(
            "encryption /{} must contain exactly {size} bytes",
            String::from_utf8_lossy(key)
        )));
    }
    Ok(())
}

fn validate_key_bits(revision: i64, bits: i64) -> Result<usize, PdfError> {
    if revision == 2 && bits == 40 {
        return Ok(5);
    }
    if revision == 3 && (40..=128).contains(&bits) && bits % 8 == 0 {
        return usize::try_from(bits / 8)
            .map_err(|_| PdfError::unsupported("R3 encryption /Length exceeds usize"));
    }
    Err(PdfError::unsupported(format!(
        "Standard Security R{revision} RC4 key length must be 40 to 128 bits in 8-bit increments"
    )))
}

fn as_dict<'a>(value: &'a Value, label: &str) -> Result<&'a BTreeMap<Vec<u8>, Value>, PdfError> {
    match value {
        Value::Dict(value) => Ok(value),
        _ => Err(PdfError::syntax(format!("{label} is not a dictionary"), 0)),
    }
}

fn as_dict_mut<'a>(
    value: &'a mut Value,
    label: &str,
) -> Result<&'a mut BTreeMap<Vec<u8>, Value>, PdfError> {
    match value {
        Value::Dict(value) => Ok(value),
        _ => Err(PdfError::syntax(format!("{label} is not a dictionary"), 0)),
    }
}

fn dict_get<'a>(value: &'a Value, key: &[u8]) -> Option<&'a Value> {
    match value {
        Value::Dict(dictionary) => dictionary.get(key),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode_hex(value: &str) -> Vec<u8> {
        value
            .as_bytes()
            .chunks_exact(2)
            .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
            .collect()
    }

    #[test]
    fn revision_two_authoritative_key_and_object_vector() {
        let security = StandardSecurity {
            version: 1,
            revision: 2,
            key_length: 5,
            owner_key: decode_hex(
                "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            ),
            user_key: decode_hex(
                "f2e39758794025127846c07006961602075fb11d4b5c227e0b27c9a2abb73e61",
            ),
            owner_encrypted_key: Vec::new(),
            user_encrypted_key: Vec::new(),
            encrypted_permissions: Vec::new(),
            permissions: -44,
            file_id: b"fixture-file-id1".to_vec(),
            encrypt_metadata: true,
            method: CryptMethod::Rc4,
            encrypt_object: None,
        };
        let file_key = security.authenticate(b"user").unwrap();
        assert_eq!(file_key, decode_hex("317dc68b94"));
        assert_eq!(
            security
                .object_key(
                    &file_key,
                    ObjectRef {
                        number: 12,
                        generation: 0
                    }
                )
                .unwrap(),
            decode_hex("ad9e0e14aac91f8edcdd")
        );
        assert_eq!(
            security
                .decrypt_object(
                    &file_key,
                    ObjectRef {
                        number: 12,
                        generation: 0
                    },
                    &decode_hex("089e5e3660ee770d564de9201bf15209a62290")
                )
                .unwrap(),
            b"Secret object bytes"
        );
    }

    #[test]
    fn revision_five_and_six_authoritative_hash_vectors() {
        let salt = decode_hex("0011223344556677");
        let owner_salt = decode_hex("8899aabbccddeeff");
        let user_data = (0_u8..48).collect::<Vec<_>>();
        assert_eq!(
            aes256_hash(5, b"password", &salt, &[]).unwrap(),
            decode_hex("00fe95dc06ac5418bb02db45df1cec6c4027f9b17799260fa9d4eac389c8069f")
        );
        assert_eq!(
            aes256_hash(5, b"owner", &owner_salt, &user_data).unwrap(),
            decode_hex("3231239aeabfccc18ca41862bc4075973f2503a264fbb053b3042f18f99122ea")
        );
        assert_eq!(
            aes256_hash(6, b"password", &salt, &[]).unwrap(),
            decode_hex("03ccc9a6b2caf2fa710326f2b867dfe523a6006e711411738233fa1831db58fb")
        );
        assert_eq!(
            aes256_hash(6, b"owner", &owner_salt, &user_data).unwrap(),
            decode_hex("659a6a83bf3ec2ea5519c00f85942c9a53fb5b42cc6df670213943c5f8e02fe1")
        );
    }

    #[test]
    fn revision_three_qpdf_key_vector_authenticates_user_and_owner() {
        let security = StandardSecurity {
            version: 2,
            revision: 3,
            key_length: 16,
            owner_key: decode_hex(
                "3a59a4c4747915b0dc733cb81e3c81530679739dac36732902d1c913ed95ff72",
            ),
            user_key: decode_hex(
                "ec6652447aa5176e384415220b40a70d0122456a91bae5134273a6db134c87c4",
            ),
            owner_encrypted_key: Vec::new(),
            user_encrypted_key: Vec::new(),
            encrypted_permissions: Vec::new(),
            permissions: -4,
            file_id: decode_hex("fce2fe96b7e142b4a0576f61e2e9c441"),
            encrypt_metadata: true,
            method: CryptMethod::Rc4,
            encrypt_object: None,
        };
        assert_eq!(
            security.authenticate(b"asdfzxcv").unwrap(),
            decode_hex("074958d8cfbbfb5bfb2ab6a91514cbdb")
        );
        assert_eq!(
            security
                .object_key(
                    &decode_hex("074958d8cfbbfb5bfb2ab6a91514cbdb"),
                    ObjectRef {
                        number: 1,
                        generation: 0,
                    },
                )
                .unwrap(),
            decode_hex("83dd75679be55a29a91428c75571529b")
        );

        let owner_security = StandardSecurity {
            version: 2,
            revision: 3,
            key_length: 16,
            owner_key: decode_hex(
                "566fa873ee33c797cd3b904fdadf814afa34df9a38f6ed41b984e2c6da2aa6f5",
            ),
            user_key: decode_hex(
                "60cb14e897d58381f951427876a7804128bf4e5e4e758a4164004e56fffa0108",
            ),
            owner_encrypted_key: Vec::new(),
            user_encrypted_key: Vec::new(),
            encrypted_permissions: Vec::new(),
            permissions: -4,
            file_id: decode_hex("00112233445566778899aabbccddeeff"),
            encrypt_metadata: true,
            method: CryptMethod::Rc4,
            encrypt_object: None,
        };
        assert_eq!(
            owner_security.authenticate(b"owner").unwrap(),
            decode_hex("8307780609c4a1fb1c5420a245859290")
        );
    }
}
