use anyhow::{Result, bail};
use reqwest::{Client, Url};
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct ProbeResult {
    pub url: String,
    pub millis: u128,
    pub status: u16,
    pub detail: String,
}

pub fn validate_url(input: &str) -> Result<Url> {
    let url = Url::parse(input)?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        bail!("请输入 HTTP/HTTPS 地址");
    }
    if !url.username().is_empty() || url.password().is_some() {
        bail!("测试地址不支持嵌入用户名密码");
    }
    Ok(url)
}

pub async fn website(input: String, port: u16, exit_ip: bool) -> Result<ProbeResult> {
    let url = validate_url(&input)?;
    let client = Client::builder()
        .no_proxy()
        .proxy(reqwest::Proxy::all(format!("http://127.0.0.1:{port}"))?)
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()?;
    let start = Instant::now();
    let response = client.get(url).send().await?;
    let millis = start.elapsed().as_millis();
    let status = response.status().as_u16();
    let detail = if exit_ip {
        let mut response = response;
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            if bytes.len() + chunk.len() > 8192 {
                bail!("IP 查询响应过大");
            }
            bytes.extend_from_slice(&chunk);
        }
        let value: serde_json::Value = serde_json::from_slice(&bytes)?;
        value
            .get("ip")
            .and_then(|v| v.as_str())
            .unwrap_or("响应中没有 ip 字段")
            .to_string()
    } else {
        format!("HTTP {status} · 响应头耗时，不是 ICMP 延迟")
    };
    Ok(ProbeResult {
        url: input,
        millis,
        status,
        detail,
    })
}

pub async fn dns(host: String) -> Result<Vec<String>> {
    if host.is_empty() || host.contains(['/', ':', ' ']) {
        bail!("DNS 查询请输入域名，例如 github.com");
    }
    let addresses = tokio::time::timeout(
        Duration::from_secs(5),
        tokio::net::lookup_host((host.as_str(), 443)),
    )
    .await??;
    let mut result: Vec<_> = addresses.map(|a| a.ip().to_string()).collect();
    result.sort();
    result.dedup();
    Ok(result)
}

pub(crate) fn trace_country(body: &str, host: &str) -> Option<String> {
    let fields: std::collections::BTreeMap<_, _> = body
        .lines()
        .filter_map(|line| line.trim().split_once('='))
        .collect();
    if fields.get("h").copied()? != host {
        return None;
    }
    country_name(fields.get("loc").copied()?)
}

pub(crate) const ISO_CODES: &str = "AD AE AF AG AI AL AM AO AQ AR AS AT AU AW AX AZ BA BB BD BE BF BG BH BI BJ BL BM BN BO BQ BR BS BT BV BW BY BZ CA CC CD CF CG CH CI CK CL CM CN CO CR CU CV CW CX CY CZ DE DJ DK DM DO DZ EC EE EG EH ER ES ET FI FJ FK FM FO FR GA GB GD GE GF GG GH GI GL GM GN GP GQ GR GS GT GU GW GY HK HM HN HR HT HU ID IE IL IM IN IO IQ IR IS IT JE JM JO JP KE KG KH KI KM KN KP KR KW KY KZ LA LB LC LI LK LR LS LT LU LV LY MA MC MD ME MF MG MH MK ML MM MN MO MP MQ MR MS MT MU MV MW MX MY MZ NA NC NE NF NG NI NL NO NP NR NU NZ OM PA PE PF PG PH PK PL PM PN PR PS PT PW PY QA RE RO RS RU RW SA SB SC SD SE SG SH SI SJ SK SL SM SN SO SR SS ST SV SX SY SZ TC TD TF TG TH TJ TK TL TM TN TO TR TT TV TW TZ UA UG UM US UY UZ VA VC VE VG VI VN VU WF WS YE YT ZA ZM ZW";

pub(crate) fn country_name(code: &str) -> Option<String> {
    let code = code.to_ascii_uppercase();
    if !ISO_CODES.split_whitespace().any(|valid| valid == code) {
        return None;
    }
    let name = match code.as_str() {
        "US" => "美国",
        "CN" => "中国",
        "HK" => "香港",
        "MO" => "澳门",
        "TW" => "台湾",
        "JP" => "日本",
        "SG" => "新加坡",
        "KR" => "韩国",
        "GB" => "英国",
        "CA" => "加拿大",
        "AU" => "澳大利亚",
        "DE" => "德国",
        "FR" => "法国",
        "NL" => "荷兰",
        "IN" => "印度",
        "MY" => "马来西亚",
        "ID" => "印度尼西亚",
        "TH" => "泰国",
        "VN" => "越南",
        "PH" => "菲律宾",
        "NZ" => "新西兰",
        "IT" => "意大利",
        "ES" => "西班牙",
        "CH" => "瑞士",
        "SE" => "瑞典",
        "FI" => "芬兰",
        "NO" => "挪威",
        "RU" => "俄罗斯",
        "BR" => "巴西",
        "MX" => "墨西哥",
        "AR" => "阿根廷",
        "ZA" => "南非",
        "AE" => "阿联酋",
        "TR" => "土耳其",
        "IE" => "爱尔兰",
        "IS" => "冰岛",
        "PL" => "波兰",
        "PT" => "葡萄牙",
        "BE" => "比利时",
        "AT" => "奥地利",
        "IL" => "以色列",
        "SA" => "沙特阿拉伯",
        "CZ" => "捷克",
        _ => return Some(code),
    };
    Some(name.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn restricts_probe_protocols() {
        assert!(validate_url("file:///etc/passwd").is_err());
        assert!(validate_url("https://user:secret@example.com").is_err());
        assert!(validate_url("https://example.com").is_ok());
    }

    #[test]
    fn country_requires_explicit_same_site_evidence() {
        assert_eq!(
            trace_country("h=chatgpt.com\nloc=JP\ncolo=LAX\nip=1.2.3.4", "chatgpt.com"),
            Some("日本".into())
        );
        assert!(trace_country("h=other.example\nloc=US", "chatgpt.com").is_none());
        assert!(trace_country("h=chatgpt.com\nloc=XX", "chatgpt.com").is_none());
        assert!(country_name("EN").is_none());
    }
}
