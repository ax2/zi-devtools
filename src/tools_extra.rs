use anyhow::{Result, bail};

pub const TEXT_LIMIT: usize = 1024 * 1024;

pub fn bounded(input: &str) -> Result<()> {
    if input.len() > TEXT_LIMIT {
        bail!("输入超过 1 MiB，请先缩小内容范围");
    }
    Ok(())
}

pub fn yaml_to_json(input: &str) -> Result<String> {
    zi_inspect_core::yaml_to_json(input)
}

pub fn json_to_yaml(input: &str) -> Result<String> {
    zi_inspect_core::json_to_yaml(input)
}

pub fn hex_encode(input: &str) -> Result<String> {
    zi_text_core::transforms::hex_encode(input)
}

pub fn hex_decode(input: &str) -> Result<String> {
    zi_text_core::transforms::hex_decode(input)
}

pub fn process_lines(input: &str, action: &str) -> Result<String> {
    zi_text_core::transforms::process_lines(input, action)
}

pub fn inspect_url(input: &str) -> Result<String> {
    zi_text_core::transforms::inspect_url(input)
}

pub fn inspect_cidr(input: &str) -> Result<String> {
    zi_inspect_core::inspect_cidr(input)
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
