use std::path::Path;
use std::fs;
use std::collections::HashMap;
use anyhow::Result;

pub struct Config {
    values: HashMap<String, String>,
    file_path: String,
}

impl Config {
    pub fn new(file_path: &str) -> Result<Self> {
        let path = Path::new(file_path);
        let values = if path.exists() {
            let content = fs::read_to_string(path)?;
            let mut map = HashMap::new();
            for line in content.lines() {
                if let Some((key, value)) = line.split_once('=') {
                    map.insert(key.trim().to_string(), value.trim().to_string());
                }
            }
            map
        } else {
            HashMap::new()
        };

        Ok(Config {
            values,
            file_path: file_path.to_string(),
        })
    }

    pub fn get(&self, key: &str) -> Option<&String> {
        self.values.get(key)
    }

    pub fn set(&mut self, key: String, value: String) {
        self.values.insert(key, value);
    }

    pub fn save(&self) -> Result<()> {
        let content: String = self.values.iter()
            .map(|(k, v)| format!("{}={}", k, v))
            .collect::<Vec<String>>()
            .join("\n");

        fs::write(&self.file_path, content)?;
        Ok(())
    }
}