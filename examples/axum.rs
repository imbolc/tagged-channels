use axum::{
    extract::{
        ws::{self, WebSocket, WebSocketUpgrade},
        Json, Query, State,
    },
    response::{
        sse::{self, Sse},
        Html, IntoResponse,
    },
    routing::{get, post},
    Router,
};
use axum_sse_manager::SseManager;
use futures::stream::Stream;
use serde::{Deserialize, Serialize};
use std::convert::Infallible;
use std::net::SocketAddr;

#[derive(Clone, Eq, Hash, PartialEq)]
enum StreamTag {
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
    let channels = SseManager::new();
    let app = Router::new()
        .route("/", get(index))
        .route("/send", post(send))
        .route("/sse", get(sse_ui))
        .route("/ws", get(ws_ui))
        .route("/sse-events", get(events))
        .route("/ws-events", get(ws_handler))
        .with_state(channels);

    let addr = SocketAddr::from(([0, 0, 0, 0], 3000));
    axum::Server::bind(&addr)
        .serve(app.into_make_service_with_connect_info::<SocketAddr>())
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

/// Handler for browser to receive SSE events
async fn events(
    State(sessions): State<SseManager<EventMessage, StreamTag>>,
    Query(ConnectionParams { user_id, is_admin }): Query<ConnectionParams>,
) -> Sse<impl Stream<Item = Result<sse::Event, Infallible>>> {
    let mut tags = Vec::new();
    if let Some(id) = user_id {
        tags.push(StreamTag::UserId(id));
    }
    if is_admin {
        tags.push(StreamTag::IsAdmin);
    }
    let stream = sessions.create_stream(tags).await;
    Sse::new(stream)
}

async fn send(
    State(sse): State<SseManager<EventMessage, StreamTag>>,
    Json(message): Json<EventMessage>,
) {
    use EventMessage::*;
    match message {
        User(data) => {
            sse.send_by_tag(&StreamTag::UserId(data.user_id), User(data))
                .await
        }
        Admin(data) => sse.send_by_tag(&StreamTag::IsAdmin, Admin(data)).await,
        Broadcast(data) => sse.broadcast(Broadcast(data)).await,
    }
}

async fn ws_handler(
    ws: WebSocketUpgrade,
    Query(ConnectionParams { user_id, is_admin }): Query<ConnectionParams>,
    State(manager): State<SseManager<EventMessage, StreamTag>>,
) -> impl IntoResponse {
    let mut tags = Vec::new();
    if let Some(id) = user_id {
        tags.push(StreamTag::UserId(id));
    }
    if is_admin {
        tags.push(StreamTag::IsAdmin);
    }
    ws.on_upgrade(move |socket| handle_socket(socket, manager, tags))
}

/// Actual websocket statemachine (one will be spawned per connection)
async fn handle_socket(
    mut socket: WebSocket,
    mut manager: SseManager<EventMessage, StreamTag>,
    tags: Vec<StreamTag>,
) {
    let mut rx = manager.create_channel(tags);
    loop {
        let Some(msg) = rx.recv().await else { break; };
        let Ok(json) = serde_json::to_string(&msg) else { continue; };
        if socket.send(ws::Message::Text(json)).await.is_err() {
            break;
        }
    }
}
