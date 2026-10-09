//! Shared complete standalone format/inspection algorithms; no host resources.
use anyhow::{Context, Result, anyhow, bail, ensure};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde_json::{Value, json};
use std::net::Ipv4Addr;
use unicode_normalization::UnicodeNormalization;

fn bounded(input: &str) -> Result<()> {
    ensure!(
        input.len() <= 1024 * 1024,
        "输入超过 1 MiB，请先缩小内容范围"
    );
    Ok(())
}
fn pretty(value: &Value) -> Result<String> {
    let text = serde_json::to_string_pretty(value)?;
    ensure!(
        text.len() <= 8 * 1024 * 1024,
        "结果超过 8 MiB，请缩小输入范围"
    );
    Ok(text)
}

pub fn yaml_to_json(input: &str) -> Result<String> {
    bounded(input)?;
    let value: serde_json::Value = serde_yaml_ng::from_str(input)?;
    Ok(serde_json::to_string_pretty(&value)?)
}

pub fn json_to_yaml(input: &str) -> Result<String> {
    bounded(input)?;
    let value: serde_json::Value = serde_json::from_str(input)?;
    Ok(serde_yaml_ng::to_string(&value)?)
}

pub fn inspect_cidr(input: &str) -> Result<String> {
    let (address, prefix) = input
        .trim()
        .split_once('/')
        .ok_or_else(|| anyhow!("请输入 IPv4/CIDR，例如 192.168.10.42/24"))?;
    let address: Ipv4Addr = address.parse().map_err(|_| anyhow!("IPv4 地址无效"))?;
    let prefix: u32 = prefix
        .parse()
        .map_err(|_| anyhow!("前缀须为 0–32 的整数"))?;
    if prefix > 32 {
        bail!("前缀须在 0–32 之间");
    }
    let mask = if prefix == 0 {
        0
    } else {
        u32::MAX << (32 - prefix)
    };
    let network = u32::from(address) & mask;
    let broadcast = network | !mask;
    let count = 1u64 << (32 - prefix);
    let (first, last, usable) = if prefix >= 31 {
        (network, broadcast, count)
    } else {
        (network + 1, broadcast - 1, count - 2)
    };
    Ok(format!(
        "网络地址  {}/{}\n子网掩码  {}\n地址上界  {}\n地址总数  {}\n可用地址  {}\n可用范围  {} — {}\n\n/31 按 RFC 3021 点对点链路计算；/32 表示单一主机。",
        Ipv4Addr::from(network),
        prefix,
        Ipv4Addr::from(mask),
        Ipv4Addr::from(broadcast),
        count,
        usable,
        Ipv4Addr::from(first),
        Ipv4Addr::from(last)
    ))
}

pub fn inspect_jwt(input: &str) -> Result<String> {
    let parts: Vec<&str> = input.trim().split('.').collect();
    if parts.len() != 3 || parts[0].is_empty() || parts[1].is_empty() {
        return Err(anyhow!("请输入由 header.payload.signature 组成的 JWT"));
    }
    let decode_json = |part: &str| -> Result<serde_json::Value> {
        if part.len() > 64 * 1024 {
            return Err(anyhow!("JWT 单段超过 64 KB 限制"));
        }
        let decoded = URL_SAFE_NO_PAD
            .decode(part)
            .context("JWT Base64URL 数据无效")?;
        let value: serde_json::Value =
            serde_json::from_slice(&decoded).context("JWT 中的 JSON 无效")?;
        if !value.is_object() {
            return Err(anyhow!("JWT header 和 payload 必须是 JSON 对象"));
        }
        Ok(value)
    };
    let header = decode_json(parts[0])?;
    let payload = decode_json(parts[1])?;
    Ok(format!(
        "仅解码内容；未验证签名、有效期或可信度。\n\nHeader:\n{}\n\nPayload:\n{}",
        serde_json::to_string_pretty(&header)?,
        serde_json::to_string_pretty(&payload)?
    ))
}

pub fn unicode(input: &str, action: usize) -> Result<String> {
    ensure!(
        input.chars().count() <= 10_000,
        "Unicode 工具最多处理 10000 个码点"
    );
    match action {
        1 => Ok(input.nfc().collect()),
        2 => Ok(input.nfkc().collect()),
        _ => {
            let points: Vec<_> = input.char_indices().map(|(offset,c)| json!({"byteOffset":offset,"character":c.to_string(),"codePoint":format!("U+{:04X}",c as u32),"utf8":c.to_string().as_bytes().iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(" "),"attention": c.is_control() || c.is_whitespace() || matches!(c,'\u{00ad}'|'\u{034f}'|'\u{061c}'|'\u{180e}'|'\u{200b}'..='\u{200f}'|'\u{202a}'..='\u{202e}'|'\u{2060}'..='\u{206f}'|'\u{feff}'|'\u{fe00}'..='\u{fe0f}') })).collect();
            pretty(
                &json!({"codePoints":points,"nfc":input.nfc().collect::<String>(),"nfkc":input.nfkc().collect::<String>(),"notes":"按 Unicode 码点而非可见字形列出；attention 标记常见控制、空白与格式字符，并非完整安全检测。NFKC 可能改变字符语义。"}),
            )
        }
    }
}
#[derive(Clone, Copy)]
pub struct Action {
    pub id: &'static str,
    pub source_tool_id: &'static str,
    pub title: &'static str,
    pub group: &'static str,
}
pub const ACTIONS: &[Action] = &[
    Action {
        id: "yaml.to_json",
        source_tool_id: "yaml",
        title: "YAML → JSON",
        group: "结构",
    },
    Action {
        id: "yaml.from_json",
        source_tool_id: "yaml",
        title: "JSON → YAML",
        group: "结构",
    },
    Action {
        id: "cidr.inspect",
        source_tool_id: "cidr",
        title: "IPv4 CIDR",
        group: "结构",
    },
    Action {
        id: "jwt.inspect",
        source_tool_id: "jwt",
        title: "JWT 内容",
        group: "结构",
    },
    Action {
        id: "unicode.inspect",
        source_tool_id: "unicode",
        title: "Unicode 码点",
        group: "字符",
    },
    Action {
        id: "unicode.nfc",
        source_tool_id: "unicode",
        title: "NFC 规范化",
        group: "字符",
    },
    Action {
        id: "unicode.nfkc",
        source_tool_id: "unicode",
        title: "NFKC 规范化",
        group: "字符",
    },
];
pub fn run(id: &str, input: &str) -> Result<String> {
    match id {
        "yaml.to_json" => yaml_to_json(input),
        "yaml.from_json" => json_to_yaml(input),
        "cidr.inspect" => inspect_cidr(input),
        "jwt.inspect" => inspect_jwt(input),
        "unicode.inspect" => unicode(input, 0),
        "unicode.nfc" => unicode(input, 1),
        "unicode.nfkc" => unicode(input, 2),
        _ => bail!("未注册该操作"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cidr_boundary_ranges_and_invalid_prefixes() {
        assert!(inspect_cidr("0.0.0.0/0").unwrap().contains("4294967296"));
        assert!(
            inspect_cidr("192.168.1.3/31")
                .unwrap()
                .contains("192.168.1.2 — 192.168.1.3")
        );
        assert!(
            inspect_cidr("127.0.0.1/32")
                .unwrap()
                .contains("可用地址  1")
        );
        for input in ["::1/64", "256.1.1.1/8", "1.1.1.1/33", "1.1.1.1/-1"] {
            assert!(inspect_cidr(input).is_err());
        }
    }

    #[test]
    fn jwt_preserves_unsigned_inspection_and_exact_wide_claims() {
        let header = URL_SAFE_NO_PAD.encode(r#"{"alg":"none"}"#);
        let payload = URL_SAFE_NO_PAD.encode(r#"{"sub":"zi","wide":9007199254740993}"#);
        let text = inspect_jwt(&format!("{header}.{payload}.")).unwrap();
        assert!(text.starts_with("仅解码内容；未验证签名、有效期或可信度。"));
        assert!(text.contains("9007199254740993"));
        let array = URL_SAFE_NO_PAD.encode("[]");
        assert!(inspect_jwt(&format!("{array}.{payload}.x")).is_err());
        assert!(inspect_jwt(&format!("{header}=.{payload}.x")).is_err());
    }

    #[test]
    fn yaml_conversion_retains_existing_number_and_duplicate_key_policy() {
        let text = yaml_to_json("wide: 9007199254740993\nitems: [true, null, 中]\n").unwrap();
        assert!(text.contains("9007199254740993"));
        // Preserve the standalone serde_json::Value map policy (last key wins).
        assert_eq!(yaml_to_json("x: 1\nx: 2\n").unwrap(), "{\n  \"x\": 2\n}");
        assert!(yaml_to_json("---\nx: 1\n---\nx: 2\n").is_err());
        assert!(
            json_to_yaml(&text)
                .unwrap()
                .contains("wide: 9007199254740993")
        );
    }

    #[test]
    fn unicode_normalization_is_explicit_and_report_keeps_byte_offsets() {
        assert_eq!(unicode("e\u{0301} Ａ①", 1).unwrap(), "é Ａ①");
        assert_eq!(unicode("e\u{0301} Ａ①", 2).unwrap(), "é A1");
        let report: Value = serde_json::from_str(&unicode("é\u{200b}", 0).unwrap()).unwrap();
        assert_eq!(report["codePoints"][1]["byteOffset"], 2);
        assert_eq!(report["codePoints"][1]["attention"], true);
        assert_eq!(unicode("", 1).unwrap(), "");
        assert!(unicode(&"a".repeat(10001), 0).is_err());
    }
}
