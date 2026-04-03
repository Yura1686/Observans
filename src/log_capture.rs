use observans_core::SharedLogBuffer;
use tracing::field::{Field, Visit};
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::Context;
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::Layer;

#[derive(Clone)]
pub struct UiLogLayer {
    logs: SharedLogBuffer,
}

impl UiLogLayer {
    pub fn new(logs: SharedLogBuffer) -> Self {
        Self { logs }
    }
}

impl<S> Layer<S> for UiLogLayer
where
    S: Subscriber + for<'span> LookupSpan<'span>,
{
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let mut visitor = EventVisitor::default();
        event.record(&mut visitor);

        let message = visitor
            .finish()
            .unwrap_or_else(|| event.metadata().name().to_string());
        self.logs
            .push_tracing(event.metadata().level(), event.metadata().target(), message);
    }
}

#[derive(Default)]
struct EventVisitor {
    message: Option<String>,
    fields: Vec<String>,
}

impl EventVisitor {
    fn finish(self) -> Option<String> {
        match (self.message, self.fields.is_empty()) {
            (Some(message), true) => Some(message),
            (Some(message), false) => Some(format!("{message} | {}", self.fields.join("  "))),
            (None, false) => Some(self.fields.join("  ")),
            (None, true) => None,
        }
    }
}

impl Visit for EventVisitor {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.message = Some(value.to_string());
        } else {
            self.fields.push(format!("{}={value}", field.name()));
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.message = Some(format!("{value:?}").trim_matches('"').to_string());
        } else {
            self.fields.push(format!("{}={value:?}", field.name()));
        }
    }
}
