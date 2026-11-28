use axum::http::{HeaderMap, StatusCode, header};
use hmac::{Hmac, Mac};
use sha2::Sha256;
use std::{
    fs::{self, OpenOptions},
    io::Write,
    sync::Mutex,
    time::{Duration, Instant, SystemTime},
};

const COOKIE: &str = "narolink";
const LIFETIME: u64 = 30 * 24 * 60 * 60;

pub(super) struct Auth {
    key: [u8; 32],
    attempts: Mutex<(Instant, u32)>,
}

impl Auth {
    pub fn load() -> Result<Self, String> {
        let root = crate::settings::state_dir()?;
        fs::create_dir_all(&root).map_err(|error| error.to_string())?;
        let file = root.join("pairing.key");
        let key = match fs::read(&file) {
            Ok(key) => key,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let key = [
                    uuid::Uuid::new_v4().into_bytes(),
                    uuid::Uuid::new_v4().into_bytes(),
                ]
                .concat();
                let mut options = OpenOptions::new();
                options.write(true).create_new(true);
                #[cfg(unix)]
                {
                    use std::os::unix::fs::OpenOptionsExt;
                    options.mode(0o600);
                }
                let mut file = options.open(&file).map_err(|error| error.to_string())?;
                file.write_all(&key)
                    .and_then(|()| file.sync_all())
                    .map_err(|error| error.to_string())?;
                key
            }
            Err(error) => return Err(error.to_string()),
        };
        Ok(Self {
            key: key.try_into().map_err(|_| "Naro 配对密钥格式无效")?,
            attempts: Mutex::new((Instant::now(), 0)),
        })
    }

    fn mac(&self, input: &str) -> Hmac<Sha256> {
        let mut mac =
            Hmac::<Sha256>::new_from_slice(&self.key).expect("HMAC accepts a 32-byte key");
        mac.update(input.as_bytes());
        mac
    }

    pub(super) fn relay_device_key(&self) -> [u8; 32] {
        self.mac("relay-device-key:v1")
            .finalize()
            .into_bytes()
            .into()
    }

    fn slot_code(&self, slot: u64) -> String {
        let tag = self.mac(&format!("pair:{slot}")).finalize().into_bytes();
        let value = u64::from_be_bytes(tag[..8].try_into().unwrap());
        format!("{:08}", value % 100_000_000)
    }

    pub fn code(&self) -> String {
        self.slot_code(now() / 300)
    }

    pub fn pair(&self, code: &str) -> Result<(), (StatusCode, &'static str)> {
        let mut attempts = self.attempts.lock().unwrap();
        if attempts.0.elapsed() > Duration::from_secs(60) {
            *attempts = (Instant::now(), 0);
        }
        if attempts.1 >= 10 {
            return Err((
                StatusCode::TOO_MANY_REQUESTS,
                "配对尝试次数过多，请一分钟后再试",
            ));
        }
        let slot = now() / 300;
        let valid = [slot, slot.saturating_sub(1)]
            .into_iter()
            .any(|slot| self.slot_code(slot) == code);
        if !valid {
            attempts.1 += 1;
            return Err((StatusCode::UNAUTHORIZED, "配对码错误或已过期"));
        }
        Ok(())
    }

    pub fn cookie(&self, secure: bool) -> String {
        let expires = now() + LIFETIME;
        let nonce = uuid::Uuid::new_v4().simple();
        let token = format!("{expires}.{nonce}");
        let tag = self.mac(&format!("device:{token}")).finalize().into_bytes();
        let signature = tag
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        format!(
            "{COOKIE}={token}.{signature}; Path=/; Max-Age={LIFETIME}; HttpOnly; SameSite=Strict{}",
            if secure { "; Secure" } else { "" }
        )
    }

    pub fn clear_cookie(secure: bool) -> String {
        format!(
            "{COOKIE}=; Path=/; Max-Age=0; HttpOnly; SameSite=Strict{}",
            if secure { "; Secure" } else { "" }
        )
    }

    pub fn authorized(&self, headers: &HeaderMap) -> bool {
        let prefix = format!("{COOKIE}=");
        let Some(value) = headers
            .get(header::COOKIE)
            .and_then(|value| value.to_str().ok())
            .and_then(|cookies| {
                cookies
                    .split(';')
                    .find_map(|cookie| cookie.trim_start().strip_prefix(&prefix))
            })
            .filter(|cookie| cookie.len() < 140)
        else {
            return false;
        };
        let Some((token, tag)) = value.rsplit_once('.') else {
            return false;
        };
        let Some((expires, _)) = token.split_once('.') else {
            return false;
        };
        if expires
            .parse::<u64>()
            .ok()
            .is_none_or(|expires| expires < now())
            || tag.len() != 64
            || !tag.is_ascii()
        {
            return false;
        }
        let decoded: Result<Vec<_>, _> = (0..64)
            .step_by(2)
            .map(|i| u8::from_str_radix(&tag[i..i + 2], 16))
            .collect();
        decoded.is_ok_and(|decoded| {
            self.mac(&format!("device:{token}"))
                .verify_slice(&decoded)
                .is_ok()
        })
    }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
