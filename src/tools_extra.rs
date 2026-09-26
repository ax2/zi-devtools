use anyhow::{Result, anyhow, bail};
use serde_json::json;
use std::{collections::HashSet, net::Ipv4Addr};

pub const TEXT_LIMIT: usize = 1024 * 1024;

pub fn bounded(input: &str) -> Result<()> {
    if input.len() > TEXT_LIMIT {
        bail!("输入超过 1 MiB，请先缩小内容范围");
    }
    Ok(())
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

pub fn hex_encode(input: &str) -> Result<String> {
    bounded(input)?;
    Ok(input
        .as_bytes()
        .iter()
        .map(|v| format!("{v:02X}"))
        .collect::<Vec<_>>()
        .join(" "))
}

pub fn hex_decode(input: &str) -> Result<String> {
    bounded(input)?;
    let value: String = input.chars().filter(|c| !c.is_ascii_whitespace()).collect();
    if !value.is_ascii() || !value.bytes().all(|c| c.is_ascii_hexdigit()) || value.len() % 2 != 0 {
        bail!("请输入成对的十六进制字符，可用空格或换行分隔，例如 E4 BD A0");
    }
    let bytes = (0..value.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&value[i..i + 2], 16))
        .collect::<std::result::Result<Vec<_>, _>>()?;
    String::from_utf8(bytes)
        .map_err(|_| anyhow!("字节不是有效的 UTF-8 文本；不会替换或丢弃无效字节"))
}

pub fn process_lines(input: &str, action: &str) -> Result<String> {
    bounded(input)?;
    let mut lines = input.lines().map(str::to_owned).collect::<Vec<_>>();
    match action {
        "去重" => {
            let mut seen = HashSet::new();
            lines.retain(|line| seen.insert(line.clone()));
        }
        "升序" => lines.sort(),
        "降序" => lines.sort_by(|a, b| b.cmp(a)),
        "去空行" => lines.retain(|line| !line.trim().is_empty()),
        "去首尾空白" => lines
            .iter_mut()
            .for_each(|line| *line = line.trim().to_owned()),
        _ => bail!("未知行处理操作"),
    }
    Ok(lines.join("\n"))
}

pub fn inspect_url(input: &str) -> Result<String> {
    bounded(input)?;
    let url = reqwest::Url::parse(input.trim())
        .map_err(|e| anyhow!("URL 无效：{e}；请包含 https:// 等协议"))?;
    Ok(serde_json::to_string_pretty(&json!({
        "scheme":url.scheme(),"host":url.host_str(),"port":url.port_or_known_default(),
        "path":url.path(),"fragment":url.fragment(),
        "credentials_present": !url.username().is_empty() || url.password().is_some(),
        "query":url.query_pairs().map(|(k,v)| json!({"key":k,"value":v})).collect::<Vec<_>>()
    }))?)
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unicode_hex_roundtrip_and_invalid_bytes() {
        assert_eq!(
            hex_decode(&hex_encode("你好🙂\n").unwrap()).unwrap(),
            "你好🙂\n"
        );
        for s in ["A", "FF", "你", "GG"] {
            assert!(hex_decode(s).is_err());
        }
    }
    #[test]
    fn cidr_special_ranges_and_validation() {
        assert!(
            inspect_cidr("192.168.1.3/31")
                .unwrap()
                .contains("192.168.1.2 — 192.168.1.3")
        );
        assert!(inspect_cidr("10.0.0.1/32").unwrap().contains("地址总数  1"));
        assert!(inspect_cidr("0.0.0.0/0").unwrap().contains("4294967296"));
        for s in ["1.1.1.1/33", "256.1.1.1/8", "::1/64"] {
            assert!(inspect_cidr(s).is_err());
        }
    }
    #[test]
    fn yaml_and_query_preserve_structure() {
        let json = yaml_to_json("enabled: true\nlist: [a, b]\n").unwrap();
        assert!(json.contains("true"));
        assert!(json_to_yaml(&json).unwrap().contains("enabled: true"));
        let url = inspect_url("https://example.com/search?a=1&a=2&q=%E4%BD%A0%E5%A5%BD").unwrap();
        let value: serde_json::Value = serde_json::from_str(&url).unwrap();
        assert_eq!(value["query"].as_array().unwrap().len(), 3);
        assert_eq!(value["query"][2]["value"], "你好");
    }
    #[test]
    fn lines_preserve_order_and_case() {
        assert_eq!(
            process_lines("b\r\na\r\nb\r\nA", "去重").unwrap(),
            "b\na\nA"
        );
        assert_eq!(process_lines(" a \n \nb", "去空行").unwrap(), " a \nb");
        assert!(bounded(&"x".repeat(TEXT_LIMIT + 1)).is_err());
    }
}
