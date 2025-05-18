#![warn(clippy::all, missing_docs, nonstandard_style, future_incompatible)]
#![doc = include_str!("../README.md")]

use std::{
    collections::{HashMap, HashSet},
    hash::Hash,
    sync::Arc,
};

use parking_lot::Mutex;
use tokio::sync::mpsc::{self, Receiver, Sender};

type ChannelId = u64;

/// Channels manager
pub struct TaggedChannels<M, T>(Arc<Mutex<ChannelsInner<M, T>>>);

impl<M, T> Clone for TaggedChannels<M, T> {
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}

/// Inner part of the manager
pub struct ChannelsInner<M, T> {
    last_id: u64,
    channels: HashMap<ChannelId, Channel<M, T>>,
    tags: HashMap<T, HashSet<ChannelId>>,
}

struct Channel<M, T> {
    tx: Sender<Arc<M>>,
    tags: Box<[T]>,
}

/// A guard to trace channels disconnection
pub struct ChannelGuard<M, T>
where
    T: Clone + Eq + Hash + PartialEq,
{
    channel_id: ChannelId,
    manager: TaggedChannels<M, T>,
}

/// A wrapper around [`Receiver`] to clean up resources on `Drop`
pub struct GuardedReceiver<M, T>
where
    T: Clone + Eq + Hash + PartialEq,
{
    rx: Receiver<Arc<M>>,
    #[allow(dead_code)]
    guard: ChannelGuard<M, T>,
}

impl<M, T> TaggedChannels<M, T>
where
    T: Clone + Eq + Hash + PartialEq,
{
    /// Creates a new channels manager
    pub fn new() -> Self {
        Default::default()
    }

    /// Creates a new channel and returns it's events receiver
    pub fn create_channel(&mut self, tags: impl Into<Vec<T>>) -> GuardedReceiver<M, T> {
        let tags = tags.into();
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

    /// Sends the `message` to all channels tagged by the `tag`
    pub async fn send_by_tag(&self, tag: &T, message: M) {
        let msg = Arc::new(message);
        for rx in self.tagged_senders(tag) {
            rx.send(Arc::clone(&msg)).await.ok();
        }
    }

    /// Send the `message` to everyone
    pub async fn broadcast(&self, message: M) {
        let msg = Arc::new(message);
        for rx in self.all_senders() {
            rx.send(Arc::clone(&msg)).await.ok();
        }
    }

    /// Returns tags of all currently connected channels
    pub fn connected_tags(&self) -> Vec<T> {
        self.0.lock().tags.keys().cloned().collect()
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

    /// Returns all senders
    fn all_senders(&self) -> Vec<Sender<Arc<M>>> {
        self.0
            .lock()
            .channels
            .values()
            .map(|c| c.tx.clone())
            .collect()
    }
}

impl<M, T> Default for TaggedChannels<M, T> {
    fn default() -> Self {
        let inner = ChannelsInner {
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
    fn new(channel_id: ChannelId, manager: TaggedChannels<M, T>) -> Self {
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

impl<M, T> ChannelsInner<M, T>
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
