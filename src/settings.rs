use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
    path::PathBuf,
};
use url::Url;

// The existing ID keeps desktop preferences and pairing data in the same location.
pub const STATE_APP_ID: &str = "NaroLink";

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
    let temporary = directory.join(format!("cli-proxy.{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| -> io::Result<()> {
        fs::create_dir_all(&directory)?;
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary)?;
        file.write_all(proxy.as_ref().map_or("", Url::as_str).as_bytes())?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, directory.join("cli-proxy"))
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result.map_err(|_| "无法保存代理设置，请检查本机数据目录的权限".into())
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
