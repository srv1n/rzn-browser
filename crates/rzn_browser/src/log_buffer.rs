use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use tracing::{Event, Subscriber};
use tracing_subscriber::{layer::Context, Layer};
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEntry {
    pub ts: u64,
    pub level: String,
    pub component: String,
    pub run_id: Option<String>,
    pub message: String,
}

impl<S> Layer<S> for LogBuffer
where
    S: Subscriber,
{
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        #[derive(Default)]
        struct Visitor {
            message: String,
            run_id: Option<String>,
        }
        impl tracing::field::Visit for Visitor {
            fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
                let value = format!("{value:?}");
                if field.name() == "message" {
                    self.message = value.trim_matches('"').to_string();
                }
                if field.name() == "run_id" {
                    self.run_id = Some(value.trim_matches('"').to_string());
                }
            }
        }
        let mut visitor = Visitor::default();
        event.record(&mut visitor);
        let meta = event.metadata();
        self.push(LogEntry {
            ts: chrono::Utc::now().timestamp_millis().max(0) as u64,
            level: meta.level().to_string().to_lowercase(),
            component: meta.target().to_string(),
            run_id: visitor.run_id,
            message: visitor.message,
        });
    }
}
#[derive(Clone)]
pub struct LogBuffer {
    inner: Arc<Mutex<VecDeque<LogEntry>>>,
    capacity: usize,
}
impl LogBuffer {
    pub fn new(capacity: usize) -> Self {
        Self {
            inner: Arc::new(Mutex::new(VecDeque::new())),
            capacity,
        }
    }
    pub fn push(&self, e: LogEntry) {
        let mut b = self.inner.lock().unwrap();
        b.push_back(e);
        while b.len() > self.capacity {
            b.pop_front();
        }
    }
    pub fn tail(
        &self,
        limit: usize,
        level: Option<&str>,
        component: Option<&str>,
        run_id: Option<&str>,
    ) -> Vec<LogEntry> {
        self.inner
            .lock()
            .unwrap()
            .iter()
            .rev()
            .filter(|e| {
                level.map_or(true, |x| e.level.eq_ignore_ascii_case(x))
                    && component.map_or(true, |x| e.component == x)
                    && run_id.map_or(true, |x| e.run_id.as_deref() == Some(x))
            })
            .take(limit.min(500))
            .cloned()
            .collect()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use tracing_subscriber::prelude::*;
    #[test]
    fn bounded_filtered() {
        let b = LogBuffer::new(2);
        for i in 0..3 {
            b.push(LogEntry {
                ts: i,
                level: if i == 2 { "error" } else { "info" }.into(),
                component: "runner".into(),
                run_id: Some("r".into()),
                message: i.to_string(),
            });
        }
        assert_eq!(b.tail(500, None, None, None).len(), 2);
        assert_eq!(b.tail(10, Some("error"), None, Some("r")).len(), 1);
    }
    #[test]
    fn tracing_layer_feeds_buffer() {
        let buffer = LogBuffer::new(10);
        tracing::subscriber::with_default(
            tracing_subscriber::registry().with(buffer.clone()),
            || {
                tracing::warn!(run_id = "run-7", "captured message");
            },
        );
        let rows = buffer.tail(10, Some("warn"), None, Some("run-7"));
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].message, "captured message");
    }
}
