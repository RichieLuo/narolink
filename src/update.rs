mod install;

use anyhow::{Context, Result, bail, ensure};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use reqwest::{StatusCode, blocking::Client};
use ring::signature::{ED25519, UnparsedPublicKey};
use semver::Version;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    time::Duration,
};

pub use install::{Installation, Restart, Staged, Startup, startup};

pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const RELEASES_URL: &str = "https://github.com/RichieLuo/narolink/releases";
const PUBLIC_KEY: &str = include_str!("../assets/update-key.txt");
const METADATA_NAME: &str = "naro-update-v1.json";
const MAX_METADATA_SIZE: u64 = 64 * 1024;
const MAX_PACKAGE_SIZE: u64 = 256 * 1024 * 1024;

#[derive(Clone)]
pub struct Release {
    pub version: Version,
    pub size: u64,
    name: String,
    sha256: [u8; 32],
}

pub enum Check {
    Current,
    Unpublished,
    Available(Release),
}

#[derive(Deserialize)]
struct SignedMetadata {
    payload: String,
    signature: String,
}

#[derive(Deserialize)]
struct Metadata {
    version: String,
    assets: BTreeMap<String, Asset>,
}

#[derive(Deserialize)]
struct Asset {
    name: String,
    size: u64,
    sha256: String,
}

pub fn check() -> Result<Check> {
    let mut response = client(Duration::from_secs(20))?
        .get(format!("{RELEASES_URL}/latest/download/{METADATA_NAME}"))
        .send()?;
    if response.status() == StatusCode::NOT_FOUND {
        return Ok(Check::Unpublished);
    }
    response = response.error_for_status()?;
    let mut body = Vec::new();
    response
        .take(MAX_METADATA_SIZE + 1)
        .read_to_end(&mut body)?;
    let target = format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH);
    let current = Version::parse(CURRENT_VERSION)?;
    let key = STANDARD.decode(PUBLIC_KEY.trim())?;
    read_metadata(&body, &key, &target, &current)
}

fn read_metadata(body: &[u8], key: &[u8], target: &str, current: &Version) -> Result<Check> {
    ensure!(
        body.len() as u64 <= MAX_METADATA_SIZE,
        "Update metadata is too large"
    );
    let signed: SignedMetadata = serde_json::from_slice(body)?;
    let payload = STANDARD.decode(signed.payload)?;
    let signature = STANDARD.decode(signed.signature)?;
    UnparsedPublicKey::new(&ED25519, key)
        .verify(&payload, &signature)
        .map_err(|_| anyhow::anyhow!("Invalid update signature"))?;

    let metadata: Metadata = serde_json::from_slice(&payload)?;
    let version = Version::parse(&metadata.version)?;
    ensure!(
        version.pre.is_empty() && version.build.is_empty(),
        "Expected a stable release version"
    );
    if version <= *current {
        return Ok(Check::Current);
    }
    let Some(asset) = metadata.assets.get(target) else {
        return Ok(Check::Unpublished);
    };
    let name = &asset.name;
    ensure!(
        !name.is_empty()
            && name.len() <= 128
            && name.as_bytes()[0].is_ascii_alphanumeric()
            && name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte)),
        "Invalid update asset name"
    );
    ensure!(
        (1..=MAX_PACKAGE_SIZE).contains(&asset.size),
        "Invalid update asset size"
    );
    let sha256 = STANDARD
        .decode(&asset.sha256)?
        .try_into()
        .map_err(|_| anyhow::anyhow!("Invalid update asset digest"))?;
    Ok(Check::Available(Release {
        version,
        size: asset.size,
        name: name.clone(),
        sha256,
    }))
}

fn download(release: &Release, output: impl Write, progress: impl FnMut(u64)) -> Result<()> {
    let url = format!(
        "{RELEASES_URL}/download/v{}/{}",
        release.version, release.name
    );
    let response = client(Duration::from_secs(600))?
        .get(url)
        .send()?
        .error_for_status()?;
    if response
        .content_length()
        .is_some_and(|size| size != release.size)
    {
        bail!("Update download size does not match the signed metadata");
    }
    copy_verified(release, response, output, progress)
}

fn copy_verified(
    release: &Release,
    input: impl Read,
    mut output: impl Write,
    mut progress: impl FnMut(u64),
) -> Result<()> {
    let mut input = input.take(release.size + 1);
    let mut sha256 = Sha256::new();
    let mut written = 0;
    let mut buffer = [0; 64 * 1024];
    loop {
        let count = input.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        written += count as u64;
        ensure!(
            written <= release.size,
            "Update download exceeded the signed size"
        );
        sha256.update(&buffer[..count]);
        output.write_all(&buffer[..count])?;
        progress(written);
    }
    ensure!(written == release.size, "Update download was incomplete");
    ensure!(
        sha256.finalize()[..] == release.sha256,
        "Update download failed integrity verification"
    );
    output.flush()?;
    Ok(())
}

fn client(timeout: Duration) -> Result<Client> {
    Client::builder()
        .https_only(true)
        .user_agent(concat!("Naro/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(10))
        .timeout(timeout)
        .build()
        .context("Could not create update client")
}

#[cfg(test)]
mod tests {
    use super::*;
    use ring::{
        rand::SystemRandom,
        signature::{Ed25519KeyPair, KeyPair},
    };
    use serde_json::json;

    #[test]
    fn metadata_requires_a_trusted_signature_and_a_newer_stable_version_for_this_target() {
        let key = Ed25519KeyPair::generate_pkcs8(&SystemRandom::new()).unwrap();
        let key = Ed25519KeyPair::from_pkcs8(key.as_ref()).unwrap();
        let trusted = key.public_key().as_ref();
        let version = Version::new(0, 0, 0);
        let target = "windows-x86_64";
        let manifest = |release_version: &str, name: &str, tamper: bool| {
            let mut payload = serde_json::to_vec(&json!({
                "version": release_version,
                "assets": {target: {"name": name, "size": 4, "sha256": STANDARD.encode([0; 32])}},
            }))
            .unwrap();
            let signature = key.sign(&payload);
            if tamper {
                payload.push(b' ');
            }
            serde_json::to_vec(&json!({
                "payload": STANDARD.encode(payload),
                "signature": STANDARD.encode(signature.as_ref()),
            }))
            .unwrap()
        };
        let check = |release_version, name, tamper| {
            read_metadata(
                &manifest(release_version, name, tamper),
                trusted,
                target,
                &version,
            )
        };
        assert!(matches!(
            check("0.0.1", "Naro.exe", false),
            Ok(Check::Available(_))
        ));
        assert!(matches!(
            check("0.0.0", "Naro.exe", false),
            Ok(Check::Current)
        ));
        assert!(check("0.0.1", "Naro.exe", true).is_err());
        assert!(check("0.0.1-beta.1", "Naro.exe", false).is_err());
        assert!(check("0.0.1", "../Naro.exe", false).is_err());
        let body = manifest("0.0.1", "Naro.exe", false);
        assert!(read_metadata(&body, &[0; 32], target, &version).is_err());
        assert!(matches!(
            read_metadata(&body, trusted, "macos-aarch64", &version),
            Ok(Check::Unpublished)
        ));
        assert!(matches!(
            read_metadata(&body, trusted, target, &Version::new(0, 0, 2)),
            Ok(Check::Current)
        ));
    }

    #[test]
    fn partial_oversized_and_modified_downloads_are_rejected() {
        let expected = b"good";
        let release = Release {
            version: Version::new(0, 0, 1),
            name: "Naro.exe".into(),
            size: expected.len() as u64,
            sha256: Sha256::digest(expected).into(),
        };
        let mut result = Vec::new();
        copy_verified(&release, &expected[..], &mut result, |_| {}).unwrap();
        assert_eq!(result, expected);
        for input in [&b"goo"[..], &b"good!"[..], &b"bad!"[..]] {
            assert!(copy_verified(&release, input, std::io::sink(), |_| {}).is_err());
        }
    }
}
