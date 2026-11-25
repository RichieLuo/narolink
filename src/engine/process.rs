use super::Provider;
use serde_json::Value;
use std::ffi::OsStr;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc::{self, Receiver, SyncSender},
};
use std::time::Duration;

/// Upper bound for each serialized CLI frame.
const MAX_INPUT_FRAME: usize = 24 * 1024 * 1024;

/// Resolve known npm packages without passing user input to cmd.exe or a shell.
pub(super) fn cli_command(provider: Provider) -> Result<Command, String> {
    let name = match provider {
        Provider::Codex => "codex",
        Provider::Claude => "claude",
        Provider::Antigravity => "agy",
    };
    let executable = std::env::var_os(format!("NAROLINK_{}_CLI", name.to_ascii_uppercase()))
        .map(PathBuf::from)
        .or_else(|| find_executable(name))
        .or_else(|| {
            if cfg!(windows) && provider == Provider::Antigravity {
                let home = directories::BaseDirs::new()?;
                return executable_on_path(name, home.data_local_dir().join("agy/bin").as_os_str());
            }
            let home = directories::BaseDirs::new()?;
            executable_on_path(name, home.home_dir().join(".local/bin").as_os_str())
        })
        .ok_or("AI CLI not found; install it or select its executable")?;
    #[cfg(windows)]
    let executable = if provider == Provider::Codex {
        let desktop_bin = directories::BaseDirs::new().map(|dirs| {
            dirs.data_local_dir()
                .join("OpenAI")
                .join("Codex")
                .join("bin")
        });
        codex_with_tool_host(
            executable,
            &std::env::var_os("PATH").unwrap_or_default(),
            desktop_bin.as_deref(),
        )
    } else {
        executable
    };
    if executable
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("cmd") || e.eq_ignore_ascii_case("bat"))
        || executable.extension().is_none() && cfg!(windows)
    {
        let parent = executable.parent().ok_or("Invalid CLI executable")?;
        let package = match provider {
            Provider::Codex => "node_modules/@openai/codex/bin/codex.js",
            Provider::Claude => "node_modules/@anthropic-ai/claude-code/cli.js",
            _ => return Err("请选择 Antigravity 的原生 agy.exe 文件".into()),
        };
        let script = parent.join(package);
        if script.is_file() {
            let node = parent.join("node.exe");
            let node = if node.is_file() {
                node
            } else {
                find_executable("node").ok_or("Node.js is required for this CLI installation")?
            };
            let mut command = Command::new(node);
            command.arg(script);
            return Ok(command);
        }
        return Err("Unsupported CLI shim; choose the native executable or install the standard npm package".into());
    }
    if executable
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("js") || e.eq_ignore_ascii_case("mjs"))
    {
        let mut command = Command::new(find_executable("node").ok_or("Node.js is required")?);
        command.arg(executable);
        Ok(command)
    } else {
        Ok(Command::new(executable))
    }
}
fn find_executable(name: &str) -> Option<PathBuf> {
    executable_on_path(name, &std::env::var_os("PATH").unwrap_or_default())
}

fn executable_on_path(name: &str, path: &OsStr) -> Option<PathBuf> {
    let extensions: &[&str] = if cfg!(windows) {
        &["exe", "cmd", ""]
    } else {
        &[""]
    };
    std::env::split_paths(path)
        .flat_map(|dir| {
            extensions.iter().map(move |extension| {
                dir.join(if extension.is_empty() {
                    name.to_owned()
                } else {
                    format!("{name}.{extension}")
                })
            })
        })
        .find(|candidate| candidate.is_file())
}

#[cfg(windows)]
fn codex_with_tool_host(executable: PathBuf, path: &OsStr, desktop_bin: Option<&Path>) -> PathBuf {
    const HOST: &str = "codex-code-mode-host.exe";
    if !executable
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("exe"))
        || executable.with_file_name(HOST).is_file()
    {
        return executable;
    }
    if let Ok(target) = executable.canonicalize()
        && target.with_file_name(HOST).is_file()
    {
        return target;
    }
    let Some(file_id) = windows_file_id(&executable) else {
        return executable;
    };

    // The Desktop CLI alias and its installation can be hard links to one file.
    let desktop_installs = desktop_bin
        .and_then(|root| root.read_dir().ok())
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .map(|entry| entry.path());
    std::env::split_paths(path)
        .chain(desktop_installs)
        .map(|dir| dir.join("codex.exe"))
        .find(|candidate| {
            candidate.with_file_name(HOST).is_file() && windows_file_id(candidate) == Some(file_id)
        })
        .unwrap_or(executable)
}

#[cfg(windows)]
fn windows_file_id(path: &Path) -> Option<(u32, u32, u32)> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle,
    };

    let file = std::fs::File::open(path).ok()?;
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    // The file owns the live handle and the API writes into the initialized struct.
    if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } == 0 {
        return None;
    }
    Some((
        info.dwVolumeSerialNumber,
        info.nFileIndexHigh,
        info.nFileIndexLow,
    ))
}

fn spawn_raw(command: &mut Command, cwd: &Path) -> Result<Child, String> {
    command
        .current_dir(cwd)
        .env_remove("NAROLINK_RELAY_TOKEN")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    command
        .spawn()
        .map_err(|_| "Cannot start AI CLI; check executable and working directory".into())
}

pub(super) struct Process {
    child: Child,
    pub writer: SyncSender<Value>,
    pub lines: Receiver<Result<Value, String>>,
    stop: Arc<AtomicBool>,
    #[cfg(windows)]
    _job: WindowsJob,
}
impl Process {
    pub fn spawn(command: &mut Command, cwd: &Path) -> Result<Self, String> {
        let mut child = spawn_raw(command, cwd)?;
        #[cfg(windows)]
        let job = match WindowsJob::attach(&child) {
            Ok(job) => job,
            Err(error) => {
                kill_tree(child.id());
                let _ = child.kill();
                let _ = child.wait();
                return Err(error);
            }
        };
        let mut stdin = child.stdin.take().ok_or("Cannot open CLI input")?;
        let stdout = child.stdout.take().ok_or("Cannot open CLI output")?;
        let stop = Arc::new(AtomicBool::new(false));
        let writer_stop = stop.clone();
        let (writer, writes) = mpsc::sync_channel::<Value>(64);
        let (tx, lines) = mpsc::sync_channel(8);
        let write_errors = tx.clone();
        std::thread::spawn(move || {
            while !writer_stop.load(Ordering::Acquire) {
                match writes.recv_timeout(Duration::from_millis(50)) {
                    Ok(value) => {
                        let mut bytes = match encode_input(&value) {
                            Ok(bytes) => bytes,
                            Err(error) => {
                                let _ = write_errors.send(Err(error));
                                break;
                            }
                        };
                        bytes.push(b'\n');
                        if stdin.write_all(&bytes).and_then(|_| stdin.flush()).is_err() {
                            let _ = write_errors.send(Err("Cannot write AI CLI input".into()));
                            break;
                        }
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                    Err(_) => break,
                }
            }
        });
        let reader_stop = stop.clone();
        std::thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                if reader_stop.load(Ordering::Acquire) {
                    break;
                }
                let result = read_cli_frame(&mut reader);
                match result {
                    Ok(Some(value)) => {
                        if tx.send(Ok(value)).is_err() {
                            break;
                        }
                    }
                    Ok(None) => {
                        // The writer also owns a sender for write failures; EOF
                        // must be explicit or its sender would hide disconnection.
                        let _ =
                            tx.send(Err("AI CLI closed output before completing the turn".into()));
                        break;
                    }
                    Err(error) => {
                        let _ = tx.send(Err(error));
                        break;
                    }
                }
            }
        });
        Ok(Self {
            child,
            writer,
            lines,
            stop,
            #[cfg(windows)]
            _job: job,
        })
    }
    pub fn send(&self, value: Value) -> Result<(), String> {
        // Count serialized bytes (including JSON escapes) before enqueueing. A
        // rejected frame must report an error rather than silently closing stdin.
        validate_input(&value)?;
        self.writer
            .try_send(value)
            .map_err(|_| "AI CLI input channel is unavailable".into())
    }
}
impl Drop for Process {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        kill_tree(self.child.id());
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Preflight without allocating another copy of a potentially large image frame.
struct FrameSize {
    bytes: usize,
    limit: usize,
}
impl Write for FrameSize {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > self.limit.saturating_sub(self.bytes) {
            return Err(std::io::Error::other("AI frame exceeded the size limit"));
        }
        self.bytes += bytes.len();
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
fn validate_input(value: &Value) -> Result<(), String> {
    serde_json::to_writer(
        &mut FrameSize {
            bytes: 0,
            limit: MAX_INPUT_FRAME,
        },
        value,
    )
    .map_err(|_| "AI CLI input exceeded the size limit".into())
}
fn encode_input(value: &Value) -> Result<Vec<u8>, String> {
    // The writer channel also accepts small cancellation frames directly.
    // Validate here as well so even these cannot silently close the pipe.
    validate_input(value)?;
    serde_json::to_vec(value).map_err(|_| "Cannot encode AI CLI input".into())
}

/// Read a bounded JSON Lines frame from a local CLI.
fn read_cli_frame(reader: &mut impl BufRead) -> Result<Option<Value>, String> {
    let Some(bytes) = read_frame_with_limit(reader, MAX_INPUT_FRAME)? else {
        return Ok(None);
    };
    let value: Value =
        serde_json::from_slice(&bytes).map_err(|_| "AI CLI emitted invalid JSON".to_owned())?;
    Ok(Some(value))
}

fn read_frame_with_limit(
    reader: &mut impl BufRead,
    limit: usize,
) -> Result<Option<Vec<u8>>, String> {
    let mut bytes = Vec::new();
    let n = reader
        .take((limit + 1) as u64)
        .read_until(b'\n', &mut bytes)
        .map_err(|_| "Cannot read AI stream")?;
    if n == 0 {
        return Ok(None);
    }
    if n > limit {
        return Err("AI stream frame exceeded the size limit".into());
    }
    Ok(Some(bytes))
}
fn kill_tree(pid: u32) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let _ = Command::new("taskkill.exe")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(0x08000000)
            .status();
    }
    #[cfg(unix)]
    {
        let _ = Command::new("kill")
            .args(["-KILL", "--", &format!("-{pid}")])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}
#[cfg(windows)]
struct WindowsJob(windows_sys::Win32::Foundation::HANDLE);
#[cfg(windows)]
impl WindowsJob {
    fn attach(child: &Child) -> Result<Self, String> {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::System::JobObjects::*;
        // SAFETY: Handles remain owned here/by Child; structures have the exact API size.
        unsafe {
            let handle = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if handle.is_null() {
                return Err("Cannot create AI process isolation job".into());
            }
            let job = Self(handle);
            let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            if SetInformationJobObject(
                handle,
                JobObjectExtendedLimitInformation,
                &limits as *const _ as *const _,
                std::mem::size_of_val(&limits) as u32,
            ) == 0
                || AssignProcessToJobObject(handle, child.as_raw_handle() as _) == 0
            {
                return Err("Cannot isolate AI process tree; refusing unsafe startup".into());
            }
            Ok(job)
        }
    }
}
#[cfg(windows)]
impl Drop for WindowsJob {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.0);
        }
    }
}
