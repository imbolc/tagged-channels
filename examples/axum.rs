use std::{convert::Infallible, net::SocketAddr};

use axum::{
    Router,
    extract::{
        Json, Query, State,
        ws::{Message as WsMessage, WebSocket, WebSocketUpgrade},
    },
    response::{
        Html, IntoResponse,
        sse::{Event as SseEvent, Sse},
    },
    routing::{get, post},
};
use futures::stream::Stream;
use serde::{Deserialize, Serialize};
use tagged_channels::TaggedChannels;

#[derive(Clone, Eq, Hash, PartialEq)]
enum ChannelTag {
    UserId(i32),
    IsAdmin,
}

#[derive(Deserialize, Serialize)]
#[serde(tag = "_type")]
enum EventMessage {
    User(UserMessage),
    Admin(SimpleMessage),
    Broadcast(SimpleMessage),
}

#[derive(Deserialize, Serialize)]
struct UserMessage {
    user_id: i32,
    message: String,
}

#[derive(Deserialize, Serialize)]
struct SimpleMessage {
    message: String,
}

#[derive(Deserialize)]
struct ConnectionParams {
    user_id: Option<i32>,
    is_admin: bool,
}

#[tokio::main]
async fn main() {
    let channels = TaggedChannels::new();
    let app = Router::new()
        .route("/", get(index))
        .route("/send", post(send))
        .route("/sse", get(sse_ui))
        .route("/ws", get(ws_ui))
        .route("/sse-events", get(events))
        .route("/ws-events", get(ws_events))
        .with_state(channels);

    let addr = SocketAddr::from(([0, 0, 0, 0], 3000));
    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await
    .unwrap();
}

async fn index() -> Html<String> {
    let page = [("WebSocket", "/ws"), ("SSE", "/sse")]
        .iter()
        .map(|(name, url)| format!(r#"<li><a href="{url}">{name} example</a></li>"#))
        .collect();
    Html(page)
}

async fn sse_ui() -> Html<String> {
    Html(include_str!("ui.html").replace("{{example}}", "sse"))
}

async fn ws_ui() -> Html<String> {
    Html(include_str!("ui.html").replace("{{example}}", "ws"))
}

async fn send(
    State(channels): State<TaggedChannels<EventMessage, ChannelTag>>,
    Json(message): Json<EventMessage>,
) {
    use EventMessage::*;
    match message {
        User(data) => {
            let tag = ChannelTag::UserId(data.user_id);
            channels.send_by_tag(&tag, User(data)).await
        }
        Admin(data) => {
            let tag = ChannelTag::IsAdmin;
            channels.send_by_tag(&tag, Admin(data)).await
        }
        Broadcast(data) => channels.broadcast(Broadcast(data)).await,
    }
}

/// Handler for browser to receive SSE events
async fn events(
    Query(params): Query<ConnectionParams>,
    State(mut channels): State<TaggedChannels<EventMessage, ChannelTag>>,
) -> Sse<impl Stream<Item = Result<SseEvent, Infallible>>> {
    let stream = async_stream::stream! {
        let mut rx = channels.create_channel(params.as_tags());
        while let Some(msg) = rx.recv().await {
            let Ok(json) = serde_json::to_string(&msg) else { continue };
            yield Ok(SseEvent::default().data(json));
        }
    };
    Sse::new(stream)
}

async fn ws_events(
    ws: WebSocketUpgrade,
    Query(params): Query<ConnectionParams>,
    State(channels): State<TaggedChannels<EventMessage, ChannelTag>>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, channels, params.as_tags()))
}

async fn handle_socket(
    mut socket: WebSocket,
    mut channels: TaggedChannels<EventMessage, ChannelTag>,
    tags: Vec<ChannelTag>,
) {
    let mut rx = channels.create_channel(tags);
    while let Some(msg) = rx.recv().await {
        let Ok(json) = serde_json::to_string(&msg) else {
            continue;
        };
        if socket.send(WsMessage::Text(json.into())).await.is_err() {
            break;
        }
    }
}

impl ConnectionParams {
    fn as_tags(&self) -> Vec<ChannelTag> {
        let mut tags = Vec::new();
        if let Some(id) = self.user_id {
            tags.push(ChannelTag::UserId(id));
        }
        if self.is_admin {
            tags.push(ChannelTag::IsAdmin);
        }
        tags
    }
}
