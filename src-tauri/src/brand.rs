use std::path::PathBuf;

pub const APP_NAME: &str = "CC Switch X";
pub const APP_CONFIG_DIR_NAME: &str = ".cc-switch-x";
pub const OFFICIAL_APP_CONFIG_DIR_NAME: &str = ".cc-switch";
pub const DEEP_LINK_SCHEME: &str = "ccswitchx";
pub const DEFAULT_PROXY_PORT: u16 = 15722;
pub const IN_APP_UPDATES_ENABLED: bool = false;

pub fn default_app_config_dir() -> PathBuf {
    crate::config::get_home_dir().join(APP_CONFIG_DIR_NAME)
}

pub fn official_app_config_dir() -> PathBuf {
    crate::config::get_home_dir().join(OFFICIAL_APP_CONFIG_DIR_NAME)
}
