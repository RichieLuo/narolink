use super::{CURRENT_VERSION, Release, download};
use anyhow::{Context, Result, ensure};
use semver::Version;
use std::{
    ffi::OsStr,
    fs::{self, File},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    process::{self, ChildStdin, Command, Stdio},
    thread,
    time::{Duration, Instant},
};
use tempfile::{Builder, TempDir};

const STAGE_PREFIX: &str = ".naro-update-";
const APPLY: &str = "--naro-apply-update";
const BOOT: &str = "--naro-update-boot";
const FAILED: &str = "--naro-update-failed";
const READY: &str = "booted";

#[derive(Clone)]
pub struct Installation {
    executable: PathBuf,
    target: PathBuf,
}

pub struct Staged {
    installation: Installation,
    directory: TempDir,
    pub version: Version,
}

pub struct Restart {
    input: ChildStdin,
    directory: TempDir,
}

#[derive(Default)]
pub struct Startup {
    probe: Option<(PathBuf, u32)>,
    pub failed: bool,
}

impl Installation {
    pub fn discover() -> Option<Self> {
        Self::at(std::env::current_exe().ok()?)
    }

    fn at(executable: PathBuf) -> Option<Self> {
        let executable = executable.canonicalize().ok()?;
        let target = match std::env::consts::OS {
            "macos" => {
                let bin = executable.parent()?;
                let contents = bin.parent()?;
                let bundle = contents.parent()?;
                if executable.file_name()? != "narolink"
                    || bin.file_name()? != "MacOS"
                    || contents.file_name()? != "Contents"
                    || bundle.extension()? != "app"
                {
                    return None;
                }
                bundle.to_owned()
            }
            "windows" if executable.file_name()?.eq_ignore_ascii_case("Naro.exe") => {
                executable.clone()
            }
            _ => return None,
        };
        Some(Self { executable, target })
    }

    pub fn stage(self, release: &Release, progress: impl FnMut(u64)) -> Result<Staged> {
        // Stage beside the installed app so replacement stays on one filesystem.
        let directory = Builder::new()
            .prefix(STAGE_PREFIX)
            .tempdir_in(self.parent())
            .context("Could not stage the update in the installation folder")?;
        let package = directory.path().join("package");
        let file = File::create_new(&package)?;
        download(release, &file, progress)?;
        file.sync_all()?;
        drop(file);
        prepare_package(directory.path(), &package, &release.version)?;
        fs::copy(&self.executable, helper_path(directory.path()))?;
        Ok(Staged {
            installation: self,
            directory,
            version: release.version.clone(),
        })
    }

    fn parent(&self) -> &Path {
        self.target.parent().expect("install directory")
    }

    fn validate_stage(&self, directory: &Path) -> Result<PathBuf> {
        let directory = directory.canonicalize()?;
        ensure!(
            directory.parent() == Some(self.parent())
                && directory
                    .file_name()
                    .and_then(OsStr::to_str)
                    .is_some_and(|name| name.starts_with(STAGE_PREFIX)),
            "Invalid update staging path"
        );
        Ok(directory)
    }
}

impl Staged {
    pub fn start(self) -> Result<Restart> {
        let mut child = Command::new(helper_path(self.directory.path()))
            .arg(APPLY)
            .arg(&self.installation.executable)
            .arg(process::id().to_string())
            .arg(self.version.to_string())
            .stdin(Stdio::piped())
            .spawn()
            .context("Could not start the update helper")?;
        Ok(Restart {
            input: child.stdin.take().expect("piped updater stdin"),
            directory: self.directory,
        })
    }
}

impl Restart {
    pub fn commit(mut self) -> Result<()> {
        self.input
            .write_all(&[1])
            .context("Could not submit the update for installation")?;
        let _ = self.directory.keep();
        Ok(())
    }
}

impl Startup {
    pub fn testing_launch(&self) -> bool {
        self.probe.is_some()
    }

    pub fn ready(&self) {
        let Some((stage, helper_pid)) = &self.probe else {
            return;
        };
        if let Err(error) = fs::write(stage.join(READY), []) {
            eprintln!("Naro: could not confirm update startup: {error}");
            return;
        }
        // Windows cannot remove the staged copy of the helper while it is running.
        if cfg!(windows) {
            let stage = stage.clone();
            let helper_pid = *helper_pid;
            thread::spawn(move || {
                if wait_for_exit(helper_pid).is_ok() {
                    let _ = fs::remove_dir_all(stage);
                }
            });
        }
    }
}

pub fn startup() -> Result<Option<Startup>> {
    let mut args = std::env::args_os().skip(1);
    match args.next().as_deref().and_then(OsStr::to_str) {
        Some(APPLY) => {
            let executable = args.next().context("Missing update destination")?;
            let parent = args
                .next()
                .context("Missing original Naro process")?
                .to_string_lossy()
                .parse()?;
            let version = Version::parse(
                &args
                    .next()
                    .context("Missing update version")?
                    .to_string_lossy(),
            )?;
            let mut commit = [0];
            if io::stdin().read_exact(&mut commit).is_ok() && commit == [1] {
                apply(PathBuf::from(executable), parent, &version)?;
            }
            Ok(None)
        }
        Some(BOOT) => {
            let stage = args.next().context("Missing update staging directory")?;
            let helper_pid = args
                .next()
                .context("Missing update helper process")?
                .to_string_lossy()
                .parse()?;
            let stage = Installation::discover()
                .context("Invalid update destination")?
                .validate_stage(Path::new(&stage))?;
            Ok(Some(Startup {
                probe: Some((stage, helper_pid)),
                failed: false,
            }))
        }
        Some(FAILED) => Ok(Some(Startup {
            failed: true,
            ..Default::default()
        })),
        _ => Ok(Some(Startup::default())),
    }
}

fn helper_path(directory: &Path) -> PathBuf {
    directory.join(format!("naro-installer{}", std::env::consts::EXE_SUFFIX))
}

fn candidate_path(directory: &Path) -> PathBuf {
    directory.join(if cfg!(target_os = "macos") {
        "Naro.app"
    } else {
        "package"
    })
}

fn apply(executable: PathBuf, parent: u32, version: &Version) -> Result<()> {
    let installation = Installation::at(executable).context("Invalid update destination")?;
    let helper = std::env::current_exe()?.canonicalize()?;
    let directory =
        installation.validate_stage(helper.parent().context("Missing update directory")?)?;
    ensure!(
        helper == helper_path(&directory),
        "Invalid update helper path"
    );
    wait_for_exit(parent)?;
    let lock = File::options()
        .write(true)
        .create(true)
        .truncate(false)
        .open(installation.parent().join(".naro-update.lock"))?;
    lock.lock()?;

    let candidate = candidate_path(&directory);
    let previous = directory.join("previous.exe");
    let install = || -> Result<()> {
        ensure!(
            *version > Version::parse(CURRENT_VERSION)?,
            "The update is no longer newer than the installation"
        );
        ensure!(
            same_file_contents(&helper, &installation.executable)?,
            "Naro changed since the update was downloaded"
        );
        replace(&installation.target, &candidate, &previous)?;
        Ok(())
    };
    let outcome = match install() {
        Ok(()) => {
            let launch = launch_updated(&installation, &directory);
            if let Err(problem) = &launch {
                restore(&installation.target, &candidate, &previous).with_context(|| {
                    format!(
                        "Could not restore the previous Naro after {problem:#}. Update files are in {}",
                        directory.display()
                    )
                })?;
            }
            launch
        }
        error => error,
    };
    if let Err(problem) = outcome {
        eprintln!("Naro: update failed: {problem:#}");
        Command::new(&installation.executable)
            .arg(FAILED)
            .stdin(Stdio::null())
            .spawn()
            .with_context(|| {
                format!("Could not reopen Naro after the update failed: {problem:#}")
            })?;
    }
    if !cfg!(windows) && directory.join(READY).is_file() {
        let _ = fs::remove_dir_all(directory);
    }
    Ok(())
}

fn same_file_contents(left: &Path, right: &Path) -> Result<bool> {
    use sha2::{Digest, Sha256};
    let digest = |path| -> io::Result<_> {
        let mut sha = Sha256::new();
        io::copy(&mut File::open(path)?, &mut sha)?;
        Ok(sha.finalize())
    };
    Ok(digest(left)? == digest(right)?)
}

fn launch_updated(installation: &Installation, stage: &Path) -> Result<()> {
    let mut child = Command::new(&installation.executable)
        .arg(BOOT)
        .arg(stage)
        .arg(process::id().to_string())
        .stdin(Stdio::null())
        .spawn()
        .context("Could not start the updated Naro")?;
    let deadline = Instant::now() + Duration::from_secs(15);
    while !stage.join(READY).is_file() {
        if let Some(status) = child.try_wait()? {
            anyhow::bail!("Updated Naro exited before starting: {status}");
        }
        if Instant::now() >= deadline {
            // Keep the previous build if the new app is slow, without killing a live app.
            break;
        }
        thread::sleep(Duration::from_millis(100));
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn prepare_package(directory: &Path, package: &Path, version: &Version) -> Result<()> {
    ensure!(
        Command::new("/usr/bin/ditto")
            .args(["-x", "-k"])
            .arg(package)
            .arg(directory)
            .status()?
            .success(),
        "Could not unpack the Naro update"
    );
    let bundle = candidate_path(directory);
    let plist = bundle.join("Contents/Info.plist");
    let version_on_disk = Command::new("/usr/bin/plutil")
        .args([
            "-extract",
            "CFBundleShortVersionString",
            "raw",
            "-o",
            "-",
            "--",
        ])
        .arg(plist)
        .output()?;
    ensure!(
        version_on_disk.status.success()
            && String::from_utf8(version_on_disk.stdout)?.trim() == version.to_string(),
        "Naro update bundle version does not match the signed metadata"
    );
    ensure!(
        Command::new("/usr/bin/codesign")
            .args([
                "--verify",
                "--deep",
                "--strict",
                "-R",
                "=identifier \"chat.bato.narolink\""
            ])
            .arg(&bundle)
            .status()?
            .success(),
        "Naro update bundle failed code signature verification"
    );
    ensure!(
        bundle.join("Contents/MacOS/narolink").is_file(),
        "Naro update executable is missing"
    );
    fs::remove_file(package)?;
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn prepare_package(_: &Path, _: &Path, _: &Version) -> Result<()> {
    Ok(())
}

#[cfg(target_os = "macos")]
fn exchange(target: &Path, candidate: &Path) -> io::Result<()> {
    use std::{ffi::CString, os::unix::ffi::OsStrExt};
    let target = CString::new(target.as_os_str().as_bytes())?;
    let candidate = CString::new(candidate.as_os_str().as_bytes())?;
    // Both paths refer to complete bundles on the same volume.
    let code = unsafe { libc::renamex_np(target.as_ptr(), candidate.as_ptr(), libc::RENAME_SWAP) };
    if code == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(target_os = "macos")]
fn replace(target: &Path, candidate: &Path, _: &Path) -> io::Result<()> {
    exchange(target, candidate)
}

#[cfg(target_os = "macos")]
fn restore(target: &Path, candidate: &Path, _: &Path) -> io::Result<()> {
    exchange(target, candidate)
}

#[cfg(not(target_os = "macos"))]
fn replace(target: &Path, candidate: &Path, previous: &Path) -> io::Result<()> {
    fs::copy(target, previous)?;
    fs::rename(candidate, target)
}

#[cfg(not(target_os = "macos"))]
fn restore(target: &Path, _: &Path, previous: &Path) -> io::Result<()> {
    fs::rename(previous, target)
}

#[cfg(windows)]
fn wait_for_exit(pid: u32) -> Result<()> {
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use windows_sys::Win32::{
        Foundation::{ERROR_INVALID_PARAMETER, WAIT_OBJECT_0},
        System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject},
    };
    let handle = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
    if handle.is_null() {
        let error = io::Error::last_os_error();
        if error.raw_os_error() == Some(ERROR_INVALID_PARAMETER as i32) {
            return Ok(());
        }
        return Err(error.into());
    }
    let handle = unsafe { OwnedHandle::from_raw_handle(handle) };
    ensure!(
        unsafe { WaitForSingleObject(handle.as_raw_handle(), 20_000) } == WAIT_OBJECT_0,
        "Naro processes did not exit in time for the update"
    );
    Ok(())
}

#[cfg(not(windows))]
fn wait_for_exit(_: u32) -> Result<()> {
    Ok(())
}
