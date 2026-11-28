use super::{ApiError, Web, live_snapshot};
use crate::service::{Run, RunRevision, Status};
use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode, header},
    response::{
        IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
};
use futures_util::stream::unfold;
use serde::Serialize;
use std::{sync::Arc, time::Duration};
use tokio::sync::watch;

struct RunStream {
    web: Web,
    run: Arc<Run>,
    changes: watch::Receiver<RunRevision>,
    previous: Option<RunRevision>,
    text: String,
    finished: bool,
}

#[derive(Serialize)]
struct Delta<'a> {
    after: Option<&'a str>,
    text: &'a str,
    activity: &'a str,
}

pub(super) async fn events(
    State(web): State<Web>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    if !headers
        .get(header::ACCEPT)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| {
            value
                .split(',')
                .any(|kind| kind.trim() == "text/event-stream")
        })
    {
        return Err(ApiError(
            StatusCode::NOT_ACCEPTABLE,
            "需要事件流请求".into(),
        ));
    }
    let run = web.service.run(&id).map_err(ApiError::missing)?;
    let changes = run.subscribe();
    let stream = unfold(
        RunStream {
            web,
            run,
            changes,
            previous: None,
            text: String::new(),
            finished: false,
        },
        |mut stream| async move {
            let event = stream.next().await?;
            Some((event, stream))
        },
    );
    let response = Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(5)));
    Ok(([("x-accel-buffering", "no")], response).into_response())
}

impl RunStream {
    async fn next(&mut self) -> Option<Result<Event, axum::Error>> {
        if self.finished {
            return None;
        }
        if let Some(previous) = self.previous {
            let unchanged = *self.changes.borrow_and_update() == previous;
            if unchanged {
                self.changes.changed().await.ok()?;
            }
            // Group fragments that arrive within the same short burst.
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        let (revision, snapshot) = self.run.stream_snapshot();
        self.finished = snapshot.status != Status::Running;
        let replies = snapshot.replies();
        let tail = replies.last()?;
        let delta = if self
            .previous
            .is_some_and(|previous| previous.structure == revision.structure)
            && let Some(text) = tail.text.strip_prefix(&self.text)
        {
            Some(Event::default().event("delta").json_data(Delta {
                after: tail.after,
                text,
                activity: &snapshot.activity,
            }))
        } else {
            None
        };
        self.previous = Some(revision);
        self.text = tail.text.to_string();
        if let Some(delta) = delta {
            return Some(delta);
        }
        let web = self.web.clone();
        let run = self.run.clone();
        tokio::task::spawn_blocking(move || {
            Event::default()
                .event("snapshot")
                .json_data(live_snapshot(&web, run.cwd(), snapshot))
        })
        .await
        .ok()
    }
}
