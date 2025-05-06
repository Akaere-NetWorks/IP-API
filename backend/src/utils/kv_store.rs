use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::RwLock;
use tokio::time;
use serde::{Deserialize, Serialize};
use tracing::{error, info};
use std::hash::Hash;

const MAX_MEMORY_BYTES: usize = 1024 * 1024 * 1024; // 1024MB
const PERSIST_INTERVAL: Duration = Duration::from_secs(60 * 10); // 10分钟
const EXPIRY_DURATION: Duration = Duration::from_secs(60 * 60 * 24); // 24小时

type SharedStore<K, V> = Arc<RwLock<KvStore<K, V>>>;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Entry<V> {
    value: V,
    expires_at: u64,
    size_bytes: usize,
}

#[derive(Debug, Serialize, Deserialize)]
struct StoreData<K, V> 
where 
    K: Hash + Eq,
{
    entries: HashMap<K, Entry<V>>,
    created_at: u64,
}

#[derive(Debug)]
#[allow(dead_code)]
pub struct KvStore<K, V> 
where 
    K: Serialize + for<'de> Deserialize<'de> + Clone + Hash + Eq,
    V: Serialize + for<'de> Deserialize<'de> + Clone,
{
    entries: HashMap<K, Entry<V>>,
    current_size_bytes: usize,
    file_path: PathBuf,
    last_persist: Instant,
}

#[allow(dead_code)]
impl<K, V> KvStore<K, V> 
where 
    K: Serialize + for<'de> Deserialize<'de> + Clone + Hash + Eq + Send + Sync + 'static,
    V: Serialize + for<'de> Deserialize<'de> + Clone + Send + Sync + 'static,
{
    pub fn new<P: AsRef<Path>>(file_path: P) -> Self {
        let path = file_path.as_ref().to_path_buf();
        
        Self {
            entries: HashMap::new(),
            current_size_bytes: 0,
            file_path: path,
            last_persist: Instant::now(),
        }
    }
    
    pub fn create_shared<P: AsRef<Path>>(file_path: P) -> SharedStore<K, V> {
        let store = Self::new(file_path);
        Arc::new(RwLock::new(store))
    }
    
    pub async fn start_background_tasks(store: SharedStore<K, V>) {
        let persist_store = store.clone();
        let cleanup_store = store.clone();
        
        // 加载持久化数据
        {
            let mut store_lock = store.write().await;
            if let Err(e) = store_lock.load_from_disk() {
                error!("从磁盘加载KV存储失败: {}", e);
            } else {
                info!("从磁盘加载KV存储成功，当前条目数: {}", store_lock.entries.len());
            }
        }
        
        // 启动定期持久化任务
        tokio::spawn(async move {
            let mut interval = time::interval(PERSIST_INTERVAL);
            loop {
                interval.tick().await;
                let mut store = persist_store.write().await;
                if let Err(e) = store.persist_to_disk() {
                    error!("持久化KV存储到磁盘失败: {}", e);
                } else {
                    info!("KV存储已持久化到磁盘，当前条目数: {}", store.entries.len());
                }
            }
        });
        
        // 启动过期数据清理任务
        tokio::spawn(async move {
            let mut interval = time::interval(Duration::from_secs(60)); // 每分钟检查一次过期数据
            loop {
                interval.tick().await;
                let mut store = cleanup_store.write().await;
                let removed = store.cleanup_expired();
                if removed > 0 {
                    info!("清理了 {} 条过期KV存储条目", removed);
                }
            }
        });
    }
    
    pub fn get(&self, key: &K) -> Option<V> {
        if let Some(entry) = self.entries.get(key) {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
                
            if entry.expires_at > now {
                return Some(entry.value.clone());
            }
        }
        None
    }
    
    pub fn set(&mut self, key: K, value: V) -> Result<(), String> {
        // 估算条目大小
        let entry_size = self.estimate_size(&key, &value)?;
        
        // 检查是否会超出内存限制
        let old_size = self.entries.get(&key)
            .map(|entry| entry.size_bytes)
            .unwrap_or(0);
            
        let new_total_size = self.current_size_bytes - old_size + entry_size;
        
        if new_total_size > MAX_MEMORY_BYTES {
            return Err("超出内存限制，无法添加新条目".to_string());
        }
        
        // 计算过期时间
        let expires_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() + EXPIRY_DURATION.as_secs();
            
        // 创建并存储条目
        let entry = Entry {
            value,
            expires_at,
            size_bytes: entry_size,
        };
        
        // 更新当前大小
        self.current_size_bytes = new_total_size;
        
        // 存储条目
        self.entries.insert(key, entry);
        
        // 检查是否需要持久化
        if self.last_persist.elapsed() >= PERSIST_INTERVAL {
            if let Err(e) = self.persist_to_disk() {
                error!("自动持久化KV存储失败: {}", e);
            }
            self.last_persist = Instant::now();
        }
        
        Ok(())
    }
    
    pub fn remove(&mut self, key: &K) -> Option<V> {
        if let Some(entry) = self.entries.remove(key) {
            self.current_size_bytes -= entry.size_bytes;
            return Some(entry.value);
        }
        None
    }
    
    pub fn contains_key(&self, key: &K) -> bool {
        if let Some(entry) = self.entries.get(key) {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
                
            return entry.expires_at > now;
        }
        false
    }
    
    fn estimate_size(&self, key: &K, value: &V) -> Result<usize, String> {
        // 使用序列化来估算对象大小
        let key_bytes = bincode::serialize(key)
            .map_err(|e| format!("无法序列化键以估算大小: {}", e))?;
            
        let value_bytes = bincode::serialize(value)
            .map_err(|e| format!("无法序列化值以估算大小: {}", e))?;
            
        // 额外的内存开销（HashMap节点、过期时间等）
        let overhead = 64; // 保守估计
        
        Ok(key_bytes.len() + value_bytes.len() + overhead)
    }
    
    fn cleanup_expired(&mut self) -> usize {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
            
        let expired_keys: Vec<K> = self.entries.iter()
            .filter(|(_, entry)| entry.expires_at <= now)
            .map(|(key, _)| key.clone())
            .collect();
            
        let count = expired_keys.len();
        
        for key in expired_keys {
            if let Some(entry) = self.entries.remove(&key) {
                self.current_size_bytes -= entry.size_bytes;
            }
        }
        
        count
    }
    
    fn persist_to_disk(&mut self) -> Result<(), String> {
        // 创建数据结构
        let store_data = StoreData {
            entries: self.entries.clone(),
            created_at: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
        };
        
        // 序列化数据
        let serialized = bincode::serialize(&store_data)
            .map_err(|e| format!("序列化KV存储失败: {}", e))?;
            
        // 确保目录存在
        if let Some(parent) = self.file_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("创建KV存储目录失败: {}", e))?;
        }
        
        // 写入临时文件
        let temp_path = self.file_path.with_extension("tmp");
        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&temp_path)
            .map_err(|e| format!("打开临时KV存储文件失败: {}", e))?;
            
        file.write_all(&serialized)
            .map_err(|e| format!("写入KV存储数据失败: {}", e))?;
            
        file.flush()
            .map_err(|e| format!("刷新KV存储文件失败: {}", e))?;
            
        // 原子替换文件
        std::fs::rename(&temp_path, &self.file_path)
            .map_err(|e| format!("替换KV存储文件失败: {}", e))?;
            
        self.last_persist = Instant::now();
        
        Ok(())
    }
    
    fn load_from_disk(&mut self) -> Result<(), String> {
        // 检查文件是否存在
        if !self.file_path.exists() {
            return Ok(());
        }
        
        // 读取文件
        let mut file = File::open(&self.file_path)
            .map_err(|e| format!("打开KV存储文件失败: {}", e))?;
            
        let mut buffer = Vec::new();
        file.read_to_end(&mut buffer)
            .map_err(|e| format!("读取KV存储文件失败: {}", e))?;
            
        // 反序列化数据
        let store_data: StoreData<K, V> = bincode::deserialize(&buffer)
            .map_err(|e| format!("反序列化KV存储数据失败: {}", e))?;
            
        // 清除当前数据
        self.entries.clear();
        self.current_size_bytes = 0;
        
        // 加载数据，跳过过期条目
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
            
        for (key, entry) in store_data.entries {
            if entry.expires_at > now {
                self.current_size_bytes += entry.size_bytes;
                self.entries.insert(key, entry);
            }
        }
        
        Ok(())
    }
    
    pub fn len(&self) -> usize {
        self.entries.len()
    }
    
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
    
    pub fn memory_usage(&self) -> usize {
        self.current_size_bytes
    }
    
    pub fn memory_usage_mb(&self) -> f64 {
        self.current_size_bytes as f64 / (1024.0 * 1024.0)
    }
} 