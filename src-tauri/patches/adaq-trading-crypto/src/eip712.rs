//! EIP-712 结构化数据哈希与 ECDSA 签名(polymarket CLOB 下单/认证用)。
//!
//! 对齐 ccxt 的 `eth_encode_structured_data` + `sign_message`:
//! - `digest(domain, types, message)` = `keccak256(0x19 0x01 || domain_separator || struct_hash)`;
//! - domain_separator 仅支持 `EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)`;
//! - struct 编码支持 `uint256/uint8/address/bytes32/string/bytes32[]` 等常见类型
//!   (polymarket 订单只用到 uint256/address/uint8/bytes32)。

use libsecp256k1::{PublicKey, RecoveryId, Signature};
use sha3::{Digest, Keccak256};

use crate::error::{Error, ErrorKind, Result};

/// 把整数(十进制,或 `0x` 前缀十六进制)转 32 字节大端。
pub fn uint256_be(v: &str) -> Result<[u8; 32]> {
    let mut out = [0u8; 32];
    let big = if let Some(hexv) = v.strip_prefix("0x") {
        num_bigint::BigUint::parse_bytes(hexv.as_bytes(), 16)
            .ok_or_else(|| Error::new(ErrorKind::BadRequest, format!("invalid uint256 {v}")))?
    } else {
        num_bigint::BigUint::parse_bytes(v.as_bytes(), 10)
            .ok_or_else(|| Error::new(ErrorKind::BadRequest, format!("invalid uint256 {v}")))?
    };
    let bytes = big.to_bytes_be();
    if bytes.len() > 32 {
        return Err(Error::new(
            ErrorKind::BadRequest,
            format!("uint256 overflow: {v}"),
        ));
    }
    out[32 - bytes.len()..].copy_from_slice(&bytes);
    Ok(out)
}

/// 把地址(0x + 40 hex)转 32 字节(左补零)。
pub fn address_bytes(addr: &str) -> Result<[u8; 32]> {
    let clean = addr.trim_start_matches("0x");
    if clean.len() != 40 {
        return Err(Error::new(
            ErrorKind::BadRequest,
            format!("invalid address: {addr}"),
        ));
    }
    let bytes = hex::decode(clean).map_err(|e| {
        Error::new(
            ErrorKind::BadRequest,
            format!("invalid address {addr}: {e}"),
        )
    })?;
    let mut out = [0u8; 32];
    out[12..].copy_from_slice(&bytes);
    Ok(out)
}

/// keccak256。
pub fn keccak256(data: &[u8]) -> [u8; 32] {
    let mut hasher = Keccak256::new();
    hasher.update(data);
    hasher.finalize().into()
}

/// EIP-712 字段。
#[derive(Debug, Clone)]
pub enum Field {
    Uint256(String),
    Uint8(u8),
    Address(String),
    Bytes32(String),
    Str(String),
}

/// 编码单个字段为 32 字节(abi.encode 的定长部分)。
pub fn encode_field(field: &Field) -> Result<[u8; 32]> {
    match field {
        Field::Uint256(v) => uint256_be(v),
        Field::Uint8(v) => {
            let mut out = [0u8; 32];
            out[31] = *v;
            Ok(out)
        }
        Field::Address(a) => address_bytes(a),
        Field::Bytes32(b) => {
            let clean = b.trim_start_matches("0x");
            let bytes = hex::decode(clean).map_err(|e| {
                Error::new(ErrorKind::BadRequest, format!("invalid bytes32 {b}: {e}"))
            })?;
            if bytes.len() != 32 {
                return Err(Error::new(
                    ErrorKind::BadRequest,
                    format!("bytes32 需 32 字节: {b}"),
                ));
            }
            let mut out = [0u8; 32];
            out.copy_from_slice(&bytes);
            Ok(out)
        }
        Field::Str(s) => {
            // 字符串:先 hash(string)
            Ok(keccak256(s.as_bytes()))
        }
    }
}

/// 计算 struct hash:`keccak256(abi.encode(type_hash, encodeData(...)))`。
pub fn struct_hash(type_string: &str, fields: &[Field]) -> Result<[u8; 32]> {
    let type_hash = keccak256(type_string.as_bytes());
    let mut data = Vec::with_capacity(32 * (1 + fields.len()));
    data.extend_from_slice(&type_hash);
    for f in fields {
        data.extend_from_slice(&encode_field(f)?);
    }
    Ok(keccak256(&data))
}

/// 仅计算 domain_separator(调试用)。
#[allow(dead_code)]
pub fn domain_separator_only(name: &str, version: &str, chain_id: u64, contract: &str) -> [u8; 32] {
    let domain_type_hash = keccak256(
        b"EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)",
    );
    let mut data = Vec::with_capacity(32 * 5);
    data.extend_from_slice(&domain_type_hash);
    data.extend_from_slice(&keccak256(name.as_bytes()));
    data.extend_from_slice(&keccak256(version.as_bytes()));
    data.extend_from_slice(&uint256_be(&chain_id.to_string()).unwrap());
    data.extend_from_slice(&address_bytes(contract).unwrap());
    keccak256(&data)
}

/// 计算 EIP-712 digest:`keccak256(0x19 0x01 || domain_separator || struct_hash)`。
pub fn digest(
    domain_name: &str,
    domain_version: &str,
    chain_id: u64,
    verifying_contract: &str,
    type_string: &str,
    fields: &[Field],
) -> Result<[u8; 32]> {
    let domain_type_hash = keccak256(
        b"EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)",
    );
    let name_hash = keccak256(domain_name.as_bytes());
    let version_hash = keccak256(domain_version.as_bytes());
    let mut domain_data = Vec::with_capacity(32 * 5);
    domain_data.extend_from_slice(&domain_type_hash);
    domain_data.extend_from_slice(&name_hash);
    domain_data.extend_from_slice(&version_hash);
    domain_data.extend_from_slice(&uint256_be(&chain_id.to_string())?);
    domain_data.extend_from_slice(&address_bytes(verifying_contract)?);
    let domain_separator = keccak256(&domain_data);
    let struct_hash = struct_hash(type_string, fields)?;
    let mut payload = Vec::with_capacity(2 + 32 + 32);
    payload.extend_from_slice(&[0x19, 0x01]);
    payload.extend_from_slice(&domain_separator);
    payload.extend_from_slice(&struct_hash);
    Ok(keccak256(&payload))
}

/// 从 secp256k1 私钥 hex(64 hex 或 0x 前缀)推导 EIP-55 校验和地址。
pub fn address_from_private_key(private_key: &str) -> Result<String> {
    let key = crate::signing::secp256k1_secret_from_hex(private_key)?;
    let public = PublicKey::from_secret_key(&key);
    // 去掉 0x04 前缀(共 65 字节),取 keccak 后 20 字节
    let pub_bytes = public.serialize();
    let hash = keccak256(&pub_bytes[1..]);
    let addr_lower = hex::encode(&hash[12..]);
    Ok(checksum(&addr_lower))
}

/// EIP-55 校验和。
pub fn checksum(addr_lower_hex: &str) -> String {
    let hash = keccak256(addr_lower_hex.as_bytes());
    let hash_hex = hex::encode(hash);
    let mut out = String::with_capacity(42);
    out.push_str("0x");
    for (i, ch) in addr_lower_hex.chars().enumerate() {
        if ch.is_ascii_digit() {
            out.push(ch);
        } else {
            let nibble = hash_hex.as_bytes()[i];
            let v = hex_digit_value(nibble);
            if v >= 8 {
                out.push(ch.to_ascii_uppercase());
            } else {
                out.push(ch);
            }
        }
    }
    out
}

fn hex_digit_value(b: u8) -> u8 {
    match b {
        b'0'..=b'9' => b - b'0',
        b'a'..=b'f' => b - b'a' + 10,
        b'A'..=b'F' => b - b'A' + 10,
        _ => 0,
    }
}

/// 测试辅助:校验签名恢复的地址。
#[allow(dead_code)]
pub fn verify_recover(digest: &[u8; 32], r: &str, s: &str, v: u8) -> Result<String> {
    let mut r_bytes = [0u8; 32];
    r_bytes.copy_from_slice(&hex::decode(r).unwrap());
    let mut s_bytes = [0u8; 32];
    s_bytes.copy_from_slice(&hex::decode(s).unwrap());
    let sig = Signature::parse_standard_slice(&[&r_bytes[..], &s_bytes[..]].concat())
        .map_err(|e| Error::new(ErrorKind::BadRequest, format!("bad sig: {e}")))?;
    let recovery_id =
        RecoveryId::parse(v - 27).map_err(|_| Error::new(ErrorKind::BadRequest, "bad v"))?;
    let message = libsecp256k1::Message::parse_slice(digest)
        .map_err(|e| Error::new(ErrorKind::BadRequest, format!("bad msg: {e}")))?;
    let pk = libsecp256k1::recover(&message, &sig, &recovery_id)
        .map_err(|e| Error::new(ErrorKind::BadRequest, format!("recover: {e}")))?;
    let hash = keccak256(&pk.serialize()[1..]);
    Ok(checksum(&hex::encode(&hash[12..])))
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &str = "fad9c8855b740a0b7ed4c221dbad0f33a83a49cad6b3fe8d5817ac83d38b6a19";

    #[test]
    fn address_from_known_key() {
        // 该私钥的已知地址
        let addr = address_from_private_key(KEY).unwrap();
        assert_eq!(addr.len(), 42);
        assert!(addr.starts_with("0x"));
        // 校验和地址重新解析应一致(大小写)
        let lower = addr[2..].to_lowercase();
        assert_eq!(checksum(&lower), addr);
    }

    #[test]
    fn struct_hash_is_deterministic() {
        let h1 = struct_hash("Order(uint256 salt)", &[Field::Uint256("1".into())]).unwrap();
        let h2 = struct_hash("Order(uint256 salt)", &[Field::Uint256("1".into())]).unwrap();
        assert_eq!(h1, h2);
        let h3 = struct_hash("Order(uint256 salt)", &[Field::Uint256("2".into())]).unwrap();
        assert_ne!(h1, h3);
    }

    #[test]
    fn digest_stable() {
        let d1 = digest(
            "Polymarket CTF Exchange",
            "2",
            137,
            "0xE111180000d2663C0091e4f400237545B87B996B",
            "Order(uint256 salt)",
            &[Field::Uint256("0".into())],
        )
        .unwrap();
        let d2 = digest(
            "Polymarket CTF Exchange",
            "2",
            137,
            "0xE111180000d2663C0091e4f400237545B87B996B",
            "Order(uint256 salt)",
            &[Field::Uint256("0".into())],
        )
        .unwrap();
        assert_eq!(d1, d2);
    }

    #[test]
    fn sign_and_recover_matches_key() {
        let digest = [7u8; 32];
        let sig = crate::signing::sign_ecdsa_recoverable(KEY, &digest).unwrap();
        assert_eq!(sig.len(), 130);
        let r = &sig[..64];
        let s = &sig[64..128];
        let v = u8::from_str_radix(&sig[128..130], 16).unwrap();
        let recovered = verify_recover(&digest, r, s, v).unwrap();
        assert_eq!(recovered, address_from_private_key(KEY).unwrap());
    }

    /// 与 eth-account(以太坊标准)交叉验证:EIP-712 Order 签名完全一致。
    /// 参考值由 `eth_account` 对同一 domain/types/message 计算(确定性 RFC6979)。
    #[test]
    fn polymarket_order_signature_matches_eth_account() {
        const ORDER_TYPE: &str = "Order(uint256 salt,address maker,address signer,uint256 tokenId,uint256 makerAmount,uint256 takerAmount,uint8 side,uint8 signatureType,uint256 timestamp,bytes32 metadata,bytes32 builder)";
        let addr = address_from_private_key(KEY).unwrap();
        assert_eq!(addr, "0x96216849c49358B10257cb55b28eA603c874b05E");
        let fields = vec![
            Field::Uint256("1".into()),
            Field::Address(addr.clone()),
            Field::Address(addr),
            Field::Uint256("123".into()),
            Field::Uint256("1000000".into()),
            Field::Uint256("500000".into()),
            Field::Uint8(0),
            Field::Uint8(0),
            Field::Uint256("1700000000".into()),
            Field::Bytes32(format!("0x{}", "00".repeat(32))),
            Field::Bytes32(format!("0x{}", "00".repeat(32))),
        ];
        let d = digest(
            "Polymarket CTF Exchange",
            "2",
            137,
            "0xE111180000d2663C0091e4f400237545B87B996B",
            ORDER_TYPE,
            &fields,
        )
        .unwrap();
        eprintln!("rust digest: {}", hex::encode(d));
        eprintln!(
            "rust domain_sep: {}",
            hex::encode(domain_separator_only(
                "Polymarket CTF Exchange",
                "2",
                137,
                "0xE111180000d2663C0091e4f400237545B87B996B"
            ))
        );
        eprintln!(
            "rust struct_hash: {}",
            hex::encode(struct_hash(ORDER_TYPE, &fields).unwrap())
        );
        let sig = crate::signing::sign_ecdsa_recoverable(KEY, &d).unwrap();
        // eth-account 参考签名(r + s + v,小写 hex)
        assert_eq!(
            sig,
            "5bb886bc59978a964ced81249702a43ccee642850c3c5edbba584e02c83f103b64276094d1bd5ebecdd8b9f07599e2aa55c41b9df6ee4b7d9a12ed7892035b731c"
        );
    }
}
