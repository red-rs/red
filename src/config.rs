use serde::{Deserialize, Serialize};
use std::path::Path;

use log2::*;
use rust_embed::Embed;

#[derive(Embed, Debug)]
#[folder = ""]
#[include = "config.toml"]
#[include = "langs/*/*"]
#[include = "themes/*"]
#[include = "tmux*"]
pub struct Asset;


#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Config {
    pub theme: String,
    pub left_panel_width: Option<usize>,
    pub language: Vec<Language>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Language {
    pub name:       String,
    pub types:      Vec<String>,
    pub comment:    String,
    pub lsp:        Option<Vec<String>>, 
    pub indent:     IndentConfig, 
    pub executable: Option<bool>,
    pub exec:       Option<String>,
    pub exectest:   Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct IndentConfig {
    pub width: i32,
    pub unit:  String,
}

pub fn get() -> Config {
    // if red_home is not set, use the assets
    let toml_str = match std::env::var("RED_HOME") {
        Ok(red_home) => {
            let config_path = Path::new(&red_home).join("config.toml");
            match std::fs::read_to_string(config_path) {
                Ok(toml_str) => toml_str,
                Err(_) => read_assests_config(),
            }
        },
        Err(_) => {
            // checkout ~/.red/config.toml
            if let Some(home) = dirs::home_dir() {
                let config_path = home.join(".red").join("config.toml");
                match std::fs::read_to_string(config_path) {
                    Ok(toml_str) => toml_str,
                    Err(_) => read_assests_config(),
                }
            } else {
                eprintln!("Couldn't find home directory");
                read_assests_config()
            }
        },
    };

    // let red_home = std::env::var("RED_HOME").expect("RED_HOME must be set");
    // let config_path = Path::new(&red_home).join("config.toml");
    // let toml_str = std::fs::read_to_string(config_path).expect("Unable to read config.toml file");
    let config: Config = toml::from_str(&toml_str).expect("Unable to parse TOML");
    config
}

pub fn get_file_content_env(file_name: &str) -> anyhow::Result<String> {
    // get the file content from red_home
    std::env::var("RED_HOME").map_err(|_| anyhow::anyhow!("RED_HOME must be set"))?;
    let red_home = std::env::var("RED_HOME").unwrap();
    let file_path = Path::new(&red_home).join(file_name);
    let file_content = std::fs::read_to_string(file_path)?;
    debug!("Read {} from RED_HOME environment successfully", file_name);
    Ok(file_content)
}

pub fn get_file_content_home(file_name: &str) -> anyhow::Result<String> {
    // get the file content from home directory
    let home = dirs::home_dir().unwrap();
    let file_path = Path::new(&home).join(".anycode").join(file_name);
    let file_content = std::fs::read_to_string(file_path)?;
    debug!("Read {} from home directory successfully", file_name);
    Ok(file_content)
}

pub fn get_file_content_assests(file_name: &str) -> anyhow::Result<String> {
    // get the file content from assests 
    let config = Asset::get(file_name);
    match config {
        Some(config) => {
            let config_str = std::str::from_utf8(config.data.as_ref())?;
            debug!("Read {} from assets successfully", file_name);
            Ok(config_str.to_string())
        }
        None => anyhow::bail!("File not found: {}", file_name),
    }
}

pub fn get_file_content(file_name: &str) -> anyhow::Result<String> {
    // get the file content, priority: env > home > assests
    get_file_content_env(file_name)
        .or_else(|_| get_file_content_home(file_name))
        .or_else(|_| get_file_content_assests(file_name))
}

pub fn read_assests_config() -> String {
    let config = Asset::get("config.toml").unwrap();
    let config_str = std::str::from_utf8(config.data.as_ref()).unwrap();
    config_str.to_string()
}

#[cfg(test)]
mod congif_tests {
    use super::*;

    #[test]
    fn test_read_config() {
        let config = crate::config::get();

        println!("Theme: {}", config.theme);
        println!();

        for language in config.language {
            println!("Language: {}", language.name);
            println!("File Types: {:?}", language.types);
            println!("Comment Token: {}", language.comment);
            println!("LSP: {:?}", language.lsp);
            println!("Indent: {:?}", language.indent);
            println!();
        }
    }

    #[test]
    fn test_assets() {
        let config = Asset::get("config.toml").unwrap();
        println!("{:?}", std::str::from_utf8(config.data.as_ref()));

        for file in Asset::iter() {
            println!("{}", file.as_ref());
        }
    }
}