use std::path::Path;
use std::sync::Arc;
use tokio::sync::RwLock;
use crate::maxmind::reader::IpInfo;
use super::kv_store::KvStore;
use tracing::info;

#[allow(dead_code)]
pub struct IpCache {
    store: Arc<RwLock<KvStore<String, IpInfo>>>,
}

#[allow(dead_code)]
impl IpCache {
    pub fn new<P: AsRef<Path>>(file_path: P) -> Self {
        let store = KvStore::create_shared(file_path);
        Self { store }
    }
    
    pub async fn start_tasks(self: &Self) {
        KvStore::start_background_tasks(self.store.clone()).await;
    }
    
    pub async fn get(&self, ip: &str) -> Option<IpInfo> {
        let store = self.store.read().await;
        store.get(&ip.to_string())
    }
    
    pub async fn set(&self, ip: &str, info: IpInfo) -> Result<(), String> {
        let mut store = self.store.write().await;
        let result = store.set(ip.to_string(), info);
        if result.is_ok() {
            info!("IP信息已缓存: {}", ip);
        }
        result
    }
    
    pub async fn contains(&self, ip: &str) -> bool {
        let store = self.store.read().await;
        store.contains_key(&ip.to_string())
    }
    
    pub async fn remove(&self, ip: &str) -> Option<IpInfo> {
        let mut store = self.store.write().await;
        store.remove(&ip.to_string())
    }
    
    pub async fn stats(&self) -> (usize, f64) {
        let store = self.store.read().await;
        (store.len(), store.memory_usage_mb())
    }
} 