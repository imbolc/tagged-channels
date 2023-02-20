//! # axum-sse-manager
//!
//! SSE channels manager for Axum framework
//!
//! ## Usage
//!
//! ```rust,no_run
//! # use serde::{Deserialize, Serialize};
//! # use axum_sse_manager::SseManager;
//! # tokio_test::block_on(async {
//!
//! // We're going to tag channels
//! #[derive(Clone, Eq, Hash, PartialEq)]
//! enum Tag {
//!     UserId(i32),
//!     IsAdmin,
//! }
//!
//! // Events we're going to send
//! #[derive(Deserialize, Serialize)]
//! #[serde(tag = "_type")]
//! enum Message {
//!     Ping,
//! }
//!
//! // Create the manager
//! let sse = SseManager::<Message, Tag>::new();
//!
//! // Connect and tag the channel as belonging to the user#1 who is an admin
//! let stream = sse.create_stream([Tag::UserId(1), Tag::IsAdmin]).await;
//!
//! # let sse = axum_sse_manager::SseManager::new();
//!
//! // Message to user#1
//! sse.send_by_tag(&Tag::UserId(1), Message::Ping).await;
//!
//! // Message to all admins
//! sse.send_by_tag(&Tag::UserId(1), Message::Ping).await;
//!
//! // Message to everyone
//! sse.broadcast(Message::Ping).await;
//! # })
//! ```
//!
//! Look at the [full example][example] for detail.
//!
//! [example]: https://github.com/imbolc/axum-sse-manager/blob/main/examples/users.rs

#![warn(clippy::all, missing_docs, nonstandard_style, future_incompatible)]

use axum::response::sse::Event;
use futures::stream::Stream;
use parking_lot::Mutex;
use serde::Serialize;
use std::{
    collections::{HashMap, HashSet},
    convert::Infallible,
    hash::Hash,
    sync::Arc,
};
use tokio::sync::mpsc::{self, Receiver, Sender};

type ChannelId = u64;

/// SSE manager
pub struct SseManager<M, T>(Arc<Mutex<ManagerInner<M, T>>>);

impl<M, T> Clone for SseManager<M, T> {
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}

/// Inner part of the manager
pub struct ManagerInner<M, T> {
    last_id: u64,
    channels: HashMap<ChannelId, Channel<M, T>>,
    tags: HashMap<T, HashSet<ChannelId>>,
}

struct Channel<M, T> {
    tx: Sender<Arc<M>>,
    tags: Box<[T]>,
}

/// An guard to trace channels disconnection
pub struct ChannelGuard<M, T>
where
    T: Clone + Eq + Hash + PartialEq,
{
    channel_id: ChannelId,
    manager: SseManager<M, T>,
}

/// A wrapper around [`Receiver`] to cleanup resources on `Drop`
pub struct GuardedReceiver<M, T>
where
    T: Clone + Eq + Hash + PartialEq,
{
    rx: Receiver<Arc<M>>,
    #[allow(dead_code)]
    guard: ChannelGuard<M, T>,
}

impl<M, T> SseManager<M, T>
where
    T: Clone + Eq + Hash + PartialEq,
{
    /// Creates a new manager
    pub fn new() -> Self {
        Default::default()
    }

    /// Creates a new stream of SSE events
    pub async fn create_stream(
        mut self,
        tags: impl Into<Vec<T>>,
    ) -> impl Stream<Item = Result<Event, Infallible>>
    where
        M: Serialize,
    {
        // TODO remove serialization
        // or maybe the method itself
        // or hide it behind `sse` flag
        let tags = tags.into();
        async_stream::stream! {
            let mut rx = self.create_channel(tags);
            loop {
                if let Some(msg) = rx.recv().await {
                    if let Ok(json) = serde_json::to_string(&msg) {
                        yield Ok(Event::default().data(json));
                    }
                }
            }
        }
    }

    /// Creates a new channel and returns it's events receiver
    pub fn create_channel(&mut self, tags: impl Into<Vec<T>>) -> GuardedReceiver<M, T> {
        let tags = tags.into();
        // TODO is there reasons for a bigger size?
        let (tx, rx) = mpsc::channel::<Arc<M>>(1);
        let channel = Channel {
            tx,
            tags: tags.clone().into_boxed_slice(),
        };

        let mut inner = self.0.lock();
        let channel_id = inner.last_id.overflowing_add(1).0;
        inner.channels.insert(channel_id, channel);
        for tag in tags {
            inner
                .tags
                .entry(tag)
                .and_modify(|set| {
                    set.insert(channel_id);
                })
                .or_insert(HashSet::from([channel_id]));
        }
        inner.last_id = channel_id;

        let guard = ChannelGuard::new(channel_id, self.clone());
        GuardedReceiver { rx, guard }
    }

    /// Returns number of active channels
    pub fn num_connections(&self) -> usize {
        self.0.lock().channels.len()
    }

    /// Sends the message to all tagged channels
    pub async fn send_by_tag(&self, tag: &T, msg: M) {
        let msg = Arc::new(msg);
        for rx in self.tagged_senders(tag) {
            rx.send(Arc::clone(&msg)).await.ok();
        }
    }

    /// Send the message to everyone
    pub async fn broadcast(&self, msg: M) {
        let msg = Arc::new(msg);
        for rx in self.all_senders() {
            rx.send(Arc::clone(&msg)).await.ok();
        }
    }

    /// Removes the channel from the manager
    fn remove_channel(&mut self, channel_id: &ChannelId) {
        let mut inner = self.0.lock();
        if let Some(channel) = inner.channels.remove(channel_id) {
            for tag in channel.tags.iter() {
                inner.remove_channel_tag(channel_id, tag);
            }
        }
    }

    /// Returns senders by tag
    fn tagged_senders(&self, tag: &T) -> Vec<Sender<Arc<M>>> {
        let inner = self.0.lock();
        inner
            .tags
            .get(tag)
            .map(|ids| ids.iter().filter_map(|id| inner.clone_tx(id)).collect())
            .unwrap_or_default()
    }

    /// Returns senders for all sessions
    fn all_senders(&self) -> Vec<Sender<Arc<M>>> {
        self.0
            .lock()
            .channels
            .values()
            .map(|c| c.tx.clone())
            .collect()
    }
}

impl<M, T> Default for SseManager<M, T> {
    fn default() -> Self {
        let inner = ManagerInner {
            last_id: 0,
            channels: HashMap::new(),
            tags: HashMap::new(),
        };
        Self(Arc::new(Mutex::new(inner)))
    }
}

impl<M, T> ChannelGuard<M, T>
where
    T: Clone + Eq + Hash + PartialEq,
{
    fn new(channel_id: ChannelId, manager: SseManager<M, T>) -> Self {
        Self {
            channel_id,
            manager,
        }
    }
}

impl<M, T> Drop for ChannelGuard<M, T>
where
    T: Clone + Eq + Hash + PartialEq,
{
    fn drop(&mut self) {
        self.manager.remove_channel(&self.channel_id);
    }
}

impl<M, T> ManagerInner<M, T>
where
    T: Eq + Hash + PartialEq,
{
    fn clone_tx(&self, channel_id: &ChannelId) -> Option<Sender<Arc<M>>> {
        self.channels.get(channel_id).map(|c| c.tx.clone())
    }

    fn remove_channel_tag(&mut self, channel_id: &ChannelId, tag: &T) {
        let empty = if let Some(ids) = self.tags.get_mut(tag) {
            ids.remove(channel_id);
            ids.is_empty()
        } else {
            false
        };
        if empty {
            self.tags.remove(tag);
        }
    }
}

impl<M, T> GuardedReceiver<M, T>
where
    T: Clone + Eq + Hash + PartialEq,
{
    /// Receives the next event from the channel
    pub async fn recv(&mut self) -> Option<Arc<M>> {
        self.rx.recv().await
    }
}
