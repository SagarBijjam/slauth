use ring::{
    signature,
    signature::{UnparsedPublicKey, VerificationAlgorithm},
};
use sha2::{Digest, Sha256};
use webpki::{EndEntityCert, SignatureAlgorithm};

use crate::webauthn::{
    error::{CredentialError, Error},
    proto::{
        constants::{
            ECDSA_CURVE_P256, ECDSA_CURVE_P384, ECDSA_Y_PREFIX_UNCOMPRESSED, WEBAUTHN_COSE_ALGORITHM_IDENTIFIER_EC2,
            WEBAUTHN_COSE_ALGORITHM_IDENTIFIER_RSA, WEBAUTHN_REQUEST_TYPE_CREATE, WEBAUTHN_REQUEST_TYPE_GET, WEBAUTHN_USER_PRESENT_FLAG,
            WEBAUTHN_USER_VERIFIED_FLAG, WEBAUTH_PUBLIC_KEY_TYPE_EC2,
        },
        raw_message::{AttestationObject, AttestationStatement, AuthenticatorData, Coordinates, CredentialPublicKey, Message},
        web_message::{
            AttestationConveyancePreference, AuthenticatorSelectionCriteria, CollectedClientData, PublicKeyCredential,
            PublicKeyCredentialCreationOptions, PublicKeyCredentialDescriptor, PublicKeyCredentialParameters,
            PublicKeyCredentialRequestOptions, PublicKeyCredentialRpEntity, PublicKeyCredentialType, PublicKeyCredentialUserEntity,
            UserVerificationRequirement,
        },
    },
};

pub struct CredentialCreationBuilder {
    challenge: Option<String>,
    user: Option<User>,
    rp: Option<Rp>,
    user_verification_requirement: Option<UserVerificationRequirement>,
    exclude_credentials: Vec<PublicKeyCredentialDescriptor>,
}

impl CredentialCreationBuilder {
    pub fn new() -> Self {
        CredentialCreationBuilder {
            challenge: None,
            user: None,
            rp: None,
            user_verification_requirement: None,
            exclude_credentials: vec![],
        }
    }

    pub fn challenge(mut self, challenge: String) -> Self {
        self.challenge = Some(challenge);
        self
    }

    pub fn user(mut self, id: String, name: String, display_name: String, icon: Option<String>) -> Self {
        self.user = Some(User {
            id,
            name,
            display_name,
            icon,
        });
        self
    }

    pub fn rp(mut self, name: String, icon: Option<String>, id: Option<String>) -> Self {
        self.rp = Some(Rp { name, icon, id });
        self
    }

    pub fn user_verification_requirement<T: Into<Option<UserVerificationRequirement>>>(mut self, uvr: T) -> Self {
        self.user_verification_requirement = uvr.into();
        self
    }

    pub fn exclude_credentials(mut self, exclude_credentials: Vec<PublicKeyCredentialDescriptor>) -> Self {
        self.exclude_credentials = exclude_credentials;
        self
    }

    pub fn build(self) -> Result<PublicKeyCredentialCreationOptions, Error> {
        let challenge = self
            .challenge
            .ok_or_else(|| Error::Other("Unable to build a WebAuthn request without a challenge".to_string()))?;

        let user = self
            .user
            .map(|user| PublicKeyCredentialUserEntity {
                id: user.id,
                name: user.name,
                display_name: user.display_name,
                icon: user.icon,
            })
            .ok_or_else(|| Error::Other("Unable to build a WebAuthn request without a user".to_string()))?;

        let rp = self
            .rp
            .map(|rp| PublicKeyCredentialRpEntity {
                id: rp.id,
                name: rp.name,
                icon: rp.icon,
            })
            .ok_or_else(|| Error::Other("Unable to build a WebAuthn request without a relying party".to_string()))?;

        Ok(PublicKeyCredentialCreationOptions {
            rp,
            user,
            challenge,
            pub_key_cred_params: vec![PublicKeyCredentialParameters {
                auth_type: PublicKeyCredentialType::PublicKey,
                alg: WEBAUTHN_COSE_ALGORITHM_IDENTIFIER_EC2,
            }],
            timeout: None,
            exclude_credentials: self.exclude_credentials,
            authenticator_selection: Some(AuthenticatorSelectionCriteria {
                authenticator_attachment: None,
                require_resident_key: None,
                user_verification: self.user_verification_requirement,
            }),
            attestation: Some(AttestationConveyancePreference::Direct),
            extensions: None,
        })
    }
}

struct User {
    pub id: String,
    pub name: String,
    pub display_name: String,
    pub icon: Option<String>,
}

struct Rp {
    pub name: String,
    pub icon: Option<String>,
    pub id: Option<String>,
}

pub struct CredentialCreationResult {
    pub public_key: CredentialPublicKey,
    pub sign_count: u32,
    pub has_user_verification: bool,
}

pub struct CredentialCreationVerifier {
    pub credential: PublicKeyCredential,
    pub context: PublicKeyCredentialCreationOptions,
    origin: String,
    cert: Option<Vec<u8>>,
}

impl CredentialCreationVerifier {
    pub fn new(credential: PublicKeyCredential, context: PublicKeyCredentialCreationOptions, origin: &str) -> Self {
        CredentialCreationVerifier {
            credential,
            context,
            cert: None,
            origin: origin.to_string(),
        }
    }

    pub fn get_cert(&self) -> Result<EndEntityCert, Error> {
        if let Some(cert) = &self.cert {
            webpki::EndEntityCert::from(cert.as_slice()).map_err(|e| Error::WebPkiError(e))
        } else {
            Err(Error::Other("certificate is missing or has not been verified yet".to_string()))
        }
    }

    pub fn verify(&mut self) -> Result<CredentialCreationResult, Error> {
        let response = self
            .credential
            .response
            .as_ref()
            .ok_or_else(|| Error::Other("Client data must be present for verification".to_string()))?;

        let client_data_json = base64::decode(&response.client_data_json)?;
        let client_data = serde_json::from_slice::<CollectedClientData>(client_data_json.as_slice())?;

        let raw_attestation = response
            .attestation_object
            .as_ref()
            .ok_or_else(|| Error::Other("attestation object must be present for verification".to_string()))?;
        let attestation = AttestationObject::from_base64(raw_attestation)?;

        if client_data.request_type != WEBAUTHN_REQUEST_TYPE_CREATE {
            return Err(Error::CredentialError(CredentialError::RequestType));
        }

        if client_data.challenge != self.context.challenge {
            return Err(Error::CredentialError(CredentialError::Challenge));
        }

        if client_data.origin != self.origin {
            return Err(Error::CredentialError(CredentialError::Origin));
        }

        let mut hasher = Sha256::new();
        hasher.input(client_data_json);
        let mut client_data_hash = hasher.result_reset().to_vec();

        hasher.input(self.context.rp.id.as_ref().unwrap_or(&self.origin));
        if attestation.auth_data.rp_id_hash != hasher.result().as_slice() {
            return Err(Error::CredentialError(CredentialError::Rp));
        }

        if (attestation.auth_data.flags & WEBAUTHN_USER_PRESENT_FLAG) == 0 {
            return Err(Error::CredentialError(CredentialError::UserPresentFlag));
        }

        let has_user_verification = (attestation.auth_data.flags & WEBAUTHN_USER_VERIFIED_FLAG) != 0;
        if let Some(Some(UserVerificationRequirement::Required)) = self
            .context
            .authenticator_selection
            .as_ref()
            .map(|auth_select| auth_select.user_verification.as_ref())
        {
            if !has_user_verification {
                return Err(Error::CredentialError(CredentialError::UserVerifiedFlag));
            }
        }

        if let Some(extensions) = &self.context.extensions {
            if !extensions.is_null() {
                return Err(Error::CredentialError(CredentialError::Extensions));
            }
        }

        let attested_credential_data = attestation
            .auth_data
            .attested_credential_data
            .ok_or_else(|| Error::CredentialError(CredentialError::AttestationMissing))?;

        if attested_credential_data.credential_public_key.key_type != WEBAUTH_PUBLIC_KEY_TYPE_EC2 {
            return Err(Error::CredentialError(CredentialError::KeyType));
        }

        match attestation.att_stmt {
            Some(AttestationStatement::Packed(packed)) => match packed.x5c {
                Some(serde_cbor::Value::Array(mut cert_arr)) => match cert_arr.pop() {
                    Some(serde_cbor::Value::Bytes(cert)) => {
                        let mut msg = attestation.raw_auth_data.clone();
                        msg.append(&mut client_data_hash);

                        self.cert = Some(cert.clone());
                        let web_cert = webpki::EndEntityCert::from(cert.as_slice())?;
                        if let Err(e) = web_cert.verify_signature(get_alg_from_cose(packed.alg), msg.as_slice(), packed.sig.as_slice()) {
                            return Err(Error::WebPkiError(e));
                        }
                    }

                    _ => {
                        return Err(Error::CredentialError(CredentialError::CertificateNotSupported));
                    }
                },
                _ => {
                    return Err(Error::CredentialError(CredentialError::CertificateNotSupported));
                }
            },

            Some(AttestationStatement::FidoU2F(fido_u2f)) => {
                if let Some(serde_cbor::Value::Array(mut cert_arr)) = fido_u2f.x5c {
                    match cert_arr.pop() {
                        Some(serde_cbor::Value::Bytes(cert)) => {
                            let mut public_key_u2f = attested_credential_data.credential_public_key.coords.to_vec().clone();
                            let mut msg = Vec::new();
                            msg.push(0x00);
                            msg.append(&mut attestation.auth_data.rp_id_hash.to_vec());
                            msg.append(&mut client_data_hash);
                            msg.append(&mut attested_credential_data.credential_id.clone());
                            msg.append(&mut public_key_u2f);

                            self.cert = Some(cert.clone());
                            let web_cert = webpki::EndEntityCert::from(cert.as_slice())?;
                            if let Err(e) = web_cert.verify_signature(&webpki::ECDSA_P256_SHA256, msg.as_slice(), fido_u2f.sig.as_slice()) {
                                return Err(Error::WebPkiError(e));
                            }
                        }

                        _ => {
                            return Err(Error::CredentialError(CredentialError::CertificateMissing));
                        }
                    }
                }
            }

            Some(AttestationStatement::AndroidKey(android_key)) => {
                if let Some(serde_cbor::Value::Array(mut cert_arr)) = android_key.x5c {
                    match cert_arr.pop() {
                        Some(serde_cbor::Value::Bytes(cert)) => {
                            let mut msg = attestation.raw_auth_data.clone();
                            msg.append(&mut client_data_hash);

                            self.cert = Some(cert.clone());
                            let web_cert = webpki::EndEntityCert::from(cert.as_slice())?;
                            if let Err(e) =
                                web_cert.verify_signature(get_alg_from_cose(android_key.alg), msg.as_slice(), android_key.sig.as_slice())
                            {
                                return Err(Error::WebPkiError(e));
                            }
                        }

                        _ => {
                            return Err(Error::CredentialError(CredentialError::CertificateMissing));
                        }
                    }
                }
            }

            Some(AttestationStatement::None) => {}

            _ => {
                return Err(Error::CredentialError(CredentialError::AttestationNotSupported));
            }
        }

        Ok(CredentialCreationResult {
            public_key: attested_credential_data.credential_public_key,
            sign_count: attestation.auth_data.sign_count,
            has_user_verification,
        })
    }
}

pub struct CredentialRequestResult {
    pub sign_count: u32,
    pub has_user_verification: bool,
}

pub struct CredentialRequestBuilder {
    challenge: Option<String>,
    rp: Option<String>,
    allow_credentials: Vec<String>,
    user_verification_requirement: Option<UserVerificationRequirement>,
}

impl CredentialRequestBuilder {
    pub fn new() -> Self {
        CredentialRequestBuilder {
            challenge: None,
            rp: None,
            allow_credentials: Vec::new(),
            user_verification_requirement: None,
        }
    }

    pub fn challenge(mut self, challenge: String) -> Self {
        self.challenge = Some(challenge);
        self
    }

    pub fn rp(mut self, rp_id: String) -> Self {
        self.rp = Some(rp_id);
        self
    }

    pub fn allow_credential(mut self, id: String) -> Self {
        self.allow_credentials.push(id);
        self
    }

    pub fn user_verification_requirement<T: Into<Option<UserVerificationRequirement>>>(mut self, uvr: T) -> Self {
        self.user_verification_requirement = uvr.into();
        self
    }

    pub fn build(self) -> Result<PublicKeyCredentialRequestOptions, Error> {
        let challenge = self
            .challenge
            .ok_or_else(|| Error::Other("Unable to build a WebAuthn request without a challenge".to_string()))?;
        let mut allow_credentials = Vec::new();
        self.allow_credentials.into_iter().for_each(|id| {
            allow_credentials.push(PublicKeyCredentialDescriptor {
                cred_type: PublicKeyCredentialType::PublicKey,
                id,
                transports: None,
            })
        });

        Ok(PublicKeyCredentialRequestOptions {
            challenge,
            timeout: None,
            rp_id: self.rp,
            allow_credentials,
            user_verification: self.user_verification_requirement,
            extensions: None,
        })
    }
}

fn get_alg_from_cose(id: i64) -> &'static SignatureAlgorithm {
    match id {
        WEBAUTHN_COSE_ALGORITHM_IDENTIFIER_EC2 => &webpki::ECDSA_P256_SHA256,
        WEBAUTHN_COSE_ALGORITHM_IDENTIFIER_RSA => &webpki::RSA_PKCS1_2048_8192_SHA256,
        _ => &webpki::ECDSA_P256_SHA256,
    }
}

pub struct CredentialRequestVerifier {
    pub credential: PublicKeyCredential,
    pub credential_pub: CredentialPublicKey,
    pub context: PublicKeyCredentialRequestOptions,
    origin: String,
    user_handle: String,
    sign_count: u32,
}

impl CredentialRequestVerifier {
    pub fn new(
        credential: PublicKeyCredential,
        credential_pub: CredentialPublicKey,
        context: PublicKeyCredentialRequestOptions,
        origin: &str,
        user_handle: &str,
        sign_count: u32,
    ) -> Self {
        CredentialRequestVerifier {
            credential,
            credential_pub,
            context,
            origin: origin.to_string(),
            user_handle: user_handle.to_string(),
            sign_count,
        }
    }

    pub fn verify(&mut self) -> Result<CredentialRequestResult, Error> {
        let response = self
            .credential
            .response
            .as_ref()
            .ok_or_else(|| Error::Other("Client data must be present for verification".to_string()))?;

        let signature = base64::decode(
            &response
                .signature
                .as_ref()
                .ok_or_else(|| Error::Other("Client data must be present for verification".to_string()))?,
        )?;

        let client_data_json = base64::decode(&response.client_data_json)?;
        let client_data = serde_json::from_slice::<CollectedClientData>(client_data_json.as_slice())?;

        let raw_auth_data = base64::decode(
            response
                .authenticator_data
                .as_ref()
                .ok_or_else(|| Error::Other("Attestation object must be present for verification".to_string()))?,
        )?;
        let (auth_data, raw_auth_data) = AuthenticatorData::from_vec(raw_auth_data)?;

        let credential_id = self.credential.id.clone();
        let descriptor = PublicKeyCredentialDescriptor {
            cred_type: PublicKeyCredentialType::PublicKey,
            id: credential_id,
            transports: None,
        };

        if !self.context.allow_credentials.contains(&descriptor) {
            return Err(Error::CredentialError(CredentialError::Other(String::from(
                "Specified credential is not allowed",
            ))));
        }

        if let Some(user_handle) = &response.user_handle {
            if *user_handle != self.user_handle {
                return Err(Error::CredentialError(CredentialError::Other(String::from(
                    "User handles do not match",
                ))));
            }
        }

        if client_data.request_type != WEBAUTHN_REQUEST_TYPE_GET {
            return Err(Error::CredentialError(CredentialError::Other(String::from(
                "Request type must be webauthn.get",
            ))));
        }

        if client_data.challenge != self.context.challenge {
            return Err(Error::CredentialError(CredentialError::Challenge));
        }

        if client_data.origin != self.origin {
            return Err(Error::CredentialError(CredentialError::Origin));
        }

        let mut hasher = Sha256::new();
        hasher.input(self.context.rp_id.as_ref().unwrap_or(&self.origin));
        if auth_data.rp_id_hash != hasher.result_reset().as_slice() {
            return Err(Error::CredentialError(CredentialError::Rp));
        }

        if (auth_data.flags & WEBAUTHN_USER_PRESENT_FLAG) == 0 {
            return Err(Error::CredentialError(CredentialError::UserPresentFlag));
        }

        let has_user_verification = (auth_data.flags & WEBAUTHN_USER_VERIFIED_FLAG) != 0;
        if let Some(UserVerificationRequirement::Required) = self.context.user_verification.as_ref() {
            if !has_user_verification {
                return Err(Error::CredentialError(CredentialError::UserVerifiedFlag));
            }
        }

        if let Some(extensions) = &self.context.extensions {
            if !extensions.is_null() {
                return Err(Error::CredentialError(CredentialError::Extensions));
            }
        }

        hasher.input(client_data_json);
        let mut client_data_hash = hasher.result().to_vec();

        let mut msg = raw_auth_data;
        msg.append(&mut client_data_hash);

        let mut key = Vec::new();
        match self.credential_pub.coords {
            Coordinates::Compressed { x, y } => {
                key.push(y);
                key.append(&mut x.to_vec());
            }

            Coordinates::Uncompressed { x, y } => {
                key.push(ECDSA_Y_PREFIX_UNCOMPRESSED);
                key.append(&mut x.to_vec());
                key.append(&mut y.to_vec());
            }
        }

        let signature_alg = get_ring_alg_from_cose(self.credential_pub.alg, self.credential_pub.curve)?;
        let public_key = UnparsedPublicKey::new(signature_alg, key.as_slice());
        public_key
            .verify(msg.as_slice(), signature.as_slice())
            .map_err(|_| Error::CredentialError(CredentialError::Other(String::from("Invalid public key or signature"))))?;

        if auth_data.sign_count < self.sign_count {
            return Err(Error::CredentialError(CredentialError::Other(String::from(
                "Sign count is inconsistent, might be a cloned key",
            ))));
        }

        Ok(CredentialRequestResult {
            sign_count: auth_data.sign_count,
            has_user_verification,
        })
    }
}

fn get_ring_alg_from_cose(id: i64, curve: i64) -> Result<&'static dyn VerificationAlgorithm, Error> {
    if id == WEBAUTHN_COSE_ALGORITHM_IDENTIFIER_EC2 {
        match curve {
            ECDSA_CURVE_P256 => Ok(&signature::ECDSA_P256_SHA256_ASN1),
            ECDSA_CURVE_P384 => Ok(&signature::ECDSA_P384_SHA384_ASN1),
            _ => Err(Error::CredentialError(CredentialError::Other(String::from(
                "Unsupported algorithm",
            )))),
        }
    } else {
        Err(Error::CredentialError(CredentialError::Other(String::from(
            "Unsupported algorithm",
        ))))
    }
}
