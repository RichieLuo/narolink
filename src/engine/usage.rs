mod antigravity;
#[cfg(test)]
mod tests;

use super::Provider;
use memchr::memmem;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    cmp::Reverse,
    collections::{HashMap, HashSet},
    fs::File,
    io::{self, BufRead, BufReader, Read},
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime},
};

const NANOS_PER_SECOND: i64 = 1_000_000_000;
const DAY: i64 = 86_400 * NANOS_PER_SECOND;
const MAX_LINE: u64 = 8 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
pub enum UsagePeriod {
    #[default]
    #[serde(rename = "24h")]
    Day,
    #[serde(rename = "7d")]
    Week,
    #[serde(rename = "30d")]
    Month,
}

impl UsagePeriod {
    pub const ALL: [Self; 3] = [Self::Day, Self::Week, Self::Month];

    const fn length(self) -> i64 {
        match self {
            Self::Day => DAY,
            Self::Week => 7 * DAY,
            Self::Month => 30 * DAY,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Day => "最近 24 小时",
            Self::Week => "最近 7 天",
            Self::Month => "最近 30 天",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub struct TokenCounts {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
}

impl TokenCounts {
    pub fn total(self) -> u64 {
        self.input_tokens.saturating_add(self.output_tokens)
    }

    fn add(&mut self, other: Self) {
        self.input_tokens = self.input_tokens.saturating_add(other.input_tokens);
        self.output_tokens = self.output_tokens.saturating_add(other.output_tokens);
        self.cache_read_tokens = self
            .cache_read_tokens
            .saturating_add(other.cache_read_tokens);
        self.cache_write_tokens = self
            .cache_write_tokens
            .saturating_add(other.cache_write_tokens);
    }

    fn merge_snapshot(&mut self, other: Self) {
        self.input_tokens = self.input_tokens.max(other.input_tokens);
        self.output_tokens = self.output_tokens.max(other.output_tokens);
        self.cache_read_tokens = self.cache_read_tokens.max(other.cache_read_tokens);
        self.cache_write_tokens = self.cache_write_tokens.max(other.cache_write_tokens);
    }

    fn from_uncached(input: u64, output: u64, read: u64, write: u64) -> Self {
        Self {
            input_tokens: input.saturating_add(read).saturating_add(write),
            output_tokens: output,
            cache_read_tokens: read,
            cache_write_tokens: write,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct UsageReport {
    pub windows: [UsageWindow; 3],
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct UsageWindow {
    pub period: UsagePeriod,
    pub total: Option<TokenCounts>,
    pub providers: [ProviderUsage; 3],
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ProviderUsage {
    pub provider: Provider,
    pub counts: Option<TokenCounts>,
    pub partial: bool,
}

#[derive(Clone, Eq, Hash, PartialEq)]
enum RecordId {
    Request(String),
    Snapshot(i64, TokenCounts),
}

struct Record {
    at: i64,
    counts: TokenCounts,
    id: Option<RecordId>,
}

#[derive(Default)]
struct ReadUsage {
    records: Vec<Record>,
    partial: bool,
}

#[derive(PartialEq, Eq)]
struct FileVersion {
    modified: SystemTime,
    size: u64,
    wal: Option<(SystemTime, u64)>,
}

impl FileVersion {
    fn read(path: &Path, provider: Provider) -> io::Result<Self> {
        let metadata = path.metadata()?;
        let wal = if provider == Provider::Antigravity {
            match path.with_extension("db-wal").metadata() {
                Ok(wal) => Some((wal.modified()?, wal.len())),
                Err(error) if error.kind() == io::ErrorKind::NotFound => None,
                Err(error) => return Err(error),
            }
        } else {
            None
        };
        Ok(Self {
            modified: metadata.modified()?,
            size: metadata.len(),
            wal,
        })
    }

    fn modified(&self) -> i64 {
        self.wal
            .map_or(self.modified, |(time, _)| self.modified.max(time))
            .duration_since(SystemTime::UNIX_EPOCH)
            .ok()
            .and_then(|time| i64::try_from(time.as_nanos()).ok())
            .unwrap_or_default()
    }
}

struct CachedFile {
    version: FileVersion,
    usage: ReadUsage,
}

#[derive(Default)]
pub(crate) struct UsageCache {
    files: HashMap<PathBuf, CachedFile>,
}

impl UsageCache {
    pub fn report(&mut self) -> UsageReport {
        let now = chrono::DateTime::<chrono::Utc>::from(SystemTime::now())
            .timestamp_nanos_opt()
            .unwrap_or_default();
        self.collect(now, Provider::ALL.map(roots))
    }

    fn collect(&mut self, now: i64, roots: [Vec<PathBuf>; 3]) -> UsageReport {
        let mut visited = HashSet::new();
        let scans: [Totals; 3] = std::array::from_fn(|index| {
            self.scan(Provider::ALL[index], &roots[index], now, &mut visited)
        });
        self.files.retain(|path, _| visited.contains(path));
        UsageReport {
            windows: std::array::from_fn(|index| {
                let providers = std::array::from_fn(|i| ProviderUsage {
                    provider: Provider::ALL[i],
                    counts: scans[i].windows[index],
                    partial: scans[i].partial,
                });
                let mut total = None;
                for counts in providers.iter().filter_map(|provider| provider.counts) {
                    total.get_or_insert_with(TokenCounts::default).add(counts);
                }
                UsageWindow {
                    period: UsagePeriod::ALL[index],
                    total,
                    providers,
                }
            }),
        }
    }

    fn scan(
        &mut self,
        provider: Provider,
        roots: &[PathBuf],
        now: i64,
        visited: &mut HashSet<PathBuf>,
    ) -> Totals {
        // Keep all three scans within the relay's request timeout. Codex rollouts
        // tend to be larger because they also contain tool output.
        let budget = if provider == Provider::Codex { 12 } else { 6 };
        let deadline = Instant::now() + Duration::from_secs(budget);
        let since = now.saturating_sub(UsagePeriod::Month.length());
        let mut scan = Discover {
            provider,
            since,
            deadline,
            files: Vec::new(),
            partial: false,
        };
        for root in roots {
            scan.walk(root, 4);
        }
        scan.files
            .sort_by_key(|(_, version)| Reverse(version.modified()));
        let mut result = Totals::default();
        let mut unique = HashMap::<RecordId, (i64, TokenCounts)>::new();
        for (path, version) in scan.files {
            if !visited.insert(path.clone()) {
                continue;
            }
            if !self
                .files
                .get(&path)
                .is_some_and(|cached| cached.version == version)
            {
                let usage = match provider {
                    Provider::Antigravity => antigravity::read(&path, &version, deadline),
                    _ => read_jsonl(&path, provider, deadline),
                };
                let Ok(usage) = usage else {
                    scan.partial = true;
                    continue;
                };
                if !usage.partial {
                    self.files
                        .insert(path.clone(), CachedFile { version, usage });
                } else {
                    scan.partial = true;
                    result.include(usage.records.iter(), now, &mut unique);
                    continue;
                }
            }
            result.include(self.files[&path].usage.records.iter(), now, &mut unique);
        }
        for (at, counts) in unique.into_values() {
            result.record(now, at, counts);
        }
        result.partial = scan.partial;
        result
    }
}

fn roots(provider: Provider) -> Vec<PathBuf> {
    let home = super::current_workspace();
    match provider {
        Provider::Codex => {
            let codex = std::env::var_os("CODEX_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join(".codex"));
            vec![codex.join("sessions"), codex.join("archived_sessions")]
        }
        Provider::Claude => super::claude::projects().into_iter().collect(),
        Provider::Antigravity => std::env::var_os("ANTIGRAVITY_DATA_DIR")
            .filter(|path| !path.is_empty())
            .map(|path| vec![PathBuf::from(path).join("conversations")])
            .unwrap_or_else(|| {
                ["antigravity-cli", "antigravity"]
                    .map(|store| home.join(".gemini").join(store).join("conversations"))
                    .into()
            }),
    }
}

struct Discover {
    provider: Provider,
    since: i64,
    deadline: Instant,
    files: Vec<(PathBuf, FileVersion)>,
    partial: bool,
}

impl Discover {
    fn walk(&mut self, directory: &Path, depth: u8) {
        let entries = match directory.read_dir() {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return,
            Err(_) => {
                self.partial = true;
                return;
            }
        };
        for entry in entries {
            if Instant::now() >= self.deadline {
                self.partial = true;
                return;
            }
            let Ok(entry) = entry else {
                self.partial = true;
                continue;
            };
            let Ok(kind) = entry.file_type() else {
                self.partial = true;
                continue;
            };
            let path = entry.path();
            if kind.is_dir() && depth > 0 {
                self.walk(&path, depth - 1);
            }
            let extension = path.extension().and_then(|extension| extension.to_str());
            let accepted = match self.provider {
                Provider::Codex => {
                    extension == Some("jsonl")
                        && entry.file_name().to_string_lossy().starts_with("rollout-")
                }
                Provider::Claude => extension == Some("jsonl"),
                Provider::Antigravity => matches!(extension, Some("db" | "pb")),
            };
            if kind.is_file() && accepted {
                match FileVersion::read(&path, self.provider) {
                    Ok(version) if version.modified() >= self.since => {
                        if extension == Some("pb") {
                            self.partial = true;
                        } else {
                            self.files.push((path, version));
                        }
                    }
                    Ok(_) => {}
                    Err(_) => self.partial = true,
                }
            }
        }
    }
}

#[derive(Default)]
struct Totals {
    windows: [Option<TokenCounts>; 3],
    partial: bool,
}

impl Totals {
    fn record(&mut self, now: i64, at: i64, counts: TokenCounts) {
        for (index, period) in UsagePeriod::ALL.into_iter().enumerate() {
            if (now.saturating_sub(period.length())..=now).contains(&at) {
                self.windows[index]
                    .get_or_insert_with(TokenCounts::default)
                    .add(counts);
            }
        }
    }

    fn include<'a>(
        &mut self,
        records: impl Iterator<Item = &'a Record>,
        now: i64,
        unique: &mut HashMap<RecordId, (i64, TokenCounts)>,
    ) {
        for record in records {
            if let Some(id) = &record.id {
                let (at, counts) = unique
                    .entry(id.clone())
                    .or_insert((record.at, record.counts));
                *at = (*at).min(record.at);
                counts.merge_snapshot(record.counts);
            } else {
                self.record(now, record.at, record.counts);
            }
        }
    }
}

fn read_jsonl(path: &Path, provider: Provider, deadline: Instant) -> Result<ReadUsage, ()> {
    let marker = memmem::Finder::new(match provider {
        Provider::Codex => b"\"token_",
        _ => b"\"usage\"",
    });
    let mut reader = BufReader::with_capacity(128 * 1024, File::open(path).map_err(|_| ())?);
    let mut line = Vec::new();
    let mut usage = ReadUsage::default();
    let mut codex = CodexReader::default();
    loop {
        if Instant::now() >= deadline {
            usage.partial = true;
            break;
        }
        line.clear();
        let read = (&mut reader)
            .take(MAX_LINE)
            .read_until(b'\n', &mut line)
            .map_err(|_| ())?;
        if read == 0 {
            break;
        }
        let relevant = marker.find_iter(&line).any(|offset| {
            provider != Provider::Codex
                || line[offset..].starts_with(b"\"token_count\"")
                || line[offset..].starts_with(b"\"token_usage_record\"")
        });
        if read as u64 == MAX_LINE && !line.ends_with(b"\n") {
            usage.partial |= relevant;
            reader.skip_until(b'\n').map_err(|_| ())?;
            continue;
        }
        if relevant && let Ok(row) = serde_json::from_slice::<Value>(&line) {
            let record = match provider {
                Provider::Codex => codex.record(&row),
                _ => claude_record(&row),
            };
            usage.records.extend(record);
        }
    }
    Ok(usage)
}

fn timestamp(row: &Value) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(row["timestamp"].as_str()?)
        .ok()?
        .timestamp_nanos_opt()
}

fn request_id(value: &Value) -> Option<RecordId> {
    let id = value
        .as_str()
        .filter(|id| !id.is_empty() && id.len() <= 512)?;
    Some(RecordId::Request(id.to_owned()))
}

fn number(value: &Value, field: &str) -> u64 {
    value[field].as_u64().unwrap_or_default()
}

#[derive(Default)]
struct CodexReader {
    previous_total: Option<TokenCounts>,
    request_since_snapshot: bool,
}

impl CodexReader {
    fn record(&mut self, row: &Value) -> Option<Record> {
        let payload = &row["payload"];
        let at = timestamp(row)?;
        let (counts, id) = match row["type"].as_str()? {
            "token_usage_record" => {
                let counts = codex_counts(&payload["usage"])?;
                self.request_since_snapshot = true;
                (counts, request_id(&payload["response_id"]))
            }
            "event_msg" if payload["type"] == "token_count" && payload["info"].is_object() => {
                let info = &payload["info"];
                let total = codex_counts(&info["total_token_usage"]);
                let unchanged = total.is_some() && total == self.previous_total;
                self.previous_total = total;
                // Current rollouts emit a request record followed by a cumulative snapshot.
                if std::mem::take(&mut self.request_since_snapshot) || unchanged {
                    return None;
                }
                (codex_counts(&info["last_token_usage"])?, None)
            }
            _ => return None,
        };
        (counts.total() > 0).then_some(Record {
            at,
            counts,
            // Rollout forks can copy the same accounting event into a second file.
            id: id.or(Some(RecordId::Snapshot(at, counts))),
        })
    }
}

fn codex_counts(usage: &Value) -> Option<TokenCounts> {
    Some(TokenCounts {
        input_tokens: usage["input_tokens"].as_u64()?,
        output_tokens: usage["output_tokens"].as_u64()?,
        cache_read_tokens: number(usage, "cached_input_tokens"),
        cache_write_tokens: number(usage, "cache_write_input_tokens"),
    })
}

fn claude_record(row: &Value) -> Option<Record> {
    if row["type"] != "assistant" || row["message"]["model"] == "<synthetic>" {
        return None;
    }
    let message = &row["message"];
    let usage = &message["usage"];
    let counts = TokenCounts::from_uncached(
        number(usage, "input_tokens"),
        number(usage, "output_tokens"),
        number(usage, "cache_read_input_tokens"),
        number(usage, "cache_creation_input_tokens"),
    );
    if counts.total() == 0 {
        return None;
    }
    Some(Record {
        at: timestamp(row)?,
        counts,
        id: request_id(&message["id"])
            .or_else(|| request_id(&row["requestId"]))
            .or_else(|| request_id(&row["uuid"])),
    })
}
