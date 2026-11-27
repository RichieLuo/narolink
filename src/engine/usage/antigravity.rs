use super::{FileVersion, NANOS_PER_SECOND, Path, ReadUsage, Record, RecordId, TokenCounts};
use rusqlite::{Connection, OpenFlags};
use std::time::{Duration, Instant};

pub(super) fn read(path: &Path, version: &FileVersion, deadline: Instant) -> Result<ReadUsage, ()> {
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY;
    let db = match Connection::open_with_flags(path, flags).and_then(|db| {
        db.busy_timeout(Duration::from_millis(100))?;
        db.query_row("PRAGMA schema_version", [], |row| row.get::<_, u32>(0))?;
        Ok(db)
    }) {
        Ok(db) => db,
        // Closed WAL databases may have no sidecar files. Open them without creating those files.
        Err(_) if version.wal.is_none() => {
            let mut uri = url::Url::from_file_path(path).map_err(|_| ())?;
            uri.set_query(Some("mode=ro&immutable=1"));
            Connection::open_with_flags(uri.as_str(), flags | OpenFlags::SQLITE_OPEN_URI)
                .map_err(|_| ())?
        }
        Err(_) => return Err(()),
    };
    let mut query = db
        .prepare("SELECT metadata FROM steps WHERE metadata IS NOT NULL")
        .map_err(|_| ())?;
    let mut rows = query.query([]).map_err(|_| ())?;
    let mut usage = ReadUsage::default();
    while let Some(row) = rows.next().map_err(|_| ())? {
        if Instant::now() >= deadline {
            usage.partial = true;
            break;
        }
        let metadata = row.get_ref(0).map_err(|_| ())?.as_blob().map_err(|_| ())?;
        if let Some(step) = Message::decode(metadata) {
            read_step(&step, &mut usage);
        } else {
            usage.partial = true;
        }
    }
    Ok(usage)
}

fn read_step(step: &Message<'_>, usage: &mut ReadUsage) {
    let at = step
        .message(8)
        .and_then(timestamp)
        .or_else(|| step.message(1).and_then(timestamp));
    // The steps table carries request usage and a timestamp. The generation metadata table
    // can include duplicate copies of those requests.
    for bytes in step.bytes(9).into_iter().chain(
        step.all_bytes(28)
            .filter_map(|retry| Message::decode(retry)?.bytes(2)),
    ) {
        let Some(request) = Message::decode(bytes) else {
            usage.partial = true;
            continue;
        };
        let output = request
            .number(3)
            .max(request.number(9).saturating_add(request.number(10)));
        let counts = TokenCounts::from_uncached(
            request.number(2),
            output,
            request.number(5),
            request.number(4),
        );
        if counts.total() == 0 {
            continue;
        }
        let Some(at) = at else {
            usage.partial = true;
            continue;
        };
        let id = [11, 12, 7].into_iter().find_map(|field| {
            let bytes = request.bytes(field)?;
            let id = std::str::from_utf8(bytes).ok()?;
            (!id.is_empty() && id.len() <= 512).then(|| RecordId::Request(format!("{field}:{id}")))
        });
        usage.records.push(Record { at, counts, id });
    }
}

fn timestamp(message: Message<'_>) -> Option<i64> {
    let seconds = i64::try_from(message.number(1)).ok()?;
    let nanos = i64::try_from(message.number(2)).ok()?;
    if seconds == 0 || nanos >= NANOS_PER_SECOND {
        return None;
    }
    seconds.checked_mul(NANOS_PER_SECOND)?.checked_add(nanos)
}

enum Field<'a> {
    Number(u64),
    Bytes(&'a [u8]),
}

struct Message<'a>(Vec<(u64, Field<'a>)>);

impl<'a> Message<'a> {
    fn decode(mut bytes: &'a [u8]) -> Option<Self> {
        let mut fields = Vec::new();
        while !bytes.is_empty() {
            let tag = varint(&mut bytes)?;
            let number = tag >> 3;
            if number == 0 {
                return None;
            }
            let value = match tag & 7 {
                0 => Field::Number(varint(&mut bytes)?),
                2 => {
                    let size = usize::try_from(varint(&mut bytes)?).ok()?;
                    let (field, tail) = bytes.split_at_checked(size)?;
                    bytes = tail;
                    Field::Bytes(field)
                }
                1 | 5 => {
                    let size = if tag & 7 == 1 { 8 } else { 4 };
                    bytes = bytes.get(size..)?;
                    continue;
                }
                _ => return None,
            };
            fields.push((number, value));
        }
        Some(Self(fields))
    }

    fn number(&self, number: u64) -> u64 {
        self.0
            .iter()
            .find_map(|(key, field)| match field {
                Field::Number(n) if *key == number => Some(*n),
                _ => None,
            })
            .unwrap_or_default()
    }

    fn all_bytes(&self, number: u64) -> impl Iterator<Item = &'a [u8]> + '_ {
        self.0.iter().filter_map(move |(key, field)| match field {
            Field::Bytes(value) if *key == number => Some(*value),
            _ => None,
        })
    }

    fn bytes(&self, number: u64) -> Option<&'a [u8]> {
        self.all_bytes(number).next()
    }

    fn message(&self, number: u64) -> Option<Self> {
        Self::decode(self.bytes(number)?)
    }
}

fn varint(bytes: &mut &[u8]) -> Option<u64> {
    let mut value = 0;
    for shift in (0..64).step_by(7) {
        let (&byte, tail) = bytes.split_first()?;
        *bytes = tail;
        if shift == 63 && byte > 1 {
            return None;
        }
        value |= u64::from(byte & 127) << shift;
        if byte < 128 {
            return Some(value);
        }
    }
    None
}
