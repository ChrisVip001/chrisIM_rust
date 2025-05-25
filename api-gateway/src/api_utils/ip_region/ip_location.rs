use std::path::Path;
use std::sync::Arc;
use once_cell::sync::OnceCell;
use tracing::{error, info};

// ip2region相关导入
use ip2region::Searcher;

// 全局IP地理位置查询实例
static IP_SEARCHER: OnceCell<Arc<Searcher>> = OnceCell::new();

/// IP位置信息
#[derive(Debug, Clone)]
pub struct IpLocationInfo {
    /// 是否是内网IP
    pub is_internal: bool,
    /// IP地址类型
    pub ip_type: IpType,
    /// IP地址
    pub ip: String,
    /// 国家
    pub country: String,
    /// 区域
    pub region: String,
    /// 省份
    pub province: String,
    /// 城市
    pub city: String,
    /// 运营商
    pub isp: String,
    /// 是否使用了地理位置数据库
    pub used_geo_db: bool,
}

/// IP地址类型
#[derive(Debug, Clone, PartialEq)]
pub enum IpType {
    /// 内网IP
    Internal,
    /// IPv4地址
    IPv4,
    /// IPv6地址
    IPv6,
    /// 未知类型
    Unknown,
}

impl Default for IpLocationInfo {
    fn default() -> Self {
        Self {
            is_internal: false,
            ip_type: IpType::Unknown,
            ip: "未知".to_string(),
            country: "未知".to_string(),
            region: "未知".to_string(),
            province: "未知".to_string(),
            city: "未知".to_string(),
            isp: "未知".to_string(),
            used_geo_db: false,
        }
    }
}

/// 初始化IP地理位置服务
pub fn init_ip_location(xdb_path: &Path) -> anyhow::Result<()> {
    if IP_SEARCHER.get().is_some() {
        info!("IP地理位置服务已经初始化");
        return Ok(());
    }

    info!("正在初始化IP地理位置服务，数据库路径: {:?}", xdb_path);
    
    // ip2region 0.1.0中，使用Searcher::new
    match Searcher::new(xdb_path.to_str().unwrap()) {
        Ok(searcher) => {
            match IP_SEARCHER.set(Arc::new(searcher)) {
                Ok(_) => {
                    info!("IP地理位置服务初始化成功");
                    Ok(())
                }
                Err(_) => {
                    error!("设置IP查询实例失败");
                    Err(anyhow::anyhow!("设置IP查询实例失败"))
                }
            }
        }
        Err(e) => {
            error!("加载IP地理位置数据库失败: {}", e);
            Err(anyhow::anyhow!("加载IP地理位置数据库失败: {}", e))
        }
    }
}

/// 解析地理位置信息
fn parse_region(region: &str) -> (String, String, String, String, String) {
    let parts: Vec<&str> = region.split('|').collect();
    
    if parts.len() >= 5 {
        (
            parts[0].to_string(), // 国家
            parts[1].to_string(), // 区域
            parts[2].to_string(), // 省份
            parts[3].to_string(), // 城市
            parts[4].to_string(), // 运营商
        )
    } else {
        (
            "未知".to_string(),
            "未知".to_string(),
            "未知".to_string(),
            "未知".to_string(),
            "未知".to_string(),
        )
    }
}

/// 获取IP地址信息
pub fn get_ip_info(ip: &str) -> IpLocationInfo {
    // 如果IP是空的或者是"未知客户端IP"
    if ip.is_empty() || ip == "未知客户端IP" {
        return IpLocationInfo {
            is_internal: false,
            ip_type: IpType::Unknown,
            ip: ip.to_string(),
            ..IpLocationInfo::default()
        };
    }

    // 判断IP类型
    let ip_type = if ip.contains(':') {
        IpType::IPv6
    } else if ip.contains('.') {
        IpType::IPv4
    } else {
        IpType::Unknown
    };

    // 判断是否是内网IP
    let is_internal = is_internal_ip(ip);
    
    // 如果是内网IP，不需要查询地理位置
    if is_internal {
        return IpLocationInfo {
            is_internal: true,
            ip_type: IpType::Internal,
            ip: ip.to_string(),
            country: "内网".to_string(),
            region: "内网".to_string(),
            province: "内网".to_string(),
            city: "内网".to_string(),
            isp: "内网".to_string(),
            used_geo_db: false,
        };
    }

    // 尝试使用IP地理位置数据库
    if let Some(searcher) = IP_SEARCHER.get() {
        // ip2region 0.1.0中，使用search方法
        match searcher.search(ip) {
            Ok(region) => {
                let (country, region, province, city, isp) = parse_region(&region);
                
                return IpLocationInfo {
                    is_internal,
                    ip_type,
                    ip: ip.to_string(),
                    country,
                    region,
                    province,
                    city,
                    isp,
                    used_geo_db: true,
                };
            }
            Err(e) => {
                error!("查询IP[{}]地理位置失败: {}", ip, e);
            }
        }
    }

    // 如果地理位置查询失败或没有初始化，返回基本信息
    IpLocationInfo {
        is_internal,
        ip_type,
        ip: ip.to_string(),
        used_geo_db: false,
        ..IpLocationInfo::default()
    }
}

/// 判断是否是内网IP
fn is_internal_ip(ip: &str) -> bool {
    // 检查是否是回环地址
    if ip == "localhost" || ip == "127.0.0.1" || ip == "::1" {
        return true;
    }

    // 检查IPv4内网范围
    if ip.starts_with("10.") || 
       ip.starts_with("192.168.") ||
       ip.starts_with("169.254.") || 
       (ip.starts_with("172.") && {
            if let Some(second_part) = ip.split('.').nth(1) {
                if let Ok(num) = second_part.parse::<u8>() {
                    (16..=31).contains(&num)
                } else {
                    false
                }
            } else {
                false
            }
       })
    {
        return true;
    }

    // 检查IPv6内网范围
    if ip.starts_with("fc") || ip.starts_with("fd") {
        return true;
    }

    false
}

/// 格式化IP地理位置信息，用于日志输出
pub fn format_ip_location(info: &IpLocationInfo) -> String {
    if info.is_internal {
        return "内网IP".to_string();
    }
    
    if info.ip_type == IpType::Unknown {
        return "未知IP".to_string();
    }
    
    let mut result = String::new();
    
    // 添加国家信息（如果不是中国，显示国家名）
    if info.country != "中国" && info.country != "未知" && !info.country.is_empty() && info.country != "0" {
        result.push_str(&info.country);
    }
    
    // 添加省份信息
    if info.province != "未知" && !info.province.is_empty() && info.province != "0" {
        if !result.is_empty() {
            result.push(' ');
        }
        result.push_str(&info.province);
    }
    
    // 添加城市信息
    if info.city != "未知" && !info.city.is_empty() && info.city != "0" && info.city != info.province {
        if !result.is_empty() {
            result.push(' ');
        }
        result.push_str(&info.city);
    }
    
    // 添加ISP信息
    if info.isp != "未知" && !info.isp.is_empty() && info.isp != "0" {
        if !result.is_empty() {
            result.push_str(" - ");
        }
        result.push_str(&info.isp);
    }
    
    // 如果没有任何地理位置信息，则返回IP地址和类型
    if result.is_empty() {
        let ip_type = match info.ip_type {
            IpType::IPv4 => "IPv4",
            IpType::IPv6 => "IPv6",
            _ => "",
        };
        
        if !ip_type.is_empty() {
            format!("{} ({})", info.ip, ip_type)
        } else {
            info.ip.clone()
        }
    } else {
        result
    }
}

