//! Git repository inspection and workspace actions.
use serde::{Deserialize, Serialize};
use std::{
    ffi::{OsStr, OsString},
    fs::File,
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};

const LOCAL_TIMEOUT: Duration = Duration::from_secs(15);
const REMOTE_TIMEOUT: Duration = Duration::from_secs(38);
const MAX_STDOUT_BYTES: usize = 4 * 1024 * 1024;
const MAX_STDERR_BYTES: usize = 64 * 1024;
const MAX_DIFF_BYTES: usize = 256 * 1024;
const MAX_FILES_PER_GROUP: usize = 500;
const MAX_COMMITS: usize = 30;
const MAX_COMMIT_MESSAGE_BYTES: usize = 64 * 1024;
const MAX_AI_CONTEXT_BYTES: usize = 16 * 1024;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitStatus {
    pub is_repo: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(default)]
    pub detached: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head_short: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upstream: Option<String>,
    #[serde(default)]
    pub ahead: usize,
    #[serde(default)]
    pub behind: usize,
    #[serde(default)]
    pub staged: Vec<GitFileChange>,
    #[serde(default)]
    pub unstaged: Vec<GitFileChange>,
    #[serde(default)]
    pub untracked: Vec<GitFileChange>,
    #[serde(default)]
    pub conflicted: Vec<GitFileChange>,
    #[serde(default)]
    pub branches: Vec<GitBranch>,
    #[serde(default)]
    pub remotes: Vec<String>,
    #[serde(default)]
    pub commits: Vec<GitCommit>,
    #[serde(default)]
    pub truncated: bool,
}

impl GitStatus {
    pub fn total_changes(&self) -> usize {
        self.staged.len() + self.unstaged.len() + self.untracked.len() + self.conflicted.len()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitFileChange {
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub orig_path: Option<String>,
    pub status: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitBranch {
    pub name: String,
    pub current: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upstream: Option<String>,
    #[serde(default)]
    pub ahead: usize,
    #[serde(default)]
    pub behind: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit_short: Option<String>,
    #[serde(default)]
    pub subject: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitCommit {
    pub hash: String,
    pub short_hash: String,
    pub author: String,
    pub relative_date: String,
    pub subject: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GitDiffScope {
    Staged,
    Unstaged,
    Untracked,
    Commit,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitDiffRequest {
    pub scope: GitDiffScope,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub commit: Option<String>,
    #[serde(default)]
    pub full_context: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitDiff {
    pub title: String,
    pub diff: String,
    pub added: usize,
    pub removed: usize,
    pub truncated: bool,
    pub binary: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum GitAction {
    Stage {
        #[serde(default)]
        paths: Vec<String>,
    },
    Unstage {
        #[serde(default)]
        paths: Vec<String>,
    },
    Discard {
        paths: Vec<String>,
        #[serde(default)]
        untracked: bool,
    },
    Commit {
        message: String,
        #[serde(default)]
        amend: bool,
    },
    Checkout {
        branch: String,
    },
    CreateBranch {
        name: String,
        #[serde(default = "default_true")]
        checkout: bool,
    },
    Fetch,
    Pull,
    Push,
}

fn default_true() -> bool {
    true
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitActionResult {
    pub summary: String,
    #[serde(default)]
    pub output: String,
    pub status: GitStatus,
}

pub fn status(cwd: &Path) -> Result<GitStatus, String> {
    let cwd = validate_cwd(cwd)?;
    let Some(root) = repo_root(&cwd)? else {
        return Ok(GitStatus::default());
    };
    status_for_root(&root)
}

pub fn commit_context(cwd: &Path, amend: bool) -> Result<String, String> {
    let cwd = validate_cwd(cwd)?;
    let root = repo_root(&cwd)?.ok_or_else(|| "当前目录不是 Git 仓库。".to_owned())?;
    let st = status_for_root(&root)?;
    let mut context = String::new();
    if let Some(branch) = &st.branch {
        context.push_str("Branch: ");
        context.push_str(branch);
        context.push('\n');
    }
    if !st.commits.is_empty() {
        context.push_str("Recent commits:\n");
        for commit in st.commits.iter().take(5) {
            context.push_str("- ");
            context.push_str(&commit.subject);
            context.push('\n');
        }
    }

    let mut has_diff = false;
    if !st.staged.is_empty() {
        let stat = run_git(
            &root,
            &["diff", "--cached", "--stat", "--no-color", "--no-ext-diff"],
            RunOptions::read_only(),
        )?;
        let patch = run_git(
            &root,
            &["diff", "--cached", "--no-color", "--no-ext-diff"],
            RunOptions::read_only(),
        )?;
        if stat.success || patch.success {
            context.push_str("\nStaged changes:\n");
            context.push_str(&String::from_utf8_lossy(&stat.stdout));
            context.push('\n');
            context.push_str(&String::from_utf8_lossy(&patch.stdout));
            has_diff = true;
        }
    } else if amend && repo_has_head(&root)? {
        let show = run_git(
            &root,
            &[
                "show",
                "--stat",
                "--patch",
                "--no-color",
                "--no-ext-diff",
                "--format=medium",
                "HEAD",
            ],
            RunOptions::read_only(),
        )?;
        if show.success {
            context.push_str("\nHEAD commit to amend:\n");
            context.push_str(&String::from_utf8_lossy(&show.stdout));
            has_diff = true;
        }
    } else if !st.unstaged.is_empty() || !st.untracked.is_empty() || !st.conflicted.is_empty() {
        if !st.unstaged.is_empty() || !st.conflicted.is_empty() {
            let stat = run_git(
                &root,
                &["diff", "--stat", "--no-color", "--no-ext-diff"],
                RunOptions::read_only(),
            )?;
            let patch = run_git(
                &root,
                &["diff", "--no-color", "--no-ext-diff"],
                RunOptions::read_only(),
            )?;
            if stat.success || patch.success {
                context.push_str("\nUnstaged changes:\n");
                context.push_str(&String::from_utf8_lossy(&stat.stdout));
                context.push('\n');
                context.push_str(&String::from_utf8_lossy(&patch.stdout));
                has_diff = true;
            }
        }
        if !st.untracked.is_empty() {
            context.push_str("\nUntracked files:\n");
            for file in st.untracked.iter().take(20) {
                context.push_str("- ");
                context.push_str(&file.path);
                context.push('\n');
            }
            for file in st.untracked.iter().take(4) {
                if context.len() >= MAX_AI_CONTEXT_BYTES {
                    break;
                }
                if let Ok(diff) = untracked_file_diff(&root, &file.path)
                    && !diff.binary
                    && !diff.diff.is_empty()
                {
                    context.push('\n');
                    let limit = 2048.min(diff.diff.len());
                    let mut end = limit;
                    while end > 0 && !diff.diff.is_char_boundary(end) {
                        end -= 1;
                    }
                    context.push_str(&diff.diff[..end]);
                    context.push('\n');
                }
            }
            has_diff = true;
        }
    }

    if !has_diff {
        return Err("当前没有可生成提交信息的更改。".to_owned());
    }
    if context.len() > MAX_AI_CONTEXT_BYTES {
        let mut boundary = MAX_AI_CONTEXT_BYTES;
        while boundary > 0 && !context.is_char_boundary(boundary) {
            boundary -= 1;
        }
        context.truncate(boundary);
        context.push_str("\n...[diff truncated]\n");
    }
    Ok(context)
}

pub fn diff(cwd: &Path, request: &GitDiffRequest) -> Result<GitDiff, String> {
    let cwd = validate_cwd(cwd)?;
    let root = repo_root(&cwd)?.ok_or_else(|| "当前目录不是 Git 仓库。".to_owned())?;
    match request.scope {
        GitDiffScope::Staged => {
            let path = validate_rel_path(request.path.as_deref().unwrap_or_default())?;
            diff_file(&root, &path, true, request.full_context)
        }
        GitDiffScope::Unstaged => {
            let path = validate_rel_path(request.path.as_deref().unwrap_or_default())?;
            diff_file(&root, &path, false, request.full_context)
        }
        GitDiffScope::Untracked => {
            let path = validate_rel_path(request.path.as_deref().unwrap_or_default())?;
            untracked_file_diff(&root, &path)
        }
        GitDiffScope::Commit => {
            let commit = validate_commit_hash(request.commit.as_deref().unwrap_or_default())?;
            let output = run_git(
                &root,
                &[
                    "show",
                    "--no-color",
                    "--no-ext-diff",
                    "--stat",
                    "--patch",
                    "--format=medium",
                    &commit,
                    "--",
                ],
                RunOptions::read_only(),
            )?;
            if !output.success {
                return Err(format_git_error("无法读取提交差异", &output));
            }
            let title = commit.chars().take(12).collect();
            Ok(build_diff(title, &output.stdout, output.stdout_truncated))
        }
    }
}

fn diff_file(root: &Path, path: &str, cached: bool, full_context: bool) -> Result<GitDiff, String> {
    let err_prefix = if cached {
        "无法读取已暂存差异"
    } else {
        "无法读取文件差异"
    };
    if full_context {
        let mut args = vec!["diff", "--no-color", "--no-ext-diff", "-U99999"];
        if cached {
            args.insert(1, "--cached");
        }
        args.push("--");
        args.push(path);
        let output = run_git(root, &args, RunOptions::read_only())?;
        if !output.success {
            return Err(format_git_error(err_prefix, &output));
        }
        if !output.stdout_truncated && output.stdout.len() <= MAX_DIFF_BYTES {
            return Ok(build_diff(path.to_owned(), &output.stdout, false));
        }
    }
    let mut args = vec!["diff", "--no-color", "--no-ext-diff"];
    if cached {
        args.insert(1, "--cached");
    }
    args.push("--");
    args.push(path);
    let output = run_git(root, &args, RunOptions::read_only())?;
    if !output.success {
        return Err(format_git_error(err_prefix, &output));
    }
    Ok(build_diff(
        path.to_owned(),
        &output.stdout,
        output.stdout_truncated,
    ))
}

pub fn run_action(cwd: &Path, action: &GitAction) -> Result<GitActionResult, String> {
    let cwd = validate_cwd(cwd)?;
    let root = repo_root(&cwd)?.ok_or_else(|| "当前目录不是 Git 仓库。".to_owned())?;
    let (summary, output_text) = match action {
        GitAction::Stage { paths } => {
            let args = if paths.is_empty() {
                vec!["add".to_owned(), "-A".to_owned()]
            } else {
                let mut args = vec!["add".to_owned(), "--".to_owned()];
                for path in paths {
                    args.push(validate_rel_path(path)?);
                }
                args
            };
            let output = run_git_owned(&root, &args, RunOptions::local_write())?;
            if !output.success {
                return Err(format_git_error("暂存失败", &output));
            }
            let summary = if paths.is_empty() {
                "已暂存全部更改".to_owned()
            } else {
                "已暂存所选更改".to_owned()
            };
            (summary, combined_output(&output))
        }
        GitAction::Unstage { paths } => {
            let has_head = repo_has_head(&root)?;
            let args = if has_head {
                let mut args = vec!["reset".to_owned(), "-q".to_owned(), "HEAD".to_owned()];
                if !paths.is_empty() {
                    args.push("--".to_owned());
                    for path in paths {
                        args.push(validate_rel_path(path)?);
                    }
                }
                args
            } else {
                let mut args = vec![
                    "rm".to_owned(),
                    "--cached".to_owned(),
                    "-r".to_owned(),
                    "-f".to_owned(),
                    "--".to_owned(),
                ];
                if paths.is_empty() {
                    args.push(".".to_owned());
                } else {
                    for path in paths {
                        args.push(validate_rel_path(path)?);
                    }
                }
                args
            };
            let output = run_git_owned(&root, &args, RunOptions::local_write())?;
            if !output.success {
                return Err(format_git_error("取消暂存失败", &output));
            }
            let summary = if paths.is_empty() {
                "已取消暂存全部更改".to_owned()
            } else {
                "已取消暂存所选更改".to_owned()
            };
            (summary, combined_output(&output))
        }
        GitAction::Discard { paths, untracked } => {
            if paths.is_empty() {
                return Err("请先指定要丢弃改动的文件。".to_owned());
            }
            let mut validated = Vec::with_capacity(paths.len());
            for path in paths {
                validated.push(validate_rel_path(path)?);
            }
            let mut args = if *untracked {
                vec!["clean".to_owned(), "-fd".to_owned(), "--".to_owned()]
            } else {
                vec!["checkout".to_owned(), "--".to_owned()]
            };
            args.extend(validated);
            let output = run_git_owned(&root, &args, RunOptions::local_write())?;
            if !output.success {
                return Err(format_git_error("丢弃更改失败", &output));
            }
            ("已丢弃所选更改".to_owned(), combined_output(&output))
        }
        GitAction::Commit { message, amend } => {
            if message.len() > MAX_COMMIT_MESSAGE_BYTES {
                return Err("提交信息过长。".to_owned());
            }
            let trimmed = message.trim();
            if !amend && trimmed.is_empty() {
                return Err("请输入提交信息。".to_owned());
            }
            let mut args = vec!["commit".to_owned()];
            let mut options = RunOptions::local_write();
            if *amend {
                args.push("--amend".to_owned());
            }
            if trimmed.is_empty() {
                args.push("--no-edit".to_owned());
            } else {
                args.extend(["-F".to_owned(), "-".to_owned()]);
                options.stdin = Some(trimmed.as_bytes().to_vec());
            }
            let output = run_git_owned(&root, &args, options)?;
            if !output.success {
                return Err(format_git_error("提交失败", &output));
            }
            let summary = if *amend {
                "已修正上次提交".to_owned()
            } else {
                "提交完成".to_owned()
            };
            (summary, combined_output(&output))
        }
        GitAction::Checkout { branch } => {
            let branch = validate_branch_name(branch)?;
            let output = run_git(
                &root,
                &["checkout", &branch, "--"],
                RunOptions::local_write(),
            )?;
            if !output.success {
                return Err(format_git_error("切换分支失败", &output));
            }
            (format!("已切换到分支 {branch}"), combined_output(&output))
        }
        GitAction::CreateBranch { name, checkout } => {
            let name = validate_branch_name(name)?;
            let check = run_git(
                &root,
                &["check-ref-format", "--branch", &name],
                RunOptions::read_only(),
            )?;
            if !check.success {
                return Err("分支名称无效。".to_owned());
            }
            let output = if *checkout {
                run_git(&root, &["checkout", "-b", &name], RunOptions::local_write())?
            } else {
                run_git(&root, &["branch", &name], RunOptions::local_write())?
            };
            if !output.success {
                return Err(format_git_error("创建分支失败", &output));
            }
            let summary = if *checkout {
                format!("已创建并切换到分支 {name}")
            } else {
                format!("已创建分支 {name}")
            };
            (summary, combined_output(&output))
        }
        GitAction::Fetch => {
            let output = run_git(&root, &["fetch", "--prune"], RunOptions::network())?;
            if !output.success {
                return Err(format_git_error("获取远端更新失败", &output));
            }
            ("已获取远端更新".to_owned(), combined_output(&output))
        }
        GitAction::Pull => {
            let output = run_git(&root, &["pull"], RunOptions::network())?;
            if !output.success {
                return Err(format_git_error("拉取失败", &output));
            }
            ("拉取完成".to_owned(), combined_output(&output))
        }
        GitAction::Push => {
            let current = status_for_root(&root)?;
            let args = if current.upstream.is_none()
                && !current.detached
                && let Some(branch) = current.branch.as_deref()
            {
                let remote = if current.remotes.iter().any(|r| r == "origin") {
                    Some("origin")
                } else if current.remotes.len() == 1 {
                    Some(current.remotes[0].as_str())
                } else {
                    None
                };
                if let Some(remote) = remote {
                    vec![
                        "push".to_owned(),
                        "-u".to_owned(),
                        remote.to_owned(),
                        branch.to_owned(),
                    ]
                } else {
                    vec!["push".to_owned()]
                }
            } else {
                vec!["push".to_owned()]
            };
            let output = run_git_owned(&root, &args, RunOptions::network())?;
            if !output.success {
                return Err(format_git_error("推送失败", &output));
            }
            ("推送完成".to_owned(), combined_output(&output))
        }
    };
    let status = status_for_root(&root)?;
    Ok(GitActionResult {
        summary,
        output: output_text,
        status,
    })
}

fn status_for_root(root: &Path) -> Result<GitStatus, String> {
    let mut status = GitStatus {
        is_repo: true,
        root: Some(root.to_string_lossy().into_owned()),
        repo_name: root
            .file_name()
            .map(|name| name.to_string_lossy().into_owned()),
        ..GitStatus::default()
    };

    let porcelain = run_git(
        root,
        &["status", "--porcelain=v2", "--branch", "-z", "-uall"],
        RunOptions::read_only(),
    )?;
    if !porcelain.success {
        return Err(format_git_error("无法读取 Git 状态", &porcelain));
    }
    parse_porcelain_v2_z(&porcelain.stdout, &mut status);

    let branches_out = run_git(
        root,
        &[
            "for-each-ref",
            "--sort=-committerdate",
            "--format=%(HEAD)%1f%(refname:short)%1f%(upstream:short)%1f%(upstream:track,nobracket)%1f%(objectname:short)%1f%(contents:subject)",
            "refs/heads",
        ],
        RunOptions::read_only(),
    )?;
    if branches_out.success {
        status.branches = parse_branches(&String::from_utf8_lossy(&branches_out.stdout));
    }
    if status.branches.is_empty()
        && let Some(branch) = status.branch.clone()
    {
        status.branches.push(GitBranch {
            name: branch,
            current: true,
            upstream: status.upstream.clone(),
            ahead: status.ahead,
            behind: status.behind,
            commit_short: status.head_short.clone(),
            subject: String::new(),
        });
    }

    let remotes_out = run_git(root, &["remote"], RunOptions::read_only())?;
    if remotes_out.success {
        status.remotes = String::from_utf8_lossy(&remotes_out.stdout)
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(str::to_owned)
            .collect();
    }

    let log_limit = MAX_COMMITS.to_string();
    let log_out = run_git(
        root,
        &[
            "log",
            "-n",
            &log_limit,
            "--format=%H%x1f%h%x1f%an%x1f%ar%x1f%s",
        ],
        RunOptions::read_only(),
    )?;
    if log_out.success {
        status.commits = parse_commits(&String::from_utf8_lossy(&log_out.stdout));
    }

    Ok(status)
}

fn parse_porcelain_v2_z(bytes: &[u8], status: &mut GitStatus) {
    let entries: Vec<&[u8]> = bytes
        .split(|&byte| byte == 0)
        .filter(|entry| !entry.is_empty())
        .collect();
    let mut index = 0;
    while index < entries.len() {
        let entry = String::from_utf8_lossy(entries[index]);
        index += 1;
        if let Some(rest) = entry.strip_prefix("# ") {
            if let Some(oid) = rest.strip_prefix("branch.oid ") {
                let oid = oid.trim();
                if oid != "(initial)" && !oid.is_empty() {
                    status.head_short = Some(oid.chars().take(7).collect());
                }
            } else if let Some(head) = rest.strip_prefix("branch.head ") {
                let head = head.trim();
                if head == "(detached)" {
                    status.detached = true;
                    status.branch = None;
                } else if !head.is_empty() {
                    status.detached = false;
                    status.branch = Some(head.to_owned());
                }
            } else if let Some(upstream) = rest.strip_prefix("branch.upstream ") {
                let upstream = upstream.trim();
                if !upstream.is_empty() {
                    status.upstream = Some(upstream.to_owned());
                }
            } else if let Some(ab) = rest.strip_prefix("branch.ab ") {
                for token in ab.split_whitespace() {
                    if let Some(ahead) = token.strip_prefix('+') {
                        status.ahead = ahead.parse().unwrap_or(0);
                    } else if let Some(behind) = token.strip_prefix('-') {
                        status.behind = behind.parse().unwrap_or(0);
                    }
                }
            }
            continue;
        }
        if let Some(rest) = entry.strip_prefix("1 ") {
            let mut parts = rest.splitn(8, ' ');
            let Some(xy) = parts.next() else { continue };
            let Some(path) = parts.nth(6) else { continue };
            push_xy_change(status, xy, path.to_owned(), None);
        } else if let Some(rest) = entry.strip_prefix("2 ") {
            let mut parts = rest.splitn(9, ' ');
            let Some(xy) = parts.next() else { continue };
            let Some(path) = parts.nth(7) else { continue };
            let orig_path = if index < entries.len() {
                let orig = String::from_utf8_lossy(entries[index]).into_owned();
                index += 1;
                Some(orig)
            } else {
                None
            };
            push_xy_change(status, xy, path.to_owned(), orig_path);
        } else if let Some(rest) = entry.strip_prefix("u ") {
            let mut parts = rest.splitn(10, ' ');
            let Some(xy) = parts.next() else { continue };
            let Some(path) = parts.nth(8) else { continue };
            push_limited(
                &mut status.conflicted,
                GitFileChange {
                    path: path.to_owned(),
                    orig_path: None,
                    status: xy.to_owned(),
                },
                &mut status.truncated,
            );
        } else if let Some(path) = entry.strip_prefix("? ") {
            push_limited(
                &mut status.untracked,
                GitFileChange {
                    path: path.to_owned(),
                    orig_path: None,
                    status: "?".to_owned(),
                },
                &mut status.truncated,
            );
        }
    }
}

fn push_xy_change(status: &mut GitStatus, xy: &str, path: String, orig_path: Option<String>) {
    let mut chars = xy.chars();
    let x = chars.next().unwrap_or('.');
    let y = chars.next().unwrap_or('.');
    if x != '.' {
        push_limited(
            &mut status.staged,
            GitFileChange {
                path: path.clone(),
                orig_path: orig_path.clone(),
                status: x.to_string(),
            },
            &mut status.truncated,
        );
    }
    if y != '.' {
        push_limited(
            &mut status.unstaged,
            GitFileChange {
                path,
                orig_path: None,
                status: y.to_string(),
            },
            &mut status.truncated,
        );
    }
}

fn push_limited(list: &mut Vec<GitFileChange>, item: GitFileChange, truncated: &mut bool) {
    if list.len() < MAX_FILES_PER_GROUP {
        list.push(item);
    } else {
        *truncated = true;
    }
}

fn parse_branches(output: &str) -> Vec<GitBranch> {
    let mut branches = Vec::new();
    for line in output.lines() {
        if line.is_empty() {
            continue;
        }
        let mut fields = line.split('\x1f');
        let head = fields.next().unwrap_or_default();
        let name = fields.next().unwrap_or_default().trim();
        if name.is_empty() {
            continue;
        }
        let upstream = fields
            .next()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned);
        let track = fields.next().unwrap_or_default();
        let (ahead, behind) = parse_track(track);
        let commit_short = fields
            .next()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned);
        let subject = fields.next().unwrap_or_default().trim().to_owned();
        branches.push(GitBranch {
            name: name.to_owned(),
            current: head == "*",
            upstream,
            ahead,
            behind,
            commit_short,
            subject,
        });
    }
    branches.sort_by(|a, b| b.current.cmp(&a.current));
    branches
}

fn parse_track(track: &str) -> (usize, usize) {
    let mut ahead = 0;
    let mut behind = 0;
    for part in track.split(',') {
        let part = part.trim();
        if let Some(rest) = part.strip_prefix("ahead ") {
            ahead = rest.trim().parse().unwrap_or(0);
        } else if let Some(rest) = part.strip_prefix("behind ") {
            behind = rest.trim().parse().unwrap_or(0);
        }
    }
    (ahead, behind)
}

fn parse_commits(output: &str) -> Vec<GitCommit> {
    output
        .lines()
        .filter_map(|line| {
            let mut fields = line.split('\x1f');
            let hash = fields.next()?.trim();
            let short_hash = fields.next()?.trim();
            let author = fields.next()?.trim();
            let relative_date = fields.next()?.trim();
            let subject = fields.next().unwrap_or_default().trim();
            if hash.is_empty() {
                return None;
            }
            Some(GitCommit {
                hash: hash.to_owned(),
                short_hash: short_hash.to_owned(),
                author: author.to_owned(),
                relative_date: relative_date.to_owned(),
                subject: subject.to_owned(),
            })
        })
        .collect()
}

fn untracked_file_diff(root: &Path, rel_path: &str) -> Result<GitDiff, String> {
    let full = root.join(rel_path);
    let metadata = std::fs::symlink_metadata(&full)
        .map_err(|_| "无法读取未跟踪文件，可能已被删除或移动。".to_owned())?;
    if metadata.file_type().is_symlink() {
        let target = std::fs::read_link(&full)
            .map(|target| target.to_string_lossy().into_owned())
            .unwrap_or_default();
        let diff = format!("--- /dev/null\n+++ b/{rel_path}\n@@ -0,0 +1,1 @@\n+{target}\n");
        return Ok(GitDiff {
            title: rel_path.to_owned(),
            diff,
            added: 1,
            removed: 0,
            truncated: false,
            binary: false,
        });
    }
    if metadata.is_dir() {
        return Ok(GitDiff {
            title: rel_path.to_owned(),
            diff: String::new(),
            added: 0,
            removed: 0,
            truncated: false,
            binary: false,
        });
    }
    let canonical_root = root
        .canonicalize()
        .map_err(|_| "无法解析仓库根目录。".to_owned())?;
    let canonical_file = full
        .canonicalize()
        .map_err(|_| "无法解析文件路径。".to_owned())?;
    if !canonical_file.starts_with(&canonical_root) {
        return Err("路径超出仓库根目录范围。".to_owned());
    }
    let file = File::open(&canonical_file).map_err(|_| "无法打开未跟踪文件。".to_owned())?;
    let mut raw = Vec::new();
    file.take(MAX_DIFF_BYTES as u64 + 1)
        .read_to_end(&mut raw)
        .map_err(|_| "读取未跟踪文件失败。".to_owned())?;
    let truncated = raw.len() > MAX_DIFF_BYTES;
    if truncated {
        raw.truncate(MAX_DIFF_BYTES);
    }
    let sample_len = raw.len().min(8192);
    if raw[..sample_len].contains(&0) {
        return Ok(GitDiff {
            title: rel_path.to_owned(),
            diff: String::new(),
            added: 0,
            removed: 0,
            truncated: false,
            binary: true,
        });
    }
    let text = String::from_utf8_lossy(&raw);
    let line_count = text.lines().count();
    let mut diff = format!("--- /dev/null\n+++ b/{rel_path}\n@@ -0,0 +1,{line_count} @@\n");
    for line in text.split_inclusive('\n') {
        diff.push('+');
        diff.push_str(line);
    }
    if !text.is_empty() && !text.ends_with('\n') {
        diff.push('\n');
    }
    Ok(GitDiff {
        title: rel_path.to_owned(),
        diff,
        added: line_count,
        removed: 0,
        truncated,
        binary: false,
    })
}

fn build_diff(title: String, bytes: &[u8], stream_truncated: bool) -> GitDiff {
    let sample_len = bytes.len().min(8192);
    let has_nul = bytes[..sample_len].contains(&0);
    let mut text = String::from_utf8_lossy(bytes).into_owned();
    let binary = has_nul
        || text
            .lines()
            .any(|line| line.starts_with("Binary files ") && line.ends_with(" differ"))
        || text.lines().any(|line| line == "GIT binary patch");
    let mut truncated = stream_truncated;
    if text.len() > MAX_DIFF_BYTES {
        let mut boundary = MAX_DIFF_BYTES;
        while boundary > 0 && !text.is_char_boundary(boundary) {
            boundary -= 1;
        }
        text.truncate(boundary);
        truncated = true;
    }
    let (added, removed) = count_diff_lines(&text);
    GitDiff {
        title,
        diff: if has_nul { String::new() } else { text },
        added,
        removed,
        truncated,
        binary,
    }
}

fn count_diff_lines(diff: &str) -> (usize, usize) {
    let mut added = 0;
    let mut removed = 0;
    for line in diff.lines() {
        if line.starts_with('+') && !line.starts_with("+++ ") {
            added += 1;
        } else if line.starts_with('-') && !line.starts_with("--- ") {
            removed += 1;
        }
    }
    (added, removed)
}

fn validate_cwd(cwd: &Path) -> Result<PathBuf, String> {
    let text = cwd.to_string_lossy();
    if text.trim().is_empty() || text.contains('\0') {
        return Err("工作目录无效。".to_owned());
    }
    if !cwd.is_dir() {
        return Err("工作目录不存在或不是文件夹。".to_owned());
    }
    Ok(cwd.to_path_buf())
}

fn validate_rel_path(raw: &str) -> Result<String, String> {
    if raw.is_empty() || raw.len() > 4096 || raw.contains('\0') {
        return Err("文件路径无效。".to_owned());
    }
    let normalized = raw.replace('\\', "/");
    let path = Path::new(&normalized);
    if path.is_absolute() {
        return Err("文件路径必须相对于仓库根目录。".to_owned());
    }
    for component in path.components() {
        match component {
            Component::Normal(_) => {}
            _ => return Err("文件路径包含非法目录引用。".to_owned()),
        }
    }
    Ok(normalized)
}

fn validate_branch_name(raw: &str) -> Result<String, String> {
    let name = raw.trim();
    if name.is_empty()
        || name.len() > 255
        || name.starts_with('-')
        || name.chars().any(char::is_control)
        || name.contains("..")
        || name.contains(' ')
        || name.contains('\t')
    {
        return Err("分支名称无效。".to_owned());
    }
    Ok(name.to_owned())
}

fn validate_commit_hash(raw: &str) -> Result<String, String> {
    let hash = raw.trim();
    if hash.len() < 4 || hash.len() > 40 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("提交哈希无效。".to_owned());
    }
    Ok(hash.to_owned())
}

fn repo_root(cwd: &Path) -> Result<Option<PathBuf>, String> {
    let output = run_git(
        cwd,
        &["rev-parse", "--show-toplevel"],
        RunOptions::read_only(),
    )?;
    if !output.success {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.contains("not a git repository") || stderr.contains("Not a git repository") {
            return Ok(None);
        }
        return Err(format_git_error("无法检查 Git 仓库", &output));
    }
    let path = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if path.is_empty() {
        return Ok(None);
    }
    Ok(Some(PathBuf::from(path)))
}

fn repo_has_head(root: &Path) -> Result<bool, String> {
    let output = run_git(
        root,
        &["rev-parse", "--verify", "HEAD"],
        RunOptions::read_only(),
    )?;
    Ok(output.success)
}

struct RunOptions {
    timeout: Duration,
    read_only: bool,
    network: bool,
    stdin: Option<Vec<u8>>,
}

impl RunOptions {
    fn read_only() -> Self {
        Self {
            timeout: LOCAL_TIMEOUT,
            read_only: true,
            network: false,
            stdin: None,
        }
    }

    fn local_write() -> Self {
        Self {
            timeout: LOCAL_TIMEOUT,
            read_only: false,
            network: false,
            stdin: None,
        }
    }

    fn network() -> Self {
        Self {
            timeout: REMOTE_TIMEOUT,
            read_only: false,
            network: true,
            stdin: None,
        }
    }
}

struct GitOutput {
    success: bool,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    stdout_truncated: bool,
}

fn run_git_owned(cwd: &Path, args: &[String], options: RunOptions) -> Result<GitOutput, String> {
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    run_git(cwd, &refs, options)
}

fn run_git(cwd: &Path, args: &[&str], options: RunOptions) -> Result<GitOutput, String> {
    let mut command = git_command()?;
    command
        .args(args)
        .current_dir(cwd)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_EDITOR", "true")
        .env("LC_ALL", "C")
        .env_remove("NAROLINK_RELAY_TOKEN")
        .stdin(if options.stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if options.read_only {
        command.env("GIT_OPTIONAL_LOCKS", "0");
    }
    if options.network
        && let Some(proxy) = crate::settings::cli_proxy().ok().flatten()
    {
        for key in [
            "HTTP_PROXY",
            "HTTPS_PROXY",
            "ALL_PROXY",
            "http_proxy",
            "https_proxy",
            "all_proxy",
        ] {
            command.env(key, proxy.as_str());
        }
        let mut bypass = "localhost,127.0.0.1,::1,[::1]".to_owned();
        if let Some(existing) = ["no_proxy", "NO_PROXY"]
            .into_iter()
            .filter_map(|key| std::env::var(key).ok())
            .find(|value| !value.trim().is_empty())
        {
            bypass.push(',');
            bypass.push_str(existing.trim());
        }
        command.env("NO_PROXY", &bypass).env("no_proxy", &bypass);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }

    let mut child = command
        .spawn()
        .map_err(|_| "未找到 Git，请先在本机安装 Git。".to_owned())?;

    if let Some(input) = options.stdin
        && let Some(mut stdin) = child.stdin.take()
    {
        std::thread::spawn(move || {
            let _ = stdin.write_all(&input);
        });
    }

    let stdout_pipe = child.stdout.take().ok_or("无法读取 Git 输出。")?;
    let stderr_pipe = child.stderr.take().ok_or("无法读取 Git 错误输出。")?;
    let (out_tx, out_rx) = mpsc::channel();
    let (err_tx, err_rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = out_tx.send(read_capped(stdout_pipe, MAX_STDOUT_BYTES));
    });
    std::thread::spawn(move || {
        let _ = err_tx.send(read_capped(stderr_pipe, MAX_STDERR_BYTES));
    });

    let status = wait_with_timeout(&mut child, options.timeout)?;
    let (stdout, stdout_truncated) = out_rx.recv().unwrap_or_default();
    let (stderr, _) = err_rx.recv().unwrap_or_default();
    Ok(GitOutput {
        success: status.success(),
        stdout,
        stderr,
        stdout_truncated,
    })
}

fn read_capped(mut reader: impl Read, limit: usize) -> (Vec<u8>, bool) {
    let mut data = Vec::new();
    let mut buf = [0u8; 8192];
    let mut truncated = false;
    loop {
        match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                if data.len() < limit {
                    let keep = n.min(limit - data.len());
                    data.extend_from_slice(&buf[..keep]);
                    if keep < n {
                        truncated = true;
                    }
                } else {
                    truncated = true;
                }
            }
            Err(_) => break,
        }
    }
    (data, truncated)
}

fn wait_with_timeout(
    child: &mut Child,
    timeout: Duration,
) -> Result<std::process::ExitStatus, String> {
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status),
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err("Git 操作超时，请检查网络、凭据或仓库状态。".to_owned());
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(error) => return Err(format!("等待 Git 进程失败：{error}")),
        }
    }
}

fn git_command() -> Result<Command, String> {
    let path = git_search_path()?;
    let executable = std::env::var_os("NAROLINK_GIT_CLI")
        .map(PathBuf::from)
        .filter(|path| path.is_file())
        .or_else(|| executable_on_path("git", &path))
        .ok_or_else(|| "未找到 Git，请先在本机安装 Git。".to_owned())?;
    let mut command = Command::new(executable);
    command.env("PATH", path);
    Ok(command)
}

fn git_search_path() -> Result<OsString, String> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    #[cfg(target_os = "macos")]
    {
        let mut paths: Vec<PathBuf> = if path.is_empty() {
            Vec::new()
        } else {
            std::env::split_paths(&path).collect()
        };
        for fallback in [
            "/opt/homebrew/bin",
            "/usr/local/bin",
            "/usr/bin",
            "/bin",
            "/usr/sbin",
            "/sbin",
        ] {
            let dir = PathBuf::from(fallback);
            if !paths.contains(&dir) {
                paths.push(dir);
            }
        }
        std::env::join_paths(paths).map_err(|_| "无法构建 Git 搜索路径。".to_owned())
    }
    #[cfg(not(target_os = "macos"))]
    Ok(path)
}

fn executable_on_path(name: &str, path: &OsStr) -> Option<PathBuf> {
    let extensions: &[&str] = if cfg!(windows) {
        &["exe", "cmd", ""]
    } else {
        &[""]
    };
    std::env::split_paths(path)
        .flat_map(|dir| {
            extensions.iter().map(move |ext| {
                dir.join(if ext.is_empty() {
                    name.to_owned()
                } else {
                    format!("{name}.{ext}")
                })
            })
        })
        .find(|candidate| candidate.is_file())
}

fn combined_output(output: &GitOutput) -> String {
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    match (stdout.is_empty(), stderr.is_empty()) {
        (true, true) => String::new(),
        (false, true) => stdout,
        (true, false) => stderr,
        (false, false) => format!("{stdout}\n{stderr}"),
    }
}

fn format_git_error(prefix: &str, output: &GitOutput) -> String {
    let detail = combined_output(output);
    if detail.is_empty() {
        prefix.to_owned()
    } else {
        let short: String = detail.chars().take(600).collect();
        format!("{prefix}：{short}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let path =
                std::env::temp_dir().join(format!("naro-git-test-{name}-{}", uuid::Uuid::new_v4()));
            fs::create_dir_all(&path).expect("create temp dir");
            Self(path)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn git_ok(dir: &Path, args: &[&str]) {
        let out = run_git(dir, args, RunOptions::local_write()).expect("run git");
        assert!(
            out.success,
            "git {:?} failed: {}",
            args,
            combined_output(&out)
        );
    }

    fn init_repo(dir: &Path) {
        git_ok(dir, &["init", "-b", "main"]);
        git_ok(dir, &["config", "user.name", "Naro Test"]);
        git_ok(dir, &["config", "user.email", "test@naro.local"]);
        git_ok(dir, &["config", "commit.gpgsign", "false"]);
    }

    #[test]
    fn non_repo_returns_is_repo_false() {
        let dir = TempDir::new("non-repo");
        let st = status(&dir.0).expect("status non-repo");
        assert!(!st.is_repo);
        assert!(st.root.is_none());
    }

    #[test]
    fn full_local_workflow_and_remote_sync() {
        let remote = TempDir::new("remote");
        git_ok(&remote.0, &["init", "--bare", "-b", "main"]);

        let repo = TempDir::new("repo");
        init_repo(&repo.0);
        git_ok(
            &repo.0,
            &["remote", "add", "origin", &remote.0.to_string_lossy()],
        );

        // Create an untracked file and inspect diff before first commit.
        fs::write(repo.0.join("hello.txt"), "line1\nline2\n").unwrap();
        let ctx_untracked = commit_context(&repo.0, false).unwrap();
        assert!(ctx_untracked.contains("hello.txt"));
        assert!(ctx_untracked.contains("+line1"));
        let st = status(&repo.0).unwrap();
        assert!(st.is_repo);
        assert_eq!(st.branch.as_deref(), Some("main"));
        assert_eq!(st.untracked.len(), 1);
        assert_eq!(st.untracked[0].path, "hello.txt");

        let untracked_diff = diff(
            &repo.0,
            &GitDiffRequest {
                scope: GitDiffScope::Untracked,
                path: Some("hello.txt".into()),
                commit: None,
                full_context: false,
            },
        )
        .unwrap();
        assert_eq!(untracked_diff.added, 2);
        assert!(untracked_diff.diff.contains("+line1"));

        // Stage, unstage (unborn HEAD), re-stage, and commit.
        let staged = run_action(
            &repo.0,
            &GitAction::Stage {
                paths: vec!["hello.txt".into()],
            },
        )
        .unwrap();
        assert_eq!(staged.status.staged.len(), 1);
        assert!(staged.status.untracked.is_empty());

        let unstaged = run_action(
            &repo.0,
            &GitAction::Unstage {
                paths: vec!["hello.txt".into()],
            },
        )
        .unwrap();
        assert!(unstaged.status.staged.is_empty());
        assert_eq!(unstaged.status.untracked.len(), 1);

        run_action(&repo.0, &GitAction::Stage { paths: vec![] }).unwrap();
        let ctx_staged = commit_context(&repo.0, false).unwrap();
        assert!(ctx_staged.contains("Staged changes:"));
        assert!(ctx_staged.contains("+line1"));
        let committed = run_action(
            &repo.0,
            &GitAction::Commit {
                message: "Initial commit".into(),
                amend: false,
            },
        )
        .unwrap();
        assert_eq!(committed.status.commits.len(), 1);
        assert_eq!(committed.status.commits[0].subject, "Initial commit");

        // Push to remote (sets upstream automatically).
        let pushed = run_action(&repo.0, &GitAction::Push).unwrap();
        assert_eq!(pushed.status.upstream.as_deref(), Some("origin/main"));
        assert_eq!(pushed.status.ahead, 0);

        // Modify file, view unstaged diff, then discard.
        fs::write(repo.0.join("hello.txt"), "line1\nchanged\n").unwrap();
        let unstaged_diff = diff(
            &repo.0,
            &GitDiffRequest {
                scope: GitDiffScope::Unstaged,
                path: Some("hello.txt".into()),
                commit: None,
                full_context: false,
            },
        )
        .unwrap();
        assert_eq!(unstaged_diff.added, 1);
        assert_eq!(unstaged_diff.removed, 1);

        let discarded = run_action(
            &repo.0,
            &GitAction::Discard {
                paths: vec!["hello.txt".into()],
                untracked: false,
            },
        )
        .unwrap();
        assert_eq!(discarded.status.total_changes(), 0);
        assert_eq!(
            fs::read_to_string(repo.0.join("hello.txt")).unwrap(),
            "line1\nline2\n"
        );

        // Create branch and checkout back.
        let created = run_action(
            &repo.0,
            &GitAction::CreateBranch {
                name: "feature/git-ui".into(),
                checkout: true,
            },
        )
        .unwrap();
        assert_eq!(created.status.branch.as_deref(), Some("feature/git-ui"));
        assert_eq!(created.status.branches.len(), 2);

        let switched = run_action(
            &repo.0,
            &GitAction::Checkout {
                branch: "main".into(),
            },
        )
        .unwrap();
        assert_eq!(switched.status.branch.as_deref(), Some("main"));

        // Commit diff & fetch/pull.
        let commit_hash = switched.status.commits[0].hash.clone();
        let commit_diff = diff(
            &repo.0,
            &GitDiffRequest {
                scope: GitDiffScope::Commit,
                path: None,
                commit: Some(commit_hash),
                full_context: false,
            },
        )
        .unwrap();
        assert!(commit_diff.diff.contains("Initial commit"));
        assert!(commit_diff.diff.contains("+line1"));

        run_action(&repo.0, &GitAction::Fetch).unwrap();
        run_action(&repo.0, &GitAction::Pull).unwrap();
    }

    #[test]
    fn rejects_invalid_paths_and_branch_names() {
        assert!(validate_rel_path("../secret").is_err());
        assert!(validate_rel_path("/etc/passwd").is_err());
        assert!(validate_rel_path("").is_err());
        assert!(validate_branch_name("-bad").is_err());
        assert!(validate_branch_name("a..b").is_err());
        assert!(validate_commit_hash("--option").is_err());
        assert!(validate_commit_hash("abcd1234").is_ok());
    }
}
