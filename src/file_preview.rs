use std::{
    fs::File,
    io::Read,
    path::{Path, PathBuf},
};
use url::Url;

pub(crate) const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;
pub(crate) const MAX_MARKDOWN_BYTES: usize = 256 * 1024;

pub(crate) type LoadResult = Result<TextFile, &'static str>;

#[derive(Debug)]
pub(crate) struct TextFile {
    pub text: String,
    pub markdown: bool,
    pub syntax: String,
    pub lines: usize,
}

pub(crate) fn read_file(path: &Path) -> LoadResult {
    // Reject directories and special files before open (e.g. FIFOs can block).
    let metadata = std::fs::metadata(path).map_err(file_error)?;
    if metadata.is_dir() {
        return Err("这是文件夹，请使用“在文件夹中显示”。");
    }
    if !metadata.is_file() {
        return Err("仅支持预览普通文件。");
    }
    if metadata.len() > MAX_FILE_BYTES {
        return Err("文件超过 2 MiB，暂不支持在应用内预览。");
    }
    let file = File::open(path).map_err(file_error)?;
    let metadata = file.metadata().map_err(file_error)?;
    if !metadata.is_file() {
        return Err("仅支持预览普通文件。");
    }
    let mut bytes = Vec::new();
    file.take(MAX_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(file_error)?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err("文件超过 2 MiB，暂不支持在应用内预览。");
    }
    let text = String::from_utf8(bytes).map_err(|_| "此文件不是 UTF-8 文本，暂不支持预览。")?;
    if text
        .chars()
        .any(|ch| ch.is_control() && !matches!(ch, '\n' | '\r' | '\t' | '\u{c}'))
    {
        return Err("此文件是二进制文件，暂不支持预览。");
    }
    let syntax = crate::syntax::detect_syntax_from_path(path);
    let markdown = matches!(
        syntax.as_str(),
        "md" | "markdown" | "mdown" | "mkd" | "mkdn"
    );
    let lines = text.bytes().filter(|&byte| byte == b'\n').count() + 1;
    Ok(TextFile {
        text,
        markdown,
        syntax,
        lines,
    })
}

fn file_error(error: std::io::Error) -> &'static str {
    match error.kind() {
        std::io::ErrorKind::NotFound => "文件不存在或已被移动。",
        std::io::ErrorKind::PermissionDenied => "没有权限读取此文件。",
        _ => "无法读取文件，请刷新重试。",
    }
}

pub(crate) fn local_file(target: &str, cwd: &Path) -> Option<PathBuf> {
    if target.is_empty()
        || target.len() > 4096
        || target.starts_with(['#', '?'])
        || target.starts_with("//")
        || target.starts_with("\\\\")
        || target.chars().any(char::is_control)
    {
        return None;
    }
    let target = target
        .rsplit_once(':')
        .filter(|(_, line)| !line.is_empty() && line.bytes().all(|byte| byte.is_ascii_digit()))
        .map_or(target, |(path, _)| path);
    let path = Path::new(target);
    if path.is_absolute() && !target.starts_with('/') {
        return Some(path.to_owned());
    }
    let url = if path.is_absolute() {
        Url::parse(&format!("file://{target}")).ok()?
    } else if let Ok(url) = Url::parse(target) {
        url
    } else {
        Url::from_directory_path(cwd).ok()?.join(target).ok()?
    };
    if url.scheme() == "file" && url.host_str().is_none() {
        return url.to_file_path().ok();
    }
    None
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) struct TestDirectory(pub(crate) PathBuf);

    impl TestDirectory {
        pub(crate) fn new() -> Self {
            let path =
                std::env::temp_dir().join(format!("naro-file-preview-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }

        pub(crate) fn file(&self, name: &str, bytes: impl AsRef<[u8]>) -> PathBuf {
            let path = self.0.join(name);
            std::fs::write(&path, bytes).unwrap();
            path
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn reads_markdown_code_and_extensionless_text_without_changing_contents() {
        let dir = TestDirectory::new();
        for name in [
            "notes.MD",
            "notes.markdown",
            "notes.mdown",
            "notes.mkd",
            "notes.mkdn",
        ] {
            let source = "\u{feff}# Naro\r\n\r\n**文件预览**\r\n";
            let path = dir.file(name, source);
            let file = read_file(&path).unwrap();
            assert!(file.markdown);
            assert_eq!(file.text, source);
            assert_eq!(std::fs::read_to_string(path).unwrap(), source);
        }
        for (name, syntax) in [("main.RS", "rs"), ("LICENSE", ""), ("empty.txt", "txt")] {
            let file = read_file(&dir.file(name, "")).unwrap();
            assert!(!file.markdown);
            assert_eq!(file.syntax, syntax);
            assert!(file.text.is_empty());
        }
    }

    #[test]
    fn rejects_missing_folders_binary_non_utf8_and_oversized_files() {
        let dir = TestDirectory::new();
        assert_eq!(
            read_file(&dir.0.join("missing.md")).unwrap_err(),
            "文件不存在或已被移动。"
        );
        assert_eq!(
            read_file(&dir.0).unwrap_err(),
            "这是文件夹，请使用“在文件夹中显示”。"
        );
        assert_eq!(
            read_file(&dir.file("binary", b"PNG\0data")).unwrap_err(),
            "此文件是二进制文件，暂不支持预览。"
        );
        assert_eq!(
            read_file(&dir.file("invalid.txt", [0xff, 0xfe])).unwrap_err(),
            "此文件不是 UTF-8 文本，暂不支持预览。"
        );
        let oversized = dir.file("huge.md", "");
        File::options()
            .write(true)
            .open(&oversized)
            .unwrap()
            .set_len(MAX_FILE_BYTES + 1)
            .unwrap();
        assert_eq!(
            read_file(&oversized).unwrap_err(),
            "文件超过 2 MiB，暂不支持在应用内预览。"
        );
        assert_eq!(
            file_error(std::io::ErrorKind::PermissionDenied.into()),
            "没有权限读取此文件。"
        );
    }

    #[cfg(unix)]
    #[test]
    fn rejects_special_files_without_reading_them() {
        assert_eq!(
            read_file(Path::new("/dev/null")).unwrap_err(),
            "仅支持预览普通文件。"
        );
    }
}
