use serde::{Deserialize, Serialize};
use std::path::Path;

// Define a struct to represent the TOML configuration
#[derive(Debug, Deserialize, Clone)]
pub struct Config {
    pub theme: String,
    pub left_panel_width: Option<usize>,
    pub language: Vec<Language>,
}

#[derive(Debug, Deserialize, Clone)]
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

#[derive(Debug, Deserialize, Clone)]
pub struct IndentConfig {
    pub width: i32,
    pub unit:  String,
}

    pub fn get() -> Config {
    let red_home = option_env!("RED_HOME").unwrap_or("./");
    let config_path = Path::new(red_home).join("config.toml");
    let toml_str = std::fs::read_to_string(config_path).expect("Unable to read config.toml file");
    let config: Config = toml::from_str(&toml_str).expect("Unable to parse TOML");
    config
}

#[cfg(test)]
mod congif_tests {
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
}