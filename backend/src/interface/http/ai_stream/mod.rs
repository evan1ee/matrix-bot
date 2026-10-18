//! Streaming web AI chat: the event type sent to the browser and the sink
//! that turns a turn's progress into those events. The handler joins them in
//! the next commit, so for now the items are used only by tests.
#![allow(dead_code, unused_imports)] // used by the chat_stream handler in the next commit; remove then

mod events;
mod sink;

pub(crate) use events::{error_event, AiStreamEvent};
pub(crate) use sink::WebStreamSink;
