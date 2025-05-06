use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::time::Duration;
use serde::{Deserialize, Serialize};
use tracing::{debug, error};

// WHOIS服务器
const RIPE_WHOIS_SERVER: &str = "whois.ripe.net";
const WHOIS_PORT: u16 = 43;
const WHOIS_TIMEOUT: Duration = Duration::from_secs(10);

/// WHOIS查询结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WhoisInfo {
    /// 国家代码 (如 CN, US, JP)
    pub country: Option<String>,
    /// 网络名称
    pub netname: Option<String>,
    /// 描述
    pub descr: Option<String>,
    /// 组织
    pub org: Option<String>,
    /// 管理员联系邮箱
    pub admin_c: Option<String>,
    /// 技术联系邮箱
    pub tech_c: Option<String>,
    /// 维护者
    pub mnt_by: Option<String>,
    /// 最后更新时间
    pub last_modified: Option<String>,
    /// 原始WHOIS响应
    pub raw_response: String,
}

/// WHOIS客户端
#[allow(dead_code)]
pub struct WhoisClient;

impl WhoisClient {
    /// 查询IP的WHOIS信息
    pub fn lookup(ip: &str) -> Result<WhoisInfo, String> {
        // 建立TCP连接
        let mut stream = match TcpStream::connect((RIPE_WHOIS_SERVER, WHOIS_PORT)) {
            Ok(s) => s,
            Err(e) => return Err(format!("无法连接到WHOIS服务器: {}", e)),
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
            return Err(format!("无法发送WHOIS查询: {}", e));
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
                    error!("读取WHOIS响应时出错: {}", e);
                    break;
                }
            }
        }

        debug!("WHOIS响应: {}", response);

        // 解析响应
        let whois_info = Self::parse_response(&response);
        Ok(whois_info)
    }

    /// 解析WHOIS响应
    fn parse_response(response: &str) -> WhoisInfo {
        let mut country = None;
        let mut netname = None;
        let mut descr = None;
        let mut org = None;
        let mut admin_c = None;
        let mut tech_c = None;
        let mut mnt_by = None;
        let mut last_modified = None;

        for line in response.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('%') || line.starts_with('#') {
                continue;
            }

            let parts: Vec<&str> = line.splitn(2, ':').collect();
            if parts.len() < 2 {
                continue;
            }

            let key = parts[0].trim();
            let value = parts[1].trim();

            match key {
                "country" => country = Some(value.to_string()),
                "netname" => netname = Some(value.to_string()),
                "descr" => {
                    if descr.is_none() {
                        descr = Some(value.to_string());
                    }
                }
                "org" | "organisation" => org = Some(value.to_string()),
                "admin-c" => admin_c = Some(value.to_string()),
                "tech-c" => tech_c = Some(value.to_string()),
                "mnt-by" => mnt_by = Some(value.to_string()),
                "last-modified" => last_modified = Some(value.to_string()),
                _ => {}
            }
        }

        WhoisInfo {
            country,
            netname,
            descr,
            org,
            admin_c,
            tech_c,
            mnt_by,
            last_modified,
            raw_response: response.to_string(),
        }
    }
} 