use crate::configuration::ConfigError::{ConfigMalformed, ConfigNotExisting};
use crate::configuration::StringOption::Key;
use serde_json::Value;
use serenity::prelude::Mutex;
use serenity::prelude::TypeMapKey;
use std::fs::File;
use std::sync::Arc;

enum ConfigError {
    ConfigNotExisting,
    ConfigMalformed,
}

pub struct ConfigLoader {
    path: Box<String>,
    value: Option<Box<Value>>,
}

impl ConfigLoader {
    pub fn default() -> Self {
        ConfigLoader::new("config.json")
    }

    pub fn new(path: &str) -> Self {
        ConfigLoader {
            path: Box::from(path.to_string()),
            value: None,
        }
    }

    fn get_value(&mut self, key: &str) -> Result<Value, ConfigError> {
        match &self.value {
            Some(value) => {
                let v = &value[key];
                if v.is_null() {
                    return Err(ConfigNotExisting);
                }
                Ok(v.clone())
            }
            None => match self.load_config() {
                Ok(_) => self.get_value(key),
                Err(err) => Err(err),
            },
        }
    }

    /*fn set_value(&mut self, key: &str, new_value: Value) -> Result<(), ()> {
        match &mut self.value {
            Some(value) => {
                value[key] = new_value;
                self.write_config()
            }
            None => {
                self.load_config();
                self.set_value(key, new_value)
            }
        }
    }*/

    fn load_config(&mut self) -> Result<(), ConfigError> {
        match File::open(self.path.as_ref()) {
            Ok(file) => match serde_json::from_reader::<File, Value>(file) {
                Ok(value) => {
                    self.value = Some(Box::from(value));
                    Ok(())
                }
                Err(_) => Err(ConfigMalformed),
            },
            Err(_) => Err(ConfigNotExisting),
        }
    }

    /*fn write_config(&self) -> Result<(), ()> {
        if let Ok(file) = File::create(self.path.as_ref()) {
            if serde_json::to_writer_pretty(file, &self.value).is_ok() {
                return Ok(());
            }
        }
        Err(())
    }*/
}

impl TypeMapKey for ConfigLoader {
    type Value = Arc<Mutex<ConfigLoader>>;
}

pub enum StringOption<'a> {
    Key(&'a str, &'a str),
}

impl<'a> StringOption<'a> {
    pub fn get_value(&self, config_loader: &mut ConfigLoader) -> String {
        match *self {
            Key(key, default) => match config_loader.get_value(key) {
                Ok(value) => {
                    if value.is_string() {
                        return value.as_str().unwrap().to_string();
                    }
                    default.to_string()
                }
                Err(err) => match err {
                    ConfigMalformed => panic!("{} is malformed", config_loader.path),
                    _ => default.to_string(),
                },
            },
        }
    }

    /*pub fn set_value(&self, value: &str, config_loader: &mut ConfigLoader) {
        match *self {
            Key(key, _) => {
                if config_loader
                    .set_value(key, Value::String(value.to_string()))
                    .is_err()
                {
                    panic!("Can't write to {}", config_loader.path);
                }
            }
        }
    }*/
}

pub const CONF_TOKEN: StringOption = StringOption::Key("token", "");
pub const CONF_AUTOPLAYLIST_PATH: StringOption = StringOption::Key("autoplaylist_path", "");
pub const CONF_PREFIX: StringOption = StringOption::Key("prefix", "~");
