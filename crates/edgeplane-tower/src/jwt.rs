use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation, decode, encode};
use rsa::{RsaPrivateKey, pkcs8::{EncodePrivateKey, EncodePublicKey, LineEnding}};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Claims embedded in a node JWT.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NodeClaims {
    /// `node:{node_id}` — the node's identity
    pub sub: String,
    /// The raw node_id without prefix (convenience field)
    pub node_id: String,
    /// JWT ID — used for revocation lookups
    pub jti: String,
    /// Issued at (Unix timestamp)
    pub iat: i64,
    /// Expires at (Unix timestamp)
    pub exp: i64,
}

/// Sign a node JWT with the given TTL in days.
/// Returns `(compact_jwt, jti)`.
pub fn sign_node_jwt(
    node_id: &str,
    encoding_key: &EncodingKey,
    ttl_days: i64,
) -> anyhow::Result<(String, String)> {
    let now = chrono::Utc::now().timestamp();
    let jti = Uuid::new_v4().to_string();
    let claims = NodeClaims {
        sub: format!("node:{node_id}"),
        node_id: node_id.to_string(),
        jti: jti.clone(),
        iat: now,
        exp: now + ttl_days * 86400,
    };
    let token = encode(&Header::new(Algorithm::RS256), &claims, encoding_key)
        .map_err(|e| anyhow::anyhow!("JWT sign error: {e}"))?;
    Ok((token, jti))
}

/// Verify a node JWT. Returns the claims on success.
pub fn verify_node_jwt(token: &str, decoding_key: &DecodingKey) -> anyhow::Result<NodeClaims> {
    let mut validation = Validation::new(Algorithm::RS256);
    validation.validate_exp = true;
    validation.leeway = 0; // no clock-skew tolerance for node tokens
    let data = decode::<NodeClaims>(token, decoding_key, &validation)
        .map_err(|e| anyhow::anyhow!("JWT verify error: {e}"))?;
    Ok(data.claims)
}

/// Claims embedded in an agent JWT.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentClaims {
    /// `agent:{agent_id}` — the agent's identity
    pub sub: String,
    /// The raw agent_id without prefix (convenience field)
    pub agent_id: String,
    /// Home domain scope — the agent is bound to this domain
    pub domain_id: String,
    /// JWT ID — used for revocation lookups
    pub jti: String,
    /// Issued at (Unix timestamp)
    pub iat: i64,
    /// Expires at (Unix timestamp)
    pub exp: i64,
}

/// Sign an agent JWT with the given TTL in hours.
/// Returns `(compact_jwt, jti)`.
pub fn sign_agent_jwt(
    agent_id: &str,
    domain_id: &str,
    encoding_key: &EncodingKey,
    ttl_hours: i64,
) -> anyhow::Result<(String, String)> {
    let now = chrono::Utc::now().timestamp();
    let jti = Uuid::new_v4().to_string();
    let claims = AgentClaims {
        sub: format!("agent:{agent_id}"),
        agent_id: agent_id.into(),
        domain_id: domain_id.into(),
        jti: jti.clone(),
        iat: now,
        exp: now + ttl_hours * 3600,
    };
    let token = encode(&Header::new(Algorithm::RS256), &claims, encoding_key)
        .map_err(|e| anyhow::anyhow!("agent JWT sign error: {e}"))?;
    Ok((token, jti))
}

/// Verify an agent JWT. Returns the claims on success.
pub fn verify_agent_jwt(token: &str, decoding_key: &DecodingKey) -> anyhow::Result<AgentClaims> {
    let mut v = Validation::new(Algorithm::RS256);
    v.validate_exp = true;
    v.leeway = 0;
    Ok(decode::<AgentClaims>(token, decoding_key, &v)
        .map_err(|e| anyhow::anyhow!("agent JWT verify error: {e}"))?
        .claims)
}

/// Generate a new RSA-2048 keypair. Returns `(private_pkcs8_pem, public_pem)`.
pub fn generate_rsa_keypair() -> anyhow::Result<(String, String)> {
    let mut rng = rand::thread_rng();
    let private_key = RsaPrivateKey::new(&mut rng, 2048)
        .map_err(|e| anyhow::anyhow!("RSA keygen error: {e}"))?;
    let private_pem = private_key
        .to_pkcs8_pem(LineEnding::LF)
        .map_err(|e| anyhow::anyhow!("PKCS8 PEM error: {e}"))?
        .to_string();
    let public_pem = private_key
        .to_public_key()
        .to_public_key_pem(LineEnding::LF)
        .map_err(|e| anyhow::anyhow!("public PEM error: {e}"))?;
    Ok((private_pem, public_pem))
}

/// Build an `EncodingKey` from a PKCS#8 PEM string.
pub fn encoding_key_from_pem(pem: &str) -> anyhow::Result<EncodingKey> {
    EncodingKey::from_rsa_pem(pem.as_bytes())
        .map_err(|e| anyhow::anyhow!("EncodingKey error: {e}"))
}

/// Build a `DecodingKey` from an RSA public key PEM string.
pub fn decoding_key_from_pem(pem: &str) -> anyhow::Result<DecodingKey> {
    DecodingKey::from_rsa_pem(pem.as_bytes())
        .map_err(|e| anyhow::anyhow!("DecodingKey error: {e}"))
}

#[cfg(test)]
mod agent_jwt_tests {
    use super::*;

    fn keys() -> (EncodingKey, DecodingKey) {
        let (pr, pu) = generate_rsa_keypair().unwrap();
        (encoding_key_from_pem(&pr).unwrap(), decoding_key_from_pem(&pu).unwrap())
    }

    #[test]
    fn round_trip() {
        let (e, d) = keys();
        let (t, jti) = sign_agent_jwt("w7", "dom-1", &e, 1).unwrap();
        let c = verify_agent_jwt(&t, &d).unwrap();
        assert_eq!(
            (c.sub.as_str(), c.agent_id.as_str(), c.domain_id.as_str()),
            ("agent:w7", "w7", "dom-1")
        );
        assert_eq!(c.jti, jti);
    }

    #[test]
    fn agent_token_not_decodable_as_node() {
        let (e, d) = keys();
        let (t, _) = sign_agent_jwt("w7", "dom-1", &e, 1).unwrap();
        assert!(verify_node_jwt(&t, &d).is_err()); // NodeClaims requires node_id
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keypair() -> (EncodingKey, DecodingKey) {
        let (priv_pem, pub_pem) = generate_rsa_keypair().unwrap();
        (
            encoding_key_from_pem(&priv_pem).unwrap(),
            decoding_key_from_pem(&pub_pem).unwrap(),
        )
    }

    #[test]
    fn roundtrip_sign_verify() {
        let (enc, dec) = keypair();
        let (token, jti) = sign_node_jwt("node-abc123", &enc, 90).unwrap();
        let claims = verify_node_jwt(&token, &dec).unwrap();
        assert_eq!(claims.node_id, "node-abc123");
        assert_eq!(claims.sub, "node:node-abc123");
        assert_eq!(claims.jti, jti);
        assert!(claims.exp > claims.iat);
    }

    #[test]
    fn expired_token_rejected() {
        let (priv_pem, pub_pem) = generate_rsa_keypair().unwrap();
        let enc = encoding_key_from_pem(&priv_pem).unwrap();
        let dec = decoding_key_from_pem(&pub_pem).unwrap();
        // Manually craft expired claims
        let now = chrono::Utc::now().timestamp();
        let claims = NodeClaims {
            sub: "node:x".into(),
            node_id: "x".into(),
            jti: "test-jti".into(),
            iat: now - 200,
            exp: now - 1,
        };
        let token = encode(&Header::new(Algorithm::RS256), &claims, &enc).unwrap();
        assert!(verify_node_jwt(&token, &dec).is_err());
    }

    #[test]
    fn wrong_key_rejected() {
        let (enc, _) = keypair();
        let (_, dec2) = keypair();
        let (token, _) = sign_node_jwt("node-xyz", &enc, 90).unwrap();
        assert!(verify_node_jwt(&token, &dec2).is_err());
    }

    #[test]
    fn node_jwt_has_two_dots() {
        let (enc, _) = keypair();
        let (token, _) = sign_node_jwt("node-abc", &enc, 90).unwrap();
        assert_eq!(token.chars().filter(|&c| c == '.').count(), 2);
    }
}
