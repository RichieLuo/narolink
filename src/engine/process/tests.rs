use super::*;

const SYSTEM_PATH: &str = "/usr/bin:/bin:/usr/sbin:/sbin";

#[test]
fn finder_path_includes_user_and_node_install_directories() {
    let home = Path::new("/Users/naro test");
    let path = macos_cli_search_path(OsStr::new(SYSTEM_PATH), Some(home)).unwrap();
    let directories: Vec<_> = std::env::split_paths(&path).collect();
    assert_eq!(
        directories,
        [
            "/usr/bin",
            "/bin",
            "/usr/sbin",
            "/sbin",
            "/Users/naro test/.local/bin",
            "/opt/homebrew/bin",
            "/usr/local/bin",
        ]
        .map(PathBuf::from)
    );
}

#[test]
fn explicit_path_order_is_preserved_without_duplicate_fallbacks() {
    let inherited = OsStr::new("/custom node/bin:/usr/local/bin:/opt/homebrew/bin:/usr/bin");
    let path = macos_cli_search_path(inherited, None).unwrap();
    let directories: Vec<_> = std::env::split_paths(&path).collect();
    assert_eq!(
        directories,
        [
            "/custom node/bin",
            "/usr/local/bin",
            "/opt/homebrew/bin",
            "/usr/bin",
            "/bin",
            "/usr/sbin",
            "/sbin",
        ]
        .map(PathBuf::from)
    );
}

#[test]
fn missing_path_does_not_add_the_current_directory() {
    let path = macos_cli_search_path(OsStr::new(""), None).unwrap();
    let directories: Vec<_> = std::env::split_paths(&path).collect();
    assert!(!directories.is_empty());
    assert!(directories.iter().all(|directory| directory.is_absolute()));
    assert!(directories.contains(&PathBuf::from("/usr/local/bin")));
    assert!(directories.contains(&PathBuf::from("/opt/homebrew/bin")));
}

struct TestDirectory(PathBuf);

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn npm_codex_and_explicit_js_start_with_a_system_only_parent_path() {
    const PROBE: &str = "NAROLINK_TEST_CLI_PATH_PROBE";
    if std::env::var_os(PROBE).is_some() {
        let mut command = cli_command(Provider::Codex).unwrap();
        let output = command.arg("--version").output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(output.stdout, b"fixture-node-runtime\n");
        // Resolving a CLI must not change the application's global environment.
        assert_eq!(std::env::var("PATH").unwrap(), SYSTEM_PATH);
        return;
    }

    use std::os::unix::fs::{PermissionsExt, symlink};
    let directory =
        TestDirectory(std::env::temp_dir().join(format!("naro-cli-path-{}", uuid::Uuid::new_v4())));
    let bin = directory.0.join(".local/bin");
    std::fs::create_dir_all(&bin).unwrap();
    let node = bin.join("node");
    std::fs::write(&node, b"#!/bin/sh\nprintf 'fixture-node-runtime\\n'\n").unwrap();
    std::fs::set_permissions(&node, std::fs::Permissions::from_mode(0o755)).unwrap();
    let script = directory.0.join("codex.mjs");
    std::fs::write(&script, b"#!/usr/bin/env node\n").unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    symlink(&script, bin.join("codex")).unwrap();

    // Isolate HOME/PATH in a child test process; do not mutate global env while
    // other tests run. Cover both an npm-style symlink and an explicit JS CLI.
    for explicit in [false, true] {
        let mut probe = Command::new(std::env::current_exe().unwrap());
        probe
            .args([
                "--exact",
                "engine::process::tests::npm_codex_and_explicit_js_start_with_a_system_only_parent_path",
                "--nocapture",
            ])
            .env(PROBE, "1")
            .env("HOME", &directory.0)
            .env("PATH", SYSTEM_PATH)
            .env_remove("NAROLINK_CODEX_CLI");
        if explicit {
            probe.env("NAROLINK_CODEX_CLI", &script);
        }
        let output = probe.output().unwrap();
        assert!(
            output.status.success(),
            "explicit={explicit}\n{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
