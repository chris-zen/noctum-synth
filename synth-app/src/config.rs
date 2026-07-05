use std::fmt;
use std::path::PathBuf;

use directories::ProjectDirs;
use serde::{Deserialize, Serialize};

use crate::ui::analysis::AnalysisTab;
use crate::ui::analysis::config::AnalysisConfig;
use crate::ui::app::Tab;
use crate::ui::settings_view::Settings;
use crate::ui::viewport::WindowGeometry;

const APP_NAME_FOLDER: &str = "AnalogSynth";
const CONFIG_FILE: &str = "config.json";

#[derive(Debug)]
pub enum ConfigError {
    NoConfigDir,
    Io(std::io::Error),
    Parse(serde_json::Error),
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoConfigDir => write!(f, "could not determine a config directory"),
            Self::Io(e) => write!(f, "failed to read config file: {e}"),
            Self::Parse(e) => write!(f, "failed to parse config file: {e}"),
        }
    }
}

impl std::error::Error for ConfigError {}

#[derive(Default, Serialize, Deserialize)]
pub struct Config {
    #[serde(skip)]
    path: PathBuf,
    pub active_tab: Tab,
    pub settings: Settings,
    pub main_viewport: Option<WindowGeometry>,
    pub analysis_open: bool,
    pub analysis_viewport: Option<WindowGeometry>,
    #[serde(default)]
    pub analysis: AnalysisConfig,
    #[serde(default, skip_serializing, rename = "analysis_tab")]
    analysis_tab_legacy: Option<AnalysisTab>,
}

impl Config {
    pub fn try_new() -> Result<Self, ConfigError> {
        let dirs = ProjectDirs::from("", "", APP_NAME_FOLDER).ok_or(ConfigError::NoConfigDir)?;
        let path = dirs.config_dir().join(CONFIG_FILE);

        let mut config = match std::fs::read_to_string(&path) {
            Ok(contents) => serde_json::from_str(&contents).map_err(ConfigError::Parse)?,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Self::default(),
            Err(err) => return Err(ConfigError::Io(err)),
        };
        config.path = path;
        if let Some(tab) = config.analysis_tab_legacy.take() {
            config.analysis.active_tab = tab;
        }
        Ok(config)
    }

    pub fn save(&self) {
        if let Some(parent) = self.path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(serialized) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(&self.path, serialized);
        }
    }
}
