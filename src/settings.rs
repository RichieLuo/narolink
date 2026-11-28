use crate::engine::{self, Provider};
use std::{
    collections::HashMap,
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
};
use url::Url;

// The existing ID keeps desktop preferences and pairing data in the same location.
pub const STATE_APP_ID: &str = "NaroLink";
const CLI_PATHS_FILE: &str = "cli-paths.json";

pub type CliPaths = HashMap<Provider, String>;

pub struct CliPathError {
    pub provider: Option<Provider>,
    pub message: String,
}

pub fn state_dir() -> Result<PathBuf, String> {
    std::env::var_os("NAROLINK_STATE_DIR")
        .map(PathBuf::from)
        .or_else(|| {
            directories::ProjectDirs::from("", "", STATE_APP_ID)
                .map(|dirs| dirs.data_local_dir().to_owned())
        })
        .ok_or_else(|| "无法定位 Naro 的本机数据目录".into())
}

pub fn cli_proxy() -> Result<Option<Url>, String> {
    match fs::read_to_string(state_dir()?.join("cli-proxy")) {
        Ok(address) => parse_proxy(&address),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err("无法读取代理设置".into()),
    }
}

pub fn save_cli_proxy(address: &str) -> Result<(), String> {
    let proxy = parse_proxy(address)?;
    let directory = state_dir()?;
    save_setting(
        &directory,
        "cli-proxy",
        proxy.as_ref().map_or("", Url::as_str).as_bytes(),
    )
    .map_err(|_| "无法保存代理设置，请检查本机数据目录的权限".into())
}

pub fn cli_paths() -> Result<CliPaths, String> {
    match fs::read(state_dir()?.join(CLI_PATHS_FILE)) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|_| "无法读取 CLI 路径设置。".into()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(CliPaths::default()),
        Err(_) => Err("无法读取 CLI 路径设置。".into()),
    }
}

pub fn save_cli_paths(paths: &mut CliPaths) -> Result<(), CliPathError> {
    let mut normalized = CliPaths::default();
    for provider in Provider::ALL {
        let Some(value) = paths
            .get(&provider)
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
        else {
            continue;
        };
        let path = normalize_cli_path(value)
            .and_then(|path| {
                engine::validate_cli_path(provider, Path::new(&path))?;
                Ok(path)
            })
            .map_err(|message| CliPathError {
                provider: Some(provider),
                message,
            })?;
        normalized.insert(provider, path);
    }
    let error = |message: &str| CliPathError {
        provider: None,
        message: message.to_owned(),
    };
    let directory = state_dir().map_err(|message| error(&message))?;
    let save_error = || error("无法保存 CLI 路径，请检查本机数据目录的权限。");
    let bytes = serde_json::to_vec(&normalized).map_err(|_| save_error())?;
    save_setting(&directory, CLI_PATHS_FILE, &bytes).map_err(|_| save_error())?;
    *paths = normalized;
    Ok(())
}

fn normalize_cli_path(value: &str) -> Result<String, String> {
    // Windows Explorer's Copy as path includes double quotes.
    let value = value
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .unwrap_or(value);
    let path = Path::new(value);
    if let Ok(relative) = path.strip_prefix("~") {
        return directories::BaseDirs::new()
            .and_then(|dirs| {
                dirs.home_dir()
                    .join(relative)
                    .into_os_string()
                    .into_string()
                    .ok()
            })
            .ok_or_else(|| "无法定位用户目录，请输入 CLI 的绝对路径。".into());
    }
    Ok(value.to_owned())
}

fn save_setting(directory: &Path, name: &str, bytes: &[u8]) -> io::Result<()> {
    let temporary = directory.join(format!("{name}.{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| -> io::Result<()> {
        fs::create_dir_all(directory)?;
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, directory.join(name))
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn parse_proxy(address: &str) -> Result<Option<Url>, String> {
    let address = address.trim();
    if address.is_empty() {
        return Ok(None);
    }
    let valid = |url: &Url| {
        matches!(url.scheme(), "http" | "https")
            && url.has_host()
            && url.port() != Some(0)
            && url.path() == "/"
            && url.query().is_none()
            && url.fragment().is_none()
    };
    Url::parse(address)
        .ok()
        .filter(|url| {
            !address.chars().any(char::is_control) && address.contains("://") && valid(url)
        })
        .map(Some)
        .ok_or_else(|| {
            "请输入有效的 HTTP 或 HTTPS 代理地址，例如 http://proxy.example.com:8080".into()
        })
}
