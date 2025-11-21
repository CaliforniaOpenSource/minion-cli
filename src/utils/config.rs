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
        let mut keys: Vec<&String> = self.values.keys().collect();
        keys.sort();

        let content: String = keys.iter()
            .map(|k| format!("{}={}", k, self.values.get(*k).unwrap()))
            .collect::<Vec<String>>()
            .join("\n");

        fs::write(&self.file_path, content)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;
    use std::io::Write;

    #[test]
    fn test_new_config_no_file() {
        let temp_dir = tempfile::tempdir().unwrap();
        let file_path = temp_dir.path().join("config.env");
        let path_str = file_path.to_str().unwrap();

        let config = Config::new(path_str).unwrap();
        assert!(config.values.is_empty());
        assert_eq!(config.file_path, path_str);
    }

    #[test]
    fn test_new_config_existing_file() -> Result<()> {
        let mut temp_file = NamedTempFile::new()?;
        writeln!(temp_file, "KEY1=value1\nKEY2=value2")?;
        let path = temp_file.path().to_str().unwrap();

        let config = Config::new(path)?;
        assert_eq!(config.get("KEY1"), Some(&"value1".to_string()));
        assert_eq!(config.get("KEY2"), Some(&"value2".to_string()));
        Ok(())
    }

    #[test]
    fn test_get_set() -> Result<()> {
        let temp_dir = tempfile::tempdir().unwrap();
        let file_path = temp_dir.path().join("test_config");
        let mut config = Config::new(file_path.to_str().unwrap())?;

        assert_eq!(config.get("NON_EXISTENT"), None);

        config.set("NEW_KEY".to_string(), "new_value".to_string());
        assert_eq!(config.get("NEW_KEY"), Some(&"new_value".to_string()));

        // Update existing
        config.set("NEW_KEY".to_string(), "updated_value".to_string());
        assert_eq!(config.get("NEW_KEY"), Some(&"updated_value".to_string()));
        Ok(())
    }

    #[test]
    fn test_save_deterministic_order() -> Result<()> {
        let temp_file = NamedTempFile::new()?;
        let path = temp_file.path().to_str().unwrap();
        
        let mut config = Config::new(path)?;
        config.set("C_KEY".to_string(), "valC".to_string());
        config.set("A_KEY".to_string(), "valA".to_string());
        config.set("B_KEY".to_string(), "valB".to_string());
        
        config.save()?;
        
        let content = fs::read_to_string(path)?;
        let lines: Vec<&str> = content.lines().collect();
        
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0], "A_KEY=valA");
        assert_eq!(lines[1], "B_KEY=valB");
        assert_eq!(lines[2], "C_KEY=valC");
        
        Ok(())
    }
}
