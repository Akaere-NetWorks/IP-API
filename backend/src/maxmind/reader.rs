use crate::config::MaxmindConfig;
use ipnet::IpNet;
use log::{error, info};
use maxminddb::{geoip2, Reader};
use std::net::IpAddr;
use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;
use serde::{Serialize, Deserialize};
use crate::utils::whois_client::WhoisInfo;
use crate::utils::bgptools_client::BgpToolsInfo;
use crate::utils::bgp_api_client::BgpApiResult;
use crate::utils::rpki_client::RpkiValidity;

pub struct MaxmindReader {
    config: Arc<MaxmindConfig>,
    asn_reader: Option<Reader<Vec<u8>>>,
    city_reader: Option<Reader<Vec<u8>>>,
    country_reader: Option<Reader<Vec<u8>>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IpInfo {
    pub ip: String,
    pub ip_range: Option<String>,
    pub country: Option<String>,
    pub city: Option<String>,
    pub asn: Option<u32>,
    pub organization: Option<String>,
    pub whois_info: Option<WhoisInfo>,
    pub bgp_info: Option<BgpToolsInfo>,
    pub bgp_api_info: Option<BgpApiResult>,
    pub rpki_info_list: Vec<RpkiValidity>,
}

fn is_reserved_ip(ip: &str) -> bool {
    use std::net::IpAddr;
    if let Ok(addr) = ip.parse::<IpAddr>() {
        match addr {
            IpAddr::V4(v4) => {
                v4.is_loopback()
                    || v4.is_private()
                    || v4.is_link_local()
                    || v4.is_broadcast()
                    || v4.is_documentation()
                    || v4.octets()[0] == 0 // 0.0.0.0/8
            }
            IpAddr::V6(v6) => {
                v6.is_loopback()
                    || v6.is_unspecified()
                    || v6.is_unique_local()
                    || v6.is_multicast()
                    || v6.is_unicast_link_local()
            }
        }
    } else {
        false
    }
}

impl MaxmindReader {
    pub fn new(config: Arc<MaxmindConfig>) -> Self {
        Self {
            config,
            asn_reader: None,
            city_reader: None,
            country_reader: None,
        }
    }

    pub fn load_databases(&mut self) -> Result<(), String> {
        info!("加载MaxMind数据库...");
        self.load_asn_database()?;
        self.load_city_database()?;
        self.load_country_database()?;
        info!("MaxMind数据库加载完成");
        Ok(())
    }

    pub fn lookup(&self, ip_str: &str) -> Result<IpInfo, String> {
        if is_reserved_ip(ip_str) {
            return Ok(IpInfo {
                ip: ip_str.to_string(),
                ip_range: None,
                country: Some("保留地址".to_string()),
                city: None,
                asn: None,
                organization: Some("保留地址".to_string()),
                whois_info: None,
                bgp_info: None,
                bgp_api_info: None,
                rpki_info_list: Vec::new(),
            });
        }
        let ip_info = if ip_str.contains('/') {
            self.lookup_cidr(ip_str)?
        } else {
            self.lookup_ip(ip_str)?
        };
        Ok(ip_info)
    }

    fn lookup_ip(&self, ip_str: &str) -> Result<IpInfo, String> {
        let ip = IpAddr::from_str(ip_str)
            .map_err(|e| format!("无效的IP地址: {}", e))?;
        let mut info = IpInfo {
            ip: ip_str.to_string(),
            ip_range: None,
            country: None,
            city: None,
            asn: None,
            organization: None,
            whois_info: None,
            bgp_info: None,
            bgp_api_info: None,
            rpki_info_list: Vec::new(),
        };
        if let Some(reader) = &self.asn_reader {
            match reader.lookup::<geoip2::Asn>(ip) {
                Ok(Some(asn)) => {
                    info.asn = asn.autonomous_system_number;
                    info.organization = asn.autonomous_system_organization.map(|s| s.to_string());
                },
                Ok(None) => {
                    info!("ASN数据库未找到该IP的ASN信息: {}", ip);
                },
                Err(e) => {
                    error!("ASN查询错误: {}", e);
                }
            }
        }
        if let Some(reader) = &self.city_reader {
            match reader.lookup::<geoip2::City>(ip) {
                Ok(Some(city_record)) => {
                    if let Some(city) = city_record.city {
                        if let Some(names) = city.names {
                            info.city = names.get("zh-CN")
                                .or_else(|| names.get("en"))
                                .map(|s| s.to_string());
                        }
                    }
                    if info.country.is_none() {
                        if let Some(country) = city_record.country {
                            if let Some(names) = country.names {
                                info.country = names.get("zh-CN")
                                    .or_else(|| names.get("en"))
                                    .map(|s| s.to_string());
                            }
                        }
                    }
                },
                Ok(None) => {},
                Err(e) => {
                    error!("城市查询错误: {}", e);
                }
            }
        }
        if info.country.is_none() {
            if let Some(reader) = &self.country_reader {
                match reader.lookup::<geoip2::Country>(ip) {
                    Ok(Some(country_record)) => {
                        if let Some(country) = country_record.country {
                            if let Some(names) = country.names {
                                info.country = names.get("zh-CN")
                                    .or_else(|| names.get("en"))
                                    .map(|s| s.to_string());
                            }
                        }
                    },
                    Ok(None) => {},
                    Err(e) => {
                        error!("国家查询错误: {}", e);
                    }
                }
            }
        }
        Ok(info)
    }
    
    fn lookup_cidr(&self, cidr_str: &str) -> Result<IpInfo, String> {
        let network = IpNet::from_str(cidr_str)
            .map_err(|e| format!("无效的CIDR: {}", e))?;
        let ip = network.addr();
        let ip_str = ip.to_string();
        let mut info = self.lookup_ip(&ip_str)?;
        info.ip = cidr_str.to_string();
        info.ip_range = Some(format!("{} - {}", network.network(), network.broadcast()));
        Ok(info)
    }

    fn load_asn_database(&mut self) -> Result<(), String> {
        let db_path = Path::new(&self.config.database_dir).join("GeoLite2-ASN.mmdb");
        if db_path.exists() {
            match Reader::open_readfile(&db_path) {
                Ok(reader) => {
                    self.asn_reader = Some(reader);
                    info!("ASN数据库加载成功");
                    Ok(())
                },
                Err(e) => Err(format!("加载ASN数据库失败: {}", e)),
            }
        } else {
            Err(format!("ASN数据库文件不存在: {}", db_path.display()))
        }
    }
    
    fn load_city_database(&mut self) -> Result<(), String> {
        let db_path = Path::new(&self.config.database_dir).join("GeoLite2-City.mmdb");
        if db_path.exists() {
            match Reader::open_readfile(&db_path) {
                Ok(reader) => {
                    self.city_reader = Some(reader);
                    info!("城市数据库加载成功");
                    Ok(())
                },
                Err(e) => Err(format!("加载城市数据库失败: {}", e)),
            }
        } else {
            Err(format!("城市数据库文件不存在: {}", db_path.display()))
        }
    }
    
    fn load_country_database(&mut self) -> Result<(), String> {
        let db_path = Path::new(&self.config.database_dir).join("GeoLite2-Country.mmdb");
        if db_path.exists() {
            match Reader::open_readfile(&db_path) {
                Ok(reader) => {
                    self.country_reader = Some(reader);
                    info!("国家数据库加载成功");
                    Ok(())
                },
                Err(e) => Err(format!("加载国家数据库失败: {}", e)),
            }
        } else {
            Err(format!("国家数据库文件不存在: {}", db_path.display()))
        }
    }
} 