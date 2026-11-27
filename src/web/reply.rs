use crate::engine::attachments::{
    HistoryImage, ImageAttachment, InlineImage, MAX_ENCODED_IMAGE_BYTES, MAX_IMAGE_SOURCE_BYTES,
    UserAttachment,
};
use crate::file_preview::{MAX_MARKDOWN_BYTES, local_file, read_file};
use pulldown_cmark::{Event, LinkType, Options, Parser, Tag, TagEnd, html};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashMap},
    path::{Path, PathBuf},
    sync::Mutex,
    time::Instant,
};
use url::Url;

#[derive(Serialize)]
pub(super) struct Rendered {
    html: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    previews: Vec<Preview>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    sections: Vec<RenderedSection>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    work_steps: BTreeMap<String, String>,
}

#[derive(Serialize)]
struct RenderedSection {
    offset: usize,
    rendered: Rendered,
}

#[derive(Clone, Serialize)]
struct Preview {
    url: String,
    name: String,
}

#[derive(Serialize)]
pub(super) struct AttachmentPreview {
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    url: Option<String>,
}

#[derive(Serialize)]
pub(super) struct FilePreview {
    name: String,
    text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    html: Option<String>,
}

#[derive(Clone)]
enum PreviewSource {
    File { path: PathBuf, root: PathBuf },
    Text { path: PathBuf, root: PathBuf },
    Inline(InlineImage),
}

impl PreviewSource {
    fn encoded_len(&self) -> usize {
        match self {
            Self::File { .. } | Self::Text { .. } => 0,
            Self::Inline(image) => image.encoded_len(),
        }
    }
}

struct PreviewEntry {
    source: PreviewSource,
    touched: Instant,
}

#[derive(Default)]
struct PreviewCache {
    entries: HashMap<String, PreviewEntry>,
    encoded_bytes: usize,
}

impl PreviewCache {
    fn source(&mut self, key: &str) -> Option<PreviewSource> {
        let entry = self.entries.get_mut(key)?;
        entry.touched = Instant::now();
        Some(entry.source.clone())
    }
}

#[derive(Default)]
pub(super) struct Previews {
    nonce: uuid::Uuid,
    cache: Mutex<PreviewCache>,
    documents: Mutex<PreviewCache>,
}

impl Previews {
    pub fn new() -> Self {
        Self {
            nonce: uuid::Uuid::new_v4(),
            ..Self::default()
        }
    }

    pub(super) fn revision(&self, revision: Option<String>) -> Option<String> {
        revision.map(|revision| format!("{}-{revision}", self.nonce.simple()))
    }

    pub(super) fn for_workspace(&self, cwd: &Path) -> ReplyRenderer<'_> {
        ReplyRenderer {
            store: self,
            workspace: cwd.canonicalize().ok(),
            cwd: cwd.to_owned(),
        }
    }

    fn register(&self, root: &Path, path: PathBuf) -> Option<Preview> {
        let name = path.file_name()?.to_string_lossy().into_owned();
        let url = self.insert(
            self.file_key(root, &path),
            PreviewSource::File {
                path,
                root: root.to_owned(),
            },
        );
        Some(Preview { url, name })
    }

    fn file_key(&self, root: &Path, path: &Path) -> String {
        let mut hash = Sha256::new();
        hash.update(self.nonce.as_bytes());
        hash.update(root.as_os_str().as_encoded_bytes());
        hash.update([0]);
        hash.update(path.as_os_str().as_encoded_bytes());
        format!("{:x}", hash.finalize())
    }

    fn insert(&self, key: String, source: PreviewSource) -> String {
        let now = Instant::now();
        let (cache, max_entries, route) = if matches!(&source, PreviewSource::Text { .. }) {
            (&self.documents, 2048, "/api/file-previews")
        } else {
            (&self.cache, 512, "/api/previews")
        };
        let mut cache = cache.lock().unwrap();
        if let Some(entry) = cache.entries.get_mut(&key) {
            entry.touched = now;
        } else {
            let size = source.encoded_len();
            while cache.entries.len() >= max_entries
                || cache.encoded_bytes + size > 2 * MAX_ENCODED_IMAGE_BYTES
            {
                let Some(oldest) = cache
                    .entries
                    .iter()
                    .filter(|(_, entry)| {
                        cache.entries.len() >= max_entries || entry.source.encoded_len() > 0
                    })
                    .min_by_key(|(_, entry)| entry.touched)
                    .map(|(key, _)| key.clone())
                else {
                    break;
                };
                cache.encoded_bytes -= cache.entries.remove(&oldest).unwrap().source.encoded_len();
            }
            cache.encoded_bytes += size;
            cache.entries.insert(
                key.clone(),
                PreviewEntry {
                    source,
                    touched: now,
                },
            );
        }
        format!("{route}/{key}")
    }

    pub(super) fn user_attachment(&self, attachment: &UserAttachment) -> AttachmentPreview {
        let url = attachment.image.as_ref().and_then(|image| match image {
            HistoryImage::Local(path) => {
                // An explicit Codex input may be outside the project, such as a photo sent from a phone.
                if !is_image_file(path) || !path.symlink_metadata().is_ok_and(|meta| meta.is_file())
                {
                    return None;
                }
                let path = path.canonicalize().ok()?;
                let parent = path.parent()?.to_owned();
                self.register(&parent, path).map(|preview| preview.url)
            }
            HistoryImage::Inline(image) => Some(self.insert(
                format!("inline-{}", image.id().simple()),
                PreviewSource::Inline(image.clone()),
            )),
            HistoryImage::Remote(_) => None,
        });
        AttachmentPreview {
            name: attachment.name.clone(),
            url,
        }
    }

    pub(super) fn image(&self, key: &str) -> Option<ImageAttachment> {
        let source = self.cache.lock().unwrap().source(key)?;
        match source {
            PreviewSource::File { path, root } => {
                let resolved = path.canonicalize().ok()?;
                if resolved != path || !resolved.starts_with(&root) || !is_image_file(&resolved) {
                    return None;
                }
                ImageAttachment::from_file(resolved).ok()
            }
            PreviewSource::Inline(image) => image.decode().ok(),
            PreviewSource::Text { .. } => None,
        }
    }

    pub(super) fn file(&self, key: &str) -> Option<Result<FilePreview, &'static str>> {
        let PreviewSource::Text { path, root } = self.documents.lock().unwrap().source(key)? else {
            return None;
        };
        // A path registered for a reply must still refer to a file in the same workspace.
        if !path.starts_with(&root) || path.canonicalize().ok().as_ref() != Some(&path) {
            return None;
        }
        let name = path.file_name()?.to_string_lossy().into_owned();
        let file = read_file(&path).map(|file| {
            let html = (file.markdown && file.text.len() <= MAX_MARKDOWN_BYTES).then(|| {
                ReplyRenderer {
                    store: self,
                    workspace: Some(root),
                    cwd: path.parent().unwrap_or(&path).to_owned(),
                }
                .render(file.text.trim_start_matches('\u{feff}'), &[])
                .html
            });
            FilePreview {
                name,
                text: file.text,
                html,
            }
        });
        Some(file)
    }
}

pub(super) struct ReplyRenderer<'a> {
    store: &'a Previews,
    workspace: Option<PathBuf>,
    cwd: PathBuf,
}

impl ReplyRenderer<'_> {
    pub(super) fn render_reply(&self, text: &str, work: &crate::engine::work::Work) -> Rendered {
        let mut rendered = self.render(text, &work.images);
        rendered.work_steps = work
            .steps
            .iter()
            .filter(|step| step.kind.is_thinking() && !step.text.trim().is_empty())
            .map(|step| (step.id.clone(), self.render(&step.text, &[]).html))
            .collect();
        if work
            .steps
            .iter()
            .any(|step| step.text_offset.is_some_and(|offset| offset > 0))
        {
            rendered.sections = work
                .reply_parts(text)
                .into_iter()
                .filter_map(|part| {
                    if let crate::engine::work::ReplyPart::Text { offset, text } = part {
                        Some(RenderedSection {
                            offset,
                            rendered: self.render(text, &[]),
                        })
                    } else {
                        None
                    }
                })
                .collect();
        }
        rendered
    }

    pub(super) fn render(&self, text: &str, generated: &[PathBuf]) -> Rendered {
        let mut rendered = Rendered {
            html: String::new(),
            previews: Vec::new(),
            sections: Vec::new(),
            work_steps: BTreeMap::new(),
        };
        let options =
            Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
        let mut links = Vec::new();
        let events = Parser::new_ext(text, options).filter_map(|event| match event {
            Event::Html(raw) | Event::InlineHtml(raw) => Some(Event::Text(raw)),
            Event::Start(Tag::Link {
                dest_url,
                title,
                id,
                link_type,
            }) => {
                let href = self.link(&dest_url, &mut rendered.previews, true);
                links.push(href.is_some());
                href.map(|href| {
                    Event::Start(Tag::Link {
                        dest_url: href.into(),
                        title,
                        id,
                        link_type,
                    })
                })
            }
            Event::Start(Tag::Image {
                dest_url,
                title,
                id,
                ..
            }) => {
                let href = self
                    .link(&dest_url, &mut rendered.previews, false)
                    .filter(|_| !links.contains(&true));
                links.push(href.is_some());
                href.map(|href| {
                    Event::Start(Tag::Link {
                        dest_url: href.into(),
                        title,
                        id,
                        link_type: LinkType::Inline,
                    })
                })
            }
            Event::End(TagEnd::Link | TagEnd::Image) => links
                .pop()
                .unwrap_or(false)
                .then_some(Event::End(TagEnd::Link)),
            event => Some(event),
        });
        html::push_html(&mut rendered.html, events);
        for path in generated {
            if let Some(preview) = self.local_image(path)
                && rendered.previews.len() < 8
            {
                add_preview(&mut rendered.previews, preview);
            }
        }
        rendered
    }

    fn link(&self, target: &str, previews: &mut Vec<Preview>, allow_text: bool) -> Option<String> {
        if target.len() > 4096 {
            return None;
        }
        if let Ok(url) = Url::parse(target) {
            if matches!(url.scheme(), "http" | "https")
                && url.has_host()
                && url.username().is_empty()
                && url.password().is_none()
            {
                return Some(url.into());
            }
        }
        self.image_link(target, previews).or_else(|| {
            if !allow_text {
                return None;
            }
            let path = local_file(target, &self.cwd)?.canonicalize().ok()?;
            let root = self.workspace.as_deref()?;
            if !path.starts_with(root) || supported_image(&path) || !path.is_file() {
                return None;
            }
            Some(self.store.insert(
                self.store.file_key(root, &path),
                PreviewSource::Text {
                    path,
                    root: root.to_owned(),
                },
            ))
        })
    }

    fn image_link(&self, target: &str, previews: &mut Vec<Preview>) -> Option<String> {
        if let Ok(url) = Url::parse(target) {
            if url.scheme() == "file" {
                if url.host_str().is_some() {
                    return None;
                }
                let path = url.to_file_path().ok()?;
                return self.preview_link(&path, previews);
            }
            if !Path::new(target).is_absolute() {
                return None;
            }
        }
        self.preview_link(&self.cwd.join(target), previews)
            .or_else(|| {
                // Markdown links may percent-encode spaces and non-ASCII path components.
                if !target.contains('%') || target.starts_with("//") {
                    return None;
                }
                let path = Url::from_directory_path(&self.cwd)
                    .ok()?
                    .join(target)
                    .ok()?
                    .to_file_path()
                    .ok()?;
                self.preview_link(&path, previews)
            })
    }

    fn preview_link(&self, path: &Path, previews: &mut Vec<Preview>) -> Option<String> {
        if previews.len() >= 8 {
            return None;
        }
        let preview = self.local_image(path)?;
        let url = preview.url.clone();
        add_preview(previews, preview);
        Some(url)
    }

    fn local_image(&self, path: &Path) -> Option<Preview> {
        if !supported_image(path) {
            return None;
        }
        let path = path.canonicalize().ok()?;
        if !is_image_file(&path) {
            return None;
        }
        if let Some(root) = self
            .workspace
            .as_deref()
            .filter(|root| path.starts_with(root))
        {
            return self.store.register(root, path);
        }
        let root = std::env::var_os("CODEX_HOME")
            .map(PathBuf::from)
            .or_else(|| directories::BaseDirs::new().map(|dirs| dirs.home_dir().join(".codex")))?
            .join("generated_images")
            .canonicalize()
            .ok()?;
        if path.starts_with(&root) {
            return self.store.register(&root, path);
        }
        None
    }
}

fn add_preview(previews: &mut Vec<Preview>, preview: Preview) {
    if !previews.iter().any(|existing| existing.url == preview.url) {
        previews.push(preview);
    }
}

fn supported_image(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|ext| ext.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("png" | "jpg" | "jpeg" | "gif" | "webp")
    )
}

fn is_image_file(path: &Path) -> bool {
    supported_image(path)
        && path
            .metadata()
            .is_ok_and(|meta| meta.is_file() && meta.len() <= MAX_IMAGE_SOURCE_BYTES as u64)
}
