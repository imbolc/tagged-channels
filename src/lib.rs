//! # axum-sse-manager
//!
//! SSE channels manager for Axum framework

#![warn(clippy::all, missing_docs, nonstandard_style, future_incompatible)]

use axum::response::sse::Event;
use futures::stream::Stream;
use serde::Serialize;
use std::{
    collections::{HashMap, HashSet},
    convert::Infallible,
    fmt::Debug,
    hash::Hash,
    sync::{Arc, Mutex, MutexGuard, PoisonError},
};
use tokio::sync::mpsc::{self, error::SendError, Receiver, Sender};

type ChannelId = u64;

/// SSE manager
#[derive(Debug, Clone)]
pub struct SseManager<M, T>(Arc<Mutex<ManagerInner<M, T>>>)
where
    M: Clone + Debug,
    T: Clone + Eq + Hash + PartialEq + Debug;

/// Inner part of the manager
#[derive(Debug, Clone)]
pub struct ManagerInner<M, T>
where
    M: Clone,
    T: Clone + Eq + Hash + PartialEq,
{
    last_id: u64,
    channels: HashMap<ChannelId, Channel<M, T>>,
    tags: HashMap<T, HashSet<ChannelId>>,
}

#[derive(Debug, Clone)]
struct Channel<M, T>
where
    M: Clone,
    T: Clone + Eq + Hash + PartialEq,
{
    tx: Sender<M>,
    tags: Box<[T]>,
}

/// An guard to trace channels disconnection
#[derive(Debug)]
pub struct ChannelGuard<M, T>
where
    M: Clone + Debug,
    T: Clone + Eq + Hash + PartialEq + Debug,
{
    channel_id: ChannelId,
    manager: SseManager<M, T>,
}

impl<M, T> SseManager<M, T>
where
    M: Clone + Debug,
    T: Clone + Eq + Hash + PartialEq + Debug,
{
    /// Creates a new manager
    pub fn new() -> Self {
        let inner = ManagerInner {
            last_id: 0,
            channels: HashMap::new(),
            tags: HashMap::new(),
        };
        Self(Arc::new(Mutex::new(inner)))
    }

    /// Creates a new stream of SSE events
    pub async fn create_stream(
        mut self,
        tags: impl Into<Vec<T>>,
    ) -> impl Stream<Item = Result<Event, Infallible>>
    where
        M: Serialize,
    {
        let tags = tags.into();
        async_stream::stream! {
            let Ok((mut rx, _guard)) = self.create_channel(tags) else { return };
            println!("connected #{}", _guard.channel_id);
            loop {
                if let Some(msg) = rx.recv().await {
                    if let Ok(json) = serde_json::to_string(&msg) {
                        yield Ok(Event::default().data(json));
                    }
                }
            }
        }
    }

    /// Creates a new channel and returns an event receiver and a guard to clean the manager when
    /// the channel is closed
    pub fn create_channel<'a>(
        &'a mut self,
        tags: impl Into<Vec<T>>,
    ) -> Result<(Receiver<M>, ChannelGuard<M, T>), PoisonError<MutexGuard<'a, ManagerInner<M, T>>>>
    {
        let tags = tags.into();
        // TODO is there reasons for a bigger size?
        let (tx, rx) = mpsc::channel::<M>(1);
        let channel = Channel {
            tx,
            tags: tags.clone().into_boxed_slice(),
        };

        let mut inner = self.0.lock()?;
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

        Ok((
            rx,
            ChannelGuard {
                channel_id,
                manager: self.clone(),
            },
        ))
    }

    /// Removes the channel from the manager
    fn remove_channel(&mut self, channel_id: &ChannelId) {
        if let Ok(mut inner) = self.0.lock() {
            if let Some(channel) = inner.channels.remove(channel_id) {
                for tag in channel.tags.iter() {
                    inner.remove_channel_tag(channel_id, tag);
                }
            }
        }
    }

    /// Returns senders by tag
    fn sender_by_tag(&self, tag: &T) -> Vec<Sender<M>> {
        let Ok(inner) = self.0.lock() else { return Default::default() };
        let Some(channel_ids) = inner.tags.get(tag) else {return Default::default() };
        channel_ids
            .iter()
            .filter_map(|id| inner.clone_tx(id))
            .collect()
    }

    /// Returns the session `Sender`
    fn one_tx(&self, channel_id: &ChannelId) -> Option<Sender<M>> {
        self.0
            .lock()
            .ok()
            .and_then(|inner| inner.clone_tx(channel_id))
    }

    /// Returns senders for multiple sessions
    fn many_tx<'a, I>(&self, channel_ids: I) -> Vec<Sender<M>>
    where
        I: IntoIterator<Item = &'a ChannelId>,
    {
        self.0
            .lock()
            .ok()
            .map(|inner| {
                channel_ids
                    .into_iter()
                    .filter_map(|id| inner.clone_tx(id))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Returns senders for all sessions
    fn all_tx(&self) -> Vec<Sender<M>> {
        self.0
            .lock()
            .ok()
            .map(|inner| inner.channels.values().map(|c| c.tx.clone()).collect())
            .unwrap_or_default()
    }

    /// Returns all session ids
    pub fn channel_ids(&self) -> Vec<ChannelId> {
        self.0
            .lock()
            .ok()
            .map(|inner| inner.channels.keys().cloned().collect())
            .unwrap_or_default()
    }

    /// Sends the message to all tagged channels
    pub async fn send_by_tag(&self, tag: &T, msg: M) {
        for rx in self.sender_by_tag(tag) {
            rx.send(msg.clone()).await.ok();
        }
    }

    /// Sends a message to the session
    pub async fn send_to_one_channel(
        &self,
        channel_id: &ChannelId,
        msg: M,
    ) -> Result<(), SendError<M>> {
        if let Some(tx) = self.one_tx(channel_id) {
            tx.send(msg).await?;
        }
        Ok(())
    }

    /// Sends a message to the session
    pub async fn send_to_many_channels<'a, I>(&self, channel_ids: I, msg: M)
    where
        I: IntoIterator<Item = &'a ChannelId>,
        M: Clone,
    {
        for rx in self.many_tx(channel_ids) {
            rx.send(msg.clone()).await.ok();
        }
    }

    /// Send the message to everyone
    pub async fn broadcast(&self, msg: M)
    where
        M: Clone,
    {
        for rx in self.all_tx() {
            rx.send(msg.clone()).await.ok();
        }
    }
}

impl<M, T> ChannelGuard<M, T>
where
    M: Clone + Debug,
    T: Clone + Eq + Hash + PartialEq + Debug,
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
    M: Clone + Debug,
    T: Clone + Eq + Hash + PartialEq + Debug,
{
    fn drop(&mut self) {
        self.manager.remove_channel(&self.channel_id);
        println!("disconnected #{}", self.channel_id);
    }
}

impl<M, T> ManagerInner<M, T>
where
    M: Clone,
    T: Clone + Eq + Hash + PartialEq,
{
    fn clone_tx(&self, channel_id: &ChannelId) -> Option<Sender<M>> {
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
