use crate::visitor::{StackdriverEventVisitor, StackdriverVisitor};
use serde::ser::{SerializeMap, Serializer as _};
use serde_json::Value;
use std::{
    fmt::{self, Write},
    io,
};
use tracing_core::{span::{Attributes, Id}, Event, Subscriber};
use tracing_serde::AsSerde;
use tracing_subscriber::{
    field::{MakeVisitor, VisitOutput},
    fmt::{time::UtcTime, FormatFields, FormattedFields, MakeWriter},
    layer::Context,
    registry::LookupSpan,
    Layer,
};
use time::format_description::well_known;

/// A tracing adapter for stackdriver
pub struct Stackdriver<W = fn() -> io::Stdout>
{
    time: UtcTime<well_known::Rfc3339>,
    make_writer: W,
    fields: StackdriverFields,
    log_span: bool,
}

impl Stackdriver {
    /// Initialize the Stackdriver Layer with the default writer (std::io::Stdout)
    pub fn new() -> Self {
        Self::default()
    }
}

impl<W> Stackdriver<W> {
    /// Initialize the Stackdriver Layer with a custom writer
    pub fn with_writer<W2>(self, make_writer: W2) -> Stackdriver<W2>
    where
        W2: for<'writer> MakeWriter<'writer> + 'static,
    {
        Stackdriver {
            time: UtcTime::rfc_3339(),
            make_writer,
            fields: StackdriverFields,
            log_span: false,
        }
    }
}

impl<W> Stackdriver<W>
where
    W: for<'writer> MakeWriter<'writer> + 'static,
{

    fn visit<S>(&self, event: &Event, context: Context<S>) -> Result<(), Error>
    where
        S: Subscriber + for<'span> LookupSpan<'span>,
    {
        let mut buffer: Vec<u8> = Default::default();
        let meta = event.metadata();
        let mut time = String::new();

        // self.time.format_time(&mut time).map_err(|_| Error::Time)?;

        let mut serializer = serde_json::Serializer::new(&mut buffer);

        let mut map = serializer.serialize_map(None)?;

        map.serialize_entry("time", &time)?;
        map.serialize_entry("severity", &meta.level().as_serde())?;
        map.serialize_entry("logger", &meta.target())?;
        map.serialize_entry(
            "logging.googleapis.com/sourceLocation",
            &SourceLocation {
                line: meta.line(),
                file: meta.file(),
            },
        )?;

        if self.log_span {
            if let Some(span) = context.lookup_current() {
                let name = &span.name();
                let extensions = span.extensions();
                let formatted_fields = extensions
                    .get::<FormattedFields<StackdriverFields>>()
                    .expect("No fields!");

                // TODO: include serializable data type in extensions instead of str
                let mut fields: Value = serde_json::from_str(&formatted_fields)?;

                fields["name"] = serde_json::json!(name);

                map.serialize_entry("span", &fields)?;
            }
        }

        // TODO: enable deeper structuring of keys and values across tracing
        // https://github.com/tokio-rs/tracing/issues/663
        let mut visitor = StackdriverEventVisitor::new(map);

        event.record(&mut visitor);

        visitor.finish().map_err(Error::from)?;

        use std::io::Write;
        let mut writer = self.make_writer.make_writer();
        buffer.write_all(b"\n")?;
        writer.write_all(&mut buffer)?;
        Ok(())
    }
}

impl Default for Stackdriver {
    fn default() -> Self {
        Self {
            time: UtcTime::rfc_3339(),
            make_writer: std::io::stdout,
            fields: StackdriverFields,
            log_span: false,
        }
    }
}

impl<S, W> Layer<S> for Stackdriver<W>
where
    S: Subscriber + for<'span> LookupSpan<'span>,
    W: for<'writer> MakeWriter<'writer> + 'static,
{
    #[allow(unused_variables)]
    fn on_event(&self, event: &Event, context: Context<S>) {
        if let Err(error) = self.visit(event, context) {
            #[cfg(test)]
            eprintln!("{}", &error)
        }
    }
}

struct StackdriverFields;

impl<'a> MakeVisitor<&'a mut dyn Write> for StackdriverFields {
    type Visitor = StackdriverVisitor<'a>;

    #[inline]
    fn make_visitor(&self, target: &'a mut dyn Write) -> Self::Visitor {
        StackdriverVisitor::new(target)
    }
}

#[derive(Debug, thiserror::Error)]
enum Error {
    #[error("Formatting error")]
    Formatting(#[from] fmt::Error),

    #[error("Serialization error")]
    Serialization(#[from] serde_json::Error),

    #[error("Time error")]
    Time,

    #[error("IO error")]
    Io(#[from] std::io::Error),
}

#[derive(serde::Serialize)]
struct SourceLocation<'a> {
    file: Option<&'a str>,
    line: Option<u32>,
}
