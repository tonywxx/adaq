//! 统一签名深模块 —— ADR-0013 `sign` 接缝的共享实现。
//!
//! 各交易所适配器的 `private_request` 仍负责**交易所专属**部分:认证串拼接
//! (binance `query+&signature` vs okx `ts+method+path+body`)与请求头名集。
//! 本模块收口与交易所无关的**全部签名原语**——HMAC / ECDSA(可恢复)/ RSA-PSS——
//! 以及凭据抽取与请求头装配,使"一处实现、全适配器可测"成立。
//!
//! 约定:`hmac(secret, msg)` —— secret 在前(归一了 binance 系 `(data, secret)` 的反向实参)。
//! 字节密钥变体(`*_bytes`)供 secret 需先解码的交易所(kraken)复用。

use crate::Config;
use crate::error::{Error, ErrorKind, Result};
use base64::Engine;
use hmac::{Hmac, KeyInit, Mac};
use libsecp256k1::{Message, SecretKey, sign};
use openssl::hash::MessageDigest;
use openssl::pkey::PKey;
use openssl::rsa::Padding;
use openssl::sign::{RsaPssSaltlen, Signer};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use sha2::{Sha256, Sha384, Sha512};

// ============================ 加密原语 ============================

/// HMAC-SHA256 → hex。
pub fn hmac_sha256_hex(secret: &str, msg: &str) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("hmac key");
    mac.update(msg.as_bytes());
    hex::encode(mac.finalize().into_bytes())
}

/// HMAC-SHA256 → base64(okx/myokx/okxus/kucoin/bitget/htx/polymarket)。
pub fn hmac_sha256_b64(secret: &str, msg: &str) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("hmac key");
    mac.update(msg.as_bytes());
    base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes())
}

/// HMAC-SHA512 → hex(gate)。
pub fn hmac_sha512_hex(secret: &str, msg: &str) -> String {
    let mut mac = Hmac::<Sha512>::new_from_slice(secret.as_bytes()).expect("hmac key");
    mac.update(msg.as_bytes());
    hex::encode(mac.finalize().into_bytes())
}

/// HMAC-SHA512 → base64(kraken)。
pub fn hmac_sha512_b64(secret: &str, msg: &str) -> String {
    let mut mac = Hmac::<Sha512>::new_from_slice(secret.as_bytes()).expect("hmac key");
    mac.update(msg.as_bytes());
    base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes())
}

/// HMAC-SHA384 → hex(gemini 部分端点)。
pub fn hmac_sha384_hex(secret: &str, msg: &str) -> String {
    let mut mac = Hmac::<Sha384>::new_from_slice(secret.as_bytes()).expect("hmac key");
    mac.update(msg.as_bytes());
    hex::encode(mac.finalize().into_bytes())
}

/// HMAC-SHA512 → base64,接受**字节密钥**(kraken 的 secret 以 base64 提供,
/// 需先 `base64::decode` 再传入;通用 [`hmac_sha512_b64`] 仅接受 `&str` 密钥)。
pub fn hmac_sha512_b64_bytes(secret: &[u8], msg: &[u8]) -> String {
    let mut mac = Hmac::<Sha512>::new_from_slice(secret).expect("hmac key");
    mac.update(msg);
    base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes())
}

// ============================ ECDSA(可恢复签名) ============================

/// 从 hex 私钥(64 hex 或 `0x` 前缀)解析 secp256k1 私钥。
pub fn secp256k1_secret_from_hex(private_key: &str) -> Result<SecretKey> {
    let clean = private_key.trim_start_matches("0x");
    let bytes = hex::decode(clean).map_err(|e| {
        Error::new(
            ErrorKind::Authentication,
            format!("invalid private key: {e}"),
        )
    })?;
    SecretKey::parse_slice(&bytes).map_err(|e| {
        Error::new(
            ErrorKind::Authentication,
            format!("invalid secp256k1 key: {e}"),
        )
    })
}

/// 对 32 字节 digest 做可恢复 ECDSA 签名,返回 `r(64hex) + s(64hex) + v(27/28)`。
/// 供 polymarket(EIP-712)等使用;EIP-712 的 digest 构造与编码留在 `eip712` 模块。
pub fn sign_ecdsa_recoverable(private_key: &str, digest: &[u8; 32]) -> Result<String> {
    let key = secp256k1_secret_from_hex(private_key)?;
    let message = Message::parse_slice(digest)
        .map_err(|e| Error::new(ErrorKind::Authentication, format!("bad message: {e}")))?;
    let (signature, recovery_id) = sign(&message, &key);
    let (r, s) = split_signature(&signature);
    let v = 27 + recovery_id.serialize();
    Ok(format!("{r}{s}{v:02x}"))
}

fn split_signature(sig: &libsecp256k1::Signature) -> (String, String) {
    let r = hex::encode(sig.r.b32());
    let s = hex::encode(sig.s.b32());
    (r, s)
}

// ============================ RSA-PSS(Kalshi 预测市场) ============================

/// 从 PEM 私钥做 RSA-PSS SHA-256 签名,输出 base64(对齐 ccxt `rsa()` → base64)。
pub fn sign_rsa_pss(payload: &str, private_key_pem: &str) -> Result<String> {
    let pkey = PKey::private_key_from_pem(private_key_pem.as_bytes()).map_err(|e| {
        Error::new(
            ErrorKind::Authentication,
            format!("invalid RSA private key PEM: {e}"),
        )
    })?;
    let mut signer = Signer::new(MessageDigest::sha256(), &pkey)
        .map_err(|e| Error::new(ErrorKind::Authentication, format!("signer init: {e}")))?;
    signer
        .set_rsa_padding(Padding::PKCS1_PSS)
        .map_err(|e| Error::new(ErrorKind::Authentication, format!("set pss: {e}")))?;
    signer
        .set_rsa_pss_saltlen(RsaPssSaltlen::DIGEST_LENGTH)
        .map_err(|e| Error::new(ErrorKind::Authentication, format!("set salt: {e}")))?;
    signer
        .update(payload.as_bytes())
        .map_err(|e| Error::new(ErrorKind::Authentication, format!("sign update: {e}")))?;
    let sig = signer
        .sign_to_vec()
        .map_err(|e| Error::new(ErrorKind::Authentication, format!("sign: {e}")))?;
    Ok(base64::engine::general_purpose::STANDARD.encode(sig))
}

// ============================ 凭据抽取 ============================

/// 取 `api_key`,缺失时返回带交易所名的 `Authentication` 错误。
pub fn require_api_key<'a>(cfg: &'a Config, exch: &str) -> Result<&'a str> {
    cfg.api_key.as_deref().ok_or_else(|| {
        Error::new(
            ErrorKind::Authentication,
            format!("{exch} api_key required"),
        )
    })
}

/// 取 `secret`,缺失时返回带交易所名的 `Authentication` 错误。
pub fn require_secret<'a>(cfg: &'a Config, exch: &str) -> Result<&'a str> {
    cfg.secret
        .as_deref()
        .ok_or_else(|| Error::new(ErrorKind::Authentication, format!("{exch} secret required")))
}

/// 取 `password`(作 passphrase,okx/kucoin),缺失时返回带交易所名的 `Authentication` 错误。
pub fn require_passphrase<'a>(cfg: &'a Config, exch: &str) -> Result<&'a str> {
    cfg.password.as_deref().ok_or_else(|| {
        Error::new(
            ErrorKind::Authentication,
            format!("{exch} passphrase required"),
        )
    })
}

// ============================ 请求头装配 ============================

/// 安全插入一个请求头。把 `HeaderName`/`HeaderValue` 构造错误转成 [`Error`],
/// 消除各适配器 `HeaderValue::from_str(...).unwrap()` 的潜在 panic。
///
/// 注意:`HeaderMap::insert` 对 `&str` 键仅接受 `'static` 字符串,故此处先把
/// `name` 构造为 [`HeaderName`](支持任意生命周期的字节),再插入。
pub fn set_header(map: &mut HeaderMap, name: &str, value: &str) -> Result<()> {
    let name = HeaderName::from_bytes(name.as_bytes()).map_err(|e| {
        Error::new(
            ErrorKind::BadRequest,
            format!("invalid header name {name}: {e}"),
        )
    })?;
    let v = HeaderValue::from_str(value).map_err(|e| {
        Error::new(
            ErrorKind::BadRequest,
            format!("invalid header value for {name}: {e}"),
        )
    })?;
    map.insert(name, v);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_hex_known_vector() {
        // binance 文档示例,secret 与 msg 均来自官方
        let sig = hmac_sha256_hex(
            "NhqPtmdSJYdKjVHjA7PZj4Mge3R5YNiP1e3UZjInClVN65XAbvqqM6A7H5fATj0j",
            "symbol=LTCBTC&side=BUY&type=LIMIT&timeInForce=GTC&quantity=1&price=0.1&recvWindow=5000&timestamp=1499827319559",
        );
        assert_eq!(
            sig,
            "c8db56825ae71d6d79447849e617115f4a920fa2acdcab2b053c4b2838bd6b71"
        );
    }

    #[test]
    fn sha256_b64_known_vector() {
        assert_eq!(
            hmac_sha256_b64(
                "okx-test-secret",
                "2024-01-01T00:00:00.000ZGET/users/self/verify"
            ),
            "Y3EnSWN5HK2xf//BKO+d0jjwaV9+I7+gaKi1pZDf764="
        );
    }

    #[test]
    fn sha512_hex_known_vector() {
        assert_eq!(
            hmac_sha512_hex("gate-test-secret", "GET\n/api/v4/spot/orders\n\n1700000000"),
            "4a36c950dea4200840f97d3aaac663364addac6f300ba168c21fb81c8add11346846e7a5a206dc9ad5a1fe9e758788d7cbcd21acc82c12ab8859df193d61059e"
        );
    }

    #[test]
    fn sha512_b64_known_vector() {
        assert_eq!(
            hmac_sha512_b64("kraken-test-secret", "method=GetWebSocketsToken"),
            "ZdDGcIAc3Y5QsWcxUuero7QdWbNC/ZIDJdMSlk+GPES5eqW1cn9tW+FWMsMV7ozcZh5qjQGS2mAmB0oRtDUK5w=="
        );
    }

    #[test]
    fn sha384_hex_known_vector() {
        assert_eq!(
            hmac_sha384_hex("gemini-test-secret", "eyJyZXF1ZXN0IjoiL2Nob2xkZXJzIn0="),
            "451b215e69cf5ca4a7ac06cea3762e6f63960269c9173390bde6aacf1f89e3fd8336119c9cc3254383fad3b094dbda47"
        );
    }

    #[test]
    fn sha512_b64_bytes_known_vector() {
        // 与 str 密钥变体(`hmac_sha512_b64`)输出一致:把 ASCII 密钥直接当字节密钥传入
        assert_eq!(
            hmac_sha512_b64_bytes(b"kraken-test-secret", b"method=GetWebSocketsToken"),
            "ZdDGcIAc3Y5QsWcxUuero7QdWbNC/ZIDJdMSlk+GPES5eqW1cn9tW+FWMsMV7ozcZh5qjQGS2mAmB0oRtDUK5w=="
        );
    }

    #[test]
    fn rsa_pss_signs_and_verifies() {
        // RSA-PSS 的 salt 是随机的,签名**非确定性**,故不断言固定值;
        // 改为验证:签名能被对应公钥正确验签(见下方)。
        const PEM: &str = r#"-----BEGIN PRIVATE KEY-----
MIIEvAIBADANBgkqhkiG9w0BAQEFAASCBKYwggSiAgEAAoIBAQClTYa88iRk6HrK
372sar0OFy2BsKznt/aOlCz/oy/FecrSSIm3ZUaWBqycsWohcc/zOiOPAhxGH8gh
rkKqySKuVLyu+8SQ5C2LQUN3Vwe+OZb3I97AL0G1S5TQBuDgpAvEzMKCYE0LpiuQ
WOHaDRXX/+r2CfrFwqwCmkC+ndWVlStt3XPgM2/7qUjbDaQftH9LouguOewTwmJ9
eTFQqflUwAksK0oUvXi+47m7bENToeBc7HhuOqrMMciAuADEAwkQiJCnt3XYLEF4
H2cNCQIhTa5juEa9IRuvUwaoQ46wE6P710kkn4TV6MHUlG+D18lDSjTkW5DScs59
ds6OriNxAgMBAAECggEAG1fCEFH7XCeqXBF7ZeA4CnMiB2BKkX+4DGiNMvHyznbx
aiwuf4QEk39iIP8lv4d8bs6woBCozZ6nM48IHwjrMCv7E/S4VIEx2WV/u+dM9BXB
l3q69045z7vDT0+79dycx75Y9EjqpaEkdpmFLlTYxqRh4LXQ4dJsMngCeqVP9ZB1
CFgRm7bNuvh3Rwjro0Xv8t9HQBcKtG1PS8SfuUauVUEl6BCYYkEtamby+1m45GkY
etm990OYnQut4hPBl+xLqFLEPgSwctcz6fH8xym92pkaG3K13kn6fqF0SZZNksrt
ky39h0H4/WM63NPExeM/fQKdbMyJhQG787A5MK6X+QKBgQDpW1bGogbPyT6QB+EL
IX/T03ob8CCaaKMXoPnc8Uqoen+1/qGA6Gqpveze06SRPxAyZN7nXDoAzKxk0IXA
49gB++fAPokT4bW3vLM+dlQBc7AEB5qKLj3itdQz5ZiFTCtk55DEHtQYwBmEmtau
6MvH1GKh2/8ozv/u/kQlnLbYHQKBgQC1V7RKXxKCPBIivSL4as9eDTZ62ygGOAKG
Hycsp3vB+HqmiMmALqjw2qzuPTb/+MY50bJei6qW+Qgi5+awVqvnyabhs8VwD263
I/sbiavH0RM3gaIUhkuYRVw6oXOGhlB/o/XVZBP8V3RWq2XXtZoEqZLwbr7Nl84N
VfAml4BgZQKBgGnc4PH9oT90WWh32pT1HotXPeccuX2zCIH4qkGcSTVDVVqX6GaV
iYX1vlacBuVJiayC7EhjI4EnWPHnUzZdZqoHwGwLMp6NX6W5+krU3WlHNUuus2IK
dlK6EEl22Alos6r0Dk2aere7thfdMpVo0MGXzSMGrauytJKUURALFzvVAoGALSTf
SiBu4CMKNMG3Afa9FuHdLSp/xUSOREtfGju7kGdGSU0GNLXo8sTNhiWRGpoY5u3w
JJ9KYXkvcFkg0cdR1ksaE1gIj8QXwNnhOlLEg+LlkMzhx5BDI24o7qOgud2YDp6H
pY7bdtdiq+zRVqjr5bs9TEOVlmLzNdiuBz1yBQ0CgYBSMg72CO1ZFNxl604d6WNv
8ivuv6GwmiBA9VEEOQ7T8BTW786hMgZcLRb1TfmvSzEA4Pol3+5Jj0QjQFQfGmKg
N3PS9MdnRfVAFFj2xZkGWqhlqP/ocmfaqMyUYURm5LlZ65Nz6Wo/LX6F7f7or5m9
ocxHieA7Y0yvMHC0w2peEQ==
-----END PRIVATE KEY-----
"#;
        let payload = "kalshi-test-payload";
        let sig = sign_rsa_pss(payload, PEM).unwrap();
        assert_eq!(sig.len(), 344, "256 字节 RSA 签名 base64 后应为 344");
        // 用对应公钥验签,确认签名有效(RSA-PSS SHA-256, saltlen = DIGEST_LENGTH)
        let rsa = openssl::rsa::Rsa::private_key_from_pem(PEM.as_bytes()).unwrap();
        let pubkey =
            openssl::pkey::PKey::public_key_from_pem(&rsa.public_key_to_pem().unwrap()).unwrap();
        let mut verifier =
            openssl::sign::Verifier::new(openssl::hash::MessageDigest::sha256(), &pubkey).unwrap();
        verifier
            .set_rsa_padding(openssl::rsa::Padding::PKCS1_PSS)
            .unwrap();
        verifier
            .set_rsa_pss_saltlen(openssl::sign::RsaPssSaltlen::DIGEST_LENGTH)
            .unwrap();
        verifier.update(payload.as_bytes()).unwrap();
        let raw = base64::engine::general_purpose::STANDARD
            .decode(&sig)
            .unwrap();
        assert!(verifier.verify(&raw).unwrap());
    }
}
