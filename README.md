[![License](https://img.shields.io/crates/l/axum-sse-manager.svg)](https://choosealicense.com/licenses/mit/)
[![Crates.io](https://img.shields.io/crates/v/axum-sse-manager.svg)](https://crates.io/crates/axum-sse-manager)
[![Docs.rs](https://docs.rs/axum-sse-manager/badge.svg)](https://docs.rs/axum-sse-manager)

<!-- cargo-sync-readme start -->

# axum-sse-manager

SSE channels manager for Axum framework

## Usage

```rust,no_run

// We're going to tag channels
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
enum Tag {
    UserId(i32),
    IsAdmin,
}

// Events we're going to send
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "_type")]
enum Message {
    Ping,
}

// Create the manager
let sse = SseManager::<Message, Tag>::new();

// Connect as an user#1, admin
let stream = sse.create_stream([Tag::UserId(1), Tag::IsAdmin]).await;


// Message to user#1
sse.send_by_tag(&Tag::UserId(1), Message::Ping).await;

// Message to all admins
sse.send_by_tag(&Tag::UserId(1), Message::Ping).await;

// Message to everyone
sse.broadcast(Message::Ping).await;
```

Look at the [full example][example] for detail.

[example]: https://github.com/imbolc/axum-sse-manager/blob/main/examples/users.rs

<!-- cargo-sync-readme end -->

## Contributing

We appreciate all kinds of contributions, thank you!


### Note on README

Most of the readme is automatically copied from the crate documentation by [cargo-sync-readme][].
This way the readme is always in sync with the docs and examples are tested.

So if you find a part of the readme you'd like to change between `<!-- cargo-sync-readme start -->`
and `<!-- cargo-sync-readme end -->` markers, don't edit `README.md` directly, but rather change
the documentation on top of `src/lib.rs` and then synchronize the readme with:
```bash
cargo sync-readme
```
(make sure the cargo command is installed):
```bash
cargo install cargo-sync-readme
```

If you have [rusty-hook] installed the changes will apply automatically on commit.


## License

This project is licensed under the [MIT license](LICENSE).

[cargo-sync-readme]: https://github.com/phaazon/cargo-sync-readme
[rusty-hook]: https://github.com/swellaby/rusty-hook
