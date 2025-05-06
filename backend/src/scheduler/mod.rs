use chrono::{DateTime, Duration, Utc};
use log::{error, info};
use std::sync::{Arc, Mutex};
use tokio::time;

pub struct Scheduler {
    tasks: Vec<(String, Arc<dyn Fn() -> Result<(), String> + Send + Sync + 'static>, Arc<Mutex<DateTime<Utc>>>, Duration)>,
}

impl Scheduler {
    pub fn new() -> Self {
        Self { tasks: Vec::new() }
    }

    pub fn schedule_daily(&mut self, name: &str, _hour: u32, _minute: u32, task: impl Fn() -> Result<(), String> + Send + Sync + 'static) {
        let task_arc = Arc::new(task);
        let last_run = Arc::new(Mutex::new(Utc::now()));
        let duration = Duration::days(1);
        self.tasks.push((name.to_string(), task_arc, last_run, duration));
    }

    pub async fn start(&self) {
        for (name, task, last_run, duration) in &self.tasks {
            let name = name.clone();
            let task = Arc::clone(task);
            let last_run = Arc::clone(last_run);
            let duration = *duration;
            
            tokio::spawn(async move {
                loop {
                    let now = Utc::now();
                    let last = {
                        let mut last = last_run.lock().unwrap();
                        
                        if now.signed_duration_since(*last) >= duration {
                            info!("执行定时任务: {}", name);
                            match task() {
                                Ok(_) => {
                                    info!("定时任务 {} 执行成功", name);
                                    *last = now;
                                },
                                Err(e) => {
                                    error!("定时任务 {} 执行失败: {}", name, e);
                                }
                            }
                        }
                        
                        *last
                    };
                    
                    let next_run = last + duration;
                    let sleep_duration = next_run.signed_duration_since(now);
                    let sleep_millis = sleep_duration.num_milliseconds().max(1000) as u64;
                    
                    time::sleep(time::Duration::from_millis(sleep_millis)).await;
                }
            });
        }
    }
} 