[![License](https://img.shields.io/crates/l/tagged-channels.svg)](https://choosealicense.com/licenses/mit/)
[![Crates.io](https://img.shields.io/crates/v/tagged-channels.svg)](https://crates.io/crates/tagged-channels)
[![Docs.rs](https://docs.rs/tagged-channels/badge.svg)](https://docs.rs/tagged-channels)

# tagged-channels

This library makes it easy to tag (WebSocket, SSE, ...) channels with e.g.
user-id and then send events to all the channels opened by a particular user.
It's framework agnostic, but for now has only an [axum example]. If you're using
it with another framework, consider PR-ing an adapted example.

## Usage

```rust,no_run
# use serde::{Deserialize, Serialize};
# use tagged_channels::TaggedChannels;
# tokio_test::block_on(async {
// We're going to tag channels
#[derive(Clone, Eq, Hash, PartialEq)]
enum Tag {
    UserId(i32),
    IsAdmin,
}

// Events we're going to send
#[derive(Deserialize, Serialize)]
#[serde(tag = "_type")]
enum Message {
    Ping,
}

// Create the manager
let mut manager = TaggedChannels::<Message, Tag>::new();

// Message to user#1
manager.send_by_tag(&Tag::UserId(1), Message::Ping).await;

// Message to all admins
manager.send_by_tag(&Tag::IsAdmin, Message::Ping).await;

// Message to everyone
manager.broadcast(Message::Ping).await;

// Connect and tag the channel as belonging to the user#1 who is an admin
let mut channel = manager.create_channel([Tag::UserId(1), Tag::IsAdmin]);

// Receive events coming from the channel
while let Some(event) = channel.recv().await {
    // send the event through WebSocket or SSE
}
# })
```

Look at the full [axum example] for detail.

## Contributing

Please run[.pre-commit.sh] before sending a PR, it will check everything.

## License

This project is licensed under the [MIT license](LICENSE).

[.pre-commit.sh]:
    https://github.com/imbolc/tagged-channels/blob/main/pre-commit.sh
[axum example]:
    https://github.com/imbolc/tagged-channels/blob/main/examples/axum.rs
