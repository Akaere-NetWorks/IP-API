use std::io::{BufRead, BufReader, Write};
use std::net::{IpAddr, TcpStream};
use std::time::Duration;
use std::str::FromStr;
use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};
use tracing::{debug, error, info};

const BGPTOOLS_WHOIS_SERVER: &str = "bgp.tools";
const BGPTOOLS_WHOIS_PORT: u16 = 43;
const WHOIS_TIMEOUT: Duration = Duration::from_secs(15);
const BGPTOOLS_WEBSITE: &str = "https://bgp.tools";
const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/136.0.0.0 Safari/537.36";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BgpToolsUpstream {
    pub asn: String,
    pub name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BgpToolsInfo {
    pub asn: Option<String>,
    pub ip: String,
    pub prefix: Option<String>,
    pub country: Option<String>,
    pub registry: Option<String>,
    pub allocated: Option<String>,
    pub as_name: Option<String>,
    pub upstreams: Vec<BgpToolsUpstream>,
    pub raw_response: Option<String>,
}

#[allow(dead_code)]
pub struct BgpToolsClient;

impl BgpToolsClient {
    /// 查询IP的BGP Tools信息
    pub async fn lookup(ip: &str) -> Result<BgpToolsInfo, String> {
        debug!("BGP Tools lookup: 查询IP {}", ip);
        // 先获取基本信息
        let whois_info = Self::query_whois(ip)?;
        debug!("BGP Tools whois_info: {:?}", whois_info);
        
        // 如果有前缀信息，查询上游信息
        let mut info = BgpToolsInfo {
            asn: whois_info.asn.clone(),
            ip: whois_info.ip.clone(),
            prefix: whois_info.prefix.clone(),
            country: whois_info.country.clone(),
            registry: whois_info.registry.clone(),
            allocated: whois_info.allocated.clone(),
            as_name: whois_info.as_name.clone(),
            upstreams: Vec::new(),
            raw_response: whois_info.raw_response.clone(),
        };
        
        // 如果有前缀，获取上游信息
        if let Some(prefix) = &info.prefix {
            debug!("BGP Tools fetch_upstreams: prefix={}", prefix);
            match Self::fetch_upstreams(prefix).await {
                Ok(upstreams) => {
                    info!("BGP Tools 上游数量: {}", upstreams.len());
                    info.upstreams = upstreams;
                }
                Err(e) => {
                    error!("获取BGP Tools上游信息失败: {}", e);
                }
            }
        } else {
            debug!("BGP Tools whois未获取到前缀，跳过上游爬取");
        }
        debug!("BGP Tools 最终info: {:?}", info);
        Ok(info)
    }
    
    /// 从BGP Tools Whois服务查询信息
    fn query_whois(ip: &str) -> Result<BgpToolsInfo, String> {
        // 验证IP格式
        let _ip_parsed = match IpAddr::from_str(ip) {
            Ok(addr) => addr,
            Err(e) => return Err(format!("无效的IP地址: {}", e)),
        };
        
        // 建立TCP连接
        let mut stream = match TcpStream::connect((BGPTOOLS_WHOIS_SERVER, BGPTOOLS_WHOIS_PORT)) {
            Ok(s) => s,
            Err(e) => return Err(format!("无法连接到BGP Tools Whois服务器: {}", e)),
        };
        
        // 设置超时
        if let Err(e) = stream.set_read_timeout(Some(WHOIS_TIMEOUT)) {
            return Err(format!("设置读取超时失败: {}", e));
        }
        if let Err(e) = stream.set_write_timeout(Some(WHOIS_TIMEOUT)) {
            return Err(format!("设置写入超时失败: {}", e));
        }
        
        // 发送查询请求
        let query = format!("{}\r\n", ip);
        if let Err(e) = stream.write_all(query.as_bytes()) {
            return Err(format!("无法发送BGP Tools Whois查询: {}", e));
        }
        
        // 读取响应
        let reader = BufReader::new(stream);
        let mut response = String::new();
        for line in reader.lines() {
            match line {
                Ok(line) => {
                    response.push_str(&line);
                    response.push('\n');
                }
                Err(e) => {
                    error!("读取BGP Tools Whois响应时出错: {}", e);
                    break;
                }
            }
        }
        
        debug!("BGP Tools Whois响应: {}", response);
        
        // 解析响应
        let info = Self::parse_whois_response(&response, ip);
        Ok(info)
    }
    
    /// 解析Whois响应
    fn parse_whois_response(response: &str, ip: &str) -> BgpToolsInfo {
        let mut asn = None;
        let mut prefix = None;
        let mut country = None;
        let mut registry = None;
        let mut allocated = None;
        let mut as_name = None;

        for line in response.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') || line.starts_with("AS ") {
                continue; // 跳过表头和注释
            }

            // 以 | 分割
            let parts: Vec<&str> = line.split('|').map(|s| s.trim()).collect();
            if parts.len() >= 7 {
                asn = Some(parts[0].to_string());
                // parts[1] 是IP
                prefix = Some(parts[2].to_string());
                country = Some(parts[3].to_string());
                registry = Some(parts[4].to_string());
                allocated = Some(parts[5].to_string());
                as_name = Some(parts[6].to_string());
                break; // 只取第一条
            }
        }

        BgpToolsInfo {
            asn,
            ip: ip.to_string(),
            prefix,
            country,
            registry,
            allocated,
            as_name,
            upstreams: Vec::new(),
            raw_response: Some(response.to_string()),
        }
    }
    
    /// 从BGP Tools网站获取上游信息
    async fn fetch_upstreams(prefix: &str) -> Result<Vec<BgpToolsUpstream>, String> {
        let url = format!("{}/prefix/{}", BGPTOOLS_WEBSITE, prefix);
        info!("BGP Tools fetch_upstreams 请求URL: {}", url);

        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .user_agent(USER_AGENT)
            .build()
            .map_err(|e| format!("创建HTTP客户端失败: {}", e))?;

        let response = client.get(&url).send().await
            .map_err(|e| format!("HTTP请求失败: {}", e))?;
        if !response.status().is_success() {
            return Err(format!("HTTP请求失败: 状态码 {}", response.status()));
        }
        let html = response.text().await
            .map_err(|e| format!("读取HTTP响应失败: {}", e))?;
        debug!("BGP Tools fetch_upstreams HTML长度: {}", html.len());

        let document = Html::parse_document(&html);

        // 选择Upstreams所在的上游区域 div
        let div_selector = Selector::parse("div.grid-row > div.column-half").unwrap();
        let h2_selector = Selector::parse("h2.heading-medium").unwrap();
        let ul_selector = Selector::parse("ul").unwrap();
        let li_selector = Selector::parse("li").unwrap();
        let a_selector = Selector::parse("a").unwrap();

        let mut upstreams = Vec::new();

        for div in document.select(&div_selector) {
            // 找到Upstreams标题
            if let Some(h2) = div.select(&h2_selector).next() {
                let h2_text = h2.text().collect::<Vec<_>>().join("").trim().to_string();
                if h2_text.contains("Upstreams") {
                    // 找ul > li
                    if let Some(ul) = div.select(&ul_selector).next() {
                        for li in ul.select(&li_selector) {
                            let asn = li.select(&a_selector)
                                .next()
                                .map(|a| a.text().collect::<Vec<_>>().join("").trim().to_string())
                                .unwrap_or_default();
                            // a标签后面的文本节点
                            let name = li.text().collect::<Vec<_>>().join("").replace(&asn, "").replace("-", "").trim().to_string();
                            let name = if !name.is_empty() { Some(name) } else { None };
                            upstreams.push(BgpToolsUpstream { asn, name });
                        }
                    }
                }
            }
        }

        info!("获取到 {} 条上游信息", upstreams.len());
        for u in &upstreams {
            debug!("BGP Tools 上游: asn={}, name={:?}", u.asn, u.name);
        }
        Ok(upstreams)
    }
} 